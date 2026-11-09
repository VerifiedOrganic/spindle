use super::canon_backfill::mark_legacy_rows_mapped_in_conn;
use super::canon_policy::{legacy_claim, legacy_snapshot_claim, mirror_legacy_claims_in_tx};
use super::*;
use spindle_core::canon::*;

impl Repository {
    pub async fn create_plot_line(&self, input: &CreatePlotLineInput) -> Result<PlotLine> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("plot_line");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let plot_type = input.plot_type.clone();
        let summary = input.summary.clone();
        let status = input
            .status
            .clone()
            .unwrap_or_else(|| "planted".to_string());
        let convergence_points: Vec<StoredStoryPlacement> = input
            .convergence_points
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let convergence_json =
            serde_json::to_string(&convergence_points).context("serializing convergence_points")?;
        let connected_conflict_json = serde_json::to_string(&input.connected_conflict_ids)
            .context("serializing connected_conflict_ids")?;
        let connected_theme_json = serde_json::to_string(&input.connected_theme_ids)
            .context("serializing connected_theme_ids")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO plot_line (id, project_id, branch_id, name, normalized_name, \
                     plot_type, summary, status, convergence_points, notes, archived_at, \
                     created_at, updated_at, connected_conflict_ids, connected_theme_ids) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?10, ?11, ?12)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &plot_type,
                        &summary,
                        &status,
                        &convergence_json,
                        now,
                        &connected_conflict_json,
                        &connected_theme_json,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_plot_line(&id_out).await
    }

    pub async fn get_plot_line(&self, id: &str) -> Result<PlotLine> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PLOT_LINE_COLUMNS} FROM plot_line WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PlotLine::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("plot_line not found"))
    }

    pub async fn create_conflict(&self, input: &CreateConflictInput) -> Result<Conflict> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("conflict");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let conflict_type = input.conflict_type.clone();
        let stakes = input.stakes.clone();
        let escalation_stages_json = serde_json::to_string(&input.escalation_stages)
            .context("serializing escalation_stages")?;
        let try_fail_cycles: Vec<StoredTryFailCycleStep> = input
            .try_fail_cycles
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let try_fail_json =
            serde_json::to_string(&try_fail_cycles).context("serializing try_fail_cycles")?;
        let stated_consequences: Vec<StoredStatedConsequence> = input
            .stated_consequences
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let consequences_json = serde_json::to_string(&stated_consequences)
            .context("serializing stated_consequences")?;
        let expected_total_cycles = input.expected_total_cycles;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO conflict (id, project_id, branch_id, name, normalized_name, \
                     conflict_type, stakes, escalation_stages, expected_total_cycles, \
                     try_fail_cycles, stated_consequences, resolution_summary, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL, ?12, ?12)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &name, &normalized,
                        &conflict_type, &stakes, &escalation_stages_json,
                        expected_total_cycles, &try_fail_json, &consequences_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_conflict(&id_out).await
    }

    pub async fn get_conflict(&self, id: &str) -> Result<Conflict> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CONFLICT_COLUMNS} FROM conflict WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Conflict::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("conflict not found"))
    }

    pub async fn create_theme(&self, input: &CreateThemeInput) -> Result<Theme> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("theme");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let theme_statement = input.theme_statement.clone();
        let thesis_antithesis = input.thesis_antithesis.clone();
        let introduction_point: Option<StoredStoryPlacement> =
            input.introduction_point.clone().map(Into::into);
        let resolution_point: Option<StoredStoryPlacement> =
            input.resolution_point.clone().map(Into::into);
        let introduction_json = match introduction_point.as_ref() {
            Some(p) => Some(serde_json::to_string(p).context("serializing introduction_point")?),
            None => None,
        };
        let resolution_json = match resolution_point.as_ref() {
            Some(p) => Some(serde_json::to_string(p).context("serializing resolution_point")?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO theme (id, project_id, branch_id, theme_statement, \
                     thesis_antithesis, introduction_point, resolution_point, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?8)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &theme_statement,
                        &thesis_antithesis,
                        &introduction_json,
                        &resolution_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_theme(&id_out).await
    }

    pub async fn get_theme(&self, id: &str) -> Result<Theme> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {THEME_COLUMNS} FROM theme WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Theme::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("theme not found"))
    }

    pub async fn create_motif(&self, input: &CreateMotifInput) -> Result<Motif> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("motif");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let description = input.description.clone();
        let max_uses_per_chapter = input.max_uses_per_chapter;
        let connected_theme_json = serde_json::to_string(&input.connected_theme_ids)
            .context("serializing connected_theme_ids")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO motif (id, project_id, branch_id, name, normalized_name, \
                     description, max_uses_per_chapter, connected_theme_ids, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &description,
                        max_uses_per_chapter,
                        &connected_theme_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_motif(&id_out).await
    }

    pub async fn get_motif(&self, id: &str) -> Result<Motif> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {MOTIF_COLUMNS} FROM motif WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Motif::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("motif not found"))
    }

    pub async fn create_narrative_promise(
        &self,
        input: &CreateNarrativePromiseInput,
    ) -> Result<NarrativePromise> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("narrative_promise");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let promise_type = input.promise_type.clone();
        let description = input.description.clone();
        let planted_at: StoredStoryPlacement = input.planted_at.clone().into();
        let planted_json = serde_json::to_string(&planted_at).context("serializing planted_at")?;
        let planned_payoff: Option<StoredStoryPlacement> =
            input.planned_payoff.clone().map(Into::into);
        let payoff_json = match planned_payoff.as_ref() {
            Some(p) => Some(serde_json::to_string(p).context("serializing planned_payoff")?),
            None => None,
        };
        let notes_json = serde_json::to_string(&input.notes).context("serializing notes")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO narrative_promise (id, project_id, branch_id, promise_type, \
                     description, status, planted_at, planned_payoff, notes, archived_at, \
                     created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'planted', ?6, ?7, ?8, NULL, ?9, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &promise_type,
                        &description,
                        &planted_json,
                        &payoff_json,
                        &notes_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_narrative_promise(&id_out).await
    }

    pub async fn get_narrative_promise(&self, id: &str) -> Result<NarrativePromise> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| NarrativePromise::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("narrative_promise not found"))
    }

    pub async fn list_plot_lines_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<PlotLine>> {
        list_branch_scoped(
            &self.inner.pool,
            PLOT_LINE_COLUMNS,
            "plot_line",
            "normalized_name",
            project_id,
            branch_id,
            |r| PlotLine::try_from(r),
            |p| p.normalized_name.clone(),
        )
        .await
    }

    pub async fn list_conflicts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Conflict>> {
        list_branch_scoped(
            &self.inner.pool,
            CONFLICT_COLUMNS,
            "conflict",
            "normalized_name",
            project_id,
            branch_id,
            |r| Conflict::try_from(r),
            |c| c.normalized_name.clone(),
        )
        .await
    }

    pub async fn list_themes_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Theme>> {
        list_branch_scoped(
            &self.inner.pool,
            THEME_COLUMNS,
            "theme",
            "created_at",
            project_id,
            branch_id,
            |r| Theme::try_from(r),
            |t| t.theme_statement.clone(),
        )
        .await
    }

    pub async fn list_motifs_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Motif>> {
        list_branch_scoped(
            &self.inner.pool,
            MOTIF_COLUMNS,
            "motif",
            "normalized_name",
            project_id,
            branch_id,
            |r| Motif::try_from(r),
            |m| m.normalized_name.clone(),
        )
        .await
    }

    pub async fn update_promise_status(
        &self,
        input: &spindle_core::models::UpdatePromiseStatusInput,
    ) -> Result<()> {
        use crate::format::{CHAPTER_RADIX, SCENE_RADIX, story_index, story_index_from_placement};
        use spindle_core::models::PromiseStatusEvent;
        anyhow::ensure!(
            !input.status.trim().is_empty(),
            "promise status must not be empty"
        );
        let input = input.clone();
        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<()> {
                    let tx = conn.transaction()?;
                    let mut promise = tx.query_row(
                        &format!("SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise WHERE id = ?1"),
                        [&input.narrative_promise_id], |row| NarrativePromise::try_from(row))?;
                    let mut at = input.at;
                    if let Some(scene_id) = &input.source_scene_id {
                        let scene = tx.query_row(&format!("SELECT {SCENE_COLUMNS} FROM scene WHERE id = ?1"),
                            [scene_id], |row| Scene::try_from(row))?;
                        let scene_lineage =
                            resolve_branch_lineage_in_conn(&tx, Some(&scene.project_id), &scene.branch_id)?;
                        anyhow::ensure!(
                            scene.project_id == promise.project_id
                                && scene_lineage.iter().any(|b| b == &promise.branch_id),
                            "promise evidence must belong to the same project and branch"
                        );
                        if let Some(at) = &at {
                            anyhow::ensure!(at.book_number == scene.book_number && at.chapter_number == scene.chapter_number
                                && at.scene_order.is_none_or(|order| order == scene.scene_order),
                                "promise placement must match its evidence scene");
                        }
                        at = Some(StoryPlacement { book_number: scene.book_number, chapter_number: scene.chapter_number,
                            scene_order: Some(scene.scene_order), note: at.and_then(|p| p.note) });
                    }
                    if let Some(at) = &at {
                        anyhow::ensure!(at.book_number > 0 && (0..CHAPTER_RADIX as i32).contains(&at.chapter_number)
                            && at.scene_order.is_none_or(|s| (0..SCENE_RADIX as i32).contains(&s)),
                            "promise placement is out of range");
                        anyhow::ensure!(story_index(at.book_number, at.chapter_number, at.scene_order.unwrap_or((SCENE_RADIX - 1) as i32))
                            >= story_index_from_placement(&promise.planted_at), "promise event cannot precede its planting");
                    }
                    if let Some(id) = &input.replaces_event_id {
                        anyhow::ensure!(promise.status_history.iter().any(|e| &e.id == id)
                            && !promise.status_history.iter().any(|e| e.replaces_event_id.as_ref() == Some(id)),
                            "replacement must name a current event of this promise");
                    }
                    let now = chrono::Utc::now();
                    let mirror_at = at.clone();
                    let mirror_project_id = promise.project_id.clone();
                    let mirror_branch_id = promise.branch_id.clone();
                    let mirror_promise_id = promise.id.clone();
                    let mirror_status = input.status.clone();
                    let mirror_replaces_event_id = input.replaces_event_id.clone();
                    let mirror_source_scene_id = input.source_scene_id.clone();
                    promise.status_history.push(PromiseStatusEvent {
                        id: mint_id("promise_event"), previous_status: promise.status.clone(),
                        status: input.status.clone(), at, source_scene_id: input.source_scene_id,
                        note: input.note, replaces_event_id: input.replaces_event_id, recorded_at: now.to_rfc3339(),
                    });
                    let status = promise.status_at(i64::MAX).unwrap_or(&input.status);
                    tx.execute("UPDATE narrative_promise SET status = ?1, status_history = ?2, updated_at = ?3 WHERE id = ?4",
                        rusqlite::params![status, serde_json::to_string(&promise.status_history)?, timestamp_to_micros(now), &promise.id])?;
                    let operation = match mirror_status.trim().to_ascii_lowercase().as_str() {
                        "plant" | "planted" => PromiseOperation::Plant,
                        "payoff" | "paid" | "paid_off" | "paid-off" | "fulfilled" => {
                            PromiseOperation::Payoff
                        }
                        "abandon" | "abandoned" => PromiseOperation::Abandon,
                        _ => PromiseOperation::Reinforce,
                    };
                    let revision_id = format!(
                        "legacy:promise:{}:{}:{}:{mirror_replaces_event_id:?}",
                        mirror_promise_id, mirror_status, mirror_at.is_some()
                    );
                    let placement = mirror_at.unwrap_or(StoryPlacement {
                        book_number: 1,
                        chapter_number: 1,
                        scene_order: None,
                        note: None,
                    });
                    let claim = legacy_claim(
                        &mirror_project_id,
                        &mirror_branch_id,
                        &revision_id,
                        mirror_source_scene_id.as_deref(),
                        &placement,
                        ClaimEntity::Resolved {
                            id: mirror_promise_id.clone(),
                        },
                        ClaimAssertion::Promise {
                            operation,
                            thread: ClaimEntity::Resolved {
                                id: mirror_promise_id.clone(),
                            },
                        },
                        None,
                        1,
                    );
                    let snapshot_claim = legacy_snapshot_claim(
                        &mirror_project_id,
                        &mirror_branch_id,
                        &revision_id,
                        mirror_source_scene_id.as_deref(),
                        &placement,
                        &mirror_promise_id,
                        "legacy.narrative_promise".to_string(),
                        &promise,
                        None,
                        0,
                    )?;
                    let mirrored = mirror_legacy_claims_in_tx(
                        &tx,
                        &mirror_project_id,
                        &mirror_branch_id,
                        &revision_id,
                        mirror_source_scene_id.as_deref(),
                        vec![snapshot_claim, claim],
                    )?;
                    if mirrored.is_some() {
                        mark_legacy_rows_mapped_in_conn(
                            &tx,
                            &mirror_project_id,
                            &mirror_branch_id,
                            &[("narrative_promise".to_string(), promise.id.clone())],
                            timestamp_to_micros(chrono::Utc::now()),
                        )?;
                    }
                    tx.commit()?;
                    Ok(())
                })())
            })
            .await??;
        Ok(())
    }

    pub async fn list_plot_lines_by_project(&self, project_id: &str) -> Result<Vec<PlotLine>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_plot_lines_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_conflicts_by_project(&self, project_id: &str) -> Result<Vec<Conflict>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_conflicts_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_themes_by_project(&self, project_id: &str) -> Result<Vec<Theme>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_themes_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_motifs_by_project(&self, project_id: &str) -> Result<Vec<Motif>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_motifs_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_narrative_promises_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<NarrativePromise>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_narrative_promises_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_narrative_promises_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<NarrativePromise>> {
        list_branch_scoped(
            &self.inner.pool,
            NARRATIVE_PROMISE_COLUMNS,
            "narrative_promise",
            "created_at",
            project_id,
            branch_id,
            |r| NarrativePromise::try_from(r),
            |p| (p.promise_type.clone(), p.description.clone()),
        )
        .await
    }

    pub async fn list_all_plot_lines(&self, project_id: &str) -> Result<Vec<PlotLine>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PLOT_LINE_COLUMNS} FROM plot_line \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PlotLine::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_conflicts(&self, project_id: &str) -> Result<Vec<Conflict>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CONFLICT_COLUMNS} FROM conflict \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Conflict::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_themes(&self, project_id: &str) -> Result<Vec<Theme>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {THEME_COLUMNS} FROM theme \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Theme::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_motifs(&self, project_id: &str) -> Result<Vec<Motif>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {MOTIF_COLUMNS} FROM motif \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Motif::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_narrative_promises(
        &self,
        project_id: &str,
    ) -> Result<Vec<NarrativePromise>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| NarrativePromise::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}
