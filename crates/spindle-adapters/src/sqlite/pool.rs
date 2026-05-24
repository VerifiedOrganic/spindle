//! SQLite connection pool: one writer, N readers, WAL.
//!
//! Writes serialize through `self.writer` (a single dedicated DB thread under
//! `tokio_rusqlite::Connection`), eliminating the `SQLITE_BUSY` error class
//! entirely. Reads dispatch round-robin across `self.readers`, which are
//! independent `tokio_rusqlite::Connection`s set to `query_only = 1` for
//! defense in depth. Concurrent reads are safe under WAL.
//!
//! Pragmas (WAL, NORMAL sync, foreign keys, 64 MB cache, 256 MB mmap) are
//! applied to every connection at open time. The `sqlite-vec` extension is
//! registered once per process via `sqlite3_auto_extension`, so every newly
//! opened connection gets `vec0` automatically.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio_rusqlite::Connection as AsyncConnection;

use super::error::SqlResultExt;
use super::migrations;

/// Number of dedicated reader connections. SQLite serializes writes regardless,
/// so this bounds *reader* concurrency only. Four matches a typical desktop CLI
/// workload; Phase 7 may tune this against observed read profile.
const READER_COUNT: usize = 4;

/// Embedded SQLite pool: one writer, `READER_COUNT` readers, all WAL.
///
/// Cloning is cheap (Arc bump) and a clone shares the underlying connections.
#[derive(Clone)]
pub struct SqlitePool {
    inner: Arc<Inner>,
}

struct Inner {
    writer: AsyncConnection,
    readers: Vec<AsyncConnection>,
    next_reader: AtomicUsize,
}

impl SqlitePool {
    /// Open the database file at `path`, apply pragmas, run any pending
    /// `refinery` migrations on the writer, then open the reader connections.
    ///
    /// If `path` does not exist, SQLite creates it.
    pub async fn open(path: &Path) -> Result<Self> {
        register_sqlite_vec_once();

        let writer = open_connection(path, /* reader = */ false)
            .await
            .context("opening writer connection")?;

        // Migrations run synchronously on the writer thread.
        writer
            .call(|conn| {
                migrations::runner()
                    .run(conn)
                    .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                Ok(())
            })
            .await
            .context("running refinery migrations")?;

        let mut readers = Vec::with_capacity(READER_COUNT);
        for _ in 0..READER_COUNT {
            let reader = open_connection(path, /* reader = */ true)
                .await
                .context("opening reader connection")?;
            readers.push(reader);
        }

        Ok(Self {
            inner: Arc::new(Inner {
                writer,
                readers,
                next_reader: AtomicUsize::new(0),
            }),
        })
    }

    /// Run a closure with exclusive write access to the database. Closures
    /// execute on the writer connection's dedicated thread; the caller's
    /// future awaits completion.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn write<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        self.inner
            .writer
            .call(move |conn| f(conn).map_err(tokio_rusqlite::Error::from))
            .await
            .with_sql_context("writer call")
    }

    /// Run a closure with read-only access. Round-robin over the reader pool;
    /// `query_only = 1` is set on every reader connection so writes attempted
    /// here will error rather than silently mutate.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn read<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let idx = self.inner.next_reader.fetch_add(1, Ordering::Relaxed) % self.inner.readers.len();
        self.inner.readers[idx]
            .call(move |conn| f(conn).map_err(tokio_rusqlite::Error::from))
            .await
            .with_sql_context("reader call")
    }
}

async fn open_connection(path: &Path, reader: bool) -> Result<AsyncConnection> {
    let conn = AsyncConnection::open(path).await?;
    conn.call(move |c| apply_pragmas(c, reader).map_err(tokio_rusqlite::Error::from))
        .await?;
    Ok(conn)
}

fn apply_pragmas(conn: &mut Connection, reader: bool) -> rusqlite::Result<()> {
    // journal_mode returns the resulting mode; execute_batch is fine here
    // since we don't need the return value.
    conn.execute_batch(
        "PRAGMA journal_mode       = WAL;\n\
         PRAGMA synchronous        = NORMAL;\n\
         PRAGMA busy_timeout       = 5000;\n\
         PRAGMA cache_size         = -65536;\n\
         PRAGMA mmap_size          = 268435456;\n\
         PRAGMA temp_store         = MEMORY;\n\
         PRAGMA foreign_keys       = ON;\n\
         PRAGMA wal_autocheckpoint = 1000;",
    )?;
    if reader {
        conn.execute_batch("PRAGMA query_only = 1;")?;
    }
    Ok(())
}

/// Register `sqlite-vec` as a SQLite auto-extension exactly once per process.
///
/// `sqlite3_auto_extension` installs an entry-point hook that runs on every
/// subsequent connection open, registering the `vec0` virtual-table module
/// and the `vec_*` functions. Connections opened *before* this call do not
/// get the extension, so this must run before any pool connection opens —
/// which is why `SqlitePool::open` calls it first thing.
fn register_sqlite_vec_once() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| unsafe {
        type SqliteExtensionInit = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::ffi::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::ffi::c_int;
        let init = sqlite_vec::sqlite3_vec_init as *const ();
        let init = std::mem::transmute::<*const (), SqliteExtensionInit>(init);
        rusqlite::ffi::sqlite3_auto_extension(Some(init));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Phase 2 exit criterion: a minimal test opens the pool, runs the
    /// migration, and round-trips a row.
    #[tokio::test]
    async fn pool_opens_runs_migrations_and_round_trips() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("spindle.db");
        let pool = SqlitePool::open(&path).await.expect("open pool");

        // Round-trip a project row through writer.
        pool.write(|conn| {
            conn.execute(
                "INSERT INTO project (id, name, project_type, genre, reader_contract, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    "project:01HJX0",
                    "smoke",
                    "novel",
                    "fantasy",
                    "{}",
                    1_000_000_i64,
                    1_000_000_i64,
                ],
            )?;
            Ok(())
        })
        .await
        .expect("insert");

        let name: String = pool
            .read(|conn| {
                conn.query_row(
                    "SELECT name FROM project WHERE id = ?1",
                    rusqlite::params!["project:01HJX0"],
                    |row| row.get(0),
                )
            })
            .await
            .expect("read");
        assert_eq!(name, "smoke");

        // Verify sqlite-vec is loaded by calling one of its functions.
        let version: String = pool
            .read(|conn| conn.query_row("SELECT vec_version()", [], |r| r.get(0)))
            .await
            .expect("vec_version");
        assert!(!version.is_empty(), "vec_version returned empty");

        // Verify reader pragmas (query_only=1): writes from a reader should fail.
        let write_via_reader = pool
            .read(|conn| {
                conn.execute(
                    "INSERT INTO project (id, name, project_type, genre, reader_contract, created_at, updated_at) \
                     VALUES ('project:NOPE', 'x', 'novel', 'fantasy', '{}', 1, 1)",
                    [],
                )
            })
            .await;
        assert!(
            write_via_reader.is_err(),
            "writes through reader must fail (query_only=1)"
        );

        // Verify FK cascade by deleting the project and asserting bible_branch goes too.
        pool.write(|conn| {
            conn.execute(
                "INSERT INTO bible_branch (id, project_id, name, status, created_at) \
                 VALUES ('bible_branch:01HJX0', 'project:01HJX0', 'main', 'active', 1)",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("insert branch");

        pool.write(|conn| {
            conn.execute("DELETE FROM project WHERE id = 'project:01HJX0'", [])?;
            Ok(())
        })
        .await
        .expect("delete project");

        let branches: i64 = pool
            .read(|conn| conn.query_row("SELECT COUNT(*) FROM bible_branch", [], |r| r.get(0)))
            .await
            .expect("count branches");
        assert_eq!(branches, 0, "cascade should have removed the branch");
    }
}
