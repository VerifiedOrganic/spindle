//! Initial planning staging, bounded route generation and human ratification.
use super::*;
use spindle_core::planning::*;
mod apply;
mod generate;

impl SqliteSpindleService {
    pub async fn begin_planning(&self, input: BeginPlanningInput) -> Result<PlanningSession> {
        self.repository.begin_planning(input).await
    }
    pub async fn get_planning_session(&self, id: &str) -> Result<PlanningSession> {
        self.repository.get_planning_session(id).await
    }
    pub async fn stage_planning(&self, input: StagePlanningInput) -> Result<PlanningProposal> {
        self.repository.stage_planning(input).await
    }
    pub async fn set_planning_policy(
        &self,
        input: SetPlanningPolicyInput,
    ) -> Result<PlanningSession> {
        self.repository.set_planning_policy(input).await
    }
    pub async fn list_planning_proposals(
        &self,
        id: &str,
        scope: Option<PlanningScope>,
    ) -> Result<Vec<PlanningProposal>> {
        self.repository.list_planning_proposals(id, scope).await
    }
    pub async fn decide_planning(
        &self,
        input: DecidePlanningInput,
    ) -> Result<Vec<PlanningProposal>> {
        if input.decision == PlanningDecision::Accept {
            let report = self.apply_planning(input).await?;
            if let Some(error) = report.error {
                anyhow::bail!(
                    "planning apply stopped: {error}; inspect planning_apply_status for durable progress"
                );
            }
            return Ok(report.proposals);
        }
        self.repository.decide_planning(input).await
    }

    pub async fn prepare_authoring_run(
        &self,
        input: spindle_core::models::AuthoringPrepareRunInput,
    ) -> Result<spindle_core::models::AuthoringPrepareRunOutput> {
        use crate::ai::DraftRoutePreflightProblem;
        use spindle_core::models::{
            AuthoringCheckpointPolicy, AuthoringMiningPolicy, AuthoringPrepareChapterDetails,
            AuthoringPrepareRunOutput,
        };
        use std::collections::BTreeSet;

        const MAX_PREPARE_CHAPTER_SPAN: i64 = 1000;

        let project_id = input.project_id.clone();
        let repo = &self.repository;

        let mut range_overflow: Option<(i32, i32)> = None;
        let end_chapter = if let Some(ec) = input.end_chapter {
            ec
        } else if let Some(cc) = input.chapter_count {
            match cc
                .checked_sub(1)
                .and_then(|c| input.start_chapter.checked_add(c))
            {
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

                        let (loc_id, rating) = planned_scene_location_and_rating_for_prepare(
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

        let model_router = repo.model_router();
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

        if input.mining_policy == Some(AuthoringMiningPolicy::ProposeAll) {
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

        if let Some(
            policy @ (AuthoringCheckpointPolicy::AutoAdvisory
            | AuthoringCheckpointPolicy::AutoStrict),
        ) = input.checkpoint_policy
        {
            let policy = policy.as_str();
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

    pub async fn planning_readiness(
        &self,
        input: PlanningReadinessInput,
    ) -> Result<PlanningReadinessReport> {
        let (session, target_range, findings, pacing_metrics) = self
            .repository
            .evaluate_planning_readiness(&input.session_id, input.target_range)
            .await?;
        if let Some(ref req_scope) = input.scope {
            anyhow::ensure!(
                session.scope.as_ref() == Some(req_scope),
                "planning scope mismatch"
            );
        }
        let scope = session.scope.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "planning_readiness requires the session to be attached to an accepted project root"
            )
        })?;
        let prepare_run = self
            .prepare_authoring_run(spindle_core::models::AuthoringPrepareRunInput {
                project_id: scope.project_id.clone(),
                book_number: target_range.book_number,
                start_chapter: target_range.chapter_start,
                end_chapter: Some(target_range.chapter_end),
                chapter_count: None,
                mining_policy: input.mining_policy,
                max_revise_attempts: None,
                checkpoint_policy: input.checkpoint_policy,
                replan_policy: None,
            })
            .await?;
        let planner_checks_passed = !findings
            .iter()
            .any(|f| f.severity == PlanningFindingSeverity::Blocking);
        let stale_projection = session.projection_stale;
        let ready_to_draft =
            planner_checks_passed && !stale_projection && prepare_run.ready_to_draft;
        Ok(PlanningReadinessReport {
            session_id: session.id,
            scope: Some(scope),
            target_range,
            ready_to_draft,
            planner_checks_passed,
            stale_projection,
            findings,
            pacing_metrics,
            prepare_run,
        })
    }
}

fn planned_scene_location_and_rating_for_prepare(
    location_id: Option<&str>,
    content_rating: Option<&spindle_core::models::ContentRating>,
    summary: &str,
    purpose: &str,
) -> (Option<String>, Option<spindle_core::models::ContentRating>) {
    let combined = format!("{} {}", summary, purpose).to_ascii_lowercase();
    let combined_orig = format!("{} {}", summary, purpose);

    let legacy_location_id = if let Some(idx) = combined.find("location:") {
        let start = idx + "location:".len();
        let remaining = &combined[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != ':')
            .unwrap_or(remaining.len());
        let id_str = &combined_orig[idx..idx + "location:".len() + end];
        Some(id_str.trim().to_string())
    } else {
        None
    };

    let legacy_rating =
        if combined.contains("rating:explicit") || combined.contains("explicit rating") {
            Some(spindle_core::models::ContentRating::Explicit)
        } else if combined.contains("rating:mature") || combined.contains("mature rating") {
            Some(spindle_core::models::ContentRating::Mature)
        } else if combined.contains("rating:teen") || combined.contains("teen rating") {
            Some(spindle_core::models::ContentRating::Teen)
        } else if combined.contains("rating:general") || combined.contains("general rating") {
            Some(spindle_core::models::ContentRating::General)
        } else if combined.contains("rating: explicit") {
            Some(spindle_core::models::ContentRating::Explicit)
        } else if combined.contains("rating: mature") {
            Some(spindle_core::models::ContentRating::Mature)
        } else if combined.contains("rating: teen") {
            Some(spindle_core::models::ContentRating::Teen)
        } else if combined.contains("rating: general") {
            Some(spindle_core::models::ContentRating::General)
        } else {
            None
        };

    let location_id = location_id
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .or(legacy_location_id);
    let rating = content_rating.cloned().or(legacy_rating);
    (location_id, rating)
}
#[cfg(test)]
mod tests;
