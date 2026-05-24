//! Error helpers for the SQLite backend.
//!
//! The existing repository surface is `anyhow::Result`-based, so we do not
//! introduce a typed error enum here. Instead this module offers small
//! conversion helpers that attach actionable context to the two driver-level
//! error sources we will encounter: [`rusqlite::Error`] and
//! [`tokio_rusqlite::Error`].
//!
//! These helpers are intentionally minimal in Phase 0 and will grow as the
//! pool and repository land in Phases 2–4.

use anyhow::Context;

/// Extension trait to attach a SQL-aware context message to a rusqlite result.
///
/// Use at the boundary where a prepared statement is executed so that the
/// query identifier surfaces in error chains:
///
/// ```ignore
/// stmt.query_row([&id], Character::try_from)
///     .with_sql_context("character::get_by_id")?;
/// ```
pub trait SqlResultExt<T> {
    fn with_sql_context(self, op: &'static str) -> anyhow::Result<T>;
}

impl<T> SqlResultExt<T> for Result<T, rusqlite::Error> {
    fn with_sql_context(self, op: &'static str) -> anyhow::Result<T> {
        self.with_context(|| format!("sqlite operation `{op}` failed"))
    }
}

impl<T> SqlResultExt<T> for Result<T, tokio_rusqlite::Error> {
    fn with_sql_context(self, op: &'static str) -> anyhow::Result<T> {
        self.with_context(|| format!("sqlite (async) operation `{op}` failed"))
    }
}
