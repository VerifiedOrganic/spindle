use super::*;

impl Repository {
    // === Authoring-run event journal (ADR 0002, migration V0027) ===========
    //
    // Append-only: `append_run_event` + `list_run_events` are the entire API.
    // There is deliberately NO update or delete method — the journal is
    // immutable history (ADR D1). Cascade delete rides the run's own lifecycle.

    /// Append one event to a run's journal, assigning the next dense 1-based
    /// `seq` for that run, and return it (the SSE resume token — ADR D3.4).
    ///
    /// **Seq density under the connection discipline.** All writes serialize
    /// through the pool's single dedicated writer thread (see [`SqlitePool`]):
    /// no two `write` closures ever run concurrently. So computing
    /// `MAX(seq)+1` and inserting inside ONE closure is race-free — the read
    /// and the insert are one atomic unit on the sole writer. Concurrent
    /// `authoring_execute_next` calls for the same run are additionally
    /// serialized a layer up by the MCP per-project tool lock, but even without
    /// that lock the single-writer guarantee alone makes `seq` dense and
    /// unique. Because there is no genuine race, no UNIQUE-violation retry is
    /// warranted (the `UNIQUE(authoring_run_id, seq)` constraint remains as an
    /// integrity guard, not a contention path).
    pub async fn append_run_event(
        &self,
        run_id: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<i64> {
        let run_id = run_id.to_string();
        let kind = kind.to_string();
        let payload_str =
            serde_json::to_string(&payload).context("serializing run-event payload")?;
        let id = mint_id("authoring_run_event");
        self.inner
            .pool
            .write(move |conn| {
                // Single-writer discipline: this MAX + INSERT is one atomic unit
                // on the sole writer thread, so the assigned seq is dense.
                let next_seq: i64 = conn.query_row(
                    "SELECT COALESCE(MAX(seq), 0) + 1 FROM authoring_run_event \
                     WHERE authoring_run_id = ?1",
                    [&run_id],
                    |row| row.get(0),
                )?;
                let now = timestamp_to_micros(chrono::Utc::now());
                conn.execute(
                    "INSERT INTO authoring_run_event \
                     (id, authoring_run_id, seq, kind, payload, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![id, run_id, next_seq, kind, payload_str, now],
                )?;
                Ok(next_seq)
            })
            .await
    }

    /// List a run's journal events in ascending `seq` order. `after_seq`
    /// replays only rows with `seq > after_seq` (the `Last-Event-ID`+1 resume
    /// path — ADR D4); `limit` caps the returned window.
    pub async fn list_run_events(
        &self,
        run_id: &str,
        after_seq: Option<i64>,
        limit: Option<usize>,
    ) -> Result<Vec<crate::sqlite::records::StoredRunEvent>> {
        let run_id = run_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut clauses = String::from("authoring_run_id = ?1");
                let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(run_id)];
                if let Some(after) = after_seq {
                    clauses.push_str(&format!(" AND seq > ?{}", binds.len() + 1));
                    binds.push(Box::new(after));
                }
                let mut sql = format!(
                    "SELECT {} FROM authoring_run_event WHERE {clauses} ORDER BY seq",
                    crate::sqlite::records::AUTHORING_RUN_EVENT_COLUMNS
                );
                if let Some(limit) = limit {
                    sql.push_str(&format!(" LIMIT {}", limit as i64));
                }
                let mut stmt = conn.prepare_cached(&sql)?;
                let bind_refs: Vec<&dyn rusqlite::ToSql> =
                    binds.iter().map(|b| b.as_ref()).collect();
                let rows = stmt
                    .query_map(bind_refs.as_slice(), |r| {
                        crate::sqlite::records::StoredRunEvent::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
    pub async fn save_authoring_run(
        &self,
        run: super::records::AuthoringRun,
        chapters: Vec<super::records::AuthoringRunChapter>,
        scenes: Vec<super::records::AuthoringRunScene>,
        checkpoints: Vec<super::records::AuthoringCheckpoint>,
    ) -> Result<()> {
        let directives_json = serde_json::to_string(&run.editorial_directives)
            .context("serializing editorial directives")?;
        let created_at_micros = super::row::timestamp_to_micros(run.created_at);
        let updated_at_micros = super::row::timestamp_to_micros(run.updated_at);

        let mut scene_data = Vec::new();
        for sc in scenes {
            let char_ids_json =
                serde_json::to_string(&sc.character_ids).context("serializing character_ids")?;
            let research_tags_json =
                serde_json::to_string(&sc.research_tags).context("serializing research_tags")?;
            let diagnostics_json = sc
                .draft_diagnostics
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .context("serializing draft diagnostics")?;
            scene_data.push((sc, char_ids_json, research_tags_json, diagnostics_json));
        }

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;

                // Upsert IN PLACE (ON CONFLICT DO UPDATE), NOT `INSERT OR
                // REPLACE`. `INSERT OR REPLACE` is a DELETE+INSERT of the run
                // row, which fires the `authoring_run_event` ON DELETE CASCADE
                // and would wipe the run's event journal (V0027) on every state
                // save. An in-place update leaves child rows untouched.
                // `created_at` is preserved from the existing row on conflict.
                tx.execute(
                    "INSERT INTO authoring_run (
                    id, project_id, active_branch_id, book_number, start_chapter, end_chapter,
                    checkpoint_interval, last_checkpoint_end_chapter, artifacts_dir,
                    editorial_directives, status, created_at, updated_at, mining_policy,
                    max_revise_attempts, checkpoint_policy, replan_policy
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                ON CONFLICT(id) DO UPDATE SET
                    project_id = excluded.project_id,
                    active_branch_id = excluded.active_branch_id,
                    book_number = excluded.book_number,
                    start_chapter = excluded.start_chapter,
                    end_chapter = excluded.end_chapter,
                    checkpoint_interval = excluded.checkpoint_interval,
                    last_checkpoint_end_chapter = excluded.last_checkpoint_end_chapter,
                    artifacts_dir = excluded.artifacts_dir,
                    editorial_directives = excluded.editorial_directives,
                    status = excluded.status,
                    updated_at = excluded.updated_at,
                    mining_policy = excluded.mining_policy,
                    max_revise_attempts = excluded.max_revise_attempts,
                    checkpoint_policy = excluded.checkpoint_policy,
                    replan_policy = excluded.replan_policy",
                    rusqlite::params![
                        run.id,
                        run.project_id,
                        run.active_branch_id,
                        run.book_number,
                        run.start_chapter,
                        run.end_chapter,
                        run.checkpoint_interval,
                        run.last_checkpoint_end_chapter,
                        run.artifacts_dir,
                        directives_json,
                        run.status,
                        created_at_micros,
                        updated_at_micros,
                        run.mining_policy,
                        run.max_revise_attempts,
                        run.checkpoint_policy,
                        run.replan_policy,
                    ],
                )?;

                tx.execute(
                    "DELETE FROM authoring_checkpoint WHERE authoring_run_id = ?1",
                    [&run.id],
                )?;
                tx.execute(
                    "DELETE FROM authoring_run_chapter WHERE authoring_run_id = ?1",
                    [&run.id],
                )?;

                for ch in chapters {
                    tx.execute(
                        "INSERT INTO authoring_run_chapter (
                        authoring_run_id, chapter_number, planned, synopsis, pov_character_id,
                        status, summary_saved, summary_artifact_path, replan_status, replan_detail
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![
                            ch.authoring_run_id,
                            ch.chapter_number,
                            ch.planned as i32,
                            ch.synopsis,
                            ch.pov_character_id,
                            ch.status,
                            ch.summary_saved as i32,
                            ch.summary_artifact_path,
                            ch.replan_status,
                            ch.replan_detail,
                        ],
                    )?;
                }

                for (sc, char_ids_json, research_tags_json, diagnostics_json) in scene_data {
                    tx.execute(
                        "INSERT INTO authoring_run_scene (
                        authoring_run_id, chapter_number, scene_order, character_ids, location_id,
                        content_rating, tone, source_path, phase, scene_id, scene_artifact_path,
                        draft_diagnostics, blocked_reason, research_required, research_tags,
                        explicit_query, mine_status, mine_detail, verify_status, verify_detail,
                        revise_attempts, last_finding_fingerprint
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                        rusqlite::params![
                            sc.authoring_run_id,
                            sc.chapter_number,
                            sc.scene_order,
                            char_ids_json,
                            sc.location_id,
                            sc.content_rating,
                            sc.tone,
                            sc.source_path,
                            sc.phase,
                            sc.scene_id,
                            sc.scene_artifact_path,
                            diagnostics_json,
                            sc.blocked_reason,
                            sc.research_required.map(|required| required as i32),
                            research_tags_json,
                            sc.explicit_query,
                            sc.mine_status,
                            sc.mine_detail,
                            sc.verify_status,
                            sc.verify_detail,
                            sc.revise_attempts,
                            sc.last_finding_fingerprint,
                        ],
                    )?;
                }

                for cp in checkpoints {
                    // Persist the pending-manual scene ids as a JSON array, or
                    // NULL when there are none (NULL reads back as the empty
                    // list — the no-fallback / pre-upgrade case). Ids only.
                    let pending_manual_json = if cp.pending_manual_scene_ids.is_empty() {
                        None
                    } else {
                        Some(
                            serde_json::to_string(&cp.pending_manual_scene_ids)
                                .expect("serializing pending_manual_scene_ids"),
                        )
                    };
                    tx.execute(
                        "INSERT INTO authoring_checkpoint (
                        authoring_run_id, start_chapter, end_chapter, save_point_id, status,
                        report_artifact_path, auto_outcome, pending_manual_scene_ids
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            cp.authoring_run_id,
                            cp.start_chapter,
                            cp.end_chapter,
                            cp.save_point_id,
                            cp.status,
                            cp.report_artifact_path,
                            cp.auto_outcome,
                            pending_manual_json,
                        ],
                    )?;
                }

                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Re-seed any non-completed authoring run's chapter and scene rows when a
    /// plan amendment rewrites an undrafted chapter inside the run's range.
    ///
    /// `scenes` carries `(original_scene_order, amended_scene)` in the new
    /// 1..=n spine order so surviving scenes preserve their existing run metadata
    /// (`tone`, `source_path`, and fallback `location_id` / `content_rating`),
    /// while newly inserted scenes (`None`) inherit fallback `location_id` /
    /// `content_rating` / `character_ids` from the chapter's existing run scenes
    /// when omitted on the plan scene.
    #[allow(clippy::too_many_arguments)]
    pub async fn reseed_authoring_runs_for_chapter_plan(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        synopsis: &str,
        pov_character_id: Option<&str>,
        scenes: &[(Option<i32>, spindle_core::models::PlanChapterSceneInput)],
        plan_revision: i64,
    ) -> Result<usize> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let synopsis = synopsis.to_string();
        let pov_character_id = pov_character_id.map(str::to_string);
        let scenes = scenes.to_vec();
        let data_dir = self.inner.data_dir.clone();
        let updated_at_micros = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let runs: Vec<(String, String)> = {
                    let mut stmt = tx.prepare_cached(
                        "SELECT id, artifacts_dir FROM authoring_run
                         WHERE project_id = ?1
                           AND active_branch_id = ?2
                           AND book_number = ?3
                           AND start_chapter <= ?4
                           AND end_chapter >= ?4
                           AND status != 'completed'",
                    )?;
                    let rows = stmt.query_map(
                        rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                };

                let ch_sql = format!(
                    "SELECT {} FROM authoring_run_chapter WHERE authoring_run_id = ?1 AND chapter_number = ?2",
                    super::records::AUTHORING_RUN_CHAPTER_COLUMNS
                );
                let sc_sql = format!(
                    "SELECT {} FROM authoring_run_scene WHERE authoring_run_id = ?1 AND chapter_number = ?2 ORDER BY scene_order",
                    super::records::AUTHORING_RUN_SCENE_COLUMNS
                );

                let mut reseeded_runs = 0usize;
                let mut stale_artifacts_to_remove: Vec<(String, String)> = Vec::new();
                for (run_id, artifacts_dir) in runs {
                    let ch_opt: Option<super::records::AuthoringRunChapter> = {
                        let mut stmt = tx.prepare_cached(&ch_sql)?;
                        stmt.query_row(rusqlite::params![&run_id, chapter_number], |r| {
                            super::records::AuthoringRunChapter::try_from(r)
                        })
                        .optional_inner()?
                    };
                    let Some(existing_ch) = ch_opt else {
                        continue;
                    };
                    if existing_ch.summary_saved {
                        continue;
                    }

                    let existing_scenes: Vec<super::records::AuthoringRunScene> = {
                        let mut stmt = tx.prepare_cached(&sc_sql)?;
                        let rows = stmt.query_map(rusqlite::params![&run_id, chapter_number], |r| {
                            super::records::AuthoringRunScene::try_from(r)
                        })?;
                        rows.collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    if existing_scenes
                        .iter()
                        .any(|sc| sc.phase != "pending" || sc.scene_id.is_some())
                    {
                        continue;
                    }

                    for sc in &existing_scenes {
                        if let Some(rel) = sc.scene_artifact_path.as_ref() {
                            stale_artifacts_to_remove.push((artifacts_dir.clone(), rel.clone()));
                        }
                    }

                    tx.execute(
                        "UPDATE authoring_run_chapter
                         SET planned = 1,
                             synopsis = ?1,
                             pov_character_id = ?2
                         WHERE authoring_run_id = ?3 AND chapter_number = ?4",
                        rusqlite::params![&synopsis, &pov_character_id, &run_id, chapter_number],
                    )?;

                    tx.execute(
                        "DELETE FROM authoring_run_scene WHERE authoring_run_id = ?1 AND chapter_number = ?2",
                        rusqlite::params![&run_id, chapter_number],
                    )?;

                    for (orig_order, plan_scene) in &scenes {
                        let prior = orig_order
                            .and_then(|ord| existing_scenes.iter().find(|s| s.scene_order == ord));
                        let fallback = prior.or_else(|| existing_scenes.first());
                        let character_ids = if !plan_scene.character_ids.is_empty() {
                            plan_scene.character_ids.clone()
                        } else if let Some(fb) = fallback {
                            fb.character_ids.clone()
                        } else if let Some(pov) = pov_character_id.as_ref() {
                            vec![pov.clone()]
                        } else {
                            Vec::new()
                        };
                        let location_id = plan_scene
                            .location_id
                            .clone()
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| fallback.map(|s| s.location_id.clone()))
                            .unwrap_or_default();
                        let content_rating = plan_scene
                            .content_rating
                            .as_ref()
                            .map(|r| r.as_str().to_string())
                            .or_else(|| fallback.map(|s| s.content_rating.clone()))
                            .unwrap_or_else(|| "general".to_string());
                        let tone = prior.and_then(|s| s.tone.clone());
                        let source_path = prior.and_then(|s| s.source_path.clone());
                        let char_ids_json = serde_json::to_string(&character_ids)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        let research_tags_json = serde_json::to_string(&plan_scene.research_tags)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        let scene_artifact_path = format!(
                            "scenes/chapter-{chapter_number:04}/scene-{:03}-r{}.json",
                            plan_scene.scene_order,
                            plan_revision.max(1)
                        );

                        tx.execute(
                            "INSERT INTO authoring_run_scene (
                                authoring_run_id, chapter_number, scene_order, character_ids, location_id,
                                content_rating, tone, source_path, phase, scene_id, scene_artifact_path,
                                draft_diagnostics, blocked_reason, research_required, research_tags,
                                explicit_query, mine_status, mine_detail, verify_status, verify_detail,
                                revise_attempts, last_finding_fingerprint
                            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL, ?9, NULL, NULL, ?10, ?11, ?12, NULL, NULL, NULL, NULL, 0, NULL)",
                            rusqlite::params![
                                &run_id,
                                chapter_number,
                                plan_scene.scene_order,
                                char_ids_json,
                                location_id,
                                content_rating,
                                tone,
                                source_path,
                                scene_artifact_path,
                                plan_scene.research_required.map(|required| required as i32),
                                research_tags_json,
                                plan_scene.explicit_query,
                            ],
                        )?;
                    }

                    tx.execute(
                        "UPDATE authoring_run SET updated_at = ?1 WHERE id = ?2",
                        rusqlite::params![updated_at_micros, &run_id],
                    )?;
                    reseeded_runs += 1;
                }

                tx.commit()?;

                let runtime_root = crate::workspace::runtime_dir(&data_dir);
                for (artifacts_dir, rel) in stale_artifacts_to_remove {
                    let root = runtime_root.join(&artifacts_dir);
                    let _ = std::fs::remove_file(root.join(&rel));
                }

                Ok(reseeded_runs)
            })
            .await
    }

    pub async fn get_authoring_run(
        &self,
        run_id: &str,
    ) -> Result<
        Option<(
            super::records::AuthoringRun,
            Vec<super::records::AuthoringRunChapter>,
            Vec<super::records::AuthoringRunScene>,
            Vec<super::records::AuthoringCheckpoint>,
        )>,
    > {
        let run_id = run_id.to_string();
        let run_id_clone = run_id.clone();

        let run_opt: Option<super::records::AuthoringRun> = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM authoring_run WHERE id = ?1",
                    super::records::AUTHORING_RUN_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&run_id], |r| super::records::AuthoringRun::try_from(r))
                    .optional_inner()
            })
            .await?;

        let Some(run) = run_opt else {
            return Ok(None);
        };

        let run_id_ch = run_id_clone.clone();
        let chapters = self.inner.pool.read(move |conn| {
            let sql = format!("SELECT {} FROM authoring_run_chapter WHERE authoring_run_id = ?1 ORDER BY chapter_number", super::records::AUTHORING_RUN_CHAPTER_COLUMNS);
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map([&run_id_ch], |r| super::records::AuthoringRunChapter::try_from(r))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }).await?;

        let run_id_sc = run_id_clone.clone();
        let scenes = self.inner.pool.read(move |conn| {
            let sql = format!("SELECT {} FROM authoring_run_scene WHERE authoring_run_id = ?1 ORDER BY chapter_number, scene_order", super::records::AUTHORING_RUN_SCENE_COLUMNS);
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map([&run_id_sc], |r| super::records::AuthoringRunScene::try_from(r))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }).await?;

        let run_id_cp = run_id_clone.clone();
        let checkpoints = self.inner.pool.read(move |conn| {
            let sql = format!("SELECT {} FROM authoring_checkpoint WHERE authoring_run_id = ?1 ORDER BY start_chapter", super::records::AUTHORING_CHECKPOINT_COLUMNS);
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map([&run_id_cp], |r| super::records::AuthoringCheckpoint::try_from(r))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }).await?;

        Ok(Some((run, chapters, scenes, checkpoints)))
    }

    pub async fn find_latest_authoring_run_id(&self, project_id: &str) -> Result<Option<String>> {
        let project_id = project_id.to_string();
        let run_id_opt = self.inner.pool.read(move |conn| {
            let mut stmt = conn.prepare_cached("SELECT id FROM authoring_run WHERE project_id = ?1 ORDER BY updated_at DESC LIMIT 1")?;
            stmt.query_row([&project_id], |r| r.get::<_, String>(0)).optional_inner()
        }).await?;
        Ok(run_id_opt)
    }
}
