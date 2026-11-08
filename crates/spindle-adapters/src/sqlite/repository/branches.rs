use super::*;

impl Repository {
    // =========================================================================
    // Bible branch
    // =========================================================================

    pub async fn get_branch(&self, id: &str) -> Result<BibleBranch> {
        let id = id.to_string();
        let branch = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {BIBLE_BRANCH_COLUMNS} FROM bible_branch WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| BibleBranch::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("bible_branch not found"))?;
        Ok(branch)
    }

    /// Look up the active branch on a project, falling back to its 'main'
    /// branch if `active_branch_id` is NULL.
    pub async fn get_active_branch(&self, project_id: &str) -> Result<BibleBranch> {
        let project = self.get_project(project_id).await?;
        if let Some(branch_id) = project.active_branch_id.clone() {
            return self.get_branch(&branch_id).await;
        }
        let project_id = project_id.to_string();
        let branch = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BIBLE_BRANCH_COLUMNS} FROM bible_branch \
                     WHERE project_id = ?1 AND name = 'main'"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&project_id], |r| BibleBranch::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("no main branch for project"))?;
        Ok(branch)
    }

    pub async fn list_branches_by_project(&self, project_id: &str) -> Result<Vec<BibleBranch>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BIBLE_BRANCH_COLUMNS} FROM bible_branch \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| BibleBranch::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn create_branch(
        &self,
        project_id: &str,
        parent_branch_id: &str,
        name: &str,
        branch_type: &str,
        description: Option<String>,
    ) -> Result<BibleBranch> {
        let id = mint_id("bible_branch");
        let project_id = project_id.to_string();
        let parent_branch_id = parent_branch_id.to_string();
        let name = name.to_string();
        let branch_type = branch_type.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let id_for_lookup = id.clone();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO bible_branch (id, project_id, parent_branch_id, name, status, \
                     branch_type, description, created_from_save_point_id, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, NULL, ?7, ?7)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &parent_branch_id,
                        &name,
                        &branch_type,
                        &description,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_branch(&id_for_lookup).await
    }

    pub async fn switch_active_branch(&self, project_id: &str, branch_id: &str) -> Result<Project> {
        let project_id_owned = project_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE project SET active_branch_id = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&branch_id_owned, now, &project_id_owned],
                )?;
                Ok(())
            })
            .await?;
        self.get_project(project_id).await
    }

    /// Return the ancestor branch chain `[branch_id, parent_id, ..., main_id]`
    /// ordered from most specific (child) to least specific (`main`).
    pub async fn resolve_branch_lineage(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<String>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id))
            .await
    }

    /// True when `branch_id` equals `ancestor_branch_id` or inherits from it
    /// along the `parent_branch_id` chain (including fallback to `main`).
    pub async fn branch_inherits_from(
        &self,
        project_id: &str,
        branch_id: &str,
        ancestor_branch_id: &str,
    ) -> Result<bool> {
        let lineage = self.resolve_branch_lineage(project_id, branch_id).await?;
        Ok(lineage.iter().any(|b| b == ancestor_branch_id))
    }

    /// Resolve a project's active branch id without round-tripping through
    /// the full Project record. Useful when create_* helpers only need the
    /// branch_id string.
    pub(super) async fn active_branch_id(&self, project_id: &str) -> Result<String> {
        Ok(self.get_active_branch(project_id).await?.id)
    }

    /// Public version of [`Self::active_branch_id`] for service-layer callers.
    pub async fn active_branch_id_public(&self, project_id: &str) -> Result<String> {
        self.active_branch_id(project_id).await
    }

    // =========================================================================
    // Save point
    // =========================================================================

    pub async fn create_save_point(
        &self,
        project_id: &str,
        branch_id: &str,
        name: &str,
        description: Option<String>,
    ) -> Result<SavePoint> {
        let id = mint_id("save_point");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let name = name.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO save_point (id, project_id, branch_id, name, description, \
                     snapshot_file_path, snapshot_format, snapshot_record_count, \
                     snapshot_created_at, snapshot_sha256, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, ?6)",
                    rusqlite::params![&id, &project_id, &branch_id, &name, &description, now],
                )?;
                Ok(())
            })
            .await?;
        self.get_save_point(&id_out).await
    }

    pub async fn get_save_point(&self, id: &str) -> Result<SavePoint> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {SAVE_POINT_COLUMNS} FROM save_point WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| SavePoint::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("save_point not found"))
    }

    pub async fn list_save_points_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SavePoint>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SAVE_POINT_COLUMNS} FROM save_point \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        SavePoint::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn delete_save_point(&self, save_point_id: &str) -> Result<()> {
        let id = save_point_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute("DELETE FROM save_point WHERE id = ?1", [&id])?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn update_save_point_snapshot(
        &self,
        save_point_id: &str,
        file_path: &str,
        format: &str,
        record_count: i64,
        sha256: &str,
    ) -> Result<SavePoint> {
        let save_point_id_owned = save_point_id.to_string();
        let file_path = file_path.to_string();
        let format = format.to_string();
        let sha256 = sha256.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute(
                    "UPDATE save_point SET snapshot_file_path = ?1, snapshot_format = ?2, \
                     snapshot_record_count = ?3, snapshot_created_at = ?4, snapshot_sha256 = ?5 \
                     WHERE id = ?6",
                    rusqlite::params![
                        &file_path,
                        &format,
                        record_count,
                        now,
                        &sha256,
                        &save_point_id_owned,
                    ],
                )?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        self.get_save_point(save_point_id).await
    }

    // =========================================================================
    // Merge branch snapshot
    //
    // Ports the SurrealDB-era `merge_branch_snapshot` to SQLite. Writes the
    // mergeable slice of a source branch (scenes, character_state,
    // relates_to, pacing_tracker) into `target_branch_id` inside a single
    // transaction. Conflict detection lives in the service layer; this fn
    // assumes the caller has already filtered out conflicting positions.
    //
    // Semantic note: scene/character_state/pacing_tracker rows are
    // re-stamped with the target branch's id and minted fresh primary keys.
    // Relationship edges (relates_to) reuse the source row's character ids
    // (in_id/out_id) — they're FK-constrained to `character(id)`, which is
    // a per-branch row in the SQLite schema. The caller is responsible for
    // ensuring those characters exist on the target branch (typical case:
    // both branches descend from the same parent main branch and share
    // characters).
    // =========================================================================

    /// Apply a pre-filtered set of source-branch rows onto `target_branch_id`
    /// in a single transaction. Mirrors `merge_branch_snapshot` from the
    /// SurrealDB-era repository (repository.rs:5414..5535 in 705b835^).
    pub async fn merge_branch_snapshot(
        &self,
        project_id: &str,
        target_branch_id: &str,
        source_scenes: &[Scene],
        source_states: &[CharacterState],
        source_relationships: &[RelatesTo],
        source_pacing: &[PacingTracker],
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let target_branch_id = target_branch_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        // Materialize owned inputs so the move-closure can take them. Each
        // entry carries everything the INSERT/UPDATE statements need, plus
        // pre-serialized JSON blobs for the columns that store JSON.
        struct SceneRow {
            book_id: String,
            chapter_id: String,
            book_number: i32,
            chapter_number: i32,
            scene_order: i32,
            full_text: String,
            summary: String,
            content_rating: String,
            tone: Option<String>,
            location_id: Option<String>,
        }
        let scenes_owned: Vec<SceneRow> = source_scenes
            .iter()
            .map(|s| SceneRow {
                book_id: s.book_id.clone(),
                chapter_id: s.chapter_id.clone(),
                book_number: s.book_number,
                chapter_number: s.chapter_number,
                scene_order: s.scene_order,
                full_text: s.full_text.clone(),
                summary: s.summary.clone(),
                content_rating: s.content_rating.clone(),
                tone: s.tone.clone(),
                location_id: s.location_id.clone(),
            })
            .collect();

        struct StateRow {
            character_id: String,
            scene_id: Option<String>,
            book_number: i32,
            chapter_number: i32,
            scene_order: i32,
            emotional_json: String,
            goals_json: String,
            status_json: String,
            notes_json: String,
            source_summary: Option<String>,
        }
        let states_owned: Vec<StateRow> = source_states
            .iter()
            .map(|s| {
                Ok(StateRow {
                    character_id: s.character_id.clone(),
                    scene_id: s.scene_id.clone(),
                    book_number: s.book_number,
                    chapter_number: s.chapter_number,
                    scene_order: s.scene_order,
                    emotional_json: serde_json::to_string(&s.emotional_state)
                        .context("serializing character_state.emotional_state")?,
                    goals_json: serde_json::to_string(&s.goals)
                        .context("serializing character_state.goals")?,
                    status_json: serde_json::to_string(&s.status)
                        .context("serializing character_state.status")?,
                    notes_json: serde_json::to_string(&s.notes)
                        .context("serializing character_state.notes")?,
                    source_summary: s.source_summary.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        struct RelRow {
            source_branch_id: String,
            in_id: String,
            out_id: String,
            relationship_type: String,
            trust: i32,
            tension: i32,
            dynamics_json: String,
            reason: Option<String>,
            last_scene_id: Option<String>,
        }
        let rels_owned: Vec<RelRow> = source_relationships
            .iter()
            .map(|r| {
                Ok(RelRow {
                    source_branch_id: r.branch_id.clone(),
                    in_id: r.in_id.clone(),
                    out_id: r.out_id.clone(),
                    relationship_type: r.relationship_type.clone(),
                    trust: r.trust,
                    tension: r.tension,
                    dynamics_json: serde_json::to_string(&r.dynamics)
                        .context("serializing relates_to.dynamics")?,
                    reason: r.reason.clone(),
                    last_scene_id: r.last_scene_id.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        struct TrackerRow {
            character_arc_id: String,
            per_book_budget_json: String,
            max_progress_per_chapter: Option<f64>,
            milestone_spacing: Option<i32>,
            sprint_allowance: Option<i32>,
            regression_budget: Option<f64>,
            current_progress: f64,
            budget_remaining: f64,
            velocity: String,
            status: String,
            next_milestone: Option<String>,
            warnings_json: String,
        }
        let trackers_owned: Vec<TrackerRow> = source_pacing
            .iter()
            .map(|t| {
                Ok(TrackerRow {
                    character_arc_id: t.character_arc_id.clone(),
                    per_book_budget_json: serde_json::to_string(&t.per_book_budget)
                        .context("serializing pacing_tracker.per_book_budget")?,
                    max_progress_per_chapter: t.max_progress_per_chapter,
                    milestone_spacing: t.milestone_spacing,
                    sprint_allowance: t.sprint_allowance,
                    regression_budget: t.regression_budget,
                    current_progress: t.current_progress,
                    budget_remaining: t.budget_remaining,
                    velocity: t.velocity.clone(),
                    status: t.status.clone(),
                    next_milestone: t.next_milestone.clone(),
                    warnings_json: serde_json::to_string(&t.warnings)
                        .context("serializing pacing_tracker.warnings")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;

                // Scenes: UPDATE-or-INSERT keyed by
                // (project_id, branch_id, book#, chapter#, scene_order).
                for scene in &scenes_owned {
                    let affected = tx.execute(
                        "UPDATE scene SET full_text = ?1, summary = ?2, \
                         content_rating = ?3, tone = ?4, updated_at = ?5 \
                         WHERE project_id = ?6 AND branch_id = ?7 \
                           AND book_number = ?8 AND chapter_number = ?9 \
                           AND scene_order = ?10",
                        rusqlite::params![
                            &scene.full_text,
                            &scene.summary,
                            &scene.content_rating,
                            &scene.tone,
                            now,
                            &project_id,
                            &target_branch_id,
                            scene.book_number,
                            scene.chapter_number,
                            scene.scene_order,
                        ],
                    )?;
                    if affected == 0 {
                        let new_id = mint_id_local("scene");
                        tx.execute(
                            "INSERT INTO scene (id, project_id, branch_id, book_id, \
                             chapter_id, book_number, chapter_number, scene_order, \
                             full_text, summary, content_rating, tone, draft_origin, \
                             location_id, created_at, updated_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                                     NULL, ?13, ?14, ?14)",
                            rusqlite::params![
                                &new_id,
                                &project_id,
                                &target_branch_id,
                                &scene.book_id,
                                &scene.chapter_id,
                                scene.book_number,
                                scene.chapter_number,
                                scene.scene_order,
                                &scene.full_text,
                                &scene.summary,
                                &scene.content_rating,
                                &scene.tone,
                                &scene.location_id,
                                now,
                            ],
                        )?;
                    }
                }

                // character_state: DELETE-then-INSERT keyed by
                // (branch_id, character_id, book#, chapter#, scene_order).
                for state in &states_owned {
                    tx.execute(
                        "DELETE FROM character_state \
                         WHERE branch_id = ?1 AND character_id = ?2 \
                           AND book_number = ?3 AND chapter_number = ?4 \
                           AND scene_order = ?5",
                        rusqlite::params![
                            &target_branch_id,
                            &state.character_id,
                            state.book_number,
                            state.chapter_number,
                            state.scene_order,
                        ],
                    )?;
                    let new_id = mint_id_local("character_state");
                    tx.execute(
                        "INSERT INTO character_state (id, project_id, branch_id, \
                         character_id, scene_id, book_number, chapter_number, \
                         scene_order, emotional_state, goals, status, notes, \
                         source_summary, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                                 ?13, ?14)",
                        rusqlite::params![
                            &new_id,
                            &project_id,
                            &target_branch_id,
                            &state.character_id,
                            &state.scene_id,
                            state.book_number,
                            state.chapter_number,
                            state.scene_order,
                            &state.emotional_json,
                            &state.goals_json,
                            &state.status_json,
                            &state.notes_json,
                            &state.source_summary,
                            now,
                        ],
                    )?;
                }

                // relates_to: DELETE-then-INSERT keyed by composite PK
                // (branch_id, in_id, out_id). Re-stamps branch_id to target.
                for rel in &rels_owned {
                    tx.execute(
                        "DELETE FROM relates_to \
                         WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3",
                        rusqlite::params![&target_branch_id, &rel.in_id, &rel.out_id,],
                    )?;
                    tx.execute(
                        "INSERT INTO relates_to (in_id, out_id, branch_id, \
                         relationship_type, trust, tension, dynamics, reason, \
                         last_scene_id, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![
                            &rel.in_id,
                            &rel.out_id,
                            &target_branch_id,
                            &rel.relationship_type,
                            rel.trust,
                            rel.tension,
                            &rel.dynamics_json,
                            &rel.reason,
                            &rel.last_scene_id,
                            now,
                        ],
                    )?;
                    if rel.source_branch_id != target_branch_id {
                        tx.execute(
                            "DELETE FROM relationship_scene_delta \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2))",
                            rusqlite::params![&target_branch_id, &rel.in_id, &rel.out_id],
                        )?;
                        tx.execute(
                            "INSERT OR REPLACE INTO relationship_scene_delta ( \
                                branch_id, in_id, out_id, scene_id, source, \
                                book_number, chapter_number, scene_order, \
                                trust_delta, tension_delta, reason, updated_at \
                             ) \
                             SELECT ?1, in_id, out_id, scene_id, source, \
                                    book_number, chapter_number, scene_order, \
                                    trust_delta, tension_delta, reason, updated_at \
                             FROM relationship_scene_delta \
                             WHERE branch_id = ?2 \
                               AND ((in_id = ?3 AND out_id = ?4) OR (in_id = ?4 AND out_id = ?3))",
                            rusqlite::params![
                                &target_branch_id,
                                &rel.source_branch_id,
                                &rel.in_id,
                                &rel.out_id,
                            ],
                        )?;
                    }
                }

                // pacing_tracker: DELETE-then-INSERT keyed by
                // (branch_id, character_arc_id). Re-stamps branch_id to target.
                for tracker in &trackers_owned {
                    tx.execute(
                        "DELETE FROM pacing_tracker \
                         WHERE branch_id = ?1 AND character_arc_id = ?2",
                        rusqlite::params![&target_branch_id, &tracker.character_arc_id],
                    )?;
                    let new_id = mint_id_local("pacing_tracker");
                    tx.execute(
                        "INSERT INTO pacing_tracker (id, project_id, branch_id, \
                         character_arc_id, per_book_budget, max_progress_per_chapter, \
                         milestone_spacing, sprint_allowance, regression_budget, \
                         current_progress, budget_remaining, velocity, status, \
                         next_milestone, warnings, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                                 ?13, ?14, ?15, ?16)",
                        rusqlite::params![
                            &new_id,
                            &project_id,
                            &target_branch_id,
                            &tracker.character_arc_id,
                            &tracker.per_book_budget_json,
                            tracker.max_progress_per_chapter,
                            tracker.milestone_spacing,
                            tracker.sprint_allowance,
                            tracker.regression_budget,
                            tracker.current_progress,
                            tracker.budget_remaining,
                            &tracker.velocity,
                            &tracker.status,
                            &tracker.next_milestone,
                            &tracker.warnings_json,
                            now,
                        ],
                    )?;
                }

                tx.commit()?;
                Ok(())
            })
            .await
    }

    // =========================================================================
    // Restore branch snapshot (inverse of merge_branch_snapshot)
    //
    // Replaces every branch-scoped row in (project_id, branch_id) with the
    // rows carried in the snapshot. Runs in a single writer transaction with
    // `PRAGMA defer_foreign_keys = 1` so the order of DELETE/INSERT statements
    // doesn't have to obey the FK graph — referential integrity is checked
    // once at COMMIT. Mirrors the intent of `restore_branch_snapshot`
    // (`repository.rs:3202..3390` in 705b835^), which used SurrealDB's
    // BEGIN/COMMIT + heterogeneous CONTENT UPSERT to achieve the same thing.
    //
    // `save_point` and `bible_branch` rows are intentionally NOT touched:
    //   * Save points outlive the branch state they describe (you need to be
    //     able to restore to a different save_point after a restore).
    //   * The bible_branch row itself defines the branch we're restoring
    //     into — wiping it would orphan every FK we're about to write.
    //
    // Tables walked: `BRANCH_RESTORE_TABLES` (defined alongside
    // `BranchRestoreSnapshot` at the top of this file).
    // =========================================================================

    /// Rewind `branch_id` (under `project_id`) to the state encoded by
    /// `snapshot`. Deletes every branch-scoped row in
    /// `BRANCH_RESTORE_TABLES` whose (project_id, branch_id) match, then
    /// re-inserts the snapshot rows. `save_point` and `bible_branch` are
    /// preserved.
    pub async fn restore_branch_snapshot(
        &self,
        project_id: &str,
        branch_id: &str,
        snapshot: &BranchRestoreSnapshot,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();

        // Materialize snapshot for the move-closure. We clone per-table so
        // the closure owns everything it needs without re-borrowing.
        let rows_by_table: std::collections::BTreeMap<String, Vec<serde_json::Map<String, Value>>> =
            snapshot.rows_by_table.clone();

        self.inner
            .pool
            .write(move |conn| {
                conn.execute_batch("PRAGMA defer_foreign_keys = 1;")?;
                let tx = conn.transaction()?;

                // Phase 1: delete the current branch content. character_*
                // child tables cascade with character; scene_* with scene;
                // book_outline/chapter_outline/relates_to/knows have their
                // own branch_id columns and need explicit deletes.
                for table in BRANCH_RESTORE_TABLES {
                    let delete_sql = match *table {
                        // (project_id, branch_id) tables — the common case.
                        "revision_marker"
                        | "dual_persona_review"
                        | "scene_beat_annotation"
                        | "canonical_fact"
                        | "scene_version"
                        | "character_state"
                        | "future_knowledge"
                        | "timeline_event"
                        | "temporal_intervention"
                        | "progression_event"
                        | "pacing_tracker"
                        | "chapter_plan"
                        | "chapter_summary"
                        | "pacing_curve"
                        | "pacing_config"
                        | "narrative_promise"
                        | "character_arc"
                        | "plot_line"
                        | "conflict"
                        | "theme"
                        | "motif"
                        | "faction"
                        | "religion"
                        | "economy"
                        | "term"
                        | "system_overlay"
                        | "knowledge_fact"
                        | "world_state"
                        | "world_rule"
                        | "scene"
                        | "location"
                        | "character"
                        | "knows" => {
                            format!("DELETE FROM {table} WHERE project_id = ?1 AND branch_id = ?2")
                        }
                        // Branch-only tables (no project_id column).
                        "book_outline"
                        | "chapter_outline"
                        | "relates_to"
                        | "relationship_scene_delta" => {
                            format!("DELETE FROM {table} WHERE branch_id = ?1")
                        }
                        // Child tables keyed by a parent_id column. Their
                        // parents (character, scene) cascade, so explicit
                        // deletes are belt-and-suspenders for the case where
                        // a snapshot row points at a parent that's still
                        // present (we want a clean slate before INSERT).
                        "scene_source_link" => format!(
                            "DELETE FROM {table} WHERE scene_id IN \
                             (SELECT id FROM scene WHERE project_id = ?1 AND branch_id = ?2)"
                        ),
                        "character_voice_profile" | "character_emotional_profile" => format!(
                            "DELETE FROM {table} WHERE character_id IN \
                             (SELECT id FROM character WHERE project_id = ?1 AND branch_id = ?2)"
                        ),
                        _ => continue,
                    };
                    if delete_sql.contains("?2") {
                        tx.execute(&delete_sql, rusqlite::params![&project_id, &branch_id])?;
                    } else {
                        tx.execute(&delete_sql, rusqlite::params![&branch_id])?;
                    }
                }
                // Phase 2: insert the snapshot rows. For each table we build
                // an INSERT statement keyed by the union of column names
                // present on the first row (the snapshot is guaranteed to
                // be self-consistent because it was produced by
                // `dump_project_table`).
                for table in BRANCH_RESTORE_TABLES {
                    let Some(rows) = rows_by_table.get(*table) else {
                        continue;
                    };
                    if rows.is_empty() {
                        continue;
                    }
                    // Use the first row's keys as the canonical column
                    // ordering. All rows for a given table come from
                    // `SELECT *` so they share the same key set.
                    let columns: Vec<&str> = rows[0].keys().map(String::as_str).collect();
                    let placeholders: Vec<String> =
                        (1..=columns.len()).map(|i| format!("?{i}")).collect();
                    let col_list = columns.join(", ");
                    let ph_list = placeholders.join(", ");
                    let insert_sql = format!("INSERT INTO {table} ({col_list}) VALUES ({ph_list})");
                    let mut stmt = tx.prepare(&insert_sql)?;
                    for row in rows {
                        // Build the value vector in column order. Anything
                        // missing from the row map becomes NULL.
                        let mut values: Vec<JsonParam> = Vec::with_capacity(columns.len());
                        for col in &columns {
                            let v = row.get(*col).cloned().unwrap_or(Value::Null);
                            values.push(JsonParam::from_value_for_column(col, v));
                        }
                        let params: Vec<&dyn rusqlite::ToSql> =
                            values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                        stmt.execute(rusqlite::params_from_iter(params.iter()))?;
                    }
                }

                tx.commit()?;
                Ok(())
            })
            .await
    }

    // =========================================================================
    // Typed project-wide list_all_* helpers
    //
    // These mirror the existing `_by_project` / `_by_project_and_branch`
    // accessors but always span every branch on the project — the shape
    // `build_project_export_payload` needs to round-trip a full project
    // snapshot through the typed records (so timestamps serialize as
    // RFC-3339 strings rather than unix-micro integers).
    //
    // Each `SELECT` order is stable so the exported `tables.*` arrays are
    // deterministic across runs.
    // =========================================================================

    pub async fn list_all_save_points(&self, project_id: &str) -> Result<Vec<SavePoint>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SAVE_POINT_COLUMNS} FROM save_point \
                     WHERE project_id = ?1 ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SavePoint::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Resolve the effective target branch for canon checks and writes anchored
    /// to a scene on `scene_branch_id`. If the project's active branch is
    /// `scene_branch_id` or a descendant branch that inherits from
    /// `scene_branch_id`, returns the active branch; otherwise returns
    /// `scene_branch_id`.
    pub async fn resolve_effective_scene_branch(
        &self,
        project_id: &str,
        scene_branch_id: &str,
    ) -> Result<String> {
        let project_id = project_id.to_string();
        let scene_branch_id = scene_branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                resolve_effective_scene_branch_in_conn(conn, &project_id, &scene_branch_id)
            })
            .await
    }
}
