mod http;
mod json_utils;
mod proxy;
mod resources;
mod server;
mod tools;

#[cfg(test)]
mod sqlite_integration_tests;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use dirs::data_local_dir;
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use spindle_adapters::SqlitePool;
use spindle_adapters::agent_config::resolve_config_path;
use spindle_adapters::sqlite::Repository as SpindleRepository;
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing_subscriber::EnvFilter;

use crate::proxy::ProxyHandler;
use crate::server::SpindleMcpServer;
use crate::tools::run_init_grok_skills;

#[derive(Parser, Debug)]
#[command(name = "spindle-mcp", about = "Spindle MCP server + helper commands")]
struct McpCli {
    #[command(subcommand)]
    command: Option<McpCommand>,
}

#[derive(Subcommand, Debug)]
enum McpCommand {
    /// Initialize Grok-compatible skill adapters so Spindle's bible://skills/* work
    /// as first-class skills in the Grok TUI (similar to how they appear in Claude).
    InitGrokSkills {
        /// Install into a specific directory instead of globally.
        /// Only needed if you want repo-scoped Spindle skills (uncommon).
        #[arg(long)]
        target_dir: Option<String>,

        /// Install into the user's global ~/.grok/skills/ directory (default).
        /// Strongly recommended because Spindle is database-driven.
        #[arg(long, default_value_t = true)]
        global: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    // Fast path: subcommands (e.g. `spindle-mcp init-grok-skills`)
    let cli = McpCli::parse();
    if let Some(cmd) = cli.command {
        match cmd {
            McpCommand::InitGrokSkills { target_dir, global } => {
                // If the user explicitly gave a target directory, they almost certainly want local/repo-scoped.
                let effective_global = if target_dir.is_some() { false } else { global };
                let output = run_init_grok_skills(target_dir, effective_global)?;
                println!("{}", output.message);
                println!("Target: {}", output.target_dir);
                for f in output.files_written {
                    println!("  wrote {f}");
                }
                return Ok(());
            }
        }
    }

    let data_dir = default_data_dir();

    // Explicit HTTP-only mode (no stdio, no proxy).
    if let Some(addr) = http_listen_addr()? {
        let db = init_sqlite(&data_dir).await?;
        let service = build_service(db, &data_dir);
        http::serve(service, addr).await?;
        return Ok(());
    }

    // Default: try to become primary, fall back to secondary with failover.
    match init_sqlite(&data_dir).await {
        Ok(db) => run_primary(build_service(db, &data_dir), &data_dir).await,
        Err(e) if is_lock_error(&e) => {
            tracing::info!("database locked, starting in proxy mode");
            run_secondary(&data_dir).await
        }
        Err(e) => Err(e),
    }
}

/// Open or create the SQLite-backed Spindle DB at the canonical path inside
/// `data_dir`. Phase 6 replaces the SurrealDB embedded engine — same data
/// directory, different on-disk format.
async fn init_sqlite(data_dir: &Path) -> anyhow::Result<SqlitePool> {
    std::fs::create_dir_all(data_dir).context("creating spindle data dir")?;
    let db_path = data_dir.join("spindle.sqlite");
    SqlitePool::open(&db_path)
        .await
        .with_context(|| format!("opening SQLite DB at {}", db_path.display()))
}

pub fn build_service(db: SqlitePool, data_dir: &Path) -> SpindleService {
    let repository = SpindleRepository::new(db, data_dir.to_path_buf());
    let service = SpindleService::new(repository);
    let _ = service.configure_agents(spindle_core::models::ConfigureAgentsInput {
        config_path: configured_agent_config_path(),
    });
    service
}

/// Primary mode: owns the DB, starts an HTTP listener for secondaries,
/// serves this session over stdio.
async fn run_primary(service: SpindleService, data_dir: &Path) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    write_addr_file(data_dir, addr)?;
    tracing::info!("primary: internal MCP listener on {addr}");

    let ct = tokio_util::sync::CancellationToken::new();
    let router = http::mcp_router(service.clone(), ct.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { ct.cancelled_owned().await })
            .await;
    });

    let server = SpindleMcpServer::new(service);
    let running = server
        .serve(stdio())
        .await
        .context("failed to start spindle mcp server")?;
    let _ = running.waiting().await;

    remove_addr_file(data_dir);
    Ok(())
}

/// Secondary mode: proxy stdio to the primary's HTTP endpoint.
/// If the primary dies, try to promote to primary or reconnect to a new one.
async fn run_secondary(data_dir: &Path) -> anyhow::Result<()> {
    // We need to own stdin/stdout across reconnections, so serve over a
    // duplex channel and bridge the real stdio ourselves.
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let stdio_bridge = tokio::spawn(bridge_stdio(client_io));

    // Initial connection to the primary.
    let addr = proxy::wait_for_primary(data_dir).await?;
    let client = proxy::connect_to_primary(addr).await?;
    let failover_signal = tokio_util::sync::CancellationToken::new();
    let handler = ProxyHandler::new(client.peer().clone(), failover_signal.clone());

    let running = handler
        .serve(server_io)
        .await
        .context("failed to start proxy server")?;
    let server_cancel = running.cancellation_token();
    tokio::spawn(async move {
        tokio::select! {
            _ = failover_signal.cancelled_owned() => {}
            _ = async move {
                let _ = client.waiting().await;
            } => {}
        }
        server_cancel.cancel();
    });

    // When the proxy session ends (primary died or stdio closed), the
    // running service finishes. If stdio is still open, we try to recover.
    let _ = running.waiting().await;

    // If stdio bridge is still alive, attempt promotion or reconnection.
    if !stdio_bridge.is_finished() {
        tracing::info!("primary connection lost, attempting failover");
        // Small delay to let the old primary release the lock.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        if proxy::try_promote(data_dir).await? {
            // Promoted — try_promote blocks until this session ends.
            return Ok(());
        }

        // Another process took the lock. Reconnect as secondary.
        // At this point we've already consumed our duplex channel, so
        // we can't transparently re-proxy. Log and exit — Claude Code
        // will restart us, and we'll reconnect to the new primary.
        tracing::warn!("could not promote; exiting for restart");
    }

    let _ = stdio_bridge.await;
    Ok(())
}

/// Bidirectional bridge between real stdin/stdout and a duplex channel.
/// Runs until stdin closes or the duplex peer drops.
pub async fn bridge_stdio(duplex: tokio::io::DuplexStream) -> anyhow::Result<()> {
    let (duplex_read, mut duplex_write) = tokio::io::split(duplex);
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    // stdin → duplex_write (client messages to server)
    let to_server = tokio::spawn(async move {
        let mut lines = BufReader::new(stdin).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut msg = line.into_bytes();
            msg.push(b'\n');
            if duplex_write.write_all(&msg).await.is_err() {
                break;
            }
        }
    });

    // duplex_read → stdout (server messages to client)
    let to_client = tokio::spawn(async move {
        let mut lines = BufReader::new(duplex_read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut msg = line.into_bytes();
            msg.push(b'\n');
            if stdout.write_all(&msg).await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
        }
    });

    let _ = tokio::try_join!(to_server, to_client);
    Ok(())
}

// ── Addr file helpers ───────────────────────────────────────────────────────

fn addr_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("spindle.addr")
}

pub fn write_addr_file(data_dir: &Path, addr: SocketAddr) -> anyhow::Result<()> {
    std::fs::write(addr_file_path(data_dir), addr.to_string()).context("failed to write addr file")
}

pub fn read_addr_file(data_dir: &Path) -> anyhow::Result<SocketAddr> {
    let content = std::fs::read_to_string(addr_file_path(data_dir))
        .context("no primary server found (missing addr file)")?;
    content.trim().parse().context("failed to parse addr file")
}

pub fn remove_addr_file(data_dir: &Path) {
    let _ = std::fs::remove_file(addr_file_path(data_dir));
}

pub fn is_lock_error(error: &anyhow::Error) -> bool {
    let msg = format!("{error:?}");
    msg.contains("already locked by another process")
}

// ── Config helpers ──────────────────────────────────────────────────────────

fn http_listen_addr() -> anyhow::Result<Option<std::net::SocketAddr>> {
    let Some(raw) = std::env::var_os("SPINDLE_HTTP_ADDR") else {
        return Ok(None);
    };
    let parsed = raw
        .to_string_lossy()
        .parse()
        .context("failed to parse SPINDLE_HTTP_ADDR as socket address")?;
    Ok(Some(parsed))
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn default_data_dir() -> PathBuf {
    std::env::var_os("SPINDLE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_platform_data_dir)
}

fn configured_agent_config_path() -> Option<String> {
    std::env::var("SPINDLE_CONFIG").ok().or_else(|| {
        resolve_config_path(None)
            .ok()
            .flatten()
            .map(|path| path.display().to_string())
    })
}

fn default_platform_data_dir() -> PathBuf {
    data_local_dir()
        .map(|path| path.join("spindle"))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".spindle-data")
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn default_platform_data_dir_has_stable_leaf_name() {
        assert_eq!(
            default_platform_data_dir()
                .file_name()
                .and_then(|value| value.to_str()),
            Some("spindle")
        );
    }

    #[test]
    fn explicit_env_data_dir_wins() {
        let expected = Path::new("/tmp/spindle-explicit");
        unsafe {
            std::env::set_var("SPINDLE_DATA_DIR", expected);
        }
        let actual = default_data_dir();
        unsafe {
            std::env::remove_var("SPINDLE_DATA_DIR");
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn http_addr_defaults_to_stdio_mode() {
        unsafe {
            std::env::remove_var("SPINDLE_HTTP_ADDR");
        }
        assert!(http_listen_addr().expect("parse addr").is_none());
    }

    #[test]
    fn explicit_config_path_wins() {
        unsafe {
            std::env::set_var("SPINDLE_CONFIG", "/tmp/spindle.toml");
        }
        assert_eq!(
            configured_agent_config_path(),
            Some("/tmp/spindle.toml".to_string())
        );
        unsafe {
            std::env::remove_var("SPINDLE_CONFIG");
        }
    }

    #[test]
    fn lock_error_detection() {
        let err = anyhow::anyhow!("Database at /foo/LOCK is already locked by another process");
        assert!(is_lock_error(&err));

        let err = anyhow::anyhow!("some other error");
        assert!(!is_lock_error(&err));
    }

    #[test]
    fn addr_file_roundtrip() {
        let temp = tempfile::tempdir().expect("temp dir");
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        write_addr_file(temp.path(), addr).expect("write");
        let read_back = read_addr_file(temp.path()).expect("read");
        assert_eq!(read_back, addr);
    }
}
