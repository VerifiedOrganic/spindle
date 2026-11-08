use super::*;

impl SqliteSpindleService {
    /// In-place scene revision: save the new prose through the
    /// production `save_scene_draft` path (which snapshots the prior
    /// version, marks dependent dual_persona_review rows stale, and
    /// resolves open validator_finding rows), then surface a structured
    /// diff plus revision markers for downstream state, scenes, and
    /// pacing trackers. Mirrors services/mod.rs:7504..7763 in 705b835^.
    ///
    /// Per-project main-branch design (Risk #6): the SurrealDB reference
    /// rejected revision when the active branch was the singleton
    /// `bible_branch:main`. With per-project main branches, the SQLite
    /// check looks up the branch by name to apply the same semantics.
    ///
    /// After the prose is saved, the response carries:
    /// * `world_rule_hits` from `scan_world_rules`,
    /// * `voice_drift` from `scan_voice_drift`,
    /// * `retcon_findings` from `scan_retcon_findings`,
    ///
    /// alongside the existing revision-marker fan-outs from
    /// `list_character_states_after_position`,
    /// `list_scenes_after_position`, and
    /// `list_pacing_trackers_by_project_and_branch`.
    pub async fn revise_scene(
        &self,
        input: spindle_core::models::ReviseSceneInput,
    ) -> Result<spindle_core::models::ReviseSceneOutput> {
        use crate::sqlite::records::RevisionMarker as RevisionMarkerRecord;
        use spindle_core::models::{
            ReviseSceneOutput, RevisionPacingImpact, RevisionSceneFlag, RevisionStateInvalidation,
            SaveSceneDraftInput,
        };

        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let existing_scene = self.repository.get_scene(&input.scene_id).await?;

        if existing_scene.project_id != input.project_id {
            anyhow::bail!(
                "scene {} does not belong to project {}",
                input.scene_id,
                input.project_id
            );
        }
        if active_branch.name == "main" {
            anyhow::bail!("revise_scene requires a non-main branch");
        }

        let prior_text = existing_scene.full_text.clone();
        let revised_text = input.full_text.clone();

        let (revised_scene, _) = self
            .repository
            .save_scene_draft(
                &input.project_id,
                &active_branch.id,
                &SaveSceneDraftInput {
                    project_id: input.project_id.clone(),
                    book_number: existing_scene.book_number,
                    chapter_number: existing_scene.chapter_number,
                    chapter_id: None,
                    scene_order: existing_scene.scene_order,
                    full_text: revised_text.clone(),
                    summary: input.summary,
                    content_rating: input.content_rating,
                    tone: input.tone,
                    source_path: None,
                    generation_id: None,
                    ..Default::default()
                },
            )
            .await?;

        let (diff, byte_offsets_changed, chars_added, chars_deleted) =
            compute_text_diff(&prior_text, &revised_text);

        let now = existing_scene.updated_at;

        let state_markers = self
            .repository
            .list_character_states_after_position(
                &input.project_id,
                &active_branch.id,
                revised_scene.book_number,
                revised_scene.chapter_number,
                revised_scene.scene_order,
            )
            .await?
            .into_iter()
            .map(|state| RevisionMarkerRecord {
                id: String::new(),
                project_id: input.project_id.clone(),
                branch_id: active_branch.id.clone(),
                scene_id: revised_scene.id.clone(),
                marker_type: "state_invalidated".to_string(),
                target_record_id: Some(state.id.clone()),
                position: format!(
                    "{}|{}:{}:{}",
                    state.character_id, state.book_number, state.chapter_number, state.scene_order
                ),
                note: "state was recorded after the revised scene on this branch".to_string(),
                status: "open".to_string(),
                created_at: now,
            });
        let scene_markers = self
            .repository
            .list_scenes_after_position(
                &input.project_id,
                &active_branch.id,
                revised_scene.book_number,
                revised_scene.chapter_number,
                revised_scene.scene_order,
            )
            .await?
            .into_iter()
            .map(|scene| RevisionMarkerRecord {
                id: String::new(),
                project_id: input.project_id.clone(),
                branch_id: active_branch.id.clone(),
                scene_id: revised_scene.id.clone(),
                marker_type: "scene_flagged".to_string(),
                target_record_id: Some(scene.id.clone()),
                position: format!(
                    "{}:{}:{}",
                    scene.book_number, scene.chapter_number, scene.scene_order
                ),
                note:
                    "scene occurs after the revised scene and may rely on invalidated branch state"
                        .to_string(),
                status: "open".to_string(),
                created_at: now,
            });
        let pacing_markers = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?
            .into_iter()
            .filter(|tracker| tracker.current_progress > 0.0 || tracker.status != "on_track")
            .map(|tracker| RevisionMarkerRecord {
                id: String::new(),
                project_id: input.project_id.clone(),
                branch_id: active_branch.id.clone(),
                scene_id: revised_scene.id.clone(),
                marker_type: "pacing_review".to_string(),
                target_record_id: Some(tracker.id.clone()),
                position: tracker.character_arc_id.clone(),
                note:
                    "review this tracker because the revised scene may change downstream arc progression"
                        .to_string(),
                status: "open".to_string(),
                created_at: now,
            });

        let mut persisted_markers = Vec::new();
        for marker in state_markers.chain(scene_markers).chain(pacing_markers) {
            let persisted = self.repository.upsert_revision_marker(&marker).await?;
            persisted_markers.push(persisted);
        }

        // ── Phase-4 scanners against the revised prose ──────────────
        // Surfaces world-rule hits, voice drift, and retcon findings the
        // new prose introduces. The Phase-4 fan-out cache for this scene
        // was already invalidated by `save_scene_draft` (which calls
        // `resolve_validator_findings_for_scenes`), so we re-run the
        // light per-scene scanners directly rather than going through
        // `run_phase_four_validator_checks_for_scenes`.
        let world_rule_hits_after_revision = self
            .scan_world_rules(
                &input.project_id,
                &revised_scene.branch_id,
                &revised_scene.full_text,
            )
            .await?;
        let voice_drift_after_revision = self
            .scan_voice_drift(&input.project_id, &revised_scene.full_text)
            .await?;
        let retcon_findings_after_revision = self
            .scan_retcon_findings(&input.project_id, &revised_scene)
            .await?;
        // Consult the scene clock so a deliberate flashback / coarse-precision
        // scene is suppressed, matching the standing `temporal_coherence` audit.
        let revised_clock = self.repository.get_scene_clock(&revised_scene.id).await?;
        let temporal_findings_after_revision = scan_temporal_findings(
            &revised_scene.id,
            &revised_scene.full_text,
            revised_clock.as_ref(),
        );

        Ok(ReviseSceneOutput {
            scene_id: revised_scene.id.clone(),
            states_invalidated: persisted_markers
                .iter()
                .filter(|m| m.marker_type == "state_invalidated")
                .map(|m| RevisionStateInvalidation {
                    state_id: m.target_record_id.clone().unwrap_or_default(),
                    character_id: m
                        .position
                        .split_once('|')
                        .map(|(left, _)| left.to_string())
                        .unwrap_or_default(),
                    position: m.position.clone(),
                    reason: m.note.clone(),
                })
                .collect(),
            downstream_scenes_flagged: persisted_markers
                .iter()
                .filter(|m| m.marker_type == "scene_flagged")
                .map(|m| RevisionSceneFlag {
                    scene_id: m.target_record_id.clone().unwrap_or_default(),
                    position: m.position.clone(),
                    reason: m.note.clone(),
                })
                .collect(),
            pacing_impact: persisted_markers
                .iter()
                .filter(|m| m.marker_type == "pacing_review")
                .map(|m| RevisionPacingImpact {
                    character_arc_id: m.position.clone(),
                    tracker_id: m.target_record_id.clone().unwrap_or_default(),
                    status: m.status.clone(),
                    note: m.note.clone(),
                })
                .collect(),
            diff,
            byte_offsets_changed,
            chars_added,
            chars_deleted,
            world_rule_hits: world_rule_hits_after_revision,
            voice_drift: voice_drift_after_revision,
            retcon_findings: retcon_findings_after_revision,
            temporal_findings: temporal_findings_after_revision,
        })
    }

    /// Drive a model-router continuation against a truncated prior output.
    /// Records the combined `prior_output + response.output` in the
    /// in-memory receipt cache so subsequent `save_scene_draft` /
    /// `revise_generation` calls can reference it by `generation_id`.
    /// Mirrors services/mod.rs:476..503 in 705b835^.
    pub async fn continue_generation(
        &self,
        input: spindle_core::models::ContinueGenerationInput,
    ) -> Result<spindle_core::models::ContinueGenerationOutput> {
        use crate::ai::RequestContext;
        use spindle_core::models::ContinueGenerationOutput;

        let context = RequestContext {
            project_id: input.project_id.clone(),
            book_id: input.book_id.clone(),
            chapter_id: input.chapter_id.clone(),
            scene_id: input.scene_id.clone(),
            run_id: input.run_id.clone(),
            prompt_template_id: input.prompt_template_id.clone(),
            prompt_version: input.prompt_version.clone(),
            context_packet_hash: input.context_packet_hash.clone(),
        };
        let context_ref = if context.is_empty() {
            None
        } else {
            Some(&context)
        };
        let response = self
            .repository
            .model_router()
            .complete_continuation(
                &input.route,
                input.rating.as_deref(),
                context_ref,
                &input.original_prompt,
                &input.prior_output,
            )
            .await?;
        let mut call_ids = self
            .model_call_ids_for_prior_output(&input.route, &input.prior_output)
            .await;
        if let Some(call_id) = response.call_id.clone()
            && !call_ids.contains(&call_id)
        {
            call_ids.push(call_id);
        }
        let receipt = self
            .register_generation_receipt_with_calls(
                &input.route,
                input.rating.as_deref(),
                &response.model_name,
                &format!("{}{}", input.prior_output, response.output),
                call_ids,
            )
            .await?;
        Ok(ContinueGenerationOutput {
            output: response.output,
            truncated: response.truncated,
            generation_id: Some(receipt.id),
            generation_agent_id: Some(receipt.agent_id),
            generation_output_sha256: Some(receipt.output_sha256),
        })
    }

    /// Revise a previously-generated draft by referencing its
    /// `generation_id`. Runs the revision through the same draft route the
    /// source receipt used (preserving rating routing — explicit receipts
    /// re-enter the explicit-capable agent, mature receipts re-enter the
    /// mature route, and so on) and registers a new receipt for the revised
    /// output. Works for any rating the source receipt carried; use it for
    /// small surgical edits without forcing a full `continue_generation`
    /// re-roll.
    pub async fn revise_generation(
        &self,
        input: spindle_core::models::ReviseGenerationInput,
    ) -> Result<spindle_core::models::ReviseGenerationOutput> {
        use crate::ai::ModelRequest;
        use spindle_core::models::ReviseGenerationOutput;

        let source_receipt = self
            .verified_revisable_draft_receipt(Some(&input.generation_id))
            .await?
            .ok_or_else(|| anyhow::anyhow!("generation_id is required"))?;
        let edit_instructions = input.edit_instructions.trim();
        if edit_instructions.is_empty() {
            anyhow::bail!("edit_instructions must not be empty");
        }

        let inherited_context = if let Some(last_call_id) = source_receipt.model_call_ids.last() {
            let call_id = last_call_id.clone();
            self.repository
                .pool()
                .read(move |conn| {
                    Ok(conn
                        .query_row(
                            "SELECT project_id, scene_id, run_id FROM model_call WHERE id = ?1",
                            rusqlite::params![call_id],
                            |row| {
                                Ok(crate::ai::RequestContext {
                                    project_id: row.get(0)?,
                                    scene_id: row.get(1)?,
                                    run_id: row.get(2)?,
                                    ..Default::default()
                                })
                            },
                        )
                        .ok()
                        .filter(|ctx| !ctx.is_empty()))
                })
                .await
                .unwrap_or(None)
        } else {
            None
        };

        let prompt = build_generation_revision_prompt(
            &source_receipt.output_text,
            edit_instructions,
            input.context.as_deref(),
        );
        let response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: source_receipt.route.clone(),
                prompt,
                rating: source_receipt.rating.clone(),
                context: inherited_context,
            })
            .await?;
        let mut call_ids = source_receipt.model_call_ids.clone();
        if let Some(call_id) = response.call_id.clone()
            && !call_ids.contains(&call_id)
        {
            call_ids.push(call_id);
        }
        let receipt = self
            .register_generation_receipt_with_calls(
                &source_receipt.route,
                source_receipt.rating.as_deref(),
                &response.model_name,
                &response.output,
                call_ids,
            )
            .await?;

        Ok(ReviseGenerationOutput {
            output: response.output,
            truncated: response.truncated,
            source_generation_id: input.generation_id,
            generation_id: Some(receipt.id),
            generation_agent_id: Some(receipt.agent_id),
            generation_output_sha256: Some(receipt.output_sha256),
        })
    }

    pub(super) async fn model_call_ids_for_prior_output(
        &self,
        route: &str,
        prior_output: &str,
    ) -> Vec<String> {
        let normalized = normalized_generation_text(prior_output);
        if normalized.is_empty() {
            return Vec::new();
        }
        let prior_sha = generation_sha256_hex(normalized.as_bytes());
        if let Ok(receipts) = self.generation_receipts.read()
            && let Some(found) = receipts
                .values()
                .rev()
                .find(|r| r.route == route && r.output_sha256 == prior_sha)
        {
            return found.model_call_ids.clone();
        }
        let route_owned = route.to_string();
        self.repository
            .pool()
            .read(move |conn| {
                let raw: Option<String> = conn
                    .query_row(
                        "SELECT model_call_ids FROM generation_receipt
                         WHERE route = ?1 AND output_sha256 = ?2
                         ORDER BY created_at DESC LIMIT 1",
                        rusqlite::params![route_owned, prior_sha],
                        |row| row.get(0),
                    )
                    .ok();
                Ok(raw
                    .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                    .unwrap_or_default())
            })
            .await
            .unwrap_or_default()
    }

    /// Register a receipt for a model-router output. Persists the row in the
    /// `generation_receipt` table (migration V0032) so the `generation_id`
    /// survives a primary restart, and also seeds an in-memory read-through
    /// cache (up to `MAX_GENERATION_RECEIPTS`, evicting oldest FIFO). Mirrors
    /// the SurrealDB reference so the `generation_id` shape
    /// (`model_generation:{seq}:{12-char-sha-prefix}`) is stable.
    ///
    /// The persisted row carries a `GENERATION_RECEIPT_TTL` expiry stamped from
    /// the current wall clock; the `verified_*` lookups enforce it on read.
    #[cfg(test)]
    pub(super) async fn register_generation_receipt(
        &self,
        route: &str,
        rating: Option<&str>,
        agent_id: &str,
        output: &str,
    ) -> Result<GenerationReceiptRecord> {
        self.register_generation_receipt_with_calls(route, rating, agent_id, output, Vec::new())
            .await
    }

    pub(super) async fn register_generation_receipt_with_calls(
        &self,
        route: &str,
        rating: Option<&str>,
        agent_id: &str,
        output: &str,
        model_call_ids: Vec<String>,
    ) -> Result<GenerationReceiptRecord> {
        use std::sync::atomic::Ordering;

        let output_text = normalized_generation_text(output);
        let output_sha256 = generation_sha256_hex(output_text.as_bytes());
        let sequence = self
            .generation_receipt_counter
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let id = format!(
            "model_generation:{sequence}:{}",
            output_sha256.chars().take(12).collect::<String>()
        );
        let receipt = GenerationReceiptRecord {
            id: id.clone(),
            route: route.to_string(),
            rating: rating.map(normalize_generation_rating),
            agent_id: agent_id.to_string(),
            output_sha256: output_sha256.clone(),
            output_text: output_text.clone(),
            explicit_capable_agent: self.agent_supports_rating(agent_id, "explicit")?,
            model_call_ids: model_call_ids.clone(),
        };

        // Persist (source of truth across restarts). A DB write failure must not
        // silently drop the receipt into an unrecoverable state: the in-memory
        // cache below still serves same-process reads, and a warn surfaces the
        // degradation. (The cross-process guarantee is only lost if the write
        // failed, which is itself logged.)
        let created_at = chrono::Utc::now();
        let expires_at = created_at + GENERATION_RECEIPT_TTL;
        if let Err(err) = self
            .repository
            .insert_generation_receipt(
                &id,
                None,
                None,
                route,
                agent_id,
                receipt.rating.as_deref(),
                receipt.explicit_capable_agent,
                &output_sha256,
                &output_text,
                created_at,
                expires_at,
            )
            .await
        {
            tracing::warn!(
                error = %err,
                generation_id = %id,
                "failed to persist generation receipt; it will not survive a restart"
            );
        } else if !model_call_ids.is_empty() {
            let receipt_id = id.clone();
            let call_ids_json =
                serde_json::to_string(&model_call_ids).unwrap_or_else(|_| "[]".to_string());
            let _ = self
                .repository
                .pool()
                .write(move |conn| {
                    conn.execute(
                        "UPDATE generation_receipt SET model_call_ids = ?1 WHERE id = ?2",
                        rusqlite::params![call_ids_json, receipt_id],
                    )?;
                    Ok(())
                })
                .await;
        }

        let mut receipts = self
            .generation_receipts
            .write()
            .map_err(|_| poisoned_lock("generation receipts write lock"))?;
        receipts.insert(id, receipt.clone());
        while receipts.len() > MAX_GENERATION_RECEIPTS {
            if let Some(oldest) = receipts.keys().next().cloned() {
                receipts.remove(&oldest);
            } else {
                break;
            }
        }

        Ok(receipt)
    }

    pub(super) fn agent_supports_rating(&self, agent_id: &str, rating: &str) -> Result<bool> {
        let expected = normalize_generation_rating(rating);
        Ok(self
            .repository
            .model_router()
            .list_agents()?
            .agents
            .into_iter()
            .find(|agent| agent.id == agent_id)
            .is_some_and(|agent| {
                agent
                    .ratings
                    .iter()
                    .any(|candidate| normalize_generation_rating(candidate) == expected)
            }))
    }

    /// Resolve a receipt by id: in-memory cache first (fast path, and the seam
    /// tests poke to flip `explicit_capable_agent`), then the persisted
    /// `generation_receipt` table (survives restarts). Expiry is enforced by the
    /// repository read (expired rows are swept there), so a DB miss here means
    /// the receipt is genuinely gone — unknown OR expired — and both map to the
    /// same `was not found or has expired` error (message preserved). A DB hit
    /// is folded into the cache so subsequent same-process reads are cheap.
    pub(super) async fn lookup_generation_receipt(
        &self,
        generation_id: &str,
    ) -> Result<GenerationReceiptRecord> {
        if let Some(cached) = self
            .generation_receipts
            .read()
            .map_err(|_| poisoned_lock("generation receipts read lock"))?
            .get(generation_id)
            .cloned()
        {
            return Ok(cached);
        }

        let stored = self
            .repository
            .get_generation_receipt(generation_id)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("generation_id {generation_id:?} was not found or has expired")
            })?;

        let stored_id = stored.id.clone();
        let model_call_ids = self
            .repository
            .pool()
            .read(move |conn| {
                let raw: Option<String> = conn
                    .query_row(
                        "SELECT model_call_ids FROM generation_receipt WHERE id = ?1",
                        rusqlite::params![stored_id],
                        |row| row.get(0),
                    )
                    .ok();
                Ok(raw
                    .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                    .unwrap_or_default())
            })
            .await
            .unwrap_or_default();

        let receipt = GenerationReceiptRecord {
            id: stored.id,
            route: stored.route,
            rating: stored.rating,
            agent_id: stored.agent_id,
            output_sha256: stored.output_sha256,
            output_text: stored.output_text,
            explicit_capable_agent: stored.explicit_capable,
            model_call_ids,
        };
        self.generation_receipts
            .write()
            .map_err(|_| poisoned_lock("generation receipts write lock"))?
            .insert(receipt.id.clone(), receipt.clone());
        Ok(receipt)
    }

    /// Look up + validate a draft generation receipt by id. Used by
    /// `revise_generation` to confirm the receipt is suitable for revision
    /// through the model router.
    ///
    /// Rating gate semantics: revisions are allowed at ANY rating the source
    /// receipt was produced at — including `general`, `teen`, `mature`, and
    /// `explicit`. The original implementation gated this to `explicit`
    /// only, on the (mistaken) assumption that revise_generation was an
    /// explicit-content tool. In practice operators want server-tracked
    /// surgical revisions of mature drafts too; without that, small edits
    /// force a full re-roll through `continue_generation`. The
    /// `explicit_capable_agent` check is kept but applies only when the
    /// receipt's rating IS explicit — for lower ratings any agent that
    /// produced the draft is fine to revise it.
    pub(super) async fn verified_revisable_draft_receipt(
        &self,
        generation_id: Option<&str>,
    ) -> Result<Option<GenerationReceiptRecord>> {
        let Some(generation_id) = generation_id
            .map(str::trim)
            .filter(|generation_id| !generation_id.is_empty())
        else {
            return Ok(None);
        };

        let receipt = self.lookup_generation_receipt(generation_id).await?;

        if receipt.route != "draft" {
            anyhow::bail!(
                "generation_id {generation_id:?} was produced for route {:?}, not \"draft\"",
                receipt.route
            );
        }
        // Explicit-content integrity: when the source receipt was explicit,
        // the producing agent must also be explicit-capable. For non-explicit
        // ratings this constraint doesn't apply — those drafts can come from
        // any configured drafting agent.
        if receipt.rating.as_deref() == Some("explicit") && !receipt.explicit_capable_agent {
            anyhow::bail!(
                "generation_id {generation_id:?} was produced by agent {:?}, which is not explicit-capable",
                receipt.agent_id
            );
        }
        if receipt.output_text.is_empty() {
            anyhow::bail!("generation_id {generation_id:?} references an empty draft output");
        }

        Ok(Some(receipt))
    }

    pub(super) async fn verified_explicit_save_receipt(
        &self,
        generation_id: Option<&str>,
    ) -> Result<GenerationReceiptRecord> {
        let receipt = self
            .verified_revisable_draft_receipt(generation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("generation_id is required for explicit draft saves"))?;
        if receipt.rating.as_deref() != Some("explicit") {
            anyhow::bail!(
                "generation_id {:?} was produced with rating {:?}, not \"explicit\"",
                receipt.id,
                receipt.rating.as_deref().unwrap_or("default")
            );
        }
        Ok(receipt)
    }

    /// Drive a deterministic, branch-creating alternative-generation
    /// pass: for each requested alternative (2..=5), spin off a new
    /// "alternative" branch from the active branch, switch into it,
    /// synthesise a scene draft from the freshly-loaded scene context,
    /// then restore the original active branch. Mirrors
    /// services/mod.rs:7835..7928 in 705b835^. No LLM — the prose
    /// synthesis is purely heuristic (`synthesize_alternative_scene` +
    /// `alternative_tone`) so MCP callers get a stable, testable
    /// fan-out.
    pub async fn generate_alternatives(
        &self,
        input: spindle_core::models::GenerateAlternativesInput,
    ) -> Result<spindle_core::models::GenerateAlternativesOutput> {
        use spindle_core::models::{
            ContentRating, ContextFormat, CreateBranchInput, GenerateAlternativesOutput,
            GeneratedAlternative, GetSceneContextInput, SaveSceneDraftInput, SwitchBranchInput,
        };

        let project = self.repository.get_project(&input.project_id).await?;
        let starting_branch = self.repository.get_active_branch(&input.project_id).await?;
        let alternative_count = input.alternatives.unwrap_or(3).clamp(2, 5);

        let context = self
            .get_scene_context(GetSceneContextInput {
                project_id: input.project_id.clone(),
                book_number: input.book_number,
                chapter_number: input.chapter_number,
                chapter_id: None,
                scene_order: input.scene_order,
                character_ids: input.character_ids.clone(),
                max_character_count: None,
                location_id: input.location_id.clone(),
                format: Some(ContextFormat::Json),
                budget_tokens: Some(3000),
                token_budget: Some(3000),
                sections: None,
            })
            .await?;

        let mut generated = Vec::new();
        for index in 0..alternative_count {
            let branch_name = format!(
                "alt-{}-{}-{}-{}",
                input.variation_strategy,
                input.book_number,
                input.chapter_number,
                index + 1
            );
            let branch_out = self
                .create_branch(CreateBranchInput {
                    project_id: input.project_id.clone(),
                    parent_branch_id: Some(starting_branch.id.clone()),
                    name: branch_name.clone(),
                    branch_type: "alternative".into(),
                    description: Some(format!(
                        "Generated {} alternative {}",
                        input.variation_strategy,
                        index + 1
                    )),
                })
                .await?;
            self.switch_branch(SwitchBranchInput {
                project_id: input.project_id.clone(),
                branch_id: branch_out.branch_id.clone(),
            })
            .await?;

            let (summary, full_text) = synthesize_alternative_scene(&context, &input, index);
            let scene = self
                .save_scene_draft(SaveSceneDraftInput {
                    project_id: input.project_id.clone(),
                    book_number: input.book_number,
                    chapter_number: input.chapter_number,
                    chapter_id: None,
                    scene_order: input.scene_order,
                    full_text,
                    summary: summary.clone(),
                    content_rating: ContentRating::Teen,
                    tone: Some(alternative_tone(&input.variation_strategy, index).to_string()),
                    generation_id: None,
                    source_path: None,
                    ..Default::default()
                })
                .await?;

            generated.push(GeneratedAlternative {
                branch_id: branch_out.branch_id,
                branch_name,
                summary,
                scene_id: scene.scene_id,
                variation_strategy: input.variation_strategy.clone(),
            });
        }

        // Restore the project's active branch (the SurrealDB reference
        // pointed back at the singleton `bible_branch:main`; here we
        // use the project's recorded `active_branch_id`, falling back to
        // the starting branch when the field is unset).
        let restore_target = project
            .active_branch_id
            .clone()
            .unwrap_or_else(|| starting_branch.id.clone());
        self.switch_branch(SwitchBranchInput {
            project_id: input.project_id.clone(),
            branch_id: restore_target,
        })
        .await?;

        Ok(GenerateAlternativesOutput {
            context,
            alternatives: generated,
        })
    }

    /// Rank a set of feature branches by a heuristic score over the
    /// latest scene draft on each. Used by the alternatives workflow
    /// before `select_alternative`. Pure SQLite reads + four
    /// deterministic scoring helpers below; no LLM.
    pub async fn compare_alternatives(
        &self,
        input: spindle_core::models::CompareAlternativesInput,
    ) -> Result<spindle_core::models::CompareAlternativesOutput> {
        use spindle_core::models::{AlternativeComparison, CompareAlternativesOutput};

        self.repository.get_project(&input.project_id).await?;

        let mut comparisons = Vec::new();
        for branch_id_str in &input.branch_ids {
            let branch = self.repository.get_branch(branch_id_str).await?;
            if branch
                .project_id
                .as_deref()
                .is_some_and(|id| id != input.project_id)
            {
                anyhow::bail!(
                    "alternative branch {} does not belong to project {}",
                    branch_id_str,
                    input.project_id
                );
            }
            let scenes = self
                .repository
                .list_scenes_by_project_and_branch(&input.project_id, branch_id_str)
                .await?;
            let latest_scene = scenes.last().ok_or_else(|| {
                anyhow::anyhow!("alternative branch {branch_id_str} has no scene draft")
            })?;

            let quality_score = score_alternative_scene(latest_scene);
            comparisons.push(AlternativeComparison {
                branch_id: branch.id.clone(),
                branch_name: branch.name,
                summary: latest_scene.summary.clone(),
                quality_score,
                strongest_trait: strongest_trait_for_scene(latest_scene),
                pacing_note: pacing_note_for_scene(latest_scene),
                hook_note: hook_note_for_scene(latest_scene),
            });
        }

        comparisons.sort_by(|left, right| {
            right
                .quality_score
                .cmp(&left.quality_score)
                .then_with(|| left.branch_name.cmp(&right.branch_name))
        });
        let recommended_branch_id = comparisons.first().map(|item| item.branch_id.clone());

        Ok(CompareAlternativesOutput {
            alternatives: comparisons,
            recommended_branch_id,
        })
    }

    /// Promote an alternative branch to the project's main and switch the
    /// active branch onto it. Wraps `merge_branch` against the project's
    /// `main` branch in the per-project design (see Risk #6).
    ///
    /// Reference: services/mod.rs:8034..8086 in 705b835^. The reference
    /// post-merge call did `switch_active_branch(project, RecordId::new(
    /// "bible_branch", "main"))` — here we look up the per-project main
    /// branch by name first, then switch to its id.
    ///
    /// Conflict behavior: if the underlying merge reports a conflict,
    /// `select_alternative` bails with a descriptive error rather than
    /// silently leaving the branch un-promoted. The caller is expected
    /// to resolve the conflict and re-run.
    pub async fn select_alternative(
        &self,
        input: spindle_core::models::SelectAlternativeInput,
    ) -> Result<spindle_core::models::SelectAlternativeOutput> {
        use spindle_core::models::{MergeBranchInput, SelectAlternativeOutput};

        let project = self.repository.get_project(&input.project_id).await?;
        let branch = self.repository.get_branch(&input.branch_id).await?;
        if branch
            .project_id
            .as_deref()
            .is_some_and(|id| id != project.id)
        {
            anyhow::bail!(
                "selected branch {} does not belong to project {}",
                branch.id,
                project.id
            );
        }

        // Per-project main lookup (Risk #6): resolve "this project's main
        // branch" by name. Captured up front so we can use the same id
        // for both the merge target and the post-merge active-branch
        // switch.
        let main_branch = self
            .repository
            .list_branches_by_project(&project.id)
            .await?
            .into_iter()
            .find(|b| b.name == "main")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "project {} has no main branch; cannot select alternative",
                    project.id
                )
            })?;

        let merged = self
            .merge_branch(MergeBranchInput {
                project_id: input.project_id.clone(),
                source_branch_id: input.branch_id.clone(),
                target_branch_id: Some(main_branch.id.clone()),
                merge_type: "fast_forward".to_string(),
            })
            .await?;
        if merged.has_conflicts {
            let position = merged
                .conflicts
                .first()
                .map(|c| format!("{}:{}:{}", c.book_number, c.chapter_number, c.scene_order))
                .unwrap_or_else(|| "unknown position".to_string());
            anyhow::bail!(
                "select_alternative is blocked by an unresolved scene conflict at {position}"
            );
        }
        self.repository
            .switch_active_branch(&project.id, &main_branch.id)
            .await?;

        Ok(SelectAlternativeOutput {
            selected_branch_id: input.branch_id,
            target_branch_id: merged.target_branch_id,
            merge_type: merged.merge_type,
        })
    }

    /// Heuristic canonical-fact extractor: splits the scene's full_text on
    /// sentence delimiters and proposes the first 12 candidate sentences as
    /// `subject_table = "scene"` invariants. Pure deterministic projection
    /// over the persisted scene — no LLM call, so the output is stable
    /// across runs and safe to use as the seed for a downstream
    /// review/accept loop. Mirrors the SurrealDB reference exactly
    /// (services/mod.rs:4570..4610 in 705b835^).
    pub async fn extract_canonical_facts_from_scene(
        &self,
        input: spindle_core::models::ExtractCanonicalFactsFromSceneInput,
    ) -> Result<spindle_core::models::ExtractCanonicalFactsFromSceneOutput> {
        use spindle_core::models::{
            CanonicalFactScope, ExtractCanonicalFactProposal, ExtractCanonicalFactsFromSceneOutput,
        };

        let scene = self.repository.get_scene(&input.scene_id).await?;
        let scene_id_string = scene.id.clone();
        let proposals = split_scene_into_fact_sentences(&scene.full_text)
            .into_iter()
            .take(12)
            .enumerate()
            .map(|(idx, sentence)| {
                let predicate = format!(
                    "scene.extracted_{}.{}",
                    idx + 1,
                    canonical_predicate_slug(&sentence)
                );
                ExtractCanonicalFactProposal {
                    subject_table: "scene".to_string(),
                    subject_id: Some(scene_id_string.clone()),
                    predicate,
                    value_kind: "string".to_string(),
                    value_text: Some(sentence.clone()),
                    value_number: None,
                    value_unit: None,
                    value_json: None,
                    aliases: vec![],
                    scope: Some(CanonicalFactScope::Invariant),
                    valid_from: None,
                    valid_until: None,
                    source_excerpt: Some(sentence),
                    rationale: Some(
                        "Sentence-level extraction proposal from committed scene text.".to_string(),
                    ),
                }
            })
            .collect();
        Ok(ExtractCanonicalFactsFromSceneOutput {
            scene_id: scene_id_string,
            proposals,
        })
    }
}
