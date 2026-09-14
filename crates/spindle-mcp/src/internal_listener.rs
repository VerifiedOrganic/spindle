//! The internal MCP listener + lazy primacy claiming (live-run bug 4a/4b).
//!
//! Spindle's run-phase actions (mining, checkpoint execution, scene verify)
//! dispatch back to the server's own MCP surface over HTTP: the executor reads
//! `.spindle/runtime/spindle.addr` and connects to `http://{addr}/mcp` (see
//! `tools.rs`). That addr file names the PRIMARY's internal listener.
//!
//! # The bugs this module fixes
//!
//! **4a — lazy primacy claiming.** Primacy used to be claimed only at startup,
//! in `run_primary`. A server that started before the workspace was fully
//! initialized (no `.spindle/runtime/`, DB not yet creatable) never became
//! primary and never retried, so every later dispatch failed with
//! `no primary server found`. Claiming is now ALSO lazy: at dispatch time, if
//! no live addr file exists, the dispatching process claims primacy on the spot
//! ([`ensure_primary_addr`]) — binds a listener, starts the accept loop, writes
//! the addr file — then proceeds. Startup claiming stays as the fast path.
//!
//! **4b — the listener survives session closes.** The internal listener is an
//! accept LOOP ([`axum::serve`] over a bound [`TcpListener`]): each accepted
//! connection is served to completion, then it accepts again. A single MCP
//! session ending never tears down the listener or removes the addr file. The
//! task is detached and process-lived (held by [`InternalListener`]); the addr
//! file is removed only on real shutdown (the `run_primary` exit path / signal).
//!
//! # Election / race-safety
//!
//! For the SQLite stack the DB does not hard-lock across processes (WAL +
//! `busy_timeout` let multiple processes open the same file), so the DB is NOT
//! the primacy token — the **addr file is**. The claim is made race-safe by an
//! atomic `create_new` write of the addr file: the process that creates the
//! file first wins and keeps its listener; a loser drops its just-bound
//! listener and defers to the winner (proceeding as a proxy would). If a stale
//! addr file names a DEAD listener (health check fails), it is reclaimed. The
//! claim is idempotent: a process that already holds a live addr file returns it
//! without rebinding.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;

use crate::{addr_file_path, http, read_addr_file, write_addr_file};

/// A running internal MCP listener. Holding this keeps the accept loop alive;
/// dropping it (or cancelling the token) shuts the loop down. The `run_primary`
/// startup path keeps one for the process lifetime; the lazy-claim path stashes
/// one in a process-global so a claim made mid-run outlives the tool call that
/// triggered it.
pub struct InternalListener {
    addr: SocketAddr,
    cancel: CancellationToken,
}

impl InternalListener {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Cancel the accept loop (real-shutdown path). Idempotent.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

impl Drop for InternalListener {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Bind a fresh internal listener on `127.0.0.1:0` and spawn its accept loop.
/// The loop ([`axum::serve`]) serves every accepted connection to completion
/// and keeps accepting until the returned listener's token is cancelled — a
/// single session close never stops it (bug 4b). Does NOT write the addr file;
/// the caller decides whether it won the election first.
pub async fn spawn_internal_listener(service: SpindleService) -> anyhow::Result<InternalListener> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding internal MCP listener")?;
    let addr = listener.local_addr().context("reading listener addr")?;
    let cancel = CancellationToken::new();
    let router = http::mcp_router(service, cancel.clone());
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        // axum::serve is the accept loop: it serves each connection to
        // completion and accepts the next, until graceful shutdown fires.
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
            .await;
    });
    Ok(InternalListener { addr, cancel })
}

/// Process-global holder for listeners claimed lazily at dispatch time, keyed by
/// workspace (canonical `data_dir`). Once a lazy claim wins, its listener must
/// outlive the single tool call that made it, so it is parked here for the
/// process lifetime (real shutdown drops it). Keying by workspace — rather than a
/// single global slot — is both correct (a process could serve more than one
/// workspace, and each has its own addr file / primacy) and what keeps distinct
/// workspaces from ever aliasing each other's listener.
static LAZY_LISTENERS: std::sync::OnceLock<
    Mutex<std::collections::HashMap<std::path::PathBuf, Arc<InternalListener>>>,
> = std::sync::OnceLock::new();

fn lazy_listeners()
-> &'static Mutex<std::collections::HashMap<std::path::PathBuf, Arc<InternalListener>>> {
    LAZY_LISTENERS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// The stable per-workspace key: the canonicalized `data_dir` (falls back to the
/// raw path if canonicalization fails, e.g. the dir does not exist yet).
fn workspace_key(data_dir: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf())
}

/// How long to wait for a health probe of an existing addr file before deciding
/// its listener is dead and reclaiming primacy.
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Resolve the primary's internal-listener address, claiming primacy lazily if
/// none is live (bug 4a). Returns an address a dispatcher can connect to at
/// `http://{addr}/mcp`.
///
/// Resolution order:
/// 1. If this process already parked a lazily-claimed listener, reuse it
///    (idempotent — no rebind).
/// 2. If the addr file names a LIVE listener (health check passes), use it —
///    another process (or our own startup path) is primary; we act as a proxy.
/// 3. Otherwise claim: bind a listener, then atomically try to write the addr
///    file. Winning the atomic write makes us primary; the listener is parked
///    for the process lifetime. Losing the write to a live primary means we drop
///    our listener and use the winner's addr.
pub async fn ensure_primary_addr(
    service: &SpindleService,
    data_dir: &Path,
) -> anyhow::Result<SocketAddr> {
    let key = workspace_key(data_dir);

    // (1) Already claimed in this process for THIS workspace.
    {
        let guard = lazy_listeners().lock().await;
        if let Some(existing) = guard.get(&key) {
            return Ok(existing.addr());
        }
    }

    // (2) A live primary already exists (its addr file points at a healthy
    // listener). No claim needed.
    if let Ok(addr) = read_addr_file(data_dir)
        && probe_health(addr).await
    {
        return Ok(addr);
    }

    // (3) Claim. Serialize the claim within this process so two concurrent
    // dispatches don't both bind + race the file.
    let mut guard = lazy_listeners().lock().await;
    if let Some(existing) = guard.get(&key) {
        return Ok(existing.addr());
    }
    // Re-check under the lock: another task may have made the file live.
    if let Ok(addr) = read_addr_file(data_dir)
        && probe_health(addr).await
    {
        return Ok(addr);
    }

    let listener = spawn_internal_listener(service.clone()).await?;
    let our_addr = listener.addr();

    // Atomic election: whoever creates the addr file first wins. A stale file
    // naming a dead listener is reclaimed.
    match claim_addr_file(data_dir, our_addr).await {
        AddrClaim::Won => {
            tracing::info!("lazily claimed primacy; internal MCP listener on {our_addr}");
            guard.insert(key, Arc::new(listener));
            Ok(our_addr)
        }
        AddrClaim::LostTo(winner) => {
            // Another process holds a LIVE primary. Drop our listener and defer.
            tracing::info!(
                "lazy primacy claim lost to existing primary at {winner}; proceeding as proxy"
            );
            listener.shutdown();
            Ok(winner)
        }
    }
}

enum AddrClaim {
    /// We created the addr file (or reclaimed a stale one) — we are primary.
    Won,
    /// A live primary already owns the addr file; connect to it instead.
    LostTo(SocketAddr),
}

/// Atomically try to become the addr-file owner. Uses `create_new` so exactly
/// one racer wins the create. If the file already exists, its listener is
/// health-probed: dead ⇒ reclaim (overwrite and win); live ⇒ lose to it.
///
/// `async` because the liveness probe must run on the CALLER's runtime. This
/// used to be a sync fn that drove the probe on a nested current-thread runtime
/// via `block_on`; every real caller reaches it from `async fn`, and tokio
/// panics ("Cannot start a runtime from within a runtime") when `block_on` runs
/// on a thread already driving a runtime. That panic fired on exactly the path
/// this function exists to handle — an addr file naming a dead listener — so a
/// stale addr was never reclaimed. The panic unwound through the MCP tool
/// handler without producing a response, which the client saw as a ~60s hang
/// rather than an error, deadlocking the whole authoring loop.
async fn claim_addr_file(data_dir: &Path, addr: SocketAddr) -> AddrClaim {
    let path = addr_file_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Atomic create: only one process wins the first create.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            use std::io::Write;
            if file.write_all(addr.to_string().as_bytes()).is_ok() {
                return AddrClaim::Won;
            }
            // Write failed after create; fall through to reclaim below.
        }
        Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
            // Unexpected IO error: best-effort overwrite so we still get a listener.
            let _ = write_addr_file(data_dir, addr);
            return AddrClaim::Won;
        }
        Err(_) => { /* AlreadyExists — someone owns it; check liveness below. */ }
    }

    // The file exists. Is its listener alive?
    match read_addr_file(data_dir) {
        Ok(existing) if probe_health(existing).await => AddrClaim::LostTo(existing),
        _ => {
            // Stale file (unparseable or dead listener) — reclaim it.
            let _ = write_addr_file(data_dir, addr);
            AddrClaim::Won
        }
    }
}

/// Async health probe of an internal listener's `/health`.
async fn probe_health(addr: SocketAddr) -> bool {
    reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .timeout(HEALTH_PROBE_TIMEOUT)
        .send()
        .await
        .is_ok()
}

/// Shut down a lazily-claimed listener on real process shutdown and, if this
/// process still owns the addr file it wrote, remove it so a successor can claim
/// primacy — mirroring `run_primary`'s addr-file removal for the startup-claim
/// path. No-op when this process never made a lazy claim.
pub async fn shutdown_lazy_listener(data_dir: &Path) {
    let key = workspace_key(data_dir);
    let mut guard = lazy_listeners().lock().await;
    if let Some(listener) = guard.remove(&key) {
        let ours = listener.addr();
        listener.shutdown();
        // Only remove the addr file if it still names OUR listener — never yank
        // a successor primary's file.
        if read_addr_file(data_dir).ok() == Some(ours) {
            crate::remove_addr_file(data_dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 4a: claiming the addr file is atomic and idempotent — two concurrent
    /// claims resolve to the SAME winning addr (one owner), never two files.
    ///
    /// Runs on a runtime deliberately: every production caller reaches
    /// `claim_addr_file` from `async fn`, and as a plain `#[test]` this ran on a
    /// runtime-less thread where the old nested-`block_on` probe happened to
    /// work — passing green while production panicked on the same path.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claim_addr_file_is_atomic_and_reclaims_stale() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // A dead addr (nothing listening) is reclaimable.
        let dead: SocketAddr = "127.0.0.1:1".parse().unwrap();
        write_addr_file(&data_dir, dead).unwrap();
        let ours: SocketAddr = "127.0.0.1:55001".parse().unwrap();
        assert!(
            matches!(claim_addr_file(&data_dir, ours).await, AddrClaim::Won),
            "a stale/dead addr file must be reclaimable"
        );
        assert_eq!(read_addr_file(&data_dir).unwrap(), ours);

        // A second claim over a now-owned-but-dead file also reclaims (ours:55001
        // has no listener either), staying idempotent on the winner side.
        let other: SocketAddr = "127.0.0.1:55002".parse().unwrap();
        assert!(matches!(
            claim_addr_file(&data_dir, other).await,
            AddrClaim::Won
        ));
    }
}
