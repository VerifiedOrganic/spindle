use super::*;

pub(super) fn chapter_range_end(start_chapter: i32, chapter_count: i32) -> Option<i32> {
    chapter_count
        .checked_sub(1)
        .and_then(|c| start_chapter.checked_add(c))
}

pub(super) fn authoring_scene_indices(
    state: &HarnessState,
    chapter_number: i32,
    scene_order: i32,
) -> Option<(usize, usize)> {
    let chapter_index = state
        .chapters
        .iter()
        .position(|chapter| chapter.chapter_number == chapter_number)?;
    let scene_index = state.chapters[chapter_index]
        .scenes
        .iter()
        .position(|scene| scene.scene_order == scene_order)?;
    Some((chapter_index, scene_index))
}

pub(super) fn authoring_artifacts_root(state_path: &Path, state: &HarnessState) -> PathBuf {
    let parent = state_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(&state.artifacts_dir)
}

pub(super) fn authoring_state_path(data_dir: &Path, run_id: &str) -> PathBuf {
    spindle_adapters::workspace::runtime_dir(data_dir).join(format!(
        "authoring_run_{}_temp.json",
        run_id.replace(":", "_")
    ))
}

pub(super) fn map_harness_to_records(
    run_id: &str,
    state: &spindle_harness::state::HarnessState,
    status: &str,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
) -> (
    spindle_adapters::sqlite::records::AuthoringRun,
    Vec<spindle_adapters::sqlite::records::AuthoringRunChapter>,
    Vec<spindle_adapters::sqlite::records::AuthoringRunScene>,
    Vec<spindle_adapters::sqlite::records::AuthoringCheckpoint>,
) {
    let now = chrono::Utc::now();
    let run = spindle_adapters::sqlite::records::AuthoringRun {
        id: run_id.to_string(),
        project_id: state.project_id.clone(),
        active_branch_id: state.active_branch_id.clone(),
        book_number: state.book_number,
        start_chapter: state.range.start_chapter,
        end_chapter: state.range.end_chapter,
        checkpoint_interval: state.checkpoint_interval as i64,
        last_checkpoint_end_chapter: state.last_checkpoint_end_chapter,
        artifacts_dir: state.artifacts_dir.clone(),
        editorial_directives: state.editorial_directives.clone(),
        status: status.to_string(),
        created_at: created_at.unwrap_or(now),
        updated_at: now,
        mining_policy: state.mining_policy.clone(),
        max_revise_attempts: state.max_revise_attempts,
        checkpoint_policy: state.checkpoint_policy.clone(),
        replan_policy: state.replan_policy.clone(),
    };

    let mut chapters = Vec::new();
    let mut scenes = Vec::new();
    for ch in &state.chapters {
        let ch_status = match ch.status {
            spindle_harness::state::ChapterStatus::Pending => "pending",
            spindle_harness::state::ChapterStatus::InProgress => "in_progress",
            spindle_harness::state::ChapterStatus::Complete => "complete",
        };
        chapters.push(spindle_adapters::sqlite::records::AuthoringRunChapter {
            authoring_run_id: run_id.to_string(),
            chapter_number: ch.chapter_number,
            planned: ch.planned,
            synopsis: ch.synopsis.clone(),
            pov_character_id: ch.pov_character_id.clone(),
            status: ch_status.to_string(),
            summary_saved: ch.summary_saved,
            summary_artifact_path: ch.summary_artifact_path.clone(),
            replan_status: ch.replan_status.clone(),
            replan_detail: ch.replan_detail.clone(),
        });

        for sc in &ch.scenes {
            let sc_phase = match sc.phase {
                spindle_harness::state::ScenePhase::Pending => "pending",
                spindle_harness::state::ScenePhase::DraftSaved => "draft_saved",
                spindle_harness::state::ScenePhase::ChangesCommitted => "changes_committed",
                spindle_harness::state::ScenePhase::BeatsAnnotated => "beats_annotated",
            };
            scenes.push(spindle_adapters::sqlite::records::AuthoringRunScene {
                authoring_run_id: run_id.to_string(),
                chapter_number: ch.chapter_number,
                scene_order: sc.scene_order,
                character_ids: sc.character_ids.clone(),
                location_id: sc.location_id.clone(),
                content_rating: sc.content_rating.as_str().to_string(),
                tone: sc.tone.clone(),
                source_path: sc.source_path.clone(),
                phase: sc_phase.to_string(),
                scene_id: sc.scene_id.clone(),
                scene_artifact_path: sc.scene_artifact_path.clone(),
                draft_diagnostics: sc
                    .draft_diagnostics
                    .as_ref()
                    .map(|d| serde_json::to_value(d).unwrap()),
                blocked_reason: sc.blocked_reason.clone(),
                research_required: sc.research_required,
                research_tags: sc.research_tags.clone(),
                explicit_query: sc.explicit_query.clone(),
                mine_status: sc.mine_status.clone(),
                mine_detail: sc.mine_detail.clone(),
                verify_status: sc.verify_status.clone(),
                verify_detail: sc.verify_detail.clone(),
                revise_attempts: sc.revise_attempts,
                // The revision-directives block is transient harness state only;
                // it is never persisted to the run tables (evolution §3.2).
                last_finding_fingerprint: sc.last_finding_fingerprint.clone(),
            });
        }
    }

    let mut checkpoints = Vec::new();
    for cp in &state.checkpoint_history {
        let cp_status = match cp.status {
            spindle_harness::state::CheckpointStatus::PendingReview => "pending_review",
            spindle_harness::state::CheckpointStatus::Reviewed => "reviewed",
        };
        checkpoints.push(spindle_adapters::sqlite::records::AuthoringCheckpoint {
            authoring_run_id: run_id.to_string(),
            start_chapter: cp.start_chapter,
            end_chapter: cp.end_chapter,
            save_point_id: cp.save_point_id.clone(),
            status: cp_status.to_string(),
            report_artifact_path: cp.report_artifact_path.clone(),
            auto_outcome: cp.auto_outcome.clone(),
            pending_manual_scene_ids: cp.pending_manual_scene_ids.clone(),
        });
    }

    (run, chapters, scenes, checkpoints)
}

pub(super) fn map_records_to_harness(
    run: &spindle_adapters::sqlite::records::AuthoringRun,
    chapters: &[spindle_adapters::sqlite::records::AuthoringRunChapter],
    scenes: &[spindle_adapters::sqlite::records::AuthoringRunScene],
    checkpoints: &[spindle_adapters::sqlite::records::AuthoringCheckpoint],
) -> spindle_harness::state::HarnessState {
    let mut ch_states = Vec::new();
    for ch in chapters {
        let mut ch_scenes = Vec::new();
        for sc in scenes {
            if sc.chapter_number == ch.chapter_number {
                let phase = match sc.phase.as_str() {
                    "pending" => spindle_harness::state::ScenePhase::Pending,
                    "draft_saved" => spindle_harness::state::ScenePhase::DraftSaved,
                    "changes_committed" => spindle_harness::state::ScenePhase::ChangesCommitted,
                    "beats_annotated" => spindle_harness::state::ScenePhase::BeatsAnnotated,
                    _ => spindle_harness::state::ScenePhase::Pending,
                };
                let content_rating = match sc.content_rating.to_ascii_lowercase().as_str() {
                    "general" => spindle_core::models::ContentRating::General,
                    "teen" => spindle_core::models::ContentRating::Teen,
                    "mature" => spindle_core::models::ContentRating::Mature,
                    "explicit" => spindle_core::models::ContentRating::Explicit,
                    _ => spindle_core::models::ContentRating::General,
                };
                let draft_diagnostics = sc
                    .draft_diagnostics
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                ch_scenes.push(spindle_harness::state::SceneState {
                    scene_order: sc.scene_order,
                    character_ids: sc.character_ids.clone(),
                    location_id: sc.location_id.clone(),
                    content_rating,
                    tone: sc.tone.clone(),
                    source_path: sc.source_path.clone(),
                    phase,
                    scene_id: sc.scene_id.clone(),
                    scene_artifact_path: sc.scene_artifact_path.clone(),
                    draft_diagnostics,
                    blocked_reason: sc.blocked_reason.clone(),
                    research_required: sc.research_required,
                    research_tags: sc.research_tags.clone(),
                    explicit_query: sc.explicit_query.clone(),
                    mine_status: sc.mine_status.clone(),
                    mine_detail: sc.mine_detail.clone(),
                    verify_status: sc.verify_status.clone(),
                    verify_detail: sc.verify_detail.clone(),
                    revise_attempts: sc.revise_attempts,
                    last_finding_fingerprint: sc.last_finding_fingerprint.clone(),
                    ..Default::default()
                });
            }
        }
        let status = match ch.status.as_str() {
            "pending" => spindle_harness::state::ChapterStatus::Pending,
            "in_progress" => spindle_harness::state::ChapterStatus::InProgress,
            "complete" => spindle_harness::state::ChapterStatus::Complete,
            _ => spindle_harness::state::ChapterStatus::Pending,
        };
        ch_states.push(spindle_harness::state::ChapterState {
            chapter_number: ch.chapter_number,
            planned: ch.planned,
            synopsis: ch.synopsis.clone(),
            pov_character_id: ch.pov_character_id.clone(),
            status,
            scenes: ch_scenes,
            summary_saved: ch.summary_saved,
            summary_artifact_path: ch.summary_artifact_path.clone(),
            replan_status: ch.replan_status.clone(),
            replan_detail: ch.replan_detail.clone(),
        });
    }

    let mut cp_history = Vec::new();
    for cp in checkpoints {
        let status = match cp.status.as_str() {
            "pending_review" => spindle_harness::state::CheckpointStatus::PendingReview,
            "reviewed" => spindle_harness::state::CheckpointStatus::Reviewed,
            _ => spindle_harness::state::CheckpointStatus::PendingReview,
        };
        cp_history.push(spindle_harness::state::CheckpointRecord {
            start_chapter: cp.start_chapter,
            end_chapter: cp.end_chapter,
            save_point_id: cp.save_point_id.clone(),
            status,
            report_artifact_path: cp.report_artifact_path.clone(),
            auto_outcome: cp.auto_outcome.clone(),
            pending_manual_scene_ids: cp.pending_manual_scene_ids.clone(),
        });
    }

    let mut state = spindle_harness::state::HarnessState {
        project_id: run.project_id.clone(),
        active_branch_id: run.active_branch_id.clone(),
        book_number: run.book_number,
        range: spindle_harness::state::ChapterRange {
            start_chapter: run.start_chapter,
            end_chapter: run.end_chapter,
        },
        checkpoint_interval: run.checkpoint_interval as usize,
        last_checkpoint_end_chapter: run.last_checkpoint_end_chapter,
        artifacts_dir: run.artifacts_dir.clone(),
        editorial_directives: run.editorial_directives.clone(),
        mining_policy: run.mining_policy.clone(),
        max_revise_attempts: run.max_revise_attempts,
        checkpoint_policy: run.checkpoint_policy.clone(),
        replan_policy: run.replan_policy.clone(),
        chapters: ch_states,
        checkpoint_history: cp_history,
    };

    if state.artifacts_dir == "artifacts" {
        state.artifacts_dir = "../artifacts".to_string();
    }
    state
}

impl ToolRouter {
    pub(super) async fn authoring_project_snapshot(
        &self,
        state: &HarnessState,
    ) -> anyhow::Result<ProjectSnapshot> {
        let repo = self.service.repository();
        let active_branch = repo.get_active_branch(&state.project_id).await?;
        let scenes = repo
            .list_scenes_by_project_and_branch(&state.project_id, &active_branch.id)
            .await?;
        let plans = repo
            .list_chapter_plans_by_project(&state.project_id)
            .await?;
        let summaries = repo
            .list_chapter_summaries_by_project(&state.project_id)
            .await?;
        let summarized_chapters = summaries
            .into_iter()
            .filter(|summary| {
                summary.branch_id == active_branch.id && summary.book_number == state.book_number
            })
            .map(|summary| summary.chapter_number)
            .collect::<BTreeSet<_>>();

        let mut chapters = BTreeMap::new();
        for chapter in &state.chapters {
            let chapter_row = repo
                .get_chapter_by_number(&state.project_id, state.book_number, chapter.chapter_number)
                .await
                .with_context(|| {
                    format!(
                        "failed to resolve chapter {} for authoring snapshot",
                        chapter.chapter_number
                    )
                })?;

            let persisted_scenes = scenes
                .iter()
                .filter(|scene| {
                    scene.book_number == state.book_number
                        && scene.chapter_number == chapter.chapter_number
                })
                .map(|scene| {
                    (
                        scene.scene_order,
                        PersistedScene {
                            scene_id: scene.id.clone(),
                            scene_order: scene.scene_order,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();

            let mut chapter_plan = None;
            if let Some(plan) = plans.iter().find(|plan| {
                plan.branch_id == active_branch.id
                    && plan.book_number == state.book_number
                    && plan.chapter_number == chapter.chapter_number
            }) {
                let mut planned_scenes = Vec::new();
                for planned in &plan.scenes {
                    let harness_scene = chapter
                        .scenes
                        .iter()
                        .find(|scene| scene.scene_order == planned.scene_order);
                    let scene_location = planned
                        .location_id
                        .clone()
                        .or_else(|| harness_scene.map(|scene| scene.location_id.clone()));
                    let research_tags = planned.research_tags.clone();
                    let explicit_query = planned.explicit_query.clone();

                    let pack = self
                        .service
                        .research_pack_for_scene(ResearchPackForSceneInput {
                            project_id: state.project_id.clone(),
                            branch_id: Some(active_branch.id.clone()),
                            scene_summary: Some(planned.summary.clone()),
                            scene_location,
                            character_ids: planned.character_ids.clone(),
                            tags: research_tags.clone(),
                            explicit_query: explicit_query.clone(),
                            limit: Some(10),
                        })
                        .await
                        .unwrap_or(ResearchPackForSceneOutput {
                            sources: vec![],
                            notes: vec![],
                            claims: vec![],
                        });

                    let research_pack_empty =
                        pack.sources.is_empty() && pack.notes.is_empty() && pack.claims.is_empty();
                    let research_tags_matched = if research_tags.is_empty() {
                        true
                    } else {
                        research_tags.iter().any(|tag| {
                            let tag_lower = tag.to_lowercase();
                            pack.sources.iter().any(|source| {
                                source
                                    .tags
                                    .iter()
                                    .any(|source_tag| source_tag.to_lowercase() == tag_lower)
                            }) || pack.notes.iter().any(|note| {
                                note.tags
                                    .iter()
                                    .any(|note_tag| note_tag.to_lowercase() == tag_lower)
                            }) || pack.claims.iter().any(|claim| {
                                claim
                                    .tags
                                    .iter()
                                    .any(|claim_tag| claim_tag.to_lowercase() == tag_lower)
                            })
                        })
                    };

                    planned_scenes.push(PlannedSceneSnapshot {
                        scene_order: planned.scene_order,
                        character_ids: planned.character_ids.clone(),
                        research_required: planned.research_required,
                        research_tags,
                        explicit_query,
                        research_pack_empty,
                        research_tags_matched,
                    });
                }

                chapter_plan = Some(ChapterPlanSnapshot {
                    synopsis: plan.synopsis.clone(),
                    pov_character_id: plan.pov_character_id.clone(),
                    scenes: planned_scenes,
                });
            }

            chapters.insert(
                chapter.chapter_number,
                ChapterSnapshot {
                    chapter_id: chapter_row.id,
                    scenes: persisted_scenes,
                    chapter_plan,
                },
            );
        }

        Ok(ProjectSnapshot {
            active_branch_id: active_branch.id,
            active_branch_name: active_branch.name,
            chapters,
            summarized_chapters,
        })
    }

    pub(super) async fn handle_authoring_prepare_run(
        &self,
        input: AuthoringPrepareRunInput,
    ) -> anyhow::Result<AuthoringPrepareRunOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let mut range_overflow: Option<(i32, i32)> = None;
        let end_chapter = if let Some(ec) = input.end_chapter {
            ec
        } else if let Some(cc) = input.chapter_count {
            match chapter_range_end(input.start_chapter, cc) {
                Some(end) => end,
                None => {
                    range_overflow = Some((input.start_chapter, cc));
                    input.start_chapter
                }
            }
        } else {
            let chapters = repo
                .list_chapters_by_book_number(&project_id, input.book_number)
                .await?;
            chapters
                .iter()
                .map(|c| c.chapter_number)
                .max()
                .unwrap_or(input.start_chapter)
        };

        let mut invalid_inputs = Vec::new();
        if input.book_number <= 0 {
            invalid_inputs.push(format!(
                "book_number must be positive, got {}",
                input.book_number
            ));
        }
        if input.start_chapter <= 0 {
            invalid_inputs.push(format!(
                "start_chapter must be positive, got {}",
                input.start_chapter
            ));
        }
        if let Some(chapter_count) = input.chapter_count
            && chapter_count <= 0
        {
            invalid_inputs.push(format!(
                "chapter_count must be positive when provided, got {}",
                chapter_count
            ));
        }
        if let Some((start_chapter, chapter_count)) = range_overflow {
            invalid_inputs.push(format!(
                "chapter range out of bounds: start_chapter {start_chapter} with chapter_count {chapter_count} exceeds the supported chapter-number range",
            ));
        }
        let span = end_chapter as i64 - input.start_chapter as i64 + 1;
        if span > MAX_PREPARE_CHAPTER_SPAN {
            invalid_inputs.push(format!(
                "chapter range spans {span} chapters, exceeding the per-call maximum of {MAX_PREPARE_CHAPTER_SPAN} chapters",
            ));
        }
        if end_chapter < input.start_chapter {
            invalid_inputs.push(format!(
                "end_chapter {} is before start_chapter {}",
                end_chapter, input.start_chapter
            ));
        }
        if !invalid_inputs.is_empty() {
            return Ok(AuthoringPrepareRunOutput {
                project_id,
                book_number: input.book_number,
                start_chapter: input.start_chapter,
                end_chapter,
                ready_to_draft: false,
                missing_requirements: invalid_inputs,
                details: Vec::new(),
            });
        }

        let active_branch = repo.get_active_branch(&project_id).await?;

        let db_chapters = repo
            .list_chapters_by_book_number(&project_id, input.book_number)
            .await?;
        let db_plans = repo.list_chapter_plans_by_project(&project_id).await?;
        let known_character_ids = repo
            .list_characters_by_project_and_branch(&project_id, &active_branch.id)
            .await?
            .into_iter()
            .map(|character| character.id)
            .collect::<BTreeSet<_>>();
        let known_location_ids = repo
            .list_locations_by_project_and_branch(&project_id, &active_branch.id)
            .await?
            .into_iter()
            .map(|location| location.id)
            .collect::<BTreeSet<_>>();

        let mut missing_requirements = Vec::new();
        let mut details = Vec::new();
        // Distinct content ratings of planned scenes across the requested
        // chapter range. Keyed by the canonical lowercase rating string so the
        // draft-route preflight below runs once per rating in a deterministic
        // order. Populated as each scene's rating is resolved in the loop.
        let mut planned_ratings: BTreeSet<&'static str> = BTreeSet::new();

        for ch_num in input.start_chapter..=end_chapter {
            let mut chapter_missing_items = Vec::new();

            let ch_exists = db_chapters.iter().any(|c| c.chapter_number == ch_num);
            if !ch_exists {
                chapter_missing_items.push("missing chapter".to_string());
                missing_requirements.push(format!("Chapter {}: missing chapter", ch_num));
            }

            let plan_opt = db_plans.iter().find(|p| {
                p.book_number == input.book_number
                    && p.chapter_number == ch_num
                    && p.branch_id == active_branch.id
            });

            if let Some(plan) = plan_opt {
                if plan.scenes.is_empty() {
                    chapter_missing_items.push("missing scene list".to_string());
                    missing_requirements.push(format!("Chapter {}: missing scene list", ch_num));
                } else {
                    for scene in &plan.scenes {
                        if scene.character_ids.is_empty() {
                            chapter_missing_items.push(format!(
                                "scene {}: missing character IDs",
                                scene.scene_order
                            ));
                            missing_requirements.push(format!(
                                "Chapter {} scene {}: missing character IDs",
                                ch_num, scene.scene_order
                            ));
                        }
                        for character_id in &scene.character_ids {
                            if !known_character_ids.contains(character_id) {
                                chapter_missing_items.push(format!(
                                    "scene {}: unknown character ID {}",
                                    scene.scene_order, character_id
                                ));
                                missing_requirements.push(format!(
                                    "Chapter {} scene {}: unknown character ID {}",
                                    ch_num, scene.scene_order, character_id
                                ));
                            }
                        }

                        let (loc_id, rating) = planned_scene_location_and_rating(
                            scene.location_id.as_deref(),
                            scene.content_rating.as_ref(),
                            &scene.summary,
                            &scene.purpose,
                        );
                        if loc_id.is_none() {
                            chapter_missing_items
                                .push(format!("scene {}: missing location ID", scene.scene_order));
                            missing_requirements.push(format!(
                                "Chapter {} scene {}: missing location ID",
                                ch_num, scene.scene_order
                            ));
                        }
                        if let Some(location_id) = loc_id.as_ref()
                            && !known_location_ids.contains(location_id)
                        {
                            chapter_missing_items.push(format!(
                                "scene {}: unknown location ID {}",
                                scene.scene_order, location_id
                            ));
                            missing_requirements.push(format!(
                                "Chapter {} scene {}: unknown location ID {}",
                                ch_num, scene.scene_order, location_id
                            ));
                        }
                        match rating.as_ref() {
                            Some(rating) => {
                                planned_ratings.insert(rating.as_str());
                            }
                            None => {
                                chapter_missing_items.push(format!(
                                    "scene {}: missing content rating",
                                    scene.scene_order
                                ));
                                missing_requirements.push(format!(
                                    "Chapter {} scene {}: missing content rating",
                                    ch_num, scene.scene_order
                                ));
                            }
                        }
                    }
                }
            } else {
                chapter_missing_items.push("missing chapter plan".to_string());
                missing_requirements.push(format!("Chapter {}: missing chapter plan", ch_num));
            }

            details.push(AuthoringPrepareChapterDetails {
                chapter_number: ch_num,
                ready: chapter_missing_items.is_empty(),
                missing_items: chapter_missing_items,
            });
        }

        // Agent-route preflight: for each distinct rating the planned scenes
        // need, verify the `draft` route resolves to a configured, key-holding
        // agent that covers the rating. Config-level only — no network I/O; a
        // route that falls back to a built-in local route serves every rating
        // and is never flagged. Any unmet requirement blocks the run so a
        // multi-hour authoring run fails fast at `prepare` instead of hours in.
        let model_router = self.service.repository().model_router();
        for rating in &planned_ratings {
            let preflight = model_router.draft_route_preflight(rating)?;
            if let Some(problem) = preflight.problem {
                let detail = match problem {
                    DraftRoutePreflightProblem::Unresolved => {
                        "no draft route is configured for this rating".to_string()
                    }
                    DraftRoutePreflightProblem::MissingApiKey { env_var } => {
                        format!("agent is missing API key env {env_var}")
                    }
                    DraftRoutePreflightProblem::RatingNotCovered => {
                        "agent does not cover this rating".to_string()
                    }
                };
                let agent = preflight
                    .agent_id
                    .map(|id| format!(" (agent {id})"))
                    .unwrap_or_default();
                missing_requirements.push(format!(
                    "Route draft cannot serve rating {rating}{agent}: {detail}"
                ));
            }
        }

        // Canon-mining preflight (evolution §3.1/§4.4): only when the run opted
        // into `propose_all`. Mining is prose-bearing, so every planned rating
        // must be servable through the mine fallback ladder — covered if EITHER
        // the `mine` route OR the `review` route resolves cleared for the
        // rating. Uncovered ratings push a `missing_requirements` entry so the
        // operator decides before the run starts, not hours in (I8).
        if input.mining_policy.as_deref() == Some("propose_all") {
            for rating in &planned_ratings {
                if let Some(preflight) = model_router.mine_fallback_preflight(rating)? {
                    let detail = match preflight.problem {
                        Some(DraftRoutePreflightProblem::Unresolved) => {
                            "no mine or review route is configured for this rating".to_string()
                        }
                        Some(DraftRoutePreflightProblem::MissingApiKey { env_var }) => {
                            format!("agent is missing API key env {env_var}")
                        }
                        Some(DraftRoutePreflightProblem::RatingNotCovered) => {
                            "no mine or review agent covers this rating".to_string()
                        }
                        None => continue,
                    };
                    let agent = preflight
                        .agent_id
                        .map(|id| format!(" (agent {id})"))
                        .unwrap_or_default();
                    missing_requirements.push(format!(
                        "Canon mining cannot serve rating {rating}{agent}: {detail}"
                    ));
                }
            }
        }

        // Auto-checkpoint preflight (evolution §3.3 K2): only when the run opted
        // into an auto policy (`auto_advisory` / `auto_strict`). The automation
        // runs its sampled dual-persona reviews AND its deep dual-persona
        // consistency pass through the `review` route (prose-bearing), so that
        // route must resolve rating-cleared for EVERY distinct rating in the
        // range (precondition (a)). Precondition (b) — a deep-check-capable
        // review route — collapses into (a) today: the same `review` route
        // serves the checkpoint's deep pass, so covering (a) covers (b). The
        // runtime explicit-manual-fallback in K3 is defense-in-depth for a
        // mid-run config change, NOT a licence to start uncovered — an auto run
        // must be fully covered at start (I8: fail at prepare, not at 2am).
        if matches!(
            input.checkpoint_policy.as_deref(),
            Some("auto_advisory") | Some("auto_strict")
        ) {
            let policy = input.checkpoint_policy.as_deref().unwrap_or("");
            for rating in &planned_ratings {
                let preflight = model_router.review_route_preflight(rating)?;
                if let Some(problem) = preflight.problem {
                    let detail = match problem {
                        DraftRoutePreflightProblem::Unresolved => {
                            "no review route is configured for this rating".to_string()
                        }
                        DraftRoutePreflightProblem::MissingApiKey { env_var } => {
                            format!("agent is missing API key env {env_var}")
                        }
                        DraftRoutePreflightProblem::RatingNotCovered => {
                            "review agent does not cover this rating".to_string()
                        }
                    };
                    let agent = preflight
                        .agent_id
                        .map(|id| format!(" (agent {id})"))
                        .unwrap_or_default();
                    missing_requirements.push(format!(
                        "Checkpoint policy {policy} requires route review to serve rating {rating}{agent}: {detail}"
                    ));
                }
            }
        }

        let ready_to_draft = missing_requirements.is_empty();
        Ok(AuthoringPrepareRunOutput {
            project_id,
            book_number: input.book_number,
            start_chapter: input.start_chapter,
            end_chapter,
            ready_to_draft,
            missing_requirements,
            details,
        })
    }

    pub(super) async fn handle_authoring_start_run(
        &self,
        input: AuthoringStartRunInput,
    ) -> anyhow::Result<AuthoringStartRunOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        if input.checkpoint_interval == 0 {
            return Ok(AuthoringStartRunOutput {
                run_id: String::new(),
                status: "blocked".to_string(),
                message: "Cannot start authoring run: checkpoint_interval must be at least 1"
                    .to_string(),
            });
        }

        // Validate the opt-in mining policy up front. Canonical `mining_policy`
        // is None (disabled/default) or Some("propose_all"); an unknown value
        // is an input error (evolution §3.1). "disabled" canonicalizes to None
        // so the run persists NULL, matching pre-upgrade runs exactly.
        let mining_policy = match spindle_core::models::validate_mining_policy(
            input.mining_policy.as_deref(),
        ) {
            Ok(policy) => policy,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: mining_policy must be one of {:?}, got {:?}",
                        spindle_core::models::AUTHORING_MINING_POLICIES,
                        rejected
                    ),
                });
            }
        };

        // Validate the opt-in bounded-revise budget (evolution §3.2). None/0 =
        // disabled (canonicalized to None so the run persists NULL, matching a
        // pre-upgrade row); 1..=2 enable the in-run verify/revise loop; anything
        // else is an input error — a bound, not a knob explosion.
        let max_revise_attempts = match spindle_core::models::validate_max_revise_attempts(
            input.max_revise_attempts,
        ) {
            Ok(budget) => budget,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: max_revise_attempts must be between 0 and {}, got {}",
                        spindle_core::models::MAX_REVISE_ATTEMPTS_UPPER_BOUND,
                        rejected
                    ),
                });
            }
        };

        // Validate the opt-in checkpoint policy (evolution §3.3). None/"manual"
        // canonicalize to None so the run persists NULL, matching a pre-upgrade
        // row exactly (I1: manual/NULL is byte-identical to today). An auto
        // policy additionally requires review-route coverage across the run's
        // ratings, enforced by prepare's preflight below (K2).
        let checkpoint_policy = match spindle_core::models::validate_checkpoint_policy(
            input.checkpoint_policy.as_deref(),
        ) {
            Ok(policy) => policy,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: checkpoint_policy must be one of {:?}, got {:?}",
                        spindle_core::models::AUTHORING_CHECKPOINT_POLICIES,
                        rejected
                    ),
                });
            }
        };

        // Validate the opt-in living-outline replan policy (ADR 0003, evolution
        // §3.5). None/"disabled" canonicalize to None so the run persists NULL,
        // matching a pre-upgrade row exactly (disabled = never replans). There is
        // NO route preflight: the replan differ is non-prose-bearing (summaries +
        // metadata only — ADR D5), so no rating clearance applies, and on a
        // NoRoute it falls to review then skips honestly. Anything else is an
        // input error.
        let replan_policy = match spindle_core::models::validate_replan_policy(
            input.replan_policy.as_deref(),
        ) {
            Ok(policy) => policy,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: replan_policy must be one of {:?}, got {:?}",
                        spindle_core::models::AUTHORING_REPLAN_POLICIES,
                        rejected
                    ),
                });
            }
        };

        let prep_input = AuthoringPrepareRunInput {
            project_id: project_id.clone(),
            book_number: input.book_number,
            start_chapter: input.start_chapter,
            end_chapter: input.end_chapter,
            chapter_count: input.chapter_count,
            // Thread the resolved policy so prepare's mine-route preflight runs
            // only when the run actually opted into propose_all.
            mining_policy: mining_policy.clone(),
            // Threaded for symmetry; prepare adds NO preflight for revise (verify
            // is deterministic and revision reuses the draft route — §3.2).
            max_revise_attempts,
            // Thread the resolved policy so prepare's review-route preflight runs
            // only when the run actually opted into an auto checkpoint policy
            // (evolution §3.3 K2 precondition).
            checkpoint_policy: checkpoint_policy.clone(),
            // Threaded for symmetry; prepare adds NO preflight for replan (the
            // differ is non-prose-bearing and skips honestly on NoRoute — §3.5).
            replan_policy: replan_policy.clone(),
        };
        let prep_report = Box::pin(self.handle_authoring_prepare_run(prep_input)).await?;
        if !prep_report.ready_to_draft {
            return Ok(AuthoringStartRunOutput {
                run_id: "".to_string(),
                status: "blocked".to_string(),
                message: format!(
                    "Cannot start authoring run due to missing requirements: {:?}",
                    prep_report.missing_requirements
                ),
            });
        }

        let active_branch = repo.get_active_branch(&project_id).await?;
        let end_chapter = prep_report.end_chapter;

        let db_plans = repo.list_chapter_plans_by_project(&project_id).await?;
        let mut chapter_seeds = Vec::new();

        for ch_num in input.start_chapter..=end_chapter {
            let plan = db_plans
                .iter()
                .find(|p| {
                    p.book_number == input.book_number
                        && p.chapter_number == ch_num
                        && p.branch_id == active_branch.id
                })
                .context("plan not found")?;

            let mut scene_seeds = Vec::new();
            for scene in &plan.scenes {
                let (loc_id, rating) = require_planned_scene_location_and_rating(
                    ch_num,
                    scene.scene_order,
                    scene.location_id.as_deref(),
                    scene.content_rating.as_ref(),
                    &scene.summary,
                    &scene.purpose,
                )?;

                scene_seeds.push(spindle_harness::state::SceneSeed {
                    scene_order: scene.scene_order,
                    character_ids: scene.character_ids.clone(),
                    location_id: loc_id,
                    content_rating: rating,
                    tone: None,
                    source_path: None,
                    research_required: scene.research_required,
                    research_tags: scene.research_tags.clone(),
                    explicit_query: scene.explicit_query.clone(),
                });
            }

            chapter_seeds.push(spindle_harness::state::ChapterSeed {
                chapter_number: ch_num,
                synopsis: plan.synopsis.clone(),
                pov_character_id: plan.pov_character_id.clone(),
                scenes: scene_seeds,
            });
        }

        let seed = spindle_harness::state::HarnessSeed {
            project_id: project_id.clone(),
            book_number: input.book_number,
            range: spindle_harness::state::ChapterRange {
                start_chapter: input.start_chapter,
                end_chapter,
            },
            checkpoint_interval: input.checkpoint_interval,
            editorial_directives: input.editorial_directives.unwrap_or_default(),
            mining_policy,
            max_revise_attempts,
            checkpoint_policy,
            replan_policy,
            chapters: chapter_seeds,
        };

        let mut harness_state =
            spindle_harness::state::HarnessState::from_seed(seed, active_branch.id.clone());
        harness_state.artifacts_dir = "../artifacts".to_string();

        let run_id = format!(
            "authoring_run:{}",
            ulid::Ulid::new().to_string().to_lowercase()
        );
        let (run, chapters, scenes, checkpoints) =
            map_harness_to_records(&run_id, &harness_state, "active", None);

        repo.save_authoring_run(run, chapters, scenes, checkpoints)
            .await?;

        // Journal: the run persisted (ADR D2 `run_started`). Emitted AFTER the
        // state change commits; a journal error never fails start_run (D3.3).
        RunJournal::new(repo)
            .emit(
                &run_id,
                "run_started",
                run_journal::run_started_payload(
                    input.book_number,
                    input.start_chapter,
                    end_chapter,
                    None,
                    harness_state.mining_policy.as_deref(),
                    harness_state.max_revise_attempts,
                ),
            )
            .await;

        Ok(AuthoringStartRunOutput {
            run_id: run_id.clone(),
            status: "active".to_string(),
            message: format!(
                "Started authoring run {} for chapters {}-{}",
                run_id, input.start_chapter, end_chapter
            ),
        })
    }

    pub(super) async fn handle_authoring_resolve_block(
        &self,
        input: AuthoringResolveBlockInput,
    ) -> anyhow::Result<AuthoringResolveBlockOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;

        let mut harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        // `redraft` is not a forward phase: it resets a poisoned/unparseable
        // scene BACK to pending-draft so the next execute re-drafts fresh (BUG
        // 3). The forward phases stay unchanged; anything else errors, and the
        // whitelist message now lists redraft.
        let target = input.target_phase.to_ascii_lowercase();
        let parsed_phase = match target.as_str() {
            "pending" => Some(spindle_harness::state::ScenePhase::Pending),
            "draft_saved" => Some(spindle_harness::state::ScenePhase::DraftSaved),
            "changes_committed" => Some(spindle_harness::state::ScenePhase::ChangesCommitted),
            "beats_annotated" => Some(spindle_harness::state::ScenePhase::BeatsAnnotated),
            "redraft" => None,
            _ => anyhow::bail!(
                "Invalid target phase: {} (expected one of: pending, draft_saved, changes_committed, beats_annotated, redraft)",
                input.target_phase
            ),
        };

        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, &run_id);
        harness_state.save(&state_path)?;

        // Distinguish scene-level from run-level blocks (defect item 1c): the
        // reconcile pass against live Spindle state is what raises run-level
        // errors (residue rules, branch mismatch, …). When it has errors, a
        // forward advance is allowed even on a scene with no blocked_reason.
        let run_level_blocked = {
            let snapshot = self.authoring_project_snapshot(&harness_state).await?;
            reconcile_state(harness_state.clone(), &snapshot).has_errors()
        };

        let resolve_result = match parsed_phase {
            Some(phase) => spindle_harness::operator::resolve_scene_block(
                &mut harness_state,
                &state_path,
                input.chapter_number,
                input.scene_order,
                phase,
                run_level_blocked,
            ),
            None => spindle_harness::operator::redraft_scene_block(
                &mut harness_state,
                &state_path,
                input.chapter_number,
                input.scene_order,
            ),
        };

        let _ = std::fs::remove_file(&state_path);

        let message = resolve_result?;

        let snapshot = self.authoring_project_snapshot(&harness_state).await?;
        let outcome = reconcile_state(harness_state.clone(), &snapshot);

        let mut final_status = "active".to_string();
        if outcome.has_errors() {
            final_status = "blocked".to_string();
        } else {
            for ch in &harness_state.chapters {
                for sc in &ch.scenes {
                    if sc.blocked_reason.is_some() {
                        final_status = "blocked".to_string();
                    }
                }
            }
        }
        if outcome.next_action == NextAction::Complete {
            final_status = "completed".to_string();
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &final_status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal: resolve_block emits NOTHING — neither the pre-existing forward
        // phases nor `redraft` journal a run event. The ADR D2 vocabulary has no
        // kind for an operator manual scene reset, `pass_skipped` is for a run
        // PASS being skipped (not an operator action), and inventing a kind is a
        // one-way door (D3.4). `authoring_status` (DB state) stays the source of
        // truth, so the reset is observable there without a journal entry.

        Ok(AuthoringResolveBlockOutput {
            run_id,
            message,
            status: final_status,
        })
    }

    pub(super) async fn handle_authoring_cancel_run(
        &self,
        input: AuthoringCancelRunInput,
    ) -> anyhow::Result<AuthoringCancelRunOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;

        let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        let status = "paused".to_string();
        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &status, Some(run.created_at));

        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2): a genuine status transition into `paused`. Only emit
        // when the status actually changed (diff prev vs new — not on a no-op
        // re-pause). Emitted AFTER the save commits (D3.3).
        if run.status != status {
            RunJournal::new(repo)
                .emit(&run_id, "run_paused", run_journal::run_status_payload(None))
                .await;
        }

        let message = format!("Successfully paused authoring run {}", run_id);
        Ok(AuthoringCancelRunOutput {
            run_id,
            message,
            status,
        })
    }
}
