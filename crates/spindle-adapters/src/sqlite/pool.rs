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
mod application;

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

impl std::fmt::Debug for SqlitePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlitePool").finish_non_exhaustive()
    }
}

struct Inner {
    db_path: std::path::PathBuf,
    writer: AsyncConnection,
    application_gate: Arc<tokio::sync::Mutex<()>>,
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

        let migration_lock = acquire_migration_lock(path).await?;

        let writer = open_connection(path, /* reader = */ false)
            .await
            .context("opening writer connection")?;

        // Migrations run synchronously on the writer thread while holding the
        // cross-process file lock so concurrent fresh starts never race
        // refinery's history check against table creation.
        writer
            .call(|conn| {
                migrations::runner()
                    .run(conn)
                    .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
                Ok(())
            })
            .await
            .context("running refinery migrations")?;

        drop(migration_lock);

        let mut readers = Vec::with_capacity(READER_COUNT);
        for _ in 0..READER_COUNT {
            let reader = open_connection(path, /* reader = */ true)
                .await
                .context("opening reader connection")?;
            readers.push(reader);
        }

        let db_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        Ok(Self {
            inner: Arc::new(Inner {
                db_path,
                writer,
                application_gate: Arc::new(tokio::sync::Mutex::new(())),
                readers,
                next_reader: AtomicUsize::new(0),
            }),
        })
    }

    /// Database file path this pool was opened against.
    pub fn db_path(&self) -> &Path {
        &self.inner.db_path
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
        let application = self.in_application_transaction();
        let _gate = if application {
            None
        } else {
            Some(self.inner.application_gate.lock().await)
        };
        self.inner
            .writer
            .call(move |conn| {
                if application {
                    application::require_open(conn)?;
                }
                f(conn).map_err(tokio_rusqlite::Error::from)
            })
            .await
            .with_sql_context("writer call")
    }

    /// Run a closure with read-only access. Round-robin over the reader pool;
    /// `query_only = 1` is set on every reader connection so writes attempted
    /// here will error rather than silently mutate. The internal application
    /// unit of work pins readbacks to its writer for read-your-writes semantics.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn read<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        if self.in_application_transaction() {
            return self
                .inner
                .writer
                .call(move |conn| {
                    application::require_open(conn)?;
                    f(conn).map_err(tokio_rusqlite::Error::from)
                })
                .await
                .with_sql_context("transaction readback");
        }
        let idx = self.inner.next_reader.fetch_add(1, Ordering::Relaxed) % self.inner.readers.len();
        self.inner.readers[idx]
            .call(move |conn| f(conn).map_err(tokio_rusqlite::Error::from))
            .await
            .with_sql_context("reader call")
    }
}

struct MigrationLockGuard {
    file: std::fs::File,
}

impl Drop for MigrationLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn migration_lock_path(db_path: &Path) -> std::path::PathBuf {
    let mut os = db_path.as_os_str().to_os_string();
    os.push(".migration.lock");
    std::path::PathBuf::from(os)
}

async fn acquire_migration_lock(db_path: &Path) -> Result<Option<MigrationLockGuard>> {
    if db_path.as_os_str().is_empty() || db_path == Path::new(":memory:") {
        return Ok(None);
    }
    let lock_path = migration_lock_path(db_path);
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = lock_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating parent directory for {}", lock_path.display())
            })?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("opening migration lock file {}", lock_path.display()))?;
        file.lock()
            .with_context(|| format!("acquiring migration lock {}", lock_path.display()))?;
        Ok(Some(MigrationLockGuard { file }))
    })
    .await
    .context("joining migration lock task")?
}

async fn open_connection(path: &Path, reader: bool) -> Result<AsyncConnection> {
    let conn = AsyncConnection::open(path).await?;
    conn.call(move |c| apply_pragmas(c, reader).map_err(tokio_rusqlite::Error::from))
        .await?;
    Ok(conn)
}

fn apply_pragmas(conn: &mut Connection, reader: bool) -> rusqlite::Result<()> {
    // Set busy_timeout first so concurrent connections opening WAL mode wait
    // instead of failing immediately with SQLITE_BUSY.
    conn.execute_batch(
        "PRAGMA busy_timeout       = 5000;\n\
         PRAGMA journal_mode       = WAL;\n\
         PRAGMA synchronous        = NORMAL;\n\
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_fresh_starts_serialize_migrations_without_race() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("race.db");
        let barrier = Arc::new(tokio::sync::Barrier::new(4));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let db_path = path.clone();
            let b = barrier.clone();
            handles.push(tokio::spawn(async move {
                b.wait().await;
                SqlitePool::open(&db_path).await
            }));
        }

        let mut pools = Vec::with_capacity(4);
        for handle in handles {
            let pool = handle.await.expect("join opener task").expect(
                "concurrent SqlitePool::open must not fail with table project already exists",
            );
            pools.push(pool);
        }

        // All 4 pools can read the migrated schema.
        for pool in &pools {
            let count: i64 = pool
                .read(|conn| conn.query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0)))
                .await
                .expect("query project table");
            assert_eq!(count, 0);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn migration_lock_blocks_until_external_holder_releases() {
        let tmp = TempDir::new().expect("tempdir");
        let db_path = tmp.path().join("locked.db");
        let lock_path = migration_lock_path(&db_path);

        let external_file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .expect("open lock file");
        external_file.lock().expect("acquire external lock");

        let db_path_clone = db_path.clone();
        let mut open_task = tokio::spawn(async move { SqlitePool::open(&db_path_clone).await });

        // While the external lock is held, SqlitePool::open must block.
        let blocked =
            tokio::time::timeout(std::time::Duration::from_millis(150), &mut open_task).await;
        assert!(
            blocked.is_err(),
            "SqlitePool::open must wait while migration lock is held"
        );

        external_file.unlock().expect("release external lock");
        drop(external_file);

        let pool = tokio::time::timeout(std::time::Duration::from_secs(30), open_task)
            .await
            .expect("SqlitePool::open should finish once lock is released")
            .expect("join open task")
            .expect("open pool");
        let count: i64 = pool
            .read(|conn| conn.query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0)))
            .await
            .expect("query project");
        assert_eq!(count, 0);
    }
}
