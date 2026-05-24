//! SQLite persistence backend.
//!
//! Embedded SQLite is the only persistence backend Spindle ships. Migrations
//! live under `crates/spindle-adapters/migrations/` and are compiled into the
//! binary via `refinery`.

pub mod error;
pub mod import;
pub mod import_service;
pub mod json_records;
mod pool;
mod project_resources;
pub mod records;
pub mod repository;
pub mod row;
pub mod service;
pub mod source_bridge;
pub mod validators;

pub use service::SqliteSpindleService;

pub use pool::SqlitePool;
pub use repository::Repository;

// `refinery::embed_migrations!` declares its own `pub mod migrations { ... }`
// containing a `runner()` function. Files live under
// `crates/spindle-adapters/migrations/` and are picked up at compile time.
refinery::embed_migrations!("./migrations");
