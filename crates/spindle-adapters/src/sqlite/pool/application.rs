//! Pinned application transactions over existing async repository calls.
//! Only the owning task bypasses the writer gate; reads in that task use the
//! writer so service readbacks see their own uncommitted writes.
use super::*;
use futures_util::FutureExt;
use std::future::Future;
use tokio::sync::OwnedMutexGuard;

tokio::task_local! { static APPLICATION_POOL: usize; }

struct Lease {
    pool: SqlitePool,
    gate: Option<OwnedMutexGuard<()>>,
    finished: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let gate = self.gate.take();
        // tokio-rusqlite::call sends synchronously on its first poll, before
        // awaiting the response. Enqueue now rather than relying on a spawned
        // Tokio task to run during cancellation/runtime shutdown. The SQLite
        // worker owns the gate until rollback completes, even if the response
        // receiver is dropped.
        let queued = self
            .pool
            .inner
            .writer
            .call(move |conn| {
                let _gate = gate;
                if !conn.is_autocommit()
                    && let Err(error) = conn.execute_batch("ROLLBACK")
                {
                    tracing::error!(%error,"application transaction rollback failed");
                    return Err(tokio_rusqlite::Error::from(error));
                }
                Ok(())
            })
            .now_or_never();
        if let Some(Err(error)) = queued {
            tracing::error!(%error,"could not queue application rollback");
        }
    }
}
impl SqlitePool {
    fn identity(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }
    pub(crate) fn in_application_transaction(&self) -> bool {
        APPLICATION_POOL
            .try_with(|id| *id == self.identity())
            .unwrap_or(false)
    }
    /// `commit=false` runs the exact service path as a rollback-only preflight.
    pub(crate) async fn application_transaction<T, F, Fut>(
        &self,
        commit: bool,
        action: F,
    ) -> Result<T>
    where
        T: Send,
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
    {
        anyhow::ensure!(
            !self.in_application_transaction(),
            "nested application transaction"
        );
        let gate = self.inner.application_gate.clone().lock_owned().await;
        let mut lease = Lease {
            pool: self.clone(),
            gate: Some(gate),
            finished: false,
        };
        self.inner
            .writer
            .call(|conn| {
                conn.execute_batch("BEGIN IMMEDIATE")?;
                Ok(())
            })
            .await
            .context("beginning application transaction")?;
        let result = APPLICATION_POOL.scope(self.identity(), action()).await;
        let statement = if commit && result.is_ok() {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        self.inner
            .writer
            .call(move |conn| {
                if statement != "ROLLBACK" || !conn.is_autocommit() {
                    conn.execute_batch(statement)?;
                }
                Ok(())
            })
            .await
            .context("finishing application transaction")?;
        lease.finished = true;
        result
    }
}

pub(super) fn require_open(conn: &Connection) -> rusqlite::Result<()> {
    if conn.is_autocommit() {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ABORT),
            Some("application transaction is no longer active".into()),
        ));
    }
    Ok(())
}
