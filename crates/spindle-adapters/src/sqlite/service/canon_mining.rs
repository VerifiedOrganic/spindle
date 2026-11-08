use super::*;

impl SqliteSpindleService {
    // ── Canon miner (mine_scene_canon — evolution §3.1, ADR 0001) ────────────

    /// Gather the secret-gated current-state digest for a scene and assemble the
    /// full mining prompt. Split out from [`Self::mine_scene_canon`] so the
    /// secret-withholding contract can be asserted against the assembled prompt
    /// in a test without a model call.
    ///
    /// Scene cast is derived from the prose (present-in-scene name match), the
    /// same way the deterministic secret-leak scan derives it — a committed
    /// scene has no stored cast list. The secret gate is resolved with an empty
    /// circle-of-trust check against that present cast, so a secret fact whose
    /// circle excludes everyone present is stripped from the fact-keys digest.
    pub(super) async fn build_mine_prompt_for_scene(
        &self,
        project_id: &str,
        scene_id: &str,
    ) -> Result<String> {
        use spindle_core::voice::{VoiceDriftCharacter, character_present_in_scene};

        let scene = self.repository.get_scene(scene_id).await?;
        let active_branch = self.repository.get_active_branch(project_id).await?;
        let cursor =
            crate::format::story_index(scene.book_number, scene.chapter_number, scene.scene_order);

        // Present cast (deterministic by id). Every project character whose name
        // appears in the prose.
        let all_characters = self
            .repository
            .list_characters_by_project(project_id)
            .await?;
        let mut present: Vec<&crate::sqlite::records::Character> = all_characters
            .iter()
            .filter(|character| {
                character_present_in_scene(
                    &scene.full_text,
                    &VoiceDriftCharacter {
                        id: character.id.clone(),
                        name: character.name.clone(),
                    },
                )
            })
            .collect();
        present.sort_by(|a, b| a.id.cmp(&b.id));
        let present_ids: Vec<String> = present.iter().map(|c| c.id.clone()).collect();

        let mut digest = MineDigest {
            cast: present
                .iter()
                .take(CANON_MINE_DIGEST_CAP)
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect(),
            ..MineDigest::default()
        };

        // Open promises (any non-paid_off), capped, deterministic by id.
        let mut promises = self
            .repository
            .list_narrative_promises_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        promises.retain(|p| p.status != "paid_off" && p.archived_at.is_none());
        promises.sort_by(|a, b| a.id.cmp(&b.id));
        digest.open_promises = promises
            .iter()
            .take(CANON_MINE_DIGEST_CAP)
            .map(|p| {
                (
                    p.id.clone(),
                    crate::format::truncate_at_chars(&p.description, CANON_MINE_PROMISE_DESC_CAP),
                    p.status.clone(),
                )
            })
            .collect();

        // Active conflicts (unarchived), with undelivered-consequence and
        // undemonstrated-escalation indexes so the model can propose closures.
        let mut conflicts = self
            .repository
            .list_conflicts_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        conflicts.retain(|c| c.archived_at.is_none() && c.resolution_summary.is_none());
        conflicts.sort_by(|a, b| a.id.cmp(&b.id));
        digest.active_conflicts = conflicts
            .iter()
            .take(CANON_MINE_DIGEST_CAP)
            .map(|c| {
                let undelivered: Vec<usize> = c
                    .stated_consequences
                    .iter()
                    .enumerate()
                    .filter(|(_, cons)| !cons.delivered)
                    .map(|(i, _)| i)
                    .collect();
                let undemonstrated: Vec<usize> = (0..c.escalation_stages.len())
                    .filter(|i| {
                        c.escalation_demonstrated
                            .get(*i)
                            .and_then(|d| d.as_ref())
                            .is_none()
                    })
                    .collect();
                (c.id.clone(), c.name.clone(), undelivered, undemonstrated)
            })
            .collect();

        // Arcs for cast (unreached milestone labels).
        let mut arcs = self
            .repository
            .list_character_arcs_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        arcs.retain(|a| present_ids.contains(&a.character_id) && a.archived_at.is_none());
        arcs.sort_by(|a, b| a.id.cmp(&b.id));
        digest.arcs = arcs
            .iter()
            .take(CANON_MINE_DIGEST_CAP)
            .filter_map(|arc| {
                let labels: Vec<String> = arc
                    .milestones
                    .iter()
                    .filter(|m| m.reached_at.is_none())
                    .map(|m| m.label.clone())
                    .collect();
                (!labels.is_empty()).then(|| (arc.id.clone(), labels))
            })
            .collect();

        // Fact keys per cast subject — secret-gated. A secret fact whose circle
        // excludes the present cast is stripped before it can reach the digest.
        let name_of: std::collections::BTreeMap<String, String> = all_characters
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        let pov_character_id = self
            .repository
            .list_chapter_plans_by_project(project_id)
            .await?
            .into_iter()
            .find(|plan| {
                plan.book_number == scene.book_number && plan.chapter_number == scene.chapter_number
            })
            .and_then(|plan| plan.pov_character_id);

        for character in &present {
            let facts = self
                .repository
                .list_canonical_facts_by_subject(
                    project_id,
                    &active_branch.id,
                    "character",
                    Some(&character.id),
                )
                .await?;
            if facts.is_empty() {
                continue;
            }
            // Resolve the secret gate for this subject's facts against the
            // present cast; withheld facts drop out of the key list.
            let secret_facts: Vec<crate::sqlite::records::CanonicalFact> =
                facts.iter().filter(|f| f.secret).cloned().collect();
            let gate = if secret_facts.is_empty() {
                SceneSecretGate::default()
            } else {
                self.resolve_scene_secret_gate(
                    project_id,
                    &active_branch.id,
                    &secret_facts,
                    &SceneSecretGateParams {
                        scene_cast: &present_ids,
                        pov: pov_character_id.as_deref(),
                        cursor,
                        name_of: &name_of,
                    },
                )
                .await?
            };
            let mut keys: Vec<String> = facts
                .iter()
                .filter(|f| !gate.gated_fact_ids.contains(&f.id))
                .map(|f| f.predicate.clone())
                .collect();
            keys.sort();
            keys.dedup();
            keys.truncate(CANON_MINE_DIGEST_CAP);
            if !keys.is_empty() {
                digest.fact_keys.push((character.name.clone(), keys));
            }
        }

        // Quantity-scheme name (first scheme's measure), if any is set.
        let quantity_scheme_name = self
            .repository
            .list_project_quantity_schemes(project_id, &active_branch.id)
            .await?
            .first()
            .map(|s| s.scheme.measure.clone());

        Ok(build_mine_prompt(
            &scene.full_text,
            &build_mine_digest(&digest),
            quantity_scheme_name.as_deref(),
        ))
    }

    /// Mine one committed scene into proposed canon deltas (evolution §3.1, ADR
    /// 0001). One rating-gated model call assembles staged rows the operator
    /// later ratifies; nothing writes to the bible here (invariant I4).
    ///
    /// Dispatch follows the design §2.3 fallback ladder: try the `mine` route,
    /// then `review` on NoRoute, both at the scene's content rating. A
    /// RatingNotCovered (or exhausted ladder) is an honest skip — the
    /// rating-gated chokepoint inside `complete()` guarantees no prose-bearing
    /// request ever reaches an uncleared agent. Malformed model output rejects
    /// the whole batch (`model_output_rejected`) rather than guessing.
    pub async fn mine_scene_canon(
        &self,
        input: spindle_core::models::MineSceneCanonInput,
    ) -> Result<spindle_core::models::MineSceneCanonOutput> {
        use crate::ai::{ModelRequest, RouteClearanceError};
        use spindle_core::models::MineSceneCanonOutput;

        self.repository.get_project(&input.project_id).await?;
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id {
            anyhow::bail!(
                "scene {} does not belong to project {}",
                input.scene_id,
                input.project_id
            );
        }
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;

        // Empty prose → skip before any model call (I8 honest skip).
        if scene.full_text.trim().is_empty() {
            return Ok(MineSceneCanonOutput {
                staged: Vec::new(),
                discarded_count: 0,
                superseded_count: 0,
                status: "skipped".to_string(),
                skip_reason: Some("empty scene".to_string()),
            });
        }

        let prompt = self
            .build_mine_prompt_for_scene(&input.project_id, &input.scene_id)
            .await?;
        let rating = Some(scene.content_rating.trim().to_ascii_lowercase());

        let mine_context = Some(crate::ai::RequestContext {
            project_id: Some(input.project_id.clone()),
            scene_id: Some(input.scene_id.clone()),
            ..Default::default()
        });

        // Fallback ladder (§2.3): mine → review (on NoRoute) → skip. The
        // chokepoint inside complete() gates the rating for the uncleared case,
        // so no prose leaves for an uncleared agent.
        let response = match self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "mine".to_string(),
                prompt: prompt.clone(),
                rating: rating.clone(),
                context: mine_context.clone(),
            })
            .await
        {
            Ok(response) => response,
            Err(err) => {
                let is_no_route = matches!(
                    err.downcast_ref::<RouteClearanceError>(),
                    Some(RouteClearanceError::NoRoute { .. })
                );
                if is_no_route {
                    // Retry via the review route (same rating).
                    match self
                        .repository
                        .model_router()
                        .complete(&ModelRequest {
                            route: "review".to_string(),
                            prompt: prompt.clone(),
                            rating: rating.clone(),
                            context: mine_context,
                        })
                        .await
                    {
                        Ok(response) => response,
                        Err(review_err) => {
                            return Ok(mine_skip_output(&review_err, "review"));
                        }
                    }
                } else {
                    return Ok(mine_skip_output(&err, "mine"));
                }
            }
        };

        // Strict parse; malformed → reject the whole batch, stage nothing.
        let raw_deltas = match parse_canon_mine_output(&response.output) {
            Ok(deltas) => deltas,
            Err(_) => {
                return Ok(MineSceneCanonOutput {
                    staged: Vec::new(),
                    discarded_count: 0,
                    superseded_count: 0,
                    status: "model_output_rejected".to_string(),
                    skip_reason: None,
                });
            }
        };

        // Validate each delta (ADR lifecycle M3). Survivors keep prompt order.
        use crate::sqlite::repository::StageCanonDeltaParams;
        let mut valid: Vec<StageCanonDeltaParams> = Vec::new();
        let mut discarded_count = 0usize;
        for raw in raw_deltas {
            match validate_canon_delta(&raw, &scene.full_text) {
                Some((delta_class, target_id, confidence, evidence, payload)) => {
                    valid.push(StageCanonDeltaParams {
                        project_id: input.project_id.clone(),
                        branch_id: active_branch.id.clone(),
                        scene_id: input.scene_id.clone(),
                        authoring_run_id: None,
                        delta_class,
                        target_id,
                        payload,
                        evidence,
                        confidence,
                    });
                }
                None => discarded_count += 1,
            }
        }

        // Supersede prior staged rows for this scene, then stage survivors in
        // prompt order (ADR D3).
        let superseded_count = self
            .repository
            .supersede_scene_deltas(&input.scene_id)
            .await? as usize;

        let mut staged = Vec::with_capacity(valid.len());
        for params in valid {
            let stored = self.repository.stage_canon_delta(params).await?;
            staged.push(stored.into_core());
        }

        Ok(MineSceneCanonOutput {
            staged,
            discarded_count,
            superseded_count,
            status: "staged".to_string(),
            skip_reason: None,
        })
    }
}
