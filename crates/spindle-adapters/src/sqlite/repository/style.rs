use super::*;

impl Repository {
    // ── Style learning from operator edits (evolution §3.9, V0031) ────────────

    /// Read a project's `style_learning` opt-in. `true` only when the column is
    /// a truthy integer; NULL (pre-upgrade + default) and 0 both read as
    /// disabled. Errors if the project row is absent (a missing project is not a
    /// silent enable). Public wrapper used by the capture path and tests.
    pub async fn project_style_learning_enabled_public(&self, project_id: &str) -> Result<bool> {
        let project = self.get_project(project_id).await?;
        Ok(project.style_learning.unwrap_or(0) != 0)
    }

    /// Capture (or replace) the pending style-edit candidate for a scene
    /// (evolution §3.9). Enforces one PENDING candidate per scene per agent
    /// draft: if a pending candidate already exists for this scene, its
    /// `operator_edit` is UPDATED in place (the latest edit is the signal) while
    /// its original `agent_draft` (the contrast) is preserved; otherwise a fresh
    /// row is inserted. `consumed`/`dismissed` rows are terminal history and are
    /// never touched.
    pub async fn capture_style_edit_candidate(
        &self,
        params: CaptureStyleEditParams,
    ) -> Result<crate::sqlite::records::StoredStyleEditCandidate> {
        let CaptureStyleEditParams {
            project_id,
            branch_id,
            scene_id,
            book_number,
            chapter_number,
            scene_order,
            agent_draft,
            operator_edit,
            content_rating,
        } = params;
        let scene_id_lookup = scene_id.clone();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                // Replace-in-place when a pending candidate already exists for
                // this scene (dedupe: latest operator edit over the same agent
                // draft). One statement, atomic; a zero rowcount means insert.
                let updated = conn.execute(
                    "UPDATE style_edit_candidate \
                     SET operator_edit = ?1, content_rating = ?2, updated_at = ?3 \
                     WHERE scene_id = ?4 AND status = 'pending'",
                    rusqlite::params![operator_edit, content_rating, now, scene_id],
                )?;
                if updated == 0 {
                    let id = mint_id_local("style_edit_candidate");
                    conn.execute(
                        "INSERT INTO style_edit_candidate \
                         (id, project_id, branch_id, scene_id, book_number, chapter_number, \
                          scene_order, agent_draft, operator_edit, content_rating, status, \
                          created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11, ?11)",
                        rusqlite::params![
                            &id,
                            &project_id,
                            &branch_id,
                            &scene_id,
                            book_number,
                            chapter_number,
                            scene_order,
                            &agent_draft,
                            &operator_edit,
                            &content_rating,
                            now,
                        ],
                    )?;
                }
                Ok(())
            })
            .await?;
        self.read_pending_style_edit_candidate_for_scene(&scene_id_lookup)
            .await?
            .ok_or_else(|| anyhow!("style_edit_candidate vanished after capture"))
    }

    pub(super) async fn read_pending_style_edit_candidate_for_scene(
        &self,
        scene_id: &str,
    ) -> Result<Option<crate::sqlite::records::StoredStyleEditCandidate>> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM style_edit_candidate \
                     WHERE scene_id = ?1 AND status = 'pending' ORDER BY updated_at DESC LIMIT 1",
                    crate::sqlite::records::STYLE_EDIT_CANDIDATE_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&scene_id], |r| {
                    crate::sqlite::records::StoredStyleEditCandidate::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    /// List style-edit candidates on a branch, optionally filtered by `status`.
    /// Deterministic order: `(created_at, id)`.
    pub async fn list_style_edit_candidates(
        &self,
        project_id: &str,
        branch_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<crate::sqlite::records::StoredStyleEditCandidate>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let status = status.map(str::to_string);
        self.inner
            .pool
            .read(move |conn| {
                let mut clauses = String::from("project_id = ?1 AND branch_id = ?2");
                let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&project_id, &branch_id];
                if let Some(status) = status.as_ref() {
                    clauses.push_str(&format!(" AND status = ?{}", binds.len() + 1));
                    binds.push(status);
                }
                let sql = format!(
                    "SELECT {} FROM style_edit_candidate WHERE {clauses} ORDER BY created_at, id",
                    crate::sqlite::records::STYLE_EDIT_CANDIDATE_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(binds.as_slice(), |r| {
                        crate::sqlite::records::StoredStyleEditCandidate::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Flip a set of PENDING candidates to a terminal status
    /// (`consumed` after a refresh feeds them; `dismissed` when dropped). Only
    /// `pending` rows transition — a decision is final. Returns the number of
    /// rows flipped.
    pub async fn set_style_edit_candidate_status(
        &self,
        candidate_ids: &[String],
        new_status: &str,
    ) -> Result<u64> {
        if candidate_ids.is_empty() {
            return Ok(0);
        }
        if !matches!(new_status, "consumed" | "dismissed") {
            return Err(anyhow!(
                "style-edit candidate status must be consumed|dismissed (got '{new_status}')"
            ));
        }
        let ids: Vec<String> = candidate_ids.to_vec();
        let new_status = new_status.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                let mut affected = 0u64;
                for id in &ids {
                    affected += conn.execute(
                        "UPDATE style_edit_candidate SET status = ?1, updated_at = ?2 \
                         WHERE id = ?3 AND status = 'pending'",
                        rusqlite::params![new_status, now, id],
                    )? as u64;
                }
                Ok(affected)
            })
            .await
    }

    // ── Fiction anti-slop suppressions (Phase 5 / V0045) ───────────────────────

    /// Persist learned suppressions. Duplicate (project, branch, shelf,
    /// excerpt) rows are ignored.
    pub async fn upsert_anti_slop_suppressions(
        &self,
        project_id: &str,
        branch_id: &str,
        entries: &[spindle_core::style::antislop::FalsePositiveSuppression],
        source: &str,
    ) -> Result<u64> {
        if entries.is_empty() {
            return Ok(0);
        }
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let source = source.to_string();
        let entries: Vec<(String, String)> = entries
            .iter()
            .map(|entry| (entry.shelf_id.clone(), entry.excerpt_normalized.clone()))
            .collect();
        self.inner
            .pool
            .write(move |conn| {
                let now = timestamp_to_micros(chrono::Utc::now());
                let mut inserted = 0u64;
                for (shelf_id, excerpt) in &entries {
                    let id = mint_id_local("anti_slop_suppression");
                    let changed = conn.execute(
                        "INSERT OR IGNORE INTO anti_slop_suppression \
                         (id, project_id, branch_id, shelf_id, excerpt_normalized, source, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        rusqlite::params![
                            &id, &project_id, &branch_id, shelf_id, excerpt, &source, now
                        ],
                    )?;
                    inserted += changed as u64;
                }
                Ok(inserted)
            })
            .await
    }

    pub async fn list_anti_slop_suppressions(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<crate::sqlite::records::StoredAntiSlopSuppression>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {} FROM anti_slop_suppression \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at, id",
                    crate::sqlite::records::ANTI_SLOP_SUPPRESSION_COLUMNS
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        crate::sqlite::records::StoredAntiSlopSuppression::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn insert_style_profile(
        &self,
        profile: &spindle_core::style::StyleProfileCard,
    ) -> Result<()> {
        let profile = profile.clone();
        self.inner.pool.write(move |conn| {
            let tx = conn.transaction()?;
            let card_json = serde_json::to_string(&profile)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let metrics_json = serde_json::to_string(&profile.metrics)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let guidance_json = serde_json::to_string(&profile.guidance)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let source_policy_json = serde_json::to_string(&profile.source_policy)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let model_receipt_json = profile.model_receipt.as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let status_str = match profile.status {
                spindle_core::style::StyleProfileStatus::Ready => "ready",
                spindle_core::style::StyleProfileStatus::NeedsReview => "needs_review",
                spindle_core::style::StyleProfileStatus::Failed => "failed",
            };

            tx.execute(
                "INSERT OR REPLACE INTO style_profile (id, project_id, name, status, card_json, \
                 metrics_json, guidance_json, source_policy_json, model_receipt_json, created_at, updated_at, archived_at, \
                 parent_profile_id, refreshed_from_profile_id, version_number, refreshed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                rusqlite::params![
                    &profile.profile_id,
                    &profile.project_id,
                    &profile.name,
                    status_str,
                    &card_json,
                    &metrics_json,
                    &guidance_json,
                    &source_policy_json,
                    &model_receipt_json,
                    &profile.created_at,
                    &profile.updated_at,
                    &profile.archived_at,
                    &profile.parent_profile_id,
                    &profile.refreshed_from_profile_id,
                    &profile.version_number,
                    &profile.refreshed_at,
                ],
            )?;

            // Delete old sources for this profile if replacing
            tx.execute("DELETE FROM style_profile_source WHERE profile_id = ?1", rusqlite::params![&profile.profile_id])?;

            for (i, source_ref) in profile.corpus.source_refs.iter().enumerate() {
                let source_id = format!("{}:source:{}", profile.profile_id, i);
                tx.execute(
                    "INSERT INTO style_profile_source (id, profile_id, display_name, canonical_path, \
                     sha256, word_count, included, skip_reason, source_order, created_at, \
                     file_size, modified_at, glob_policy_metadata) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    rusqlite::params![
                        &source_id,
                        &profile.profile_id,
                        &source_ref.display_name,
                        &source_ref.canonical_path,
                        &source_ref.sha256,
                        &(source_ref.word_count as i64),
                        if source_ref.included { 1i64 } else { 0i64 },
                        &source_ref.skip_reason,
                        &(i as i64),
                        &profile.created_at,
                        &source_ref.file_size.map(|s| s as i64),
                        &source_ref.modified_at,
                        &source_ref.glob_policy_metadata,
                    ],
                )?;
            }

            tx.commit()?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn list_style_profiles(
        &self,
        project_id: &str,
    ) -> Result<Vec<spindle_core::style::StyleProfileCard>> {
        let project_id = project_id.to_string();
        let profiles = self.inner.pool.read(move |conn| {
            let mut stmt = conn.prepare("SELECT card_json FROM style_profile WHERE project_id = ?1 AND (archived_at IS NULL) ORDER BY created_at DESC")?;
            let rows = stmt.query_map([&project_id], |r| {
                let card_json: String = r.get(0)?;
                Ok(card_json)
            })?;
            let mut res = Vec::new();
            for card_json_res in rows {
                let card_json = card_json_res?;
                let profile: spindle_core::style::StyleProfileCard = serde_json::from_str(&card_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                res.push(profile);
            }
            Ok(res)
        }).await?;
        Ok(profiles)
    }

    pub async fn set_active_style_profile_id(
        &self,
        project_id: &str,
        active_style_profile_id: Option<String>,
    ) -> Result<Project> {
        let project_id_owned = project_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE project SET active_style_profile_id = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&active_style_profile_id, now, &project_id_owned],
                )?;
                Ok(())
            })
            .await?;
        self.get_project(project_id).await
    }

    pub async fn archive_style_profile(
        &self,
        project_id: &str,
        profile_id: &str,
    ) -> Result<String> {
        let project_id = project_id.to_string();
        let profile_id = profile_id.to_string();
        let archived_at = chrono::Utc::now().to_rfc3339();
        let archived_at_clone = archived_at.clone();

        self.inner
            .pool
            .write(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT card_json FROM style_profile WHERE project_id = ?1 AND id = ?2",
                )?;
                let card_json: String = stmt.query_row([&project_id, &profile_id], |r| r.get(0))?;
                let mut profile: spindle_core::style::StyleProfileCard = serde_json::from_str(&card_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                profile.archived_at = Some(archived_at_clone.clone());
                let new_card_json = serde_json::to_string(&profile)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                conn.execute(
                    "UPDATE style_profile SET archived_at = ?1, card_json = ?2, updated_at = ?3 WHERE project_id = ?4 AND id = ?5",
                    rusqlite::params![&archived_at_clone, &new_card_json, &archived_at_clone, &project_id, &profile_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(archived_at)
    }

    pub async fn get_style_profile(
        &self,
        project_id: &str,
        profile_id: &str,
    ) -> Result<Option<spindle_core::style::StyleProfileCard>> {
        let project_id = project_id.to_string();
        let profile_id = profile_id.to_string();
        let profile_opt = self
            .inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT card_json FROM style_profile WHERE project_id = ?1 AND id = ?2",
                )?;
                stmt.query_row([&project_id, &profile_id], |r| {
                    let card_json: String = r.get(0)?;
                    let profile: spindle_core::style::StyleProfileCard =
                        serde_json::from_str(&card_json)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    Ok(profile)
                })
                .optional_inner()
            })
            .await?;
        Ok(profile_opt)
    }

    pub async fn insert_style_profile_application(
        &self,
        app: &spindle_core::style::StyleProfileApplicationRecord,
    ) -> Result<()> {
        let app = app.clone();
        self.inner.pool.write(move |conn| {
            let before_voice = serde_json::to_string(&app.before_narrator_voice)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let after_voice = serde_json::to_string(&app.after_narrator_voice)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let before_notes = serde_json::to_string(&app.before_style_notes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let after_notes = serde_json::to_string(&app.after_style_notes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let added_notes = serde_json::to_string(&app.added_style_notes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let removed_notes = serde_json::to_string(&app.removed_style_notes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let mode_str = match app.apply_mode {
                spindle_core::style::StyleProfileApplyMode::Merge => "merge",
                spindle_core::style::StyleProfileApplyMode::ReplaceGeneratedStyleNotes => "replace_generated_style_notes",
            };
            let action_str = app.style_rule_action.clone();

            conn.execute(
                "INSERT INTO style_profile_application (id, project_id, profile_id, applied_at, apply_mode, \
                 before_narrator_voice_json, after_narrator_voice_json, before_style_notes_json, after_style_notes_json, \
                 added_style_notes_json, removed_style_notes_json, style_rule_id, style_rule_action, \
                 style_rule_previous_description, invalidated_validator_count, rolled_back_at, rollback_status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                rusqlite::params![
                    &app.id,
                    &app.project_id,
                    &app.profile_id,
                    &app.applied_at,
                    mode_str,
                    &before_voice,
                    &after_voice,
                    &before_notes,
                    &after_notes,
                    &added_notes,
                    &removed_notes,
                    &app.style_rule_id,
                    &action_str,
                    &app.style_rule_previous_description,
                    &(app.invalidated_validator_count as i64),
                    &app.rolled_back_at,
                    &app.rollback_status,
                ],
            )?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn insert_style_revision_patch_audit(
        &self,
        audit: &spindle_core::style::StyleRevisionPatchAuditRecord,
    ) -> Result<()> {
        let audit = audit.clone();
        self.inner
            .pool
            .write(move |conn| {
                let target_ids = serde_json::to_string(&audit.target_ids)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let before_hashes = serde_json::to_string(&audit.before_hashes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let after_hashes = serde_json::to_string(&audit.after_hashes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let receipt_json = audit
                    .model_receipt
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                conn.execute(
                "INSERT INTO style_revision_patch_audit (id, project_id, profile_id, applied_at, \
                 target_ids_json, before_hashes_json, after_hashes_json, model_receipt_json, \
                 rolled_back_at, rollback_status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    &audit.id,
                    &audit.project_id,
                    &audit.profile_id,
                    &audit.applied_at,
                    &target_ids,
                    &before_hashes,
                    &after_hashes,
                    &receipt_json,
                    &audit.rolled_back_at,
                    &audit.rollback_status,
                ],
            )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_style_revision_patch_audits(
        &self,
        project_id: &str,
    ) -> Result<Vec<spindle_core::style::StyleRevisionPatchAuditRecord>> {
        let project_id = project_id.to_string();
        let audits = self.inner.pool.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, profile_id, applied_at, target_ids_json, before_hashes_json, \
                 after_hashes_json, model_receipt_json, rolled_back_at, rollback_status \
                 FROM style_revision_patch_audit WHERE project_id = ?1 ORDER BY applied_at DESC",
            )?;
            let rows = stmt.query_map([&project_id], |r| {
                let id: String = r.get(0)?;
                let project_id: String = r.get(1)?;
                let profile_id: String = r.get(2)?;
                let applied_at: String = r.get(3)?;
                let target_ids_json: String = r.get(4)?;
                let before_hashes_json: String = r.get(5)?;
                let after_hashes_json: String = r.get(6)?;
                let receipt_json: Option<String> = r.get(7)?;
                let rolled_back_at: Option<String> = r.get(8)?;
                let rollback_status: String = r.get(9)?;

                let target_ids: Vec<String> = serde_json::from_str(&target_ids_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let before_hashes: Vec<String> = serde_json::from_str(&before_hashes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let after_hashes: Vec<String> = serde_json::from_str(&after_hashes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let model_receipt = receipt_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok(spindle_core::style::StyleRevisionPatchAuditRecord {
                    id,
                    project_id,
                    profile_id,
                    applied_at,
                    target_ids,
                    before_hashes,
                    after_hashes,
                    model_receipt,
                    rolled_back_at,
                    rollback_status,
                })
            })?;

            let mut list = Vec::new();
            for row in rows {
                list.push(row?);
            }
            Ok(list)
        }).await?;
        Ok(audits)
    }

    pub async fn get_style_revision_patch_audit(
        &self,
        project_id: &str,
        audit_id: &str,
    ) -> Result<Option<spindle_core::style::StyleRevisionPatchAuditRecord>> {
        let project_id = project_id.to_string();
        let audit_id = audit_id.to_string();
        let audit = self.inner.pool.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, profile_id, applied_at, target_ids_json, before_hashes_json, \
                 after_hashes_json, model_receipt_json, rolled_back_at, rollback_status \
                 FROM style_revision_patch_audit WHERE project_id = ?1 AND id = ?2",
            )?;
            let mut rows = stmt.query_map([&project_id, &audit_id], |r| {
                let id: String = r.get(0)?;
                let project_id: String = r.get(1)?;
                let profile_id: String = r.get(2)?;
                let applied_at: String = r.get(3)?;
                let target_ids_json: String = r.get(4)?;
                let before_hashes_json: String = r.get(5)?;
                let after_hashes_json: String = r.get(6)?;
                let receipt_json: Option<String> = r.get(7)?;
                let rolled_back_at: Option<String> = r.get(8)?;
                let rollback_status: String = r.get(9)?;

                let target_ids: Vec<String> = serde_json::from_str(&target_ids_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let before_hashes: Vec<String> = serde_json::from_str(&before_hashes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let after_hashes: Vec<String> = serde_json::from_str(&after_hashes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let model_receipt = receipt_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok(spindle_core::style::StyleRevisionPatchAuditRecord {
                    id,
                    project_id,
                    profile_id,
                    applied_at,
                    target_ids,
                    before_hashes,
                    after_hashes,
                    model_receipt,
                    rolled_back_at,
                    rollback_status,
                })
            })?;

            if let Some(row) = rows.next() {
                Ok(Some(row?))
            } else {
                Ok(None)
            }
        }).await?;
        Ok(audit)
    }

    pub async fn update_style_revision_patch_audit_rollback(
        &self,
        audit_id: &str,
        rolled_back_at: Option<String>,
        rollback_status: &str,
    ) -> Result<()> {
        let audit_id = audit_id.to_string();
        let rollback_status = rollback_status.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE style_revision_patch_audit SET rolled_back_at = ?1, rollback_status = ?2 WHERE id = ?3",
                    rusqlite::params![&rolled_back_at, &rollback_status, &audit_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_style_profile_applications(
        &self,
        project_id: &str,
    ) -> Result<Vec<spindle_core::style::StyleProfileApplicationRecord>> {
        let project_id = project_id.to_string();
        let apps = self.inner.pool.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, profile_id, applied_at, apply_mode, before_narrator_voice_json, \
                 after_narrator_voice_json, before_style_notes_json, after_style_notes_json, added_style_notes_json, \
                 removed_style_notes_json, style_rule_id, style_rule_action, style_rule_previous_description, \
                 invalidated_validator_count, rolled_back_at, rollback_status \
                 FROM style_profile_application WHERE project_id = ?1 ORDER BY applied_at DESC",
            )?;
            let rows = stmt.query_map([&project_id], |r| {
                let id: String = r.get(0)?;
                let project_id: String = r.get(1)?;
                let profile_id: String = r.get(2)?;
                let applied_at: String = r.get(3)?;
                let apply_mode_str: String = r.get(4)?;
                let before_voice_json: String = r.get(5)?;
                let after_voice_json: String = r.get(6)?;
                let before_notes_json: String = r.get(7)?;
                let after_notes_json: String = r.get(8)?;
                let added_notes_json: String = r.get(9)?;
                let removed_notes_json: String = r.get(10)?;
                let style_rule_id: Option<String> = r.get(11)?;
                let style_rule_action: String = r.get(12)?;
                let style_rule_prev_desc: Option<String> = r.get(13)?;
                let invalidated_count: i64 = r.get(14)?;
                let rolled_back_at: Option<String> = r.get(15)?;
                let rollback_status: String = r.get(16)?;

                let apply_mode = match apply_mode_str.as_str() {
                    "replace_generated_style_notes" => spindle_core::style::StyleProfileApplyMode::ReplaceGeneratedStyleNotes,
                    _ => spindle_core::style::StyleProfileApplyMode::Merge,
                };
                let before_narrator_voice = serde_json::from_str(&before_voice_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let after_narrator_voice = serde_json::from_str(&after_voice_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let before_style_notes = serde_json::from_str(&before_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let after_style_notes = serde_json::from_str(&after_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let added_style_notes = serde_json::from_str(&added_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let removed_style_notes = serde_json::from_str(&removed_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok(spindle_core::style::StyleProfileApplicationRecord {
                    id,
                    project_id,
                    profile_id,
                    applied_at,
                    apply_mode,
                    before_narrator_voice,
                    after_narrator_voice,
                    before_style_notes,
                    after_style_notes,
                    added_style_notes,
                    removed_style_notes,
                    style_rule_id,
                    style_rule_action,
                    style_rule_previous_description: style_rule_prev_desc,
                    invalidated_validator_count: invalidated_count as usize,
                    rolled_back_at,
                    rollback_status,
                })
            })?;

            let mut res = Vec::new();
            for app_res in rows {
                res.push(app_res?);
            }
            Ok(res)
        }).await?;
        Ok(apps)
    }

    pub async fn get_style_profile_application(
        &self,
        project_id: &str,
        application_id: &str,
    ) -> Result<Option<spindle_core::style::StyleProfileApplicationRecord>> {
        let project_id = project_id.to_string();
        let application_id = application_id.to_string();
        let app_opt = self.inner.pool.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, profile_id, applied_at, apply_mode, before_narrator_voice_json, \
                 after_narrator_voice_json, before_style_notes_json, after_style_notes_json, added_style_notes_json, \
                 removed_style_notes_json, style_rule_id, style_rule_action, style_rule_previous_description, \
                 invalidated_validator_count, rolled_back_at, rollback_status \
                 FROM style_profile_application WHERE project_id = ?1 AND id = ?2",
            )?;
            stmt.query_row([&project_id, &application_id], |r| {
                let id: String = r.get(0)?;
                let project_id: String = r.get(1)?;
                let profile_id: String = r.get(2)?;
                let applied_at: String = r.get(3)?;
                let apply_mode_str: String = r.get(4)?;
                let before_voice_json: String = r.get(5)?;
                let after_voice_json: String = r.get(6)?;
                let before_notes_json: String = r.get(7)?;
                let after_notes_json: String = r.get(8)?;
                let added_notes_json: String = r.get(9)?;
                let removed_notes_json: String = r.get(10)?;
                let style_rule_id: Option<String> = r.get(11)?;
                let style_rule_action: String = r.get(12)?;
                let style_rule_prev_desc: Option<String> = r.get(13)?;
                let invalidated_count: i64 = r.get(14)?;
                let rolled_back_at: Option<String> = r.get(15)?;
                let rollback_status: String = r.get(16)?;

                let apply_mode = match apply_mode_str.as_str() {
                    "replace_generated_style_notes" => spindle_core::style::StyleProfileApplyMode::ReplaceGeneratedStyleNotes,
                    _ => spindle_core::style::StyleProfileApplyMode::Merge,
                };
                let before_narrator_voice = serde_json::from_str(&before_voice_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let after_narrator_voice = serde_json::from_str(&after_voice_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let before_style_notes = serde_json::from_str(&before_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let after_style_notes = serde_json::from_str(&after_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let added_style_notes = serde_json::from_str(&added_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let removed_style_notes = serde_json::from_str(&removed_notes_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok(spindle_core::style::StyleProfileApplicationRecord {
                    id,
                    project_id,
                    profile_id,
                    applied_at,
                    apply_mode,
                    before_narrator_voice,
                    after_narrator_voice,
                    before_style_notes,
                    after_style_notes,
                    added_style_notes,
                    removed_style_notes,
                    style_rule_id,
                    style_rule_action,
                    style_rule_previous_description: style_rule_prev_desc,
                    invalidated_validator_count: invalidated_count as usize,
                    rolled_back_at,
                    rollback_status,
                })
            }).optional_inner()
        }).await?;
        Ok(app_opt)
    }

    pub async fn update_style_profile_application_rollback(
        &self,
        application_id: &str,
        rolled_back_at: &str,
        rollback_status: &str,
    ) -> Result<()> {
        let application_id = application_id.to_string();
        let rolled_back_at = rolled_back_at.to_string();
        let rollback_status = rollback_status.to_string();
        self.inner.pool.write(move |conn| {
            conn.execute(
                "UPDATE style_profile_application SET rolled_back_at = ?1, rollback_status = ?2 WHERE id = ?3",
                rusqlite::params![&rolled_back_at, &rollback_status, &application_id],
            )?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn get_most_recently_applied_profile_id(
        &self,
        project_id: &str,
    ) -> Result<Option<String>> {
        let project_id = project_id.to_string();
        let id_opt = self.inner.pool.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT profile_id FROM style_profile_application \
                 WHERE project_id = ?1 AND (rollback_status IS NULL OR rollback_status != 'rolled_back') \
                 ORDER BY applied_at DESC LIMIT 1",
            )?;
            stmt.query_row([&project_id], |r| r.get(0)).optional_inner()
        }).await?;
        Ok(id_opt)
    }
}
