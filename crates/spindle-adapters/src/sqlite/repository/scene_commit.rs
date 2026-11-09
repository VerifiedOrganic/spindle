use super::*;

/// In-transaction write helpers backing the atomic `commit_scene` boundary.
///
/// Every method here runs inside the single SQLite writer transaction opened by
/// [`Repository::with_transaction`]. A failure returns `Err`, the surrounding
/// closure aborts, and SQLite rolls back the prose, continuity package, beats,
/// chapter summary, writer cursor, and idempotency receipt together.
impl CanonTransaction<'_> {
    fn get_scene_in_tx(&self, id: &str) -> Result<Scene> {
        let sql = format!("SELECT {SCENE_COLUMNS} FROM scene WHERE id = ?1");
        let mut stmt = self.raw_tx().prepare_cached(&sql)?;
        stmt.query_row([id], |r| Scene::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("scene not found"))
    }

    pub fn find_scene_by_natural_key_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<Scene>> {
        let sql = format!(
            "SELECT {SCENE_COLUMNS} FROM scene \
             WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3 \
               AND chapter_number = ?4 AND scene_order = ?5 LIMIT 1"
        );
        let mut stmt = self.raw_tx().prepare_cached(&sql)?;
        Ok(stmt
            .query_row(
                rusqlite::params![
                    project_id,
                    branch_id,
                    book_number,
                    chapter_number,
                    scene_order
                ],
                |r| Scene::try_from(r),
            )
            .optional_inner()?)
    }

    fn book_id_in_tx(&self, project_id: &str, book_number: i32) -> Result<String> {
        self.raw_tx()
            .query_row(
                "SELECT id FROM book WHERE project_id = ?1 AND book_number = ?2 LIMIT 1",
                rusqlite::params![project_id, book_number],
                |r| r.get::<_, String>(0),
            )
            .optional_inner()
            .map_err(anyhow::Error::from)?
            .ok_or_else(|| anyhow!("book {book_number} not found for project"))
    }

    fn chapter_id_in_tx(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<String> {
        self.raw_tx()
            .query_row(
                "SELECT id FROM chapter WHERE project_id = ?1 AND book_number = ?2 \
                 AND chapter_number = ?3 LIMIT 1",
                rusqlite::params![project_id, book_number, chapter_number],
                |r| r.get::<_, String>(0),
            )
            .optional_inner()
            .map_err(anyhow::Error::from)?
            .ok_or_else(|| anyhow!("chapter {book_number}.{chapter_number} not found for project"))
    }

    /// Current persisted scene version: the highest historical version number,
    /// or 1 for a scene that has never been superseded. Matches the value
    /// `prepare_scene` reports as `scene_version`.
    pub fn current_scene_version_in_tx(&self, scene_id: &str) -> Result<i32> {
        let latest: i64 = self.raw_tx().query_row(
            "SELECT COALESCE(MAX(version_number), 0) FROM scene_version WHERE scene_id = ?1",
            [scene_id],
            |r| r.get(0),
        )?;
        Ok((latest as i32 + 1).max(1))
    }

    /// Create or update a scene and its version snapshot inside the caller's
    /// transaction. Mirrors `Repository::persist_scene` but does not open a
    /// second connection, so the prose commits with the rest of the commit unit.
    pub fn persist_scene_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        input: &SaveSceneDraftInput,
        receipt_claim: Option<(&str, &str)>,
        draft_origin: Option<&str>,
    ) -> Result<(Scene, bool)> {
        if input.book_number < 0 {
            anyhow::bail!(
                "book_number must be non-negative (got {})",
                input.book_number
            );
        }
        if !(0..crate::format::CHAPTER_RADIX as i32).contains(&input.chapter_number) {
            anyhow::bail!(
                "chapter_number {} is out of range 0..{}; it would collide in story-position ordering",
                input.chapter_number,
                crate::format::CHAPTER_RADIX
            );
        }
        if !(0..crate::format::SCENE_RADIX as i32).contains(&input.scene_order) {
            anyhow::bail!(
                "scene_order {} is out of range 0..{}; it would collide in story-position ordering",
                input.scene_order,
                crate::format::SCENE_RADIX
            );
        }
        // An in-run commit must honor the same stale handoff-plan guard as
        // save_scene_draft: a superseded handoff may not commit stale prose.
        super::authoring::check_active_run_scene_handoff_plan_revision_in_conn(
            self.raw_tx(),
            project_id,
            branch_id,
            input.book_number,
            input.chapter_number,
            input.scene_order,
        )?;
        let book_id = self.book_id_in_tx(project_id, input.book_number)?;
        let chapter_id =
            self.chapter_id_in_tx(project_id, input.book_number, input.chapter_number)?;
        let content_rating = input.content_rating.as_db_str();
        let now = timestamp_to_micros(chrono::Utc::now());
        let existing = self.find_scene_by_natural_key_in_tx(
            project_id,
            branch_id,
            input.book_number,
            input.chapter_number,
            input.scene_order,
        )?;

        if let Some(existing) = existing {
            let scene_changed = existing.full_text != input.full_text
                || existing.summary != input.summary
                || existing.content_rating != content_rating
                || existing.tone != input.tone;
            if let Some((receipt_id, scene_key)) = receipt_claim {
                claim_generation_receipt_in_tx(self.raw_tx(), receipt_id, scene_key)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
            }
            if scene_changed {
                let latest: i64 = self.raw_tx().query_row(
                    "SELECT COALESCE(MAX(version_number), 0) FROM scene_version WHERE scene_id = ?1",
                    [&existing.id],
                    |r| r.get(0),
                )?;
                let version_id = mint_id_local("scene_version");
                self.raw_tx().execute(
                    "INSERT INTO scene_version (id, project_id, branch_id, scene_id, \
                     version_number, book_number, chapter_number, scene_order, \
                     full_text, summary, content_rating, tone, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    rusqlite::params![
                        &version_id,
                        project_id,
                        branch_id,
                        &existing.id,
                        latest + 1,
                        existing.book_number,
                        existing.chapter_number,
                        existing.scene_order,
                        &existing.full_text,
                        &existing.summary,
                        &existing.content_rating,
                        &existing.tone,
                        now,
                    ],
                )?;
            }
            self.raw_tx().execute(
                "UPDATE scene SET full_text = ?1, summary = ?2, content_rating = ?3, \
                 tone = ?4, location_id = COALESCE(?5, location_id), \
                 draft_origin = COALESCE(?6, draft_origin), updated_at = ?7 WHERE id = ?8",
                rusqlite::params![
                    &input.full_text,
                    &input.summary,
                    content_rating,
                    &input.tone,
                    &input.location_id,
                    draft_origin,
                    now,
                    &existing.id,
                ],
            )?;
            if scene_changed {
                self.raw_tx().execute(
                    "UPDATE validator_finding SET resolved_at = ?1 \
                     WHERE scene_id = ?2 AND branch_id = ?3 AND resolved_at IS NULL",
                    rusqlite::params![now, &existing.id, branch_id],
                )?;
                self.raw_tx().execute(
                    "UPDATE dual_persona_review SET status = 'stale', updated_at = ?1 \
                     WHERE branch_id = ?2 AND scene_id = ?3",
                    rusqlite::params![now, branch_id, &existing.id],
                )?;
                super::memory::invalidate_layered_memory_for_scene_in_conn(
                    self.raw_tx(),
                    project_id,
                    branch_id,
                    &existing.id,
                    "scene_edit",
                )?;
            }
            Ok((self.get_scene_in_tx(&existing.id)?, false))
        } else {
            let scene_id = mint_id("scene");
            if let Some((receipt_id, scene_key)) = receipt_claim {
                claim_generation_receipt_in_tx(self.raw_tx(), receipt_id, scene_key)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
            }
            self.raw_tx().execute(
                "INSERT INTO scene (id, project_id, branch_id, book_id, chapter_id, \
                 book_number, chapter_number, scene_order, full_text, summary, \
                 content_rating, tone, draft_origin, location_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
                rusqlite::params![
                    &scene_id,
                    project_id,
                    branch_id,
                    &book_id,
                    &chapter_id,
                    input.book_number,
                    input.chapter_number,
                    input.scene_order,
                    &input.full_text,
                    &input.summary,
                    content_rating,
                    &input.tone,
                    draft_origin,
                    &input.location_id,
                    now,
                ],
            )?;
            super::memory::invalidate_layered_memory_for_scene_in_conn(
                self.raw_tx(),
                project_id,
                branch_id,
                &scene_id,
                "scene_create",
            )?;
            Ok((self.get_scene_in_tx(&scene_id)?, true))
        }
    }

    /// Replace a scene's beat annotation inside the caller's transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn annotate_scene_beats_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        motif_ids: &[String],
        theme_ids: &[String],
        conflict_ids: &[String],
        beats: &[AnnotatedBeat],
        intensity: Option<f64>,
    ) -> Result<String> {
        let effective_intensity = match intensity {
            Some(value) => Some(value),
            None => self
                .raw_tx()
                .query_row(
                    "SELECT intensity FROM scene_beat_annotation \
                     WHERE branch_id = ?1 AND scene_id = ?2 LIMIT 1",
                    rusqlite::params![branch_id, scene_id],
                    |r| r.get::<_, Option<f64>>(0),
                )
                .optional_inner()?
                .flatten(),
        };
        let stored_beats: Vec<StoredAnnotatedBeat> =
            beats.iter().cloned().map(Into::into).collect();
        let beats_json = serde_json::to_string(&stored_beats)?;
        let motif_json = serde_json::to_string(motif_ids)?;
        let theme_json = serde_json::to_string(theme_ids)?;
        let conflict_json = serde_json::to_string(conflict_ids)?;
        let id = mint_id("scene_beat_annotation");
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "DELETE FROM scene_beat_annotation WHERE branch_id = ?1 AND scene_id = ?2",
            rusqlite::params![branch_id, scene_id],
        )?;
        self.raw_tx().execute(
            "INSERT INTO scene_beat_annotation (id, project_id, branch_id, scene_id, \
             beats, motif_ids, theme_ids, conflict_ids, created_at, updated_at, intensity) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
            rusqlite::params![
                &id,
                project_id,
                branch_id,
                scene_id,
                &beats_json,
                &motif_json,
                &theme_json,
                &conflict_json,
                now,
                effective_intensity,
            ],
        )?;
        Ok(id)
    }

    /// Upsert a chapter summary and re-derive its book digest inside the
    /// caller's transaction. Mirrors `Repository::save_summary`.
    #[allow(clippy::too_many_arguments)]
    pub fn save_chapter_summary_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        summary: &str,
        key_events: &[String],
        character_changes: &[String],
        relationship_shifts: &[String],
        arc_advances: &[String],
        promise_events: &[String],
    ) -> Result<String> {
        use crate::format::{build_book_synopsis, build_open_threads, story_index};
        let key_events_json = serde_json::to_string(key_events)?;
        let character_changes_json = serde_json::to_string(character_changes)?;
        let relationship_shifts_json = serde_json::to_string(relationship_shifts)?;
        let arc_advances_json = serde_json::to_string(arc_advances)?;
        let promise_events_json = serde_json::to_string(promise_events)?;
        let id = mint_id("chapter_summary");
        let digest_id = mint_id("book_digest");
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "DELETE FROM chapter_summary \
             WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3 AND chapter_number = ?4",
            rusqlite::params![project_id, branch_id, book_number, chapter_number],
        )?;
        self.raw_tx().execute(
            "INSERT INTO chapter_summary (id, project_id, branch_id, book_number, \
             chapter_number, summary, key_events, character_changes, relationship_shifts, \
             arc_advances, promise_events, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            rusqlite::params![
                &id,
                project_id,
                branch_id,
                book_number,
                chapter_number,
                summary,
                &key_events_json,
                &character_changes_json,
                &relationship_shifts_json,
                &arc_advances_json,
                &promise_events_json,
                now,
            ],
        )?;

        const BOOK_DIGEST_CHAR_CAP: usize = 6000;
        let mut parts: Vec<(i32, String)> = Vec::new();
        {
            let mut stmt = self.raw_tx().prepare(
                "SELECT chapter_number, summary FROM chapter_summary \
                 WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3 \
                 ORDER BY chapter_number",
            )?;
            let mut rows = stmt.query(rusqlite::params![project_id, branch_id, book_number])?;
            while let Some(row) = rows.next()? {
                parts.push((row.get::<_, i64>(0)? as i32, row.get(1)?));
            }
        }
        let last_chapter_covered = parts.iter().map(|(chapter, _)| *chapter).max().unwrap_or(0);
        let (synopsis, truncated) = build_book_synopsis(&parts, BOOK_DIGEST_CHAR_CAP);
        let token_estimate = (synopsis.len() / 4) as i64;
        let promises = {
            let mut stmt = self.raw_tx().prepare(&format!(
                "SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise \
                 WHERE project_id = ?1 AND branch_id = ?2 AND archived_at IS NULL"
            ))?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                NarrativePromise::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let conflicts = {
            let mut stmt = self.raw_tx().prepare(&format!(
                "SELECT {CONFLICT_COLUMNS} FROM conflict \
                 WHERE project_id = ?1 AND branch_id = ?2 AND archived_at IS NULL"
            ))?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                Conflict::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let plot_lines = {
            let mut stmt = self.raw_tx().prepare(&format!(
                "SELECT {PLOT_LINE_COLUMNS} FROM plot_line \
                 WHERE project_id = ?1 AND branch_id = ?2 AND archived_at IS NULL"
            ))?;
            stmt.query_map(rusqlite::params![project_id, branch_id], |r| {
                PlotLine::try_from(r)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let current_index = story_index(book_number, last_chapter_covered, 0);
        let open_threads = build_open_threads(&promises, &conflicts, &plot_lines, current_index);
        let open_threads_json = serde_json::to_string(&open_threads)?;
        self.raw_tx().execute(
            "INSERT INTO book_digest (id, project_id, branch_id, book_number, synopsis, \
             open_threads, last_chapter_covered, token_estimate, truncated, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
             ON CONFLICT(project_id, branch_id, book_number) DO UPDATE SET \
               synopsis = excluded.synopsis, open_threads = excluded.open_threads, \
               last_chapter_covered = excluded.last_chapter_covered, \
               token_estimate = excluded.token_estimate, truncated = excluded.truncated, \
               updated_at = excluded.updated_at",
            rusqlite::params![
                &digest_id,
                project_id,
                branch_id,
                book_number,
                &synopsis,
                &open_threads_json,
                last_chapter_covered,
                token_estimate,
                truncated as i32,
                now,
            ],
        )?;
        Ok(id)
    }

    /// Upsert the branch writer cursor inside the caller's transaction.
    pub fn upsert_writer_position_in_tx(&self, params: UpsertWriterPositionParams) -> Result<()> {
        let id = mint_id("writer_position");
        let updated_at = timestamp_to_micros(params.updated_at);
        self.raw_tx().execute(
            "INSERT INTO writer_position (id, project_id, branch_id, book_id, chapter_id, \
             scene_id, intent, next_focus, updated_by, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT (project_id, branch_id) DO UPDATE SET \
                 book_id = excluded.book_id, chapter_id = excluded.chapter_id, \
                 scene_id = excluded.scene_id, intent = excluded.intent, \
                 next_focus = excluded.next_focus, updated_by = excluded.updated_by, \
                 updated_at = excluded.updated_at",
            rusqlite::params![
                &id,
                &params.project_id,
                &params.branch_id,
                &params.book_id,
                &params.chapter_id,
                &params.scene_id,
                &params.intent,
                &params.next_focus,
                &params.updated_by,
                updated_at,
            ],
        )?;
        Ok(())
    }

    /// Append a quantity state inside the caller's transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn append_quantity_state_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        subject_table: &str,
        subject_id: &str,
        measure: &str,
        state: &spindle_core::models::QuantityState,
        scene_id: Option<&str>,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<String> {
        let id = mint_id("quantity_state");
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "INSERT INTO quantity_state \
             (id, project_id, branch_id, subject_table, subject_id, measure, amount, \
              unit, band, change_reason, scene_id, book_number, chapter_number, \
              scene_order, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                &id,
                project_id,
                branch_id,
                subject_table,
                subject_id,
                measure,
                state.amount,
                &state.unit,
                &state.band,
                &state.change_reason,
                scene_id,
                book_number,
                chapter_number,
                scene_order,
                now,
            ],
        )?;
        Ok(id)
    }

    /// Plant a narrative promise inside the caller's transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn create_narrative_promise_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        promise_type: &str,
        description: &str,
        planted_at: &spindle_core::models::StoryPlacement,
        planned_payoff: Option<&spindle_core::models::StoryPlacement>,
        notes: &[String],
    ) -> Result<String> {
        let id = mint_id("narrative_promise");
        let planted: StoredStoryPlacement = planted_at.clone().into();
        let planted_json = serde_json::to_string(&planted)?;
        let payoff_json = planned_payoff
            .cloned()
            .map(StoredStoryPlacement::from)
            .map(|placement| serde_json::to_string(&placement))
            .transpose()?;
        let notes_json = serde_json::to_string(notes)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "INSERT INTO narrative_promise (id, project_id, branch_id, promise_type, \
             description, status, planted_at, planned_payoff, notes, archived_at, \
             created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'planted', ?6, ?7, ?8, NULL, ?9, ?9)",
            rusqlite::params![
                &id,
                project_id,
                branch_id,
                promise_type,
                description,
                &planted_json,
                &payoff_json,
                &notes_json,
                now,
            ],
        )?;
        Ok(id)
    }

    /// Create a timeline event inside the caller's transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn create_timeline_event_in_tx(
        &self,
        project_id: &str,
        branch_id: &str,
        title: &str,
        event_type: &str,
        placement: &spindle_core::models::StoryPlacement,
        summary: &str,
        related_entity_ids: &[String],
    ) -> Result<String> {
        let id = mint_id("timeline_event");
        let stored: StoredStoryPlacement = placement.clone().into();
        let placement_json = serde_json::to_string(&stored)?;
        let related_json = serde_json::to_string(related_entity_ids)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "INSERT INTO timeline_event (id, project_id, branch_id, title, event_type, \
             placement, summary, related_entity_ids, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            rusqlite::params![
                &id,
                project_id,
                branch_id,
                title,
                event_type,
                &placement_json,
                summary,
                &related_json,
                now,
            ],
        )?;
        Ok(id)
    }

    /// Load a stored idempotency receipt for this key, if any.
    pub fn find_scene_commit_receipt_in_tx(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<(String, String)>> {
        Ok(self
            .raw_tx()
            .query_row(
                "SELECT payload_hash, receipt_json FROM scene_commit_receipt \
                 WHERE idempotency_key = ?1",
                [idempotency_key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional_inner()?)
    }

    /// Store the idempotency receipt in the same transaction as the commit.
    #[allow(clippy::too_many_arguments)]
    pub fn store_scene_commit_receipt_in_tx(
        &self,
        operation_id: &str,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        idempotency_key: &str,
        payload_hash: &str,
        receipt_json: &str,
    ) -> Result<()> {
        let now = timestamp_to_micros(chrono::Utc::now());
        self.raw_tx().execute(
            "INSERT INTO scene_commit_receipt \
             (operation_id, project_id, branch_id, scene_id, idempotency_key, payload_hash, \
              receipt_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                operation_id,
                project_id,
                branch_id,
                scene_id,
                idempotency_key,
                payload_hash,
                receipt_json,
                now,
            ],
        )?;
        Ok(())
    }
}

impl Repository {
    /// Read a persisted `commit_scene` idempotency receipt by key.
    pub async fn find_scene_commit_receipt(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<(String, String)>> {
        let idempotency_key = idempotency_key.to_string();
        self.pool()
            .read(move |conn| {
                conn.query_row(
                    "SELECT payload_hash, receipt_json FROM scene_commit_receipt \
                     WHERE idempotency_key = ?1",
                    [&idempotency_key],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional_inner()
            })
            .await
    }
}
