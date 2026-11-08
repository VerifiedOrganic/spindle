mod handshake;
mod http;
mod internal_listener;
mod json_utils;
mod proxy;
mod resources;
mod run_journal;
mod server;
mod tools;

#[cfg(test)]
mod sqlite_integration_tests;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use spindle_adapters::SqlitePool;
use spindle_adapters::sqlite::Repository as SpindleRepository;
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use tracing_subscriber::EnvFilter;

use crate::proxy::ProxyHandler;
use crate::server::SpindleMcpServer;
use crate::tools::run_init_grok_skills;

#[derive(Parser, Debug)]
#[command(name = "spindle-mcp", about = "Spindle MCP server + helper commands", after_help = tools::profile_help())]
struct McpCli {
    #[command(subcommand)]
    command: Option<McpCommand>,
}

#[derive(Subcommand, Debug)]
enum McpCommand {
    /// List startup profiles, enabled workflow units, and required capabilities.
    Profiles {
        #[arg(long)]
        json: bool,
    },
    /// Install the Spindle writing skill globally for Codex (~/.agents/skills/).
    #[command(name = "init-codex", alias = "init-codex-skills")]
    InitCodexSkills {
        /// Install in this book folder's .agents/skills/ instead of globally.
        #[arg(long)]
        target_dir: Option<PathBuf>,
    },
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
    /// Initialize a local workspace (.spindle/) and install the global Codex skill.
    Init,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    // Fast path: subcommands (e.g. `spindle-mcp init-grok-skills`)
    let cli = McpCli::parse();
    if let Some(cmd) = cli.command {
        match cmd {
            McpCommand::Profiles { json } => {
                tools::print_profiles(json);
                return Ok(());
            }
            McpCommand::InitCodexSkills { target_dir } => {
                print_codex_skill(&init_codex_skill(target_dir.as_deref())?);
                return Ok(());
            }
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
            McpCommand::Init => {
                let cwd = std::env::current_dir().context("getting current directory")?;
                let spindle_dir = init_project_workspace(&cwd)?;

                println!(
                    "Initialized Spindle project-local workspace at {}",
                    spindle_dir.display()
                );
                print_codex_skill(&init_codex_skill(None)?);
                return Ok(());
            }
        }
    }

    let tool_profile = match std::env::var("SPINDLE_TOOL_PROFILE") {
        Ok(profile) => Some(profile),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(anyhow::anyhow!("invalid SPINDLE_TOOL_PROFILE: {error}")),
    };
    tools::validate_tool_profile(tool_profile.as_deref())?;

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let env_data_dir = std::env::var_os("SPINDLE_DATA_DIR").map(PathBuf::from);
    let env_config = std::env::var_os("SPINDLE_CONFIG").map(PathBuf::from);
    let ws = spindle_adapters::workspace::resolve_workspace(&cwd, env_data_dir, env_config);

    let data_dir = ws.data_dir;
    let db_path = ws.db_path;
    let config_path = ws.config_path.map(|p| p.display().to_string());

    // Explicit HTTP-only mode (no stdio, no proxy).
    //
    // HTTP-mode primacy decision (live-run bug 4, HTTP-mode arm): an HTTP-mode
    // server that holds the DB ALSO claims primacy and writes the addr file.
    // Rationale: the server's own run-phase actions (mining, checkpoint
    // execution, scene verify) dispatch back to `/mcp` by reading the addr file
    // (tools.rs), NOT by reading SPINDLE_HTTP_ADDR. Without the addr file an
    // `authoring_execute_next` served over HTTP would fail its dispatch arm with
    // `no primary server found` — exactly the stdio-mode bug, one transport over.
    // Because HTTP mode's `/mcp` listener serves the identical MCP surface at a
    // known address, the addr file simply points at that same address. The addr
    // file is removed on shutdown. (The external harness still reaches the server
    // via --server-url; the addr file serves the server's own dispatch-back.)
    if let Some(addr) = http_listen_addr()? {
        return run_http_mode(&data_dir, &db_path, config_path, addr).await;
    }

    // Default: try to become primary via cross-process runtime lock, fall back
    // to secondary proxy with failover when another process already holds it.
    match try_acquire_primary_lock(&data_dir)? {
        Some(primary_lock) => {
            let db = init_sqlite(&db_path).await?;
            let service = build_service(db, &data_dir, config_path);
            run_primary(service, &data_dir, primary_lock).await
        }
        None => {
            tracing::info!("primary lock held by another process, starting in proxy mode");
            run_secondary(&data_dir, &db_path).await
        }
    }
}

async fn run_http_mode(
    data_dir: &Path,
    db_path: &Path,
    config_path: Option<String>,
    addr: SocketAddr,
) -> anyhow::Result<()> {
    run_http_mode_with_shutdown(
        data_dir,
        db_path,
        config_path,
        addr,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
}

pub(crate) async fn run_http_mode_with_shutdown(
    data_dir: &Path,
    db_path: &Path,
    config_path: Option<String>,
    addr: SocketAddr,
    shutdown: tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    let Some(_primary_lock) = try_acquire_primary_lock(data_dir)? else {
        let existing = read_addr_file(data_dir)
            .map(|existing| format!(" (active primary listener at {existing})"))
            .unwrap_or_default();
        anyhow::bail!(
            "cannot start HTTP server on {addr}: primary lock for workspace {} is already held by another process{existing}",
            data_dir.display()
        );
    };

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding HTTP listener on {addr}"))?;
    let bound_addr = listener
        .local_addr()
        .context("reading bound HTTP listener address")?;

    let db = init_sqlite(db_path).await?;
    let service = build_service(db, data_dir, config_path);
    let _addr_guard = match internal_listener::claim_addr_file(data_dir, bound_addr).await? {
        internal_listener::AddrClaim::Won => {
            tracing::info!("http mode: claimed primacy; addr file points at {bound_addr}/mcp");
            AddrFileGuard::new(data_dir.to_path_buf(), bound_addr)
        }
        internal_listener::AddrClaim::LostTo(existing) => {
            anyhow::bail!(
                "cannot start HTTP server on {bound_addr}: existing primary at {existing} already owns addr file"
            );
        }
    };

    let result = http::serve_with_listener(service, listener, shutdown).await;
    internal_listener::shutdown_lazy_listener(data_dir).await;
    result
}

/// Open or create the SQLite-backed Spindle DB at the canonical path inside
/// `data_dir`. Phase 6 replaces the SurrealDB embedded engine — same data
/// directory, different on-disk format.
async fn init_sqlite(db_path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).context("creating spindle data dir")?;
    }
    SqlitePool::open(db_path)
        .await
        .with_context(|| format!("opening SQLite DB at {}", db_path.display()))
}

pub fn build_service(
    db: SqlitePool,
    data_dir: &Path,
    config_path: Option<String>,
) -> SpindleService {
    let repository = SpindleRepository::new(db, data_dir.to_path_buf());
    let service = SpindleService::new(repository);
    if let Err(err) = service.configure_agents(spindle_core::models::ConfigureAgentsInput {
        config_path: config_path.clone(),
    }) {
        tracing::error!(
            config_path = config_path.as_deref().unwrap_or("<default>"),
            error = %err,
            "failed to load agent configuration; drafting routes will fall back to stub unless reconfigured"
        );
    }
    service
}

fn init_project_workspace(cwd: &Path) -> anyhow::Result<PathBuf> {
    let spindle_dir = cwd.join(".spindle");
    std::fs::create_dir_all(&spindle_dir).context("creating .spindle directory")?;
    std::fs::create_dir_all(spindle_dir.join("artifacts"))
        .context("creating artifacts directory")?;
    std::fs::create_dir_all(spindle_adapters::workspace::runtime_dir(&spindle_dir))
        .context("creating runtime directory")?;

    let config_path = spindle_dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, DEFAULT_LOCAL_CONFIG)
            .context("writing default config.toml")?;
    }

    Ok(spindle_dir)
}

/// Install one entry skill; load specialist workflows from MCP on demand.
/// Default to the global Codex discovery directory; an explicit book folder
/// opts into project scope. Never replace an author's customized skill.
fn init_codex_skill(target: Option<&Path>) -> anyhow::Result<PathBuf> {
    use std::io::Write;

    let root = target
        .map(Path::to_path_buf)
        .or_else(dirs::home_dir)
        .context("could not determine home directory for global Codex skills")?;
    let skill = spindle_adapters::get_skill("spindle").context("missing embedded spindle skill")?;
    let directory = root.join(".agents/skills/spindle");
    std::fs::create_dir_all(&directory).context("creating Codex skill directory")?;
    let path = directory.join("SKILL.md");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file
            .write_all(skill.markdown.as_bytes())
            .context("writing Codex skill")?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let current = std::fs::read_to_string(&path).context("reading existing Codex skill")?;
            anyhow::ensure!(
                current == skill.markdown,
                "{} already exists with different content; preserve your edits before replacing it",
                path.display()
            );
        }
        Err(error) => return Err(error).context("creating Codex skill file"),
    }
    Ok(path)
}

fn print_codex_skill(path: &Path) {
    println!("Spindle skill ready for Codex: {}", path.display());
    println!("Connect the spindle MCP server in Codex, select GPT-6 Astra, and use $spindle.");
}

const DEFAULT_LOCAL_CONFIG: &str = r#"# Spindle local agent configuration
# Documented at docs/spindle-agent-config.md

# [[agents]]
# id = "local-http"
# name = "Local HTTP model"
# provider = "openai-compatible"
# endpoint = "http://localhost:11434/v1"
# model = "mistral"
# api_key_env = "OPENAI_API_KEY"

# [[routing]]
# route = "draft"
# agent = "local-http"
"#;

/// Primary mode: owns the DB, starts the internal MCP listener for
/// dispatch-back (and secondaries), serves this session over stdio.
///
/// The internal listener is an accept LOOP that serves each connection to
/// completion and keeps accepting (bug 4b — a single internal session ending
/// never tears it down). It runs for the whole process lifetime; the addr file
/// is written on claim and removed only here on real shutdown (when this
/// process's own stdio session ends — the primary's exit path).
async fn run_primary(
    service: SpindleService,
    data_dir: &Path,
    _primary_lock: PrimaryLock,
) -> anyhow::Result<()> {
    // Serve over a duplex and bridge the real stdio ourselves so the handshake
    // shim can answer pre-`initialize` vendor probes (Antigravity sends
    // `server/discover` first) that rmcp would otherwise treat as fatal.
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let stdio_bridge = tokio::spawn(bridge_stdio(client_io));

    let result = run_primary_with_io(service, data_dir, server_io).await;

    // Drain the bridge before tearing down: a tool listing is ~500 KB on a
    // single line, and aborting here would truncate a response already handed
    // to the transport. The timeout is a backstop against a wedged client pipe.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), stdio_bridge).await;
    result
}

pub(crate) async fn run_primary_with_io<T, E, A>(
    service: SpindleService,
    data_dir: &Path,
    server_io: T,
) -> anyhow::Result<()>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleServer, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let listener = internal_listener::spawn_internal_listener(service.clone()).await?;
    let addr = listener.addr();
    let addr_guard = match internal_listener::claim_addr_file(data_dir, addr).await {
        Ok(internal_listener::AddrClaim::Won) => {
            tracing::info!("primary: internal MCP listener on {addr}");
            Some(AddrFileGuard::new(data_dir.to_path_buf(), addr))
        }
        Ok(internal_listener::AddrClaim::LostTo(existing)) => {
            tracing::warn!(
                "primary: internal MCP listener bound on {addr}, but live listener at {existing} already owns addr file"
            );
            None
        }
        Err(err) => {
            listener.shutdown();
            return Err(err);
        }
    };

    let server = SpindleMcpServer::new(service);
    let serve_result = async {
        let running = server
            .serve(server_io)
            .await
            .context("failed to start spindle mcp server")?;
        let _ = running.waiting().await;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    // Real shutdown (or early-return cleanup if `serve` failed): stop the
    // accept loop and drop the addr file (only if still owned by `addr`) so a
    // successor can claim primacy. Also tear down any listener this process
    // claimed lazily.
    listener.shutdown();
    drop(addr_guard);
    internal_listener::shutdown_lazy_listener(data_dir).await;
    serve_result
}

/// Secondary mode: proxy stdio to the primary's HTTP endpoint.
/// If the primary dies, try to promote to primary or reconnect to a new one.
async fn run_secondary(data_dir: &Path, db_path: &Path) -> anyhow::Result<()> {
    // We need to own stdin/stdout across reconnections, so serve over a
    // duplex channel and bridge the real stdio ourselves.
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let stdio_bridge = tokio::spawn(bridge_stdio(client_io));

    run_secondary_with_io(data_dir, server_io).await?;

    // If stdio bridge is still alive, attempt promotion or reconnection.
    if !stdio_bridge.is_finished() {
        tracing::info!("primary connection lost, attempting failover");
        // Small delay to let the old primary release the lock.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        if proxy::try_promote(data_dir, db_path).await? {
            // Primary lock is available — exit cleanly so the MCP client
            // restarts us as the new primary.
            return Ok(());
        }

        // Another process took the lock. Reconnect as secondary.
        // At this point we've already consumed our duplex channel, so
        // we can't transparently re-proxy. Log and exit — Claude Code
        // will restart us, and we'll reconnect to the new primary.
        tracing::warn!("could not promote; exiting for restart");
    }

    let _ = stdio_bridge.await;
    // Real shutdown: tear down any listener this secondary claimed lazily.
    internal_listener::shutdown_lazy_listener(data_dir).await;
    Ok(())
}

pub(crate) async fn run_secondary_with_io<T, E, A>(
    data_dir: &Path,
    server_io: T,
) -> anyhow::Result<()>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleServer, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let addr = proxy::wait_for_primary(data_dir).await?;
    let client = proxy::connect_to_primary(addr).await?;
    let failover_signal = tokio_util::sync::CancellationToken::new();
    let handler = ProxyHandler::new(
        client.peer().clone(),
        failover_signal.clone(),
        std::env::var("SPINDLE_TOOL_PROFILE").ok(),
    );

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

    let _ = running.waiting().await;
    Ok(())
}

/// Bidirectional bridge between real stdin/stdout and a duplex channel.
/// Runs until stdin closes or the duplex peer drops.
///
/// Client traffic passes through the handshake shim, which answers vendor
/// probes sent before `initialize` instead of letting rmcp abort the process.
pub async fn bridge_stdio(duplex: tokio::io::DuplexStream) -> anyhow::Result<()> {
    handshake::bridge_with_shim(tokio::io::stdin(), tokio::io::stdout(), duplex).await
}

// ── Primary election & Addr file helpers ────────────────────────────────────

/// Cross-process primary election guard backed by an advisory OS file lock on
/// `.spindle/runtime/spindle.lock`.
///
/// The lock is held on an open file descriptor for the lifetime of the primary
/// process (or until dropped in tests) and released automatically by the OS if
/// the process exits or crashes. The lock file itself is intentionally never
/// unlinked on drop so concurrent processes always contend on the same inode.
pub struct PrimaryLock {
    file: std::fs::File,
}

impl Drop for PrimaryLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn primary_lock_path(data_dir: &Path) -> PathBuf {
    spindle_adapters::workspace::runtime_dir(data_dir).join("spindle.lock")
}

/// Attempt a non-blocking acquisition of the cross-process primary lock for
/// `data_dir`. Returns `Ok(Some(lock))` if this caller won election, or
/// `Ok(None)` if another process currently holds the lock.
pub fn try_acquire_primary_lock(data_dir: &Path) -> anyhow::Result<Option<PrimaryLock>> {
    let lock_path = primary_lock_path(data_dir);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create runtime directory")?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("opening primary lock file at {}", lock_path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(PrimaryLock { file })),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(err)) => Err(err)
            .with_context(|| format!("locking primary lock file at {}", lock_path.display())),
    }
}

pub(crate) struct AddrFileLockGuard {
    file: std::fs::File,
}

impl Drop for AddrFileLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn acquire_addr_file_lock(data_dir: &Path) -> Option<AddrFileLockGuard> {
    let lock_path = spindle_adapters::workspace::runtime_dir(data_dir).join("spindle.addr.lock");
    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .ok()?;
    file.lock().ok()?;
    Some(AddrFileLockGuard { file })
}

/// RAII guard for `.spindle/runtime/spindle.addr` that removes the addr file on
/// drop ONLY if it still names `self.addr`.
pub struct AddrFileGuard {
    data_dir: PathBuf,
    addr: SocketAddr,
}

impl AddrFileGuard {
    pub fn new(data_dir: PathBuf, addr: SocketAddr) -> Self {
        Self { data_dir, addr }
    }
}

impl Drop for AddrFileGuard {
    fn drop(&mut self) {
        remove_addr_file_if_owner(&self.data_dir, self.addr);
    }
}

pub(crate) fn addr_file_path(data_dir: &Path) -> PathBuf {
    spindle_adapters::workspace::runtime_dir(data_dir).join("spindle.addr")
}

pub fn write_addr_file(data_dir: &Path, addr: SocketAddr) -> anyhow::Result<()> {
    let _lock = acquire_addr_file_lock(data_dir);
    write_addr_file_unlocked(data_dir, addr)
}

pub(crate) fn write_addr_file_unlocked(data_dir: &Path, addr: SocketAddr) -> anyhow::Result<()> {
    let path = addr_file_path(data_dir);
    let parent = path
        .parent()
        .context("addr file path must have a parent directory")?;
    std::fs::create_dir_all(parent).context("failed to create runtime directory")?;

    static NEXT_TMP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let seq = NEXT_TMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp_path = parent.join(format!("spindle.addr.tmp.{}.{seq}", std::process::id()));
    std::fs::write(&tmp_path, addr.to_string()).context("failed to write temporary addr file")?;
    if let Err(err) = std::fs::rename(&tmp_path, &path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err).context("failed to atomically replace addr file");
    }
    Ok(())
}

pub fn read_addr_file(data_dir: &Path) -> anyhow::Result<SocketAddr> {
    let content = std::fs::read_to_string(addr_file_path(data_dir))
        .context("no primary server found (missing addr file)")?;
    content.trim().parse().context("failed to parse addr file")
}

/// Remove the addr file only if it currently points at `expected_addr`.
/// Returns `true` if the file was owned by `expected_addr` and removed.
pub fn remove_addr_file_if_owner(data_dir: &Path, expected_addr: SocketAddr) -> bool {
    let _lock = acquire_addr_file_lock(data_dir);
    if read_addr_file(data_dir).ok() == Some(expected_addr) {
        let _ = std::fs::remove_file(addr_file_path(data_dir));
        true
    } else {
        false
    }
}

pub fn remove_addr_file(data_dir: &Path) {
    let _lock = acquire_addr_file_lock(data_dir);
    let _ = std::fs::remove_file(addr_file_path(data_dir));
}

pub fn is_lock_error(error: &anyhow::Error) -> bool {
    let msg = format!("{error:?}");
    msg.contains("already locked by another process")
        || msg.contains("primary lock held by another process")
}

// ── Config helpers ──────────────────────────────────────────────────────────

fn http_listen_addr() -> anyhow::Result<Option<std::net::SocketAddr>> {
    let Some(raw) = std::env::var_os("SPINDLE_HTTP_ADDR") else {
        return Ok(None);
    };
    let parsed: std::net::SocketAddr = raw
        .to_string_lossy()
        .parse()
        .context("failed to parse SPINDLE_HTTP_ADDR as socket address")?;
    if !parsed.ip().is_loopback()
        && std::env::var("SPINDLE_HTTP_ALLOW_NONLOOPBACK").as_deref() != Ok("1")
    {
        anyhow::bail!(
            "refusing to bind non-loopback address {parsed} (set SPINDLE_HTTP_ALLOW_NONLOOPBACK=1 to override)"
        );
    }
    Ok(Some(parsed))
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
fn default_data_dir() -> PathBuf {
    spindle_adapters::workspace::default_data_dir()
}

#[cfg(test)]
fn configured_agent_config_path() -> Option<String> {
    std::env::var("SPINDLE_CONFIG").ok().or_else(|| {
        spindle_adapters::workspace::default_config_path().map(|path| path.display().to_string())
    })
}

#[cfg(test)]
fn default_platform_data_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    spindle_adapters::workspace::default_platform_data_dir_for_cwd(&cwd)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Serializes tests that mutate process env. Every test that calls
    /// `set_var`/`remove_var` must hold this lock for its whole body —
    /// otherwise parallel test threads clobber each other's variables.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap()
    }

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
        let _guard = env_lock();
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
        let _guard = env_lock();
        unsafe {
            std::env::remove_var("SPINDLE_HTTP_ADDR");
        }
        assert!(http_listen_addr().expect("parse addr").is_none());
    }

    #[test]
    fn http_addr_loopback_bind_is_allowed() {
        let _guard = env_lock();
        unsafe {
            std::env::set_var("SPINDLE_HTTP_ADDR", "127.0.0.1:8937");
            std::env::remove_var("SPINDLE_HTTP_ALLOW_NONLOOPBACK");
        }
        let addr = http_listen_addr().expect("loopback bind must succeed");
        unsafe {
            std::env::remove_var("SPINDLE_HTTP_ADDR");
        }
        assert_eq!(addr, Some("127.0.0.1:8937".parse().unwrap()));
    }

    #[test]
    fn http_addr_non_loopback_bind_is_rejected() {
        let _guard = env_lock();
        unsafe {
            std::env::set_var("SPINDLE_HTTP_ADDR", "0.0.0.0:8937");
            std::env::remove_var("SPINDLE_HTTP_ALLOW_NONLOOPBACK");
        }
        let err = http_listen_addr().expect_err("non-loopback bind must fail");
        unsafe {
            std::env::remove_var("SPINDLE_HTTP_ADDR");
        }
        assert!(
            err.to_string().contains("0.0.0.0:8937"),
            "error must name the address: {err:#}"
        );
    }

    #[test]
    fn http_addr_non_loopback_bind_allowed_with_override() {
        let _guard = env_lock();
        unsafe {
            std::env::set_var("SPINDLE_HTTP_ADDR", "0.0.0.0:8937");
            std::env::set_var("SPINDLE_HTTP_ALLOW_NONLOOPBACK", "1");
        }
        let addr = http_listen_addr().expect("override must allow non-loopback");
        unsafe {
            std::env::remove_var("SPINDLE_HTTP_ADDR");
            std::env::remove_var("SPINDLE_HTTP_ALLOW_NONLOOPBACK");
        }
        assert_eq!(addr, Some("0.0.0.0:8937".parse().unwrap()));
    }

    #[test]
    fn explicit_config_path_wins() {
        let _guard = env_lock();
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
        assert!(temp.path().join("runtime").join("spindle.addr").exists());
        assert!(!temp.path().join("spindle.addr").exists());
    }

    #[test]
    fn init_project_workspace_creates_local_layout() {
        let temp = tempfile::tempdir().expect("temp dir");
        let workspace = init_project_workspace(temp.path()).expect("init workspace");

        assert_eq!(workspace, temp.path().join(".spindle"));
        assert!(workspace.join("artifacts").is_dir());
        assert!(workspace.join("runtime").is_dir());
        assert!(workspace.join("config.toml").is_file());

        let config = std::fs::read_to_string(workspace.join("config.toml")).expect("read config");
        assert!(config.contains("Spindle local agent configuration"));
    }

    #[test]
    fn codex_skill_install_is_repeatable_and_preserves_existing_work() {
        let temp = tempfile::tempdir().expect("temp dir");
        let book = temp.path().join("book with spaces");
        let workspace = init_project_workspace(&book).expect("workspace");
        let config_path = workspace.join("config.toml");
        std::fs::write(&config_path, "# author's existing routes\n").unwrap();

        let path = init_codex_skill(Some(&book)).expect("install");
        assert_eq!(path, book.join(".agents/skills/spindle/SKILL.md"));
        // Keep only one discovery entry; specialist prompts stay available over MCP.
        assert_eq!(
            std::fs::read_dir(book.join(".agents/skills"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            spindle_adapters::get_skill("spindle").unwrap().markdown
        );
        assert_eq!(init_codex_skill(Some(&book)).expect("repeat"), path);
        std::fs::write(&path, "author's customized skill").unwrap();
        assert!(init_codex_skill(Some(&book)).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "author's customized skill"
        );
        assert_eq!(
            std::fs::read_to_string(config_path).unwrap(),
            "# author's existing routes\n"
        );

        let empty_book = temp.path().join("another book");
        init_codex_skill(Some(&empty_book)).expect("skills only");
        assert!(!empty_book.join(".spindle").exists());
        assert!(!empty_book.join(".codex").exists());
    }

    #[test]
    fn codex_init_defaults_to_global_and_keeps_skills_alias() {
        for command in ["init-codex", "init-codex-skills"] {
            let cli = McpCli::try_parse_from(["spindle-mcp", command]).unwrap();
            assert!(matches!(
                cli.command,
                Some(McpCommand::InitCodexSkills { target_dir: None })
            ));
        }
        let cli =
            McpCli::try_parse_from(["spindle-mcp", "init-codex", "--target-dir", "/tmp/book"])
                .unwrap();
        assert!(
            matches!(cli.command, Some(McpCommand::InitCodexSkills { target_dir: Some(path) }) if path == Path::new("/tmp/book"))
        );
    }

    #[tokio::test]
    async fn primary_election_lock_allows_only_one_holder_and_releases_on_drop() {
        let temp = tempfile::tempdir().expect("temp dir");
        let data_dir = temp.path().join(".spindle");
        let db_path = data_dir.join("spindle.db");

        let first = try_acquire_primary_lock(&data_dir)
            .expect("first lock attempt")
            .expect("first caller must win primary election");
        assert!(
            primary_lock_path(&data_dir).exists(),
            "runtime/spindle.lock should exist once acquired"
        );

        let second = try_acquire_primary_lock(&data_dir).expect("second lock attempt");
        assert!(
            second.is_none(),
            "concurrent caller on the same data_dir must not become primary"
        );
        assert!(
            !proxy::try_promote(&data_dir, &db_path)
                .await
                .expect("try_promote while locked"),
            "try_promote must return false while primary lock is held"
        );

        drop(first);

        assert!(
            proxy::try_promote(&data_dir, &db_path)
                .await
                .expect("try_promote after primary lock drop"),
            "try_promote must succeed once previous primary releases the lock"
        );
        let third = try_acquire_primary_lock(&data_dir)
            .expect("third lock attempt")
            .expect("successor must acquire primary lock after release");
        drop(third);
    }

    #[tokio::test]
    async fn addr_file_only_removed_by_owner_and_cleaned_up_on_early_return() {
        let temp = tempfile::tempdir().expect("temp dir");
        let data_dir = temp.path().join(".spindle");
        let owner_a: SocketAddr = "127.0.0.1:41001".parse().unwrap();
        let owner_b: SocketAddr = "127.0.0.1:41002".parse().unwrap();

        write_addr_file(&data_dir, owner_a).expect("write owner_a");
        let guard_a = AddrFileGuard::new(data_dir.clone(), owner_a);

        // Simulate owner_b replacing the addr file; dropping guard_a must NOT
        // delete owner_b's addr file.
        write_addr_file(&data_dir, owner_b).expect("write owner_b");
        drop(guard_a);
        assert_eq!(
            read_addr_file(&data_dir).expect("owner_b addr must remain"),
            owner_b
        );

        // Removing with wrong owner is a no-op; removing with matching owner deletes it.
        assert!(!remove_addr_file_if_owner(&data_dir, owner_a));
        assert!(addr_file_path(&data_dir).exists());
        assert!(remove_addr_file_if_owner(&data_dir, owner_b));
        assert!(!addr_file_path(&data_dir).exists());

        // Early-return cleanup in run_primary_with_io: if the stdio transport
        // closes before handshake completes (`server.serve(server_io).await?`
        // returns an error), `run_primary_with_io` must remove its addr file
        // and shut down its internal listener.
        let db = init_sqlite(&data_dir.join("spindle.db"))
            .await
            .expect("init sqlite");
        let service = build_service(db, &data_dir, None);
        let (server_io, client_io) = tokio::io::duplex(1024);
        drop(client_io);

        let err = run_primary_with_io(service, &data_dir, server_io)
            .await
            .expect_err("closed client stream must fail MCP handshake");
        assert!(
            format!("{err:#}").contains("failed to start spindle mcp server"),
            "unexpected error: {err:#}"
        );
        assert!(
            !addr_file_path(&data_dir).exists(),
            "early return from run_primary must clean up spindle.addr"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn secondary_proxies_tool_calls_to_primary_over_http() {
        let temp = tempfile::tempdir().expect("temp dir");
        let data_dir = temp.path().join(".spindle");
        let db_path = data_dir.join("spindle.db");

        // Instance 1 wins primary election and starts `run_primary_with_io`.
        let primary_lock = try_acquire_primary_lock(&data_dir)
            .expect("lock attempt")
            .expect("instance 1 wins primary lock");
        let db = init_sqlite(&db_path).await.expect("init sqlite");
        let service = build_service(db, &data_dir, None);

        let (primary_server_io, primary_client_io) = tokio::io::duplex(64 * 1024);
        let primary_data_dir = data_dir.clone();
        let primary_service = service.clone();
        let primary_task = tokio::spawn(async move {
            let _lock = primary_lock;
            run_primary_with_io(primary_service, &primary_data_dir, primary_server_io).await
        });

        let primary_client = ().serve(primary_client_io).await.expect("primary client");

        // Instance 2 sees the primary lock held and starts `run_secondary_with_io`.
        assert!(
            try_acquire_primary_lock(&data_dir)
                .expect("instance 2 lock attempt")
                .is_none(),
            "instance 2 must lose primary election and run as secondary"
        );
        let (secondary_server_io, secondary_client_io) = tokio::io::duplex(64 * 1024);
        let secondary_data_dir = data_dir.clone();
        let secondary_task = tokio::spawn(async move {
            run_secondary_with_io(&secondary_data_dir, secondary_server_io).await
        });

        let secondary_client = ().serve(secondary_client_io).await.expect("secondary proxy client");
        let tools = secondary_client
            .list_all_tools()
            .await
            .expect("list tools via secondary proxy");
        assert!(
            tools.iter().any(|t| t.name == "create_project"),
            "secondary proxy must return primary tool list"
        );

        let create_result = secondary_client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("create_project").with_arguments(
                    serde_json::json!({
                        "name": "Proxied Novel",
                        "project_type": "novel",
                        "genre": "fantasy",
                        "reader_contract": {
                            "promise": "Created through secondary proxy"
                        }
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .expect("create_project via secondary proxy");
        assert_eq!(create_result.is_error, Some(false));

        let projects = service
            .list_projects()
            .await
            .expect("list projects on primary");
        assert_eq!(projects.projects.len(), 1);
        assert_eq!(projects.projects[0].name, "Proxied Novel");

        let _ = secondary_client.cancel().await;
        let _ = secondary_task.await;
        let _ = primary_client.cancel().await;
        let _ = primary_task.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_mode_requires_primary_lock_and_cooperates_with_stdio_and_http_peers() {
        let temp = tempfile::tempdir().expect("temp dir");
        let data_dir = temp.path().join(".spindle");
        let db_path = data_dir.join("spindle.db");

        // (1) While a stdio primary holds the primary lock, starting HTTP mode
        // on the same workspace must fail fast before binding or overwriting
        // the stdio primary's addr file.
        let stdio_lock = try_acquire_primary_lock(&data_dir)
            .expect("lock attempt")
            .expect("stdio primary wins lock");
        let stdio_addr: SocketAddr = "127.0.0.1:42001".parse().unwrap();
        write_addr_file(&data_dir, stdio_addr).expect("write stdio addr");

        let err = run_http_mode_with_shutdown(
            &data_dir,
            &db_path,
            None,
            "127.0.0.1:0".parse().unwrap(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect_err("HTTP mode must reject startup when stdio primary holds workspace lock");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("primary lock for workspace") && msg.contains("42001"),
            "error should report held workspace lock and active primary addr: {msg}"
        );
        assert_eq!(
            read_addr_file(&data_dir).expect("stdio addr must remain intact"),
            stdio_addr
        );
        remove_addr_file(&data_dir);
        drop(stdio_lock);

        // (2) Once the lock is free, HTTP mode claims primacy, writes its bound
        // addr to `spindle.addr`, rejects a second concurrent HTTP server on the
        // same workspace, and allows a stdio secondary to proxy through it.
        let http_shutdown = tokio_util::sync::CancellationToken::new();
        let http_data_dir = data_dir.clone();
        let http_db_path = db_path.clone();
        let http_shutdown_clone = http_shutdown.clone();
        let http_task = tokio::spawn(async move {
            run_http_mode_with_shutdown(
                &http_data_dir,
                &http_db_path,
                None,
                "127.0.0.1:0".parse().unwrap(),
                http_shutdown_clone,
            )
            .await
        });

        let active_http_addr = proxy::wait_for_primary(&data_dir)
            .await
            .expect("wait for HTTP primary to become healthy");

        let second_http_err = run_http_mode_with_shutdown(
            &data_dir,
            &db_path,
            None,
            "127.0.0.1:0".parse().unwrap(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect_err("second HTTP server on same workspace must fail primary election");
        assert!(
            format!("{second_http_err:#}").contains("primary lock for workspace"),
            "unexpected second HTTP error: {second_http_err:#}"
        );
        assert_eq!(
            read_addr_file(&data_dir).expect("first HTTP primary addr must remain intact"),
            active_http_addr
        );

        // A stdio instance started alongside the HTTP primary loses primary
        // election and proxies through the HTTP primary.
        assert!(
            try_acquire_primary_lock(&data_dir)
                .expect("stdio lock check")
                .is_none()
        );
        let (secondary_server_io, secondary_client_io) = tokio::io::duplex(64 * 1024);
        let secondary_data_dir = data_dir.clone();
        let secondary_task = tokio::spawn(async move {
            run_secondary_with_io(&secondary_data_dir, secondary_server_io).await
        });
        let secondary_client =
            ().serve(secondary_client_io)
                .await
                .expect("connect stdio secondary to HTTP primary");
        let create_result = secondary_client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("create_project").with_arguments(
                    serde_json::json!({
                        "name": "HTTP Primary Novel",
                        "project_type": "novel",
                        "genre": "fantasy",
                        "reader_contract": {
                            "promise": "Created via stdio secondary proxying to HTTP primary"
                        }
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .expect("create_project via stdio secondary to HTTP primary");
        assert_eq!(create_result.is_error, Some(false));

        let _ = secondary_client.cancel().await;
        let _ = secondary_task.await;
        http_shutdown.cancel();
        http_task
            .await
            .expect("join http task")
            .expect("http primary clean exit");
        assert!(
            !addr_file_path(&data_dir).exists(),
            "HTTP primary must remove its addr file on shutdown"
        );
    }
}
