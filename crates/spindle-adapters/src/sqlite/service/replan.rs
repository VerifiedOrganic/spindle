use super::*;

impl SqliteSpindleService {
    /// The replan differ (evolution §3.5, ADR 0003). Compares the realized
    /// reality of one just-summarized chapter (its summary, beat annotations,
    /// current promise/arc states) against every not-yet-drafted future chapter's
    /// plan in the same book, staging plan-amendment proposals the operator later
    /// ratifies (Part B). Nothing writes to the outline here (ADR D5
    /// no-auto-accept) — every applied amendment is a human decision.
    ///
    /// **Non-prose-bearing (ADR D5).** The differ's inputs are summaries and
    /// metadata only — never scene prose — so the `replan` route carries no
    /// rating clearance: every dispatch here uses `rating: None`. This keeps the
    /// route outside the rating perimeter by construction, the same reasoning
    /// class as the import exemption.
    ///
    /// Dispatch follows the design §2.3 ladder: try the `replan` route (absent
    /// from `default_routes` by design — never added), then `review` on NoRoute,
    /// then an honest skip on transport error (never an error return, never
    /// blocks a run).
    pub async fn replan_chapter(
        &self,
        input: spindle_core::models::ReplanChapterInput,
    ) -> Result<spindle_core::models::ReplanChapterOutput> {
        use crate::ai::{ModelRequest, RouteClearanceError};
        use spindle_core::models::ReplanChapterOutput;

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;

        // Eligibility 1: the source chapter must have a saved summary (else
        // there is no realized reality to compare against — honest skip).
        let source_summary = self
            .repository
            .get_chapter_summary(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.source_chapter,
            )
            .await?;
        let source_summary = match source_summary {
            Some(summary) => summary,
            None => {
                return Ok(ReplanChapterOutput {
                    staged: Vec::new(),
                    discarded_count: 0,
                    superseded_count: 0,
                    status: "no_summary".to_string(),
                    skip_reason: None,
                });
            }
        };

        // Eligibility 2: the target set is chapters WITH plans, strictly greater
        // chapter_number, same book, having ZERO persisted scenes on the active
        // branch at staging time (cheap sanity — Part B re-checks the
        // immutability guard at apply, ADR D3).
        let plans = self
            .repository
            .list_chapter_plans_by_project(&input.project_id)
            .await?;
        let scenes = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;
        let drafted_chapters: std::collections::BTreeSet<i32> = scenes
            .iter()
            .filter(|s| s.book_number == input.book_number)
            .map(|s| s.chapter_number)
            .collect();
        let mut targets: Vec<&crate::sqlite::records::ChapterPlan> = plans
            .iter()
            .filter(|plan| {
                plan.book_number == input.book_number
                    && plan.chapter_number > input.source_chapter
                    && !drafted_chapters.contains(&plan.chapter_number)
            })
            .collect();
        targets.sort_by_key(|plan| plan.chapter_number);
        let eligible_chapters: std::collections::BTreeSet<i32> =
            targets.iter().map(|plan| plan.chapter_number).collect();

        if targets.is_empty() {
            return Ok(ReplanChapterOutput {
                staged: Vec::new(),
                discarded_count: 0,
                superseded_count: 0,
                status: "no_targets".to_string(),
                skip_reason: None,
            });
        }

        // Current promise states (id + status + urgency verdict), deterministic.
        let mut promises = self
            .repository
            .list_narrative_promises_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;
        promises.retain(|p| p.archived_at.is_none());
        promises.sort_by(|a, b| a.id.cmp(&b.id));
        let source_cursor = crate::format::story_index(input.book_number, input.source_chapter, 1);
        let promise_states: Vec<(String, String, String)> = promises
            .iter()
            .take(REPLAN_DIGEST_CAP)
            .map(|p| {
                let verdict = crate::format::promise_timing_verdict(p, source_cursor);
                (
                    p.id.clone(),
                    p.status.clone(),
                    format!("{:?}", verdict.urgency).to_ascii_lowercase(),
                )
            })
            .collect();

        // Arc tracker states for the project's arcs (id + status + progress).
        let mut arcs = self
            .repository
            .list_character_arcs_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;
        arcs.retain(|a| a.archived_at.is_none());
        arcs.sort_by(|a, b| a.id.cmp(&b.id));
        let arc_states: Vec<(String, String, f64)> = arcs
            .iter()
            .take(REPLAN_DIGEST_CAP)
            .map(|a| (a.id.clone(), a.status.clone(), a.progress))
            .collect();

        // Source chapter's realized beat annotations: motif/theme/conflict ids +
        // intensities across its scenes, deterministic by scene order.
        let mut source_scenes: Vec<&crate::sqlite::records::Scene> = scenes
            .iter()
            .filter(|s| {
                s.book_number == input.book_number && s.chapter_number == input.source_chapter
            })
            .collect();
        source_scenes.sort_by_key(|s| s.scene_order);
        let mut source_beats: Vec<ReplanBeatDigest> = Vec::new();
        for scene in source_scenes.iter().take(REPLAN_DIGEST_CAP) {
            if let Some(ann) = self
                .repository
                .get_scene_beat_annotation(&active_branch.id, &scene.id)
                .await?
            {
                if ann.motif_ids.is_empty()
                    && ann.theme_ids.is_empty()
                    && ann.conflict_ids.is_empty()
                    && ann.intensity.is_none()
                {
                    continue;
                }
                source_beats.push(ReplanBeatDigest {
                    scene_order: scene.scene_order,
                    motif_ids: ann.motif_ids.clone(),
                    theme_ids: ann.theme_ids.clone(),
                    conflict_ids: ann.conflict_ids.clone(),
                    intensity: ann.intensity,
                });
            }
        }

        // Assemble the realized digest for the source chapter + the target plans.
        let realized = ReplanRealizedDigest {
            source_chapter: input.source_chapter,
            summary: crate::format::truncate_at_chars(&source_summary.summary, REPLAN_TEXT_CAP),
            key_events: cap_string_list(&source_summary.key_events),
            character_changes: cap_string_list(&source_summary.character_changes),
            relationship_shifts: cap_string_list(&source_summary.relationship_shifts),
            arc_advances: cap_string_list(&source_summary.arc_advances),
            promise_events: cap_string_list(&source_summary.promise_events),
            beats: source_beats,
            promise_states,
            arc_states,
        };
        let target_digests: Vec<ReplanTargetDigest> = targets
            .iter()
            .map(|plan| ReplanTargetDigest::from_plan(plan))
            .collect();
        let prompt = build_replan_prompt(&realized, &target_digests);

        let replan_context = Some(crate::ai::RequestContext {
            project_id: Some(input.project_id.clone()),
            ..Default::default()
        });

        // Dispatch ladder (§2.3): replan → review (on NoRoute) → skip. The
        // route is non-prose-bearing, so rating is None throughout.
        let response = match self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "replan".to_string(),
                prompt: prompt.clone(),
                rating: None,
                context: replan_context.clone(),
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
                    // Retry via the review route (non-prose-bearing, rating None).
                    match self
                        .repository
                        .model_router()
                        .complete(&ModelRequest {
                            route: "review".to_string(),
                            prompt: prompt.clone(),
                            rating: None,
                            context: replan_context,
                        })
                        .await
                    {
                        Ok(response) => response,
                        Err(review_err) => {
                            return Ok(replan_skip_output(&review_err, "review"));
                        }
                    }
                } else {
                    return Ok(replan_skip_output(&err, "replan"));
                }
            }
        };

        // Strict parse; malformed → reject the whole batch, stage nothing.
        let raw_amendments = match parse_replan_output(&response.output) {
            Ok(amendments) => amendments,
            Err(_) => {
                return Ok(ReplanChapterOutput {
                    staged: Vec::new(),
                    discarded_count: 0,
                    superseded_count: 0,
                    status: "model_output_rejected".to_string(),
                    skip_reason: None,
                });
            }
        };

        // Validate each amendment (ADR D5 discard discipline). Survivors keep
        // prompt order; the per-pass cap of 8 is applied AFTER validation, and
        // overflow counts toward `discarded_count`.
        use crate::sqlite::repository::StagePlanAmendmentParams;
        let mut valid: Vec<StagePlanAmendmentParams> = Vec::new();
        let mut discarded_count = 0usize;
        for raw in raw_amendments {
            match validate_plan_amendment(&raw, &eligible_chapters) {
                Some((amendment_class, target_chapter, confidence, rationale, payload)) => {
                    if valid.len() >= REPLAN_STAGE_CAP {
                        // Overflow beyond the cap is dropped and counted.
                        discarded_count += 1;
                        continue;
                    }
                    valid.push(StagePlanAmendmentParams {
                        project_id: input.project_id.clone(),
                        branch_id: active_branch.id.clone(),
                        source_chapter: input.source_chapter,
                        book_number: input.book_number,
                        authoring_run_id: None,
                        amendment_class,
                        target_chapter,
                        payload,
                        rationale,
                        confidence,
                    });
                }
                None => discarded_count += 1,
            }
        }

        // Supersede prior staged rows for this source chapter, then stage
        // survivors in prompt order (ADR D2 supersede-on-replan).
        let superseded_count = self
            .repository
            .supersede_source_chapter_amendments(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.source_chapter,
            )
            .await? as usize;

        let mut staged = Vec::with_capacity(valid.len());
        for params in valid {
            let stored = self.repository.stage_plan_amendment(params).await?;
            staged.push(stored.into_core());
        }

        Ok(ReplanChapterOutput {
            staged,
            discarded_count,
            superseded_count,
            status: "staged".to_string(),
            skip_reason: None,
        })
    }
}
