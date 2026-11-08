use super::*;

impl Repository {
    // =========================================================================
    // Writer position
    // =========================================================================

    pub async fn get_writer_position(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Option<WriterPosition>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WRITER_POSITION_COLUMNS} FROM writer_position \
                     WHERE project_id = ?1 AND branch_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&project_id, &branch_id], |r| {
                    WriterPosition::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn upsert_writer_position(
        &self,
        params: UpsertWriterPositionParams,
    ) -> Result<WriterPosition> {
        // The schema has `UNIQUE (project_id, branch_id)`. Use INSERT ON CONFLICT
        // to update the existing row in-place; mint a fresh id only on insert.
        let id = mint_id("writer_position");
        let project_id = params.project_id.clone();
        let branch_id = params.branch_id.clone();
        let book_id = params.book_id;
        let chapter_id = params.chapter_id;
        let scene_id = params.scene_id;
        let intent = params.intent;
        let next_focus = params.next_focus;
        let updated_by = params.updated_by;
        let updated_at = timestamp_to_micros(params.updated_at);

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO writer_position (id, project_id, branch_id, book_id, chapter_id, \
                     scene_id, intent, next_focus, updated_by, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                     ON CONFLICT (project_id, branch_id) DO UPDATE SET \
                         book_id = excluded.book_id, \
                         chapter_id = excluded.chapter_id, \
                         scene_id = excluded.scene_id, \
                         intent = excluded.intent, \
                         next_focus = excluded.next_focus, \
                         updated_by = excluded.updated_by, \
                         updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &book_id,
                        &chapter_id,
                        &scene_id,
                        &intent,
                        &next_focus,
                        &updated_by,
                        updated_at,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_writer_position(&params.project_id, &params.branch_id)
            .await?
            .ok_or_else(|| anyhow!("writer_position vanished after upsert"))
    }

    pub async fn delete_writer_position(&self, project_id: &str, branch_id: &str) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM writer_position WHERE project_id = ?1 AND branch_id = ?2",
                    rusqlite::params![&project_id, &branch_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // =========================================================================
    // Session activity (append-only log)
    // =========================================================================

    pub async fn append_session_activity(
        &self,
        params: AppendSessionActivityParams,
    ) -> Result<SessionActivity> {
        let id = mint_id("session_activity");
        let id_for_lookup = id.clone();
        let details_json = match params.details_json {
            Some(v) => Some(serde_json::to_string(&v).context("serializing details_json")?),
            None => None,
        };
        let project_id = params.project_id;
        let branch_id = params.branch_id;
        let kind = params.kind;
        let subject_table = params.subject_table;
        let subject_id = params.subject_id;
        let summary = params.summary;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO session_activity (id, project_id, branch_id, kind, subject_table, \
                     subject_id, summary, details_json, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &kind,
                        &subject_table,
                        &subject_id,
                        &summary,
                        &details_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        let id_lookup = id_for_lookup.clone();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SESSION_ACTIVITY_COLUMNS} FROM session_activity WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| SessionActivity::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("session_activity vanished after insert"))
    }

    pub async fn list_recent_session_activity(
        &self,
        project_id: &str,
        branch_id: &str,
        limit: i64,
    ) -> Result<Vec<SessionActivity>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SESSION_ACTIVITY_COLUMNS} FROM session_activity \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at DESC LIMIT ?3"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id, limit], |r| {
                        SessionActivity::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
