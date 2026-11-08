use super::*;

impl Repository {
    // ── Plan amendments (ADR 0003 — living-outline replanning) ────────────────

    /// Stage a proposed plan amendment (ADR 0003 D2). Validates:
    ///   * `amendment_class` ∈ `PLAN_AMENDMENT_CLASSES` (unknown classes are
    ///     rejected — forward-compat additions ship via a new constant entry,
    ///     never a free-form label);
    ///   * `rationale` non-empty (trimmed) and ≤500 **chars** (char-safe, so a
    ///     multibyte reasoning string is measured correctly — the miner's
    ///     evidence discipline transplanted to rationale);
    ///   * `confidence` ∈ {high, medium, low};
    ///   * `target_chapter` present for every class EXCEPT `promise_followup`,
    ///     and absent (forbidden-none) for `promise_followup` (which targets a
    ///     future placement, not a chapter row — ADR D1/D2).
    ///
    /// Records nothing about the decision seam here — apply-dispatch to plan
    /// write paths is the service layer's job (Part B).
    pub async fn stage_plan_amendment(
        &self,
        params: StagePlanAmendmentParams,
    ) -> Result<crate::sqlite::records::StoredPlanAmendment> {
        let StagePlanAmendmentParams {
            project_id,
            branch_id,
            source_chapter,
            book_number,
            authoring_run_id,
            amendment_class,
            target_chapter,
            payload,
            rationale,
            confidence,
        } = params;

        if !spindle_core::models::is_plan_amendment_class(&amendment_class) {
            return Err(anyhow!(
                "unknown plan amendment class '{amendment_class}' (not in PLAN_AMENDMENT_CLASSES)"
            ));
        }
        if rationale.trim().is_empty() {
            return Err(anyhow!(
                "plan amendment rationale is mandatory — an amendment with no stated reasoning is not stageable"
            ));
        }
        let rationale_chars = rationale.chars().count();
        if rationale_chars > 500 {
            return Err(anyhow!(
                "plan amendment rationale must be ≤500 chars (got {rationale_chars})"
            ));
        }
        if !matches!(confidence.as_str(), "high" | "medium" | "low") {
            return Err(anyhow!(
                "plan amendment confidence must be one of high|medium|low (got '{confidence}')"
            ));
        }
        // `promise_followup` targets a future placement, not a chapter row: its
        // target_chapter must be absent. Every other class amends a specific
        // future chapter and must name it (ADR D1/D2). The rule is
        // class-conditional, so it lives here, not in a SQL CHECK.
        if amendment_class == "promise_followup" {
            if target_chapter.is_some() {
                return Err(anyhow!(
                    "plan amendment class 'promise_followup' must NOT carry a target_chapter (it targets a future placement)"
                ));
            }
        } else if target_chapter.is_none() {
            return Err(anyhow!(
                "plan amendment class '{amendment_class}' requires a target_chapter"
            ));
        }

        let payload_str =
            serde_json::to_string(&payload).context("serializing plan amendment payload")?;
        let id = mint_id("plan_amendment");
        let id_lookup = id.clone();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO plan_amendment \
                     (id, project_id, branch_id, source_chapter, book_number, authoring_run_id, \
                      amendment_class, target_chapter, payload, rationale, confidence, status, \
                      decided_at, decided_by, prior_state, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'staged', NULL, NULL, \
                             NULL, ?12, ?13)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        source_chapter,
                        book_number,
                        &authoring_run_id,
                        &amendment_class,
                        target_chapter,
                        &payload_str,
                        &rationale,
                        &confidence,
                        now,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.read_plan_amendment(&id_lookup)
            .await?
            .ok_or_else(|| anyhow!("plan_amendment vanished after insert"))
    }

    /// Fetch one plan amendment by id, or `None` if absent. Public read used by
    /// the service-layer apply dispatcher (Part B) to pre-flight each decision.
    pub async fn read_plan_amendment_public(
        &self,
        id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredPlanAmendment>> {
        self.read_plan_amendment(id).await
    }

    /// Fetch one plan amendment by id, or `None` if absent.
    pub(super) async fn read_plan_amendment(
        &self,
        id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredPlanAmendment>> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM plan_amendment WHERE id = ?1",
                    crate::sqlite::records::PLAN_AMENDMENT_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| {
                    crate::sqlite::records::StoredPlanAmendment::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    /// List plan amendments on a branch, optionally filtered by `status` and/or
    /// provenance `(book_number, source_chapter)`. Deterministic order:
    /// `(created_at, id)`. The book/source-chapter filter is all-or-nothing —
    /// pass `Some((book, chapter))` to scope to one replan pass's provenance.
    pub async fn list_plan_amendments(
        &self,
        project_id: &str,
        branch_id: &str,
        status: Option<&str>,
        book_source_chapter: Option<(i32, i32)>,
    ) -> Result<Vec<crate::sqlite::records::StoredPlanAmendment>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let status = status.map(str::to_string);
        self.inner
            .pool
            .read(move |conn| {
                // Bind only the filters that are present. Params are pushed in
                // the same order the placeholders are appended.
                let mut clauses = String::from("project_id = ?1 AND branch_id = ?2");
                let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&project_id, &branch_id];
                if let Some(status) = status.as_ref() {
                    clauses.push_str(&format!(" AND status = ?{}", binds.len() + 1));
                    binds.push(status);
                }
                if let Some((book, source_chapter)) = book_source_chapter.as_ref() {
                    clauses.push_str(&format!(
                        " AND book_number = ?{} AND source_chapter = ?{}",
                        binds.len() + 1,
                        binds.len() + 2
                    ));
                    binds.push(book);
                    binds.push(source_chapter);
                }
                let sql = format!(
                    "SELECT {} FROM plan_amendment WHERE {clauses} ORDER BY created_at, id",
                    crate::sqlite::records::PLAN_AMENDMENT_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(binds.as_slice(), |r| {
                        crate::sqlite::records::StoredPlanAmendment::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Record the operator's ratification of a staged plan amendment (ADR 0003
    /// D5). Errors if the amendment is not currently `staged` — decisions are
    /// final history, never revised (a second decide errors; deciding a
    /// superseded row errors). Stamps `decided_at`/`decided_by`, sets `status`
    /// to `applied`/`rejected`, and persists the supplied `prior_state` snapshot
    /// (ADR D4 — the affected plan slice captured before the apply write; `None`
    /// on reject or for `promise_followup`).
    ///
    /// This records the decision + prior state **only**. Apply-dispatch to the
    /// class's plan write path is the service layer's job (Part B) — the
    /// repository decides, the service applies.
    pub async fn decide_plan_amendment(
        &self,
        id: &str,
        decision: PlanAmendmentDecision,
        decided_by: &str,
        prior_state: Option<String>,
    ) -> Result<crate::sqlite::records::StoredPlanAmendment> {
        let id_owned = id.to_string();
        let id_lookup = id_owned.clone();
        let decided_by = decided_by.to_string();
        let new_status = decision.status();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                // Guard: only a `staged` row may be decided. The status
                // predicate in the UPDATE makes the transition atomic; a zero
                // rowcount means the row was absent or already terminal.
                let affected = conn.execute(
                    "UPDATE plan_amendment \
                     SET status = ?1, decided_at = ?2, decided_by = ?3, prior_state = ?4, \
                         updated_at = ?2 \
                     WHERE id = ?5 AND status = 'staged'",
                    rusqlite::params![new_status, now, decided_by, prior_state, id_owned],
                )?;
                Ok(affected)
            })
            .await
            .and_then(|affected| {
                if affected == 0 {
                    Err(anyhow!(
                        "plan amendment '{id_lookup}' is not staged (already decided or absent — \
                         decisions are final)"
                    ))
                } else {
                    Ok(())
                }
            })?;
        self.read_plan_amendment(&id_lookup)
            .await?
            .ok_or_else(|| anyhow!("plan_amendment '{id_lookup}' vanished after decide"))
    }

    /// Supersede-on-replan (ADR 0003 D2): flip only this source chapter's
    /// `staged` plan amendments to `superseded`. `applied`/`rejected`/already-
    /// `superseded` rows are untouched — decisions are history. Scoped to one
    /// branch + book + source chapter so a rerun for a different chapter never
    /// touches these. Returns the number of rows flipped.
    pub async fn supersede_source_chapter_amendments(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        source_chapter: i32,
    ) -> Result<u64> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                let affected = conn.execute(
                    "UPDATE plan_amendment SET status = 'superseded', updated_at = ?1 \
                     WHERE project_id = ?2 AND branch_id = ?3 AND book_number = ?4 \
                       AND source_chapter = ?5 AND status = 'staged'",
                    rusqlite::params![now, project_id, branch_id, book_number, source_chapter],
                )?;
                Ok(affected as u64)
            })
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn annotate_scene_beats(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        motif_ids: Vec<String>,
        theme_ids: Vec<String>,
        conflict_ids: Vec<String>,
        beats: Vec<AnnotatedBeat>,
        intensity: Option<f64>,
    ) -> Result<SceneBeatAnnotation> {
        // Preserve a previously recorded intensity when this re-annotation omits
        // it (annotate is DELETE-then-INSERT, so a bare None would erase it).
        let effective_intensity = match intensity {
            Some(value) => Some(value),
            None => self
                .get_scene_beat_annotation(branch_id, scene_id)
                .await?
                .and_then(|annotation| annotation.intensity),
        };
        let stored_beats: Vec<StoredAnnotatedBeat> = beats.into_iter().map(Into::into).collect();
        let beats_json = serde_json::to_string(&stored_beats)?;
        let motif_json = serde_json::to_string(&motif_ids)?;
        let theme_json = serde_json::to_string(&theme_ids)?;
        let conflict_json = serde_json::to_string(&conflict_ids)?;
        let id = mint_id("scene_beat_annotation");
        let now = timestamp_to_micros(chrono::Utc::now());
        let project_id_owned = project_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let scene_id_owned = scene_id.to_string();

        self.inner
            .pool
            .write({
                let branch_id = branch_id_owned.clone();
                let scene_id = scene_id_owned.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM scene_beat_annotation \
                         WHERE branch_id = ?1 AND scene_id = ?2",
                        rusqlite::params![&branch_id, &scene_id],
                    )?;
                    tx.execute(
                        "INSERT INTO scene_beat_annotation (id, project_id, branch_id, scene_id, \
                         beats, motif_ids, theme_ids, conflict_ids, created_at, updated_at, intensity) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
                        rusqlite::params![
                            &id,
                            &project_id_owned,
                            &branch_id,
                            &scene_id,
                            &beats_json,
                            &motif_json,
                            &theme_json,
                            &conflict_json,
                            now,
                            effective_intensity,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_scene_beat_annotation(branch_id, scene_id)
            .await?
            .ok_or_else(|| anyhow!("scene_beat_annotation vanished after upsert"))
    }

    pub async fn get_scene_beat_annotation(
        &self,
        branch_id: &str,
        scene_id: &str,
    ) -> Result<Option<SceneBeatAnnotation>> {
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_BEAT_ANNOTATION_COLUMNS} FROM scene_beat_annotation \
                     WHERE branch_id = ?1 AND scene_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&branch_id, &scene_id], |r| {
                    SceneBeatAnnotation::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    // =========================================================================
    // Pacing: config, curve, tracker
    // =========================================================================

    pub async fn create_pacing_config(
        &self,
        input: &CreatePacingConfigInput,
    ) -> Result<PacingConfig> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("pacing_config");
        let project_id = input.project_id.clone();
        let branch_id_out = branch_id.clone();
        let total_planned_books = input.total_planned_books;
        let avg_chapters = input.avg_chapters_per_book;
        let avg_scenes = input.avg_scenes_per_chapter;
        let tension_model = input.tension_model.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO pacing_config (id, project_id, branch_id, total_planned_books, \
                     avg_chapters_per_book, avg_scenes_per_chapter, tension_model, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
                     ON CONFLICT (project_id, branch_id) DO UPDATE SET \
                        total_planned_books = excluded.total_planned_books, \
                        avg_chapters_per_book = excluded.avg_chapters_per_book, \
                        avg_scenes_per_chapter = excluded.avg_scenes_per_chapter, \
                        tension_model = excluded.tension_model, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id, &project_id, &branch_id, total_planned_books, avg_chapters,
                        avg_scenes, &tension_model, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        // Read back by the upsert key, not the minted id: on the conflict path the
        // pre-existing row keeps its original id, so the minted id may not exist.
        self.get_pacing_config_for_branch(&input.project_id, &branch_id_out)
            .await
    }

    pub async fn get_pacing_config(&self, id: &str) -> Result<PacingConfig> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {PACING_CONFIG_COLUMNS} FROM pacing_config WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PacingConfig::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_config not found"))
    }

    pub(super) async fn get_pacing_config_for_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<PacingConfig> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_CONFIG_COLUMNS} FROM pacing_config \
                     WHERE project_id = ?1 AND branch_id = ?2"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&project_id, &branch_id], |r| PacingConfig::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_config not found"))
    }

    pub async fn create_pacing_curve(&self, input: &CreatePacingCurveInput) -> Result<PacingCurve> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("pacing_curve");
        let project_id = input.project_id.clone();
        let branch_id_out = branch_id.clone();
        let book_number = input.book_number;
        let act_breakpoints_json = serde_json::to_string(&input.act_breakpoints)?;
        let scene_density_json = serde_json::to_string(&input.scene_type_density)?;
        let intensity_points: Vec<StoredIntensityPoint> = input
            .intensity_points
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let intensity_points_json =
            serde_json::to_string(&intensity_points).context("serializing intensity_points")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO pacing_curve (id, project_id, branch_id, book_number, \
                     act_breakpoints, scene_type_density, created_at, updated_at, intensity_points) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8) \
                     ON CONFLICT (project_id, branch_id, book_number) DO UPDATE SET \
                        act_breakpoints = excluded.act_breakpoints, \
                        scene_type_density = excluded.scene_type_density, \
                        intensity_points = excluded.intensity_points, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        book_number,
                        &act_breakpoints_json,
                        &scene_density_json,
                        now,
                        &intensity_points_json,
                    ],
                )?;
                Ok(())
            })
            .await?;
        // Read back by the upsert key, not the minted id: on the conflict path the
        // pre-existing row keeps its original id, so the minted id may not exist.
        self.get_pacing_curve_for_branch(&input.project_id, &branch_id_out, book_number)
            .await
    }

    pub async fn get_pacing_curve(&self, id: &str) -> Result<PacingCurve> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PACING_CURVE_COLUMNS} FROM pacing_curve WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PacingCurve::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_curve not found"))
    }

    pub(super) async fn get_pacing_curve_for_branch(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
    ) -> Result<PacingCurve> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_CURVE_COLUMNS} FROM pacing_curve \
                     WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&project_id, &branch_id, book_number],
                    |r| PacingCurve::try_from(r),
                )
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_curve not found"))
    }

    pub async fn create_pacing_tracker(
        &self,
        project_id: &str,
        character_arc_id: &str,
    ) -> Result<PacingTracker> {
        let branch_id = self.active_branch_id(project_id).await?;
        let id = mint_id("pacing_tracker");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let character_arc_id = character_arc_id.to_string();
        let empty_budget_json = "{}".to_string();
        let empty_warnings_json = "[]".to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO pacing_tracker (id, project_id, branch_id, character_arc_id, \
                     per_book_budget, max_progress_per_chapter, milestone_spacing, sprint_allowance, \
                     regression_budget, current_progress, budget_remaining, velocity, status, \
                     next_milestone, warnings, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, 0.0, 1.0, 'normal', \
                             'on_track', NULL, ?6, ?7)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &character_arc_id, &empty_budget_json,
                        &empty_warnings_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_pacing_tracker(&id_out).await
    }

    /// Look up a pacing tracker by its character_arc_id (1:1 relationship).
    pub async fn get_pacing_tracker_by_arc(&self, character_arc_id: &str) -> Result<PacingTracker> {
        let character_arc_id = character_arc_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_TRACKER_COLUMNS} FROM pacing_tracker \
                     WHERE character_arc_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&character_arc_id], |r| PacingTracker::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_tracker not found"))
    }

    pub async fn get_pacing_tracker(&self, id: &str) -> Result<PacingTracker> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {PACING_TRACKER_COLUMNS} FROM pacing_tracker WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PacingTracker::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_tracker not found"))
    }

    /// Update the pacing-constraint fields on a pacing_tracker. Used by
    /// the set_arc_pacing_constraints service tool.
    pub async fn set_arc_pacing_constraints(
        &self,
        pacing_tracker_id: &str,
        per_book_budget: std::collections::BTreeMap<String, f64>,
        max_progress_per_chapter: Option<f64>,
        milestone_spacing: Option<i32>,
        sprint_allowance: Option<i32>,
        regression_budget: Option<f64>,
    ) -> Result<PacingTracker> {
        let id = pacing_tracker_id.to_string();
        let id_out = id.clone();
        let per_book_budget_json = serde_json::to_string(&per_book_budget)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE pacing_tracker SET per_book_budget = ?1, \
                     max_progress_per_chapter = ?2, milestone_spacing = ?3, \
                     sprint_allowance = ?4, regression_budget = ?5, updated_at = ?6 \
                     WHERE id = ?7",
                    rusqlite::params![
                        &per_book_budget_json,
                        max_progress_per_chapter,
                        milestone_spacing,
                        sprint_allowance,
                        regression_budget,
                        now,
                        &id,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_pacing_tracker(&id_out).await
    }

    pub async fn update_pacing_tracker_progress(
        &self,
        tracker_id: &str,
        current_progress: f64,
        budget_remaining: f64,
        status: &str,
    ) -> Result<()> {
        let tracker_id = tracker_id.to_string();
        let status = status.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE pacing_tracker SET current_progress = ?1, budget_remaining = ?2, \
                     status = ?3, updated_at = ?4 WHERE id = ?5",
                    rusqlite::params![
                        current_progress,
                        budget_remaining,
                        &status,
                        timestamp_to_micros(chrono::Utc::now()),
                        &tracker_id,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Recompute realized pacing for every arc tracker on the branch from the
    /// number of chapter summaries in `book_number`. Each chapter advances the
    /// arc by `max_progress_per_chapter`; when realized progress exceeds the
    /// book's `per_book_budget` the tracker goes over budget (surfaced by
    /// `pacing_budget_audit`). Deterministic and idempotent; trackers without a
    /// budget for this book are left untouched.
    pub async fn recompute_pacing_for_book(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
    ) -> Result<()> {
        let chapter_count = {
            let project_id = project_id.to_string();
            let branch_id = branch_id.to_string();
            self.inner
                .pool
                .read(move |conn| {
                    let count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM chapter_summary \
                         WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3",
                        rusqlite::params![&project_id, &branch_id, book_number],
                        |r| r.get(0),
                    )?;
                    Ok(count)
                })
                .await?
        };
        let trackers = self
            .list_pacing_trackers_by_project_and_branch(project_id, branch_id)
            .await?;
        let book_key = book_number.to_string();
        for tracker in &trackers {
            let Some(book_budget) = tracker.per_book_budget.get(&book_key).copied() else {
                continue;
            };
            let rate = tracker.max_progress_per_chapter.unwrap_or(0.1);
            let realized = chapter_count as f64 * rate;
            let budget_remaining = book_budget - realized;
            let current_progress = realized.min(1.5);
            let status = if budget_remaining < 0.0 {
                "ahead"
            } else {
                "on_track"
            };
            self.update_pacing_tracker_progress(
                &tracker.id,
                current_progress,
                budget_remaining,
                status,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn list_pacing_trackers_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<PacingTracker>> {
        list_branch_scoped(
            &self.inner.pool,
            PACING_TRACKER_COLUMNS,
            "pacing_tracker",
            "updated_at DESC",
            project_id,
            branch_id,
            |r| PacingTracker::try_from(r),
            |t| t.character_arc_id.clone(),
        )
        .await
    }

    /// List every pacing_config row for a project, across branches. The
    /// import-hydration pass uses this to decide whether to create a fresh
    /// config when seeding pacing.
    pub async fn list_pacing_configs_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<PacingConfig>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_CONFIG_COLUMNS} FROM pacing_config \
                     WHERE project_id = ?1 ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PacingConfig::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every pacing_curve row for a project, across branches.
    pub async fn list_pacing_curves_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<PacingCurve>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_CURVE_COLUMNS} FROM pacing_curve \
                     WHERE project_id = ?1 ORDER BY book_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PacingCurve::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Chapter summary (save_summary)
    // =========================================================================

    pub async fn save_summary(&self, input: &SaveSummaryInput) -> Result<ChapterSummary> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let project_id = input.project_id.clone();
        let book_number = input.book_number;
        let chapter_number = input.chapter_number;
        let summary = input.summary.clone();
        let key_events_json = serde_json::to_string(&input.key_events)?;
        let character_changes_json = serde_json::to_string(&input.character_changes)?;
        let relationship_shifts_json = serde_json::to_string(&input.relationship_shifts)?;
        let arc_advances_json = serde_json::to_string(&input.arc_advances)?;
        let promise_events_json = serde_json::to_string(&input.promise_events)?;
        let id = mint_id("chapter_summary");
        let digest_id = mint_id("book_digest");
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM chapter_summary \
                         WHERE project_id = ?1 AND branch_id = ?2 \
                           AND book_number = ?3 AND chapter_number = ?4",
                        rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                    )?;
                    tx.execute(
                        "INSERT INTO chapter_summary (id, project_id, branch_id, book_number, \
                         chapter_number, summary, key_events, character_changes, relationship_shifts, \
                         arc_advances, promise_events, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                        rusqlite::params![
                            &id, &project_id, &branch_id, book_number, chapter_number, &summary,
                            &key_events_json, &character_changes_json, &relationship_shifts_json,
                            &arc_advances_json, &promise_events_json, now,
                        ],
                    )?;

                    // Atomically re-derive this book's "story so far" digest from
                    // all of its chapter summaries (idempotent; deterministic —
                    // a model compaction pass can replace the cap later).
                    const BOOK_DIGEST_CHAR_CAP: usize = 6000;
                    let mut parts: Vec<(i32, String)> = Vec::new();
                    {
                        let mut stmt = tx.prepare(
                            "SELECT chapter_number, summary FROM chapter_summary \
                             WHERE project_id = ?1 AND branch_id = ?2 AND book_number = ?3 \
                             ORDER BY chapter_number",
                        )?;
                        let mut rows = stmt
                            .query(rusqlite::params![&project_id, &branch_id, book_number])?;
                        while let Some(row) = rows.next()? {
                            let chapter: i64 = row.get(0)?;
                            let chapter_summary: String = row.get(1)?;
                            parts.push((chapter as i32, chapter_summary));
                        }
                    }
                    let last_chapter_covered =
                        parts.iter().map(|(chapter, _)| *chapter).max().unwrap_or(0);
                    let (synopsis, truncated) =
                        crate::format::build_book_synopsis(&parts, BOOK_DIGEST_CHAR_CAP);
                    let token_estimate = (synopsis.len() / 4) as i64;

                    // Derive the still-open narrative threads (promises, conflicts,
                    // plot lines) so distant thread state survives into later-book
                    // drafting via [STORY SO FAR]. Deterministic ordering lives in
                    // `build_open_threads`.
                    let promises = {
                        let mut stmt = tx.prepare(&format!(
                            "SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise \
                             WHERE project_id = ?1 AND branch_id = ?2 AND archived_at IS NULL"
                        ))?;
                        stmt.query_map(rusqlite::params![&project_id, &branch_id], |r| {
                            NarrativePromise::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    let conflicts = {
                        let mut stmt = tx.prepare(&format!(
                            "SELECT {CONFLICT_COLUMNS} FROM conflict \
                             WHERE project_id = ?1 AND branch_id = ?2 AND archived_at IS NULL"
                        ))?;
                        stmt.query_map(rusqlite::params![&project_id, &branch_id], |r| {
                            Conflict::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    let plot_lines = {
                        let mut stmt = tx.prepare(&format!(
                            "SELECT {PLOT_LINE_COLUMNS} FROM plot_line \
                             WHERE project_id = ?1 AND branch_id = ?2 AND archived_at IS NULL"
                        ))?;
                        stmt.query_map(rusqlite::params![&project_id, &branch_id], |r| {
                            PlotLine::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    let current_index =
                        crate::format::story_index(book_number, last_chapter_covered, 0);
                    let open_threads = crate::format::build_open_threads(
                        &promises,
                        &conflicts,
                        &plot_lines,
                        current_index,
                    );
                    let open_threads_json = serde_json::to_string(&open_threads)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                    tx.execute(
                        "INSERT INTO book_digest (id, project_id, branch_id, book_number, synopsis, \
                         open_threads, last_chapter_covered, token_estimate, truncated, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
                         ON CONFLICT(project_id, branch_id, book_number) DO UPDATE SET \
                           synopsis = excluded.synopsis, \
                           open_threads = excluded.open_threads, \
                           last_chapter_covered = excluded.last_chapter_covered, \
                           token_estimate = excluded.token_estimate, \
                           truncated = excluded.truncated, \
                           updated_at = excluded.updated_at",
                        rusqlite::params![
                            &digest_id, &project_id, &branch_id, book_number, &synopsis,
                            &open_threads_json, last_chapter_covered, token_estimate,
                            truncated as i32, now,
                        ],
                    )?;

                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_chapter_summary(&project_id, &branch_id, book_number, chapter_number)
            .await?
            .ok_or_else(|| anyhow!("chapter_summary vanished after save"))
    }

    /// All per-book "story so far" digests on the branch, ordered by book.
    pub async fn list_book_digests_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<crate::sqlite::records::StoredBookDigest>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {} FROM book_digest \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number",
                    crate::sqlite::records::BOOK_DIGEST_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for candidate_branch_id in &chain {
                    let rows = stmt
                        .query_map([&project_id, candidate_branch_id], |r| {
                            crate::sqlite::records::StoredBookDigest::try_from(r)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for row in rows {
                        if seen.insert(row.book_number) {
                            out.push(row);
                        }
                    }
                }
                out.sort_by_key(|d| d.book_number);
                Ok(out)
            })
            .await
    }

    pub async fn get_chapter_summary(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Option<ChapterSummary>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
                let sql = format!(
                    "SELECT {CHAPTER_SUMMARY_COLUMNS} FROM chapter_summary \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                for candidate_branch_id in &chain {
                    if let Some(summary) = stmt
                        .query_row(
                            rusqlite::params![
                                &project_id,
                                candidate_branch_id,
                                book_number,
                                chapter_number
                            ],
                            |r| ChapterSummary::try_from(r),
                        )
                        .optional_inner()?
                    {
                        return Ok(Some(summary));
                    }
                }
                Ok(None)
            })
            .await
    }

    // =========================================================================
    // Book / chapter outlines (one row per (target, branch) pair)
    // =========================================================================

    // =========================================================================
    // Chapter planning
    // =========================================================================

    /// Plan a chapter: ensure book + chapter exist by number, then upsert
    /// the chapter_plan row and the chapter_outline that mirrors it. The
    /// SurrealDB version did all of this in one multi-statement BEGIN/COMMIT;
    /// the SQLite version uses a single writer transaction for the same
    /// atomicity. Returns the persisted ChapterPlan.
    pub async fn plan_chapter(&self, input: &PlanChapterInput) -> Result<ChapterPlan> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let chapter = self
            .ensure_chapter(&input.project_id, input.book_number, input.chapter_number)
            .await?;

        let project_id = input.project_id.clone();
        let book_number = input.book_number;
        let chapter_number = input.chapter_number;
        let pov_character_id = input.pov_character_id.clone();
        let synopsis = input.synopsis.clone();
        let target_theme_ids_json = serde_json::to_string(&input.target_theme_ids)?;
        let target_conflict_ids_json = serde_json::to_string(&input.target_conflict_ids)?;
        let target_plot_line_ids_json = serde_json::to_string(&input.target_plot_line_ids)?;

        let planned_scenes: Vec<PlannedScene> = input
            .scenes
            .iter()
            .map(|s: &PlanChapterSceneInput| PlannedScene {
                scene_order: s.scene_order,
                summary: s.summary.clone(),
                beat_structure: s.beat_structure.clone(),
                character_ids: s.character_ids.clone(),
                location_id: s.location_id.clone(),
                content_rating: s.content_rating.clone(),
                purpose: s.purpose.clone(),
                research_required: s.research_required,
                research_tags: s.research_tags.clone(),
                explicit_query: s.explicit_query.clone(),
            })
            .collect();
        let scenes_json = serde_json::to_string(&planned_scenes)?;

        // chapter_outline mirrors the plan as ChapterOutlineBeat[] for the
        // outline view. Same shape every time so we synthesize from scenes.
        let outline_beats: Vec<ChapterOutlineBeat> = input
            .scenes
            .iter()
            .map(|s| ChapterOutlineBeat {
                order: s.scene_order,
                summary: s.summary.clone(),
                scene_id: None,
                status: "planned".to_string(),
            })
            .collect();
        let outline_content = serde_json::to_string(&outline_beats)?;

        let plan_id = mint_id("chapter_plan");
        let outline_id = mint_id("chapter_outline");
        let plan_id_lookup = plan_id.clone();
        let chapter_id = chapter.id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        // The outline goes through set_chapter_outline (which already
        // implements the proper INSERT ON CONFLICT), then we do the plan
        // INSERT in a separate transaction. Two writes is fine — both target
        // the writer connection and SQLite gives us per-statement durability.
        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    // Upsert the chapter_plan row keyed by (project, branch,
                    // book#, chapter#) — the schema's UNIQUE index.
                    tx.execute(
                        "DELETE FROM chapter_plan \
                         WHERE project_id = ?1 AND branch_id = ?2 \
                           AND book_number = ?3 AND chapter_number = ?4",
                        rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                    )?;
                    tx.execute(
                        "INSERT INTO chapter_plan (id, project_id, branch_id, book_number, \
                         chapter_number, pov_character_id, synopsis, target_theme_ids, \
                         target_conflict_ids, target_plot_line_ids, scenes, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                        rusqlite::params![
                            &plan_id, &project_id, &branch_id, book_number, chapter_number,
                            &pov_character_id, &synopsis, &target_theme_ids_json,
                            &target_conflict_ids_json, &target_plot_line_ids_json, &scenes_json,
                            now,
                        ],
                    )?;
                    // Upsert the chapter_outline for the same (chapter, branch).
                    tx.execute(
                        "INSERT INTO chapter_outline (id, chapter_id, branch_id, format, content, beats, updated_at) \
                         VALUES (?1, ?2, ?3, 'json', ?4, ?4, ?5) \
                         ON CONFLICT (chapter_id, branch_id) DO UPDATE SET \
                            format = excluded.format, \
                            content = excluded.content, \
                            beats = excluded.beats, \
                            updated_at = excluded.updated_at",
                        rusqlite::params![
                            &outline_id, &chapter_id, &branch_id, &outline_content, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_chapter_plan(&plan_id_lookup).await
    }

    /// Set `chapter_plan.plan_revision` for one chapter on the active branch
    /// (ADR 0003 D4). Used by the plan-amendment apply dispatcher: the
    /// [`Self::plan_chapter`] replay path rewrites the row and resets the column
    /// to NULL (it is not in the INSERT list), so the dispatcher writes the
    /// incremented value here AFTER the replay to make the increment survive.
    /// Idempotent absolute write (not a `+= 1`); the caller computes the target
    /// value. Returns the affected row count.
    pub async fn set_chapter_plan_revision(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        plan_revision: i64,
    ) -> Result<u64> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                let affected = conn.execute(
                    "UPDATE chapter_plan SET plan_revision = ?1, updated_at = ?2 \
                     WHERE project_id = ?3 AND branch_id = ?4 \
                       AND book_number = ?5 AND chapter_number = ?6",
                    rusqlite::params![
                        plan_revision,
                        now,
                        project_id,
                        branch_id,
                        book_number,
                        chapter_number
                    ],
                )?;
                Ok(affected as u64)
            })
            .await
    }

    /// Active-branch list of every chapter_plan in a project, ordered by
    /// (book#, chapter#).
    pub async fn list_chapter_plans_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ChapterPlan>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let mut plans = list_branch_scoped(
            &self.inner.pool,
            CHAPTER_PLAN_COLUMNS,
            "chapter_plan",
            "book_number, chapter_number",
            project_id,
            &branch_id,
            |r| ChapterPlan::try_from(r),
            |p| (p.book_number, p.chapter_number),
        )
        .await?;
        plans.sort_by_key(|p| (p.book_number, p.chapter_number));
        Ok(plans)
    }

    pub async fn delete_chapter_plans_for_chapter(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM chapter_plan \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4",
                    rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_chapter_summaries_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ChapterSummary>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let mut summaries = list_branch_scoped(
            &self.inner.pool,
            CHAPTER_SUMMARY_COLUMNS,
            "chapter_summary",
            "book_number, chapter_number",
            project_id,
            &branch_id,
            |r| ChapterSummary::try_from(r),
            |s| (s.book_number, s.chapter_number),
        )
        .await?;
        summaries.sort_by_key(|s| (s.book_number, s.chapter_number));
        Ok(summaries)
    }

    pub async fn delete_chapter_summaries_for_chapter(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM chapter_summary \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4",
                    rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_scene_beat_annotations_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<SceneBeatAnnotation>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_BEAT_ANNOTATION_COLUMNS} FROM scene_beat_annotation \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY scene_id"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        SceneBeatAnnotation::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_chapter_plan(&self, id: &str) -> Result<ChapterPlan> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CHAPTER_PLAN_COLUMNS} FROM chapter_plan WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ChapterPlan::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("chapter_plan not found"))
    }

    pub async fn get_book_outline(
        &self,
        book_id: &str,
        branch_id: &str,
    ) -> Result<Option<BookOutline>> {
        let book_id = book_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_OUTLINE_COLUMNS} FROM book_outline \
                     WHERE book_id = ?1 AND branch_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&book_id, &branch_id], |r| {
                    BookOutline::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn set_book_outline(
        &self,
        book_id: &str,
        branch_id: &str,
        format: &str,
        content: &str,
    ) -> Result<BookOutline> {
        let book_id_owned = book_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let format = format.to_string();
        let content = content.to_string();
        let new_id = mint_id("book_outline");
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO book_outline (id, book_id, branch_id, format, content, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                     ON CONFLICT (book_id, branch_id) DO UPDATE SET \
                        format = excluded.format, \
                        content = excluded.content, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![&new_id, &book_id_owned, &branch_id_owned, &format, &content, now],
                )?;
                Ok(())
            })
            .await?;
        self.get_book_outline(book_id, branch_id)
            .await?
            .ok_or_else(|| anyhow!("book_outline vanished after upsert"))
    }

    pub async fn get_chapter_outline(
        &self,
        chapter_id: &str,
        branch_id: &str,
    ) -> Result<Option<ChapterOutline>> {
        let chapter_id = chapter_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_OUTLINE_COLUMNS} FROM chapter_outline \
                     WHERE chapter_id = ?1 AND branch_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&chapter_id, &branch_id], |r| {
                    ChapterOutline::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn set_chapter_outline(
        &self,
        chapter_id: &str,
        branch_id: &str,
        format: &str,
        content: &str,
        beats: Vec<ChapterOutlineBeat>,
    ) -> Result<ChapterOutline> {
        let chapter_id_owned = chapter_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let format = format.to_string();
        let content = content.to_string();
        let stored_beats: Vec<StoredChapterOutlineBeat> =
            beats.into_iter().map(Into::into).collect();
        let beats_json = serde_json::to_string(&stored_beats)?;
        let new_id = mint_id("chapter_outline");
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO chapter_outline (id, chapter_id, branch_id, format, content, beats, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT (chapter_id, branch_id) DO UPDATE SET \
                        format = excluded.format, \
                        content = excluded.content, \
                        beats = excluded.beats, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &new_id,
                        &chapter_id_owned,
                        &branch_id_owned,
                        &format,
                        &content,
                        &beats_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_chapter_outline(chapter_id, branch_id)
            .await?
            .ok_or_else(|| anyhow!("chapter_outline vanished after upsert"))
    }

    /// List every pacing_tracker row on the project's active branch. Mirrors
    /// the `list_pacing_trackers_by_project_and_branch` shape resolved
    /// against `active_branch_id`; used by the `pacing/overview` project
    /// resource reader and other branch-implicit callers.
    pub async fn list_pacing_trackers_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<PacingTracker>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_pacing_trackers_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_all_pacing_trackers(&self, project_id: &str) -> Result<Vec<PacingTracker>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_TRACKER_COLUMNS} FROM pacing_tracker \
                     WHERE project_id = ?1 ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PacingTracker::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_book_outlines_by_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<BookOutline>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {BOOK_OUTLINE_COLUMNS} FROM book_outline \
                     WHERE branch_id IN ({placeholders}) \
                     ORDER BY branch_id, book_id"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        BookOutline::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_chapter_outlines_by_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<ChapterOutline>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {CHAPTER_OUTLINE_COLUMNS} FROM chapter_outline \
                     WHERE branch_id IN ({placeholders}) \
                     ORDER BY branch_id, chapter_id"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ChapterOutline::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_chapter_plans(&self, project_id: &str) -> Result<Vec<ChapterPlan>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_PLAN_COLUMNS} FROM chapter_plan \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ChapterPlan::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_scene_beat_annotations(
        &self,
        project_id: &str,
    ) -> Result<Vec<SceneBeatAnnotation>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_BEAT_ANNOTATION_COLUMNS} FROM scene_beat_annotation \
                     WHERE project_id = ?1 ORDER BY scene_id"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SceneBeatAnnotation::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_chapter_summaries(
        &self,
        project_id: &str,
    ) -> Result<Vec<ChapterSummary>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_SUMMARY_COLUMNS} FROM chapter_summary \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ChapterSummary::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
