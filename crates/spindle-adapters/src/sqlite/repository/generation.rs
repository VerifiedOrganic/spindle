use super::*;

impl Repository {
    // ── Generation receipts (migration V0032, live-run bug 4c) ───────────────
    //
    // Persisted so a `generation_id` issued in one process survives a primary
    // restart. `insert_generation_receipt` writes the row; `get_generation_receipt`
    // reads it back with expiry enforced ON READ and expired rows swept lazily
    // (no background task). Timestamps are unix microseconds (house convention).

    /// Persist one generation receipt. Idempotent on `id` via UPSERT so a
    /// re-register of the same (deterministic) id is harmless.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_generation_receipt(
        &self,
        id: &str,
        project_id: Option<&str>,
        branch_id: Option<&str>,
        route: &str,
        agent_id: &str,
        rating: Option<&str>,
        explicit_capable: bool,
        output_sha256: &str,
        output_text: &str,
        created_at: chrono::DateTime<chrono::Utc>,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let id = id.to_string();
        let project_id = project_id.map(str::to_string);
        let branch_id = branch_id.map(str::to_string);
        let route = route.to_string();
        let agent_id = agent_id.to_string();
        let rating = rating.map(str::to_string);
        let output_sha256 = output_sha256.to_string();
        let output_text = output_text.to_string();
        let created = timestamp_to_micros(created_at);
        let expires = timestamp_to_micros(expires_at);
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO generation_receipt \
                     (id, project_id, branch_id, route, agent_id, rating, \
                      explicit_capable, output_sha256, output_text, created_at, expires_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                     ON CONFLICT(id) DO UPDATE SET \
                       project_id = excluded.project_id, \
                       branch_id = excluded.branch_id, \
                       route = excluded.route, \
                       agent_id = excluded.agent_id, \
                       rating = excluded.rating, \
                       explicit_capable = excluded.explicit_capable, \
                       output_sha256 = excluded.output_sha256, \
                       output_text = excluded.output_text, \
                       created_at = excluded.created_at, \
                       expires_at = excluded.expires_at",
                    rusqlite::params![
                        id,
                        project_id,
                        branch_id,
                        route,
                        agent_id,
                        rating,
                        explicit_capable as i64,
                        output_sha256,
                        output_text,
                        created,
                        expires,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Fetch a generation receipt by id, enforcing expiry on read. Returns
    /// `None` when the id is unknown OR expired; in the expired case the row
    /// (and any other rows whose `expires_at` has passed) is deleted first, so
    /// expiry doubles as the lazy GC. The write happens on the single writer
    /// thread, so the delete-then-read is race-free.
    pub async fn get_generation_receipt(
        &self,
        id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredGenerationReceipt>> {
        let id = id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                // Lazy cleanup: drop every receipt whose TTL has passed.
                conn.execute(
                    "DELETE FROM generation_receipt WHERE expires_at <= ?1",
                    rusqlite::params![now],
                )?;
                let sql = format!(
                    "SELECT {} FROM generation_receipt WHERE id = ?1",
                    crate::sqlite::records::GENERATION_RECEIPT_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut rows = stmt.query(rusqlite::params![id])?;
                match rows.next()? {
                    Some(row) => Ok(Some(
                        crate::sqlite::records::StoredGenerationReceipt::try_from(row)?,
                    )),
                    None => Ok(None),
                }
            })
            .await
    }
}
