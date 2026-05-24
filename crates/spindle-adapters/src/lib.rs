// Phase 6: SurrealDB modules deleted. The SQLite-backed stack under
// `sqlite/` is the sole persistence layer. Public-facing helpers
// (guidance, model routing, agent config) live alongside as crate-level
// utilities consumed by both the SQLite repository/service and MCP.

pub mod agent_config;
pub mod ai;
pub mod export;
pub mod format;
pub mod guidance;
pub mod sqlite;

pub use ai::ModelRouter;
pub use guidance::{
    EmbeddedReference, get_reference, get_skill, list_references, list_skills, standards_text,
};

// SQLite-backed entry points.
pub use sqlite::SqlitePool;
pub use sqlite::repository::Repository as SqliteRepository;

/// Open (or create) a SQLite-backed Spindle database at the given path.
/// Runs all `refinery` migrations on the writer connection before returning.
pub async fn init_sqlite_db(path: &std::path::Path) -> anyhow::Result<SqlitePool> {
    SqlitePool::open(path).await
}
