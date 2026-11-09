use super::*;
use spindle_core::canon::{
    KnowledgeValidationFactV1, SceneValidationInputV1, SecretValidationCandidateV1,
    detect_knowledge_paraphrase_leak, detect_secret_generic_disclosure,
    detect_secret_narration_leak,
};

impl SqliteSpindleService {
    /// Run the consistency validator suite over a project. Mirrors
    /// `services/mod.rs:4762..5736` in 705b835^.
    ///
    /// Phase-4 fan-out (`canonical_fact_prose_drift`,
    /// `world_rule_semantic_drift`, `voice_drift`, `retcon_reachability`)
    /// is wired through `run_phase_four_validator_checks_for_scenes` and
    /// the results are grouped into `report_sections`. Setting
    /// `deep_check = true` additionally invokes the model-router-driven
    /// `deep_world_rule_compliance_issues` audit on top of the pattern-
    /// based path. `scene_divergence` is wired through
    /// `evaluate_scene_divergence` in `super::source_bridge`.
    pub async fn check_consistency(
        &self,
        input: spindle_core::models::CheckConsistencyInput,
    ) -> Result<spindle_core::models::CheckConsistencyOutput> {
        use crate::format::{
            DEFAULT_CHECK_CONSISTENCY_BUDGET_TOKENS, chapter_keys_from_scenes, end_scope_index,
            scope_contains_chapter, scope_contains_position, scoped_chapter_plans,
            scoped_chapter_summaries, scoped_narrative_promises, scoped_scenes,
            story_index_from_placement,
        };
        use spindle_core::models::{ConsistencyIssue, ConsistencySummary, ContextFormat};
        use std::collections::{BTreeMap, BTreeSet};

        let project_id = input.project_id.clone();
        let project = self.repository.get_project(&project_id).await?;
        let active_branch = self.repository.get_active_branch(&project_id).await?;
        let mut issues: Vec<ConsistencyIssue> = Vec::new();
        let requested_checks_set = requested_checks(&input.checks);
        let requested_severities_set = requested_severities(&input.severity_filter)?;
        let scope = input.scope.to_scope().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deep_check = input.deep_check.unwrap_or(false);
        let deep_offset = input.deep_scan_offset.unwrap_or(0);
        let mut audit_coverage = Vec::new();

        let scenes_raw = self
            .repository
            .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
            .await?;
        let scenes_scoped = scoped_scenes(scenes_raw, &scope);
        let scenes = self
            .narrow_scenes_by_subjects(
                &project_id,
                &active_branch.id,
                scenes_scoped,
                &input.subjects,
            )
            .await?;

        let scene_ids: BTreeSet<String> = scenes.iter().map(|scene| scene.id.clone()).collect();
        let scoped_summaries = scoped_chapter_summaries(
            self.repository
                .list_chapter_summaries_by_project(&project_id)
                .await?,
            &scope,
        );
        let scoped_plans = scoped_chapter_plans(
            self.repository
                .list_chapter_plans_by_project(&project_id)
                .await?,
            &scope,
        );
        let scoped_annotations = self
            .repository
            .list_scene_beat_annotations_by_project(&project_id)
            .await?
            .into_iter()
            .filter(|annotation| scene_ids.contains(&annotation.scene_id))
            .collect::<Vec<_>>();

        if should_run_check(&requested_checks_set, "scene_spine_integrity") {
            for issue in self
                .find_orphan_scenes(&project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|issue| {
                    scope_contains_position(
                        &scope,
                        issue.book_number,
                        issue.chapter_number,
                        issue.scene_order,
                    )
                })
            {
                issues.push(ConsistencyIssue {
                    severity: issue.severity.to_string(),
                    check_type: "scene_spine_integrity".to_string(),
                    message: issue.message,
                    entity_ids: issue.entity_ids,
                    suggested_action: issue.suggested_action,
                });
            }
        }

        // Stub-scene gate (migration V0036): placeholder bodies and prose
        // below the project word floor. A live run shipped such scenes to
        // EPUB while consistency reported clean. Severity is `info`, not
        // `warning`: stubbiness is a manuscript-readiness state observation,
        // not an in-story consistency violation, and the authoring loop's
        // checkpoint policies approve/block on errors+warnings — the HARD
        // gate against shipping stubs is the Blocking `scene_stub_text`
        // preflight issue, which export enforces.
        if should_run_check(&requested_checks_set, "anti_slop")
            && let Some(pack) = load_anti_slop_pack()
        {
            let overlay = project_anti_slop_overlay();
            let suppressions =
                load_suppression_store(&self.repository, &project_id, &active_branch.id).await;
            let experimental = project_anti_slop_experimental_structural();
            let mut chapter_priors: std::collections::BTreeMap<
                (i32, i32),
                spindle_core::style::antislop::ChapterCounters,
            > = std::collections::BTreeMap::new();
            for scene in &scenes {
                let key = (scene.book_number, scene.chapter_number);
                let prior = chapter_priors.get(&key);
                let report = spindle_core::style::antislop::scan_with_options(
                    &pack,
                    &spindle_core::style::antislop::ScanInput::fiction(&scene.full_text),
                    spindle_core::style::antislop::ScanOptions {
                        overlay: overlay.as_ref(),
                        chapter_prior: prior,
                        suppressions: Some(&suppressions),
                        experimental_structural: experimental,
                    },
                );
                for finding in spindle_core::style::antislop::verify_findings(&report) {
                    issues.push(ConsistencyIssue {
                        severity: finding.severity,
                        check_type: finding.check_type,
                        message: finding.message,
                        entity_ids: vec![scene.id.clone()],
                        suggested_action: Some(finding.suggested_action),
                    });
                }
                chapter_priors
                    .entry(key)
                    .or_default()
                    .add_scene(&pack, &scene.full_text);
            }
        }

        if should_run_check(&requested_checks_set, "scene_stub_text") {
            let min_scene_words = project
                .min_scene_word_count
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n > 0)
                .unwrap_or(DEFAULT_MIN_SCENE_WORD_COUNT);
            for scene in &scenes {
                if let Some(reason) = scene_stub_reason(&scene.full_text, min_scene_words) {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "scene_stub_text".to_string(),
                        message: format!(
                            "Book {} chapter {} scene {} looks like a stub: {reason}",
                            scene.book_number, scene.chapter_number, scene.scene_order
                        ),
                        entity_ids: vec![scene.id.clone()],
                        suggested_action: Some(
                            "Draft the scene fully, or delete it if it is abandoned.".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "narrative_promise_tracking") {
            let promises = scoped_narrative_promises(
                self.repository
                    .list_narrative_promises_by_project(&project_id)
                    .await?,
                &scope,
            );
            if let Some(current_index) = end_scope_index(&scope, &scenes) {
                for promise in promises {
                    let verdict = crate::format::promise_timing_verdict(&promise, current_index);
                    let (severity, message, suggested_action) = match verdict.urgency {
                        crate::format::PromiseUrgency::Overdue => (
                            "warning",
                            format!(
                                "promise '{}' is {} chapter(s) past its planned payoff and still open",
                                promise.description, verdict.overdue_by_chapters
                            ),
                            Some(
                                "pay it off, reinforce it, call update_promise_status, or \
                                 retarget it with update_entity(narrative_promise, \
                                 changes={planned_payoff: {book_number, chapter_number}})"
                                    .to_string(),
                            ),
                        ),
                        crate::format::PromiseUrgency::Due => (
                            "warning",
                            format!(
                                "promise '{}' has reached its planned payoff point but is still open",
                                promise.description
                            ),
                            Some("pay it off now or move its planned_payoff".to_string()),
                        ),
                        crate::format::PromiseUrgency::Soon => (
                            "info",
                            format!(
                                "promise '{}' is approaching payoff ({} chapter(s) open)",
                                promise.description, verdict.chapters_since_plant
                            ),
                            Some("begin landing this promise".to_string()),
                        ),
                        crate::format::PromiseUrgency::Watch
                        | crate::format::PromiseUrgency::Resolved => continue,
                    };
                    issues.push(ConsistencyIssue {
                        severity: severity.to_string(),
                        check_type: "narrative_promise_tracking".to_string(),
                        message,
                        entity_ids: vec![promise.id.clone()],
                        suggested_action,
                    });
                }
            }
        }

        let arcs = self
            .repository
            .list_character_arcs_by_project(&project_id)
            .await?;
        let trackers = self
            .repository
            .list_pacing_trackers_by_project(&project_id)
            .await?;

        if should_run_check(&requested_checks_set, "pacing_budget_audit") {
            let tracker_by_arc = trackers
                .iter()
                .map(|tracker| (tracker.character_arc_id.clone(), tracker))
                .collect::<BTreeMap<_, _>>();
            let pacing_configs = self
                .repository
                .list_pacing_configs_by_project(&project_id)
                .await?;
            let pacing_curves = self
                .repository
                .list_pacing_curves_by_project(&project_id)
                .await?;

            if pacing_configs.is_empty() && !arcs.is_empty() {
                issues.push(ConsistencyIssue {
                    severity: "warning".to_string(),
                    check_type: "pacing_budget_audit".to_string(),
                    message: "project has character arcs but no pacing configuration".to_string(),
                    entity_ids: arcs.iter().map(|arc| arc.id.clone()).collect(),
                    suggested_action: Some(
                        "create_pacing_config before using arc pacing checks".to_string(),
                    ),
                });
            }

            for arc in &arcs {
                let arc_id = arc.id.clone();
                let Some(tracker) = tracker_by_arc.get(&arc_id) else {
                    issues.push(ConsistencyIssue {
                        severity: "error".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!("character arc '{}' is missing a pacing tracker", arc_id),
                        entity_ids: vec![arc_id],
                        suggested_action: Some(
                            "create_character_arc should seed a pacing tracker".to_string(),
                        ),
                    });
                    continue;
                };

                if tracker.budget_remaining < 0.0 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' is over budget by {:.2}",
                            tracker.character_arc_id,
                            tracker.budget_remaining.abs()
                        ),
                        entity_ids: vec![tracker.id.clone(), tracker.character_arc_id.clone()],
                        suggested_action: Some(
                            "slow the arc or rebalance pacing constraints".to_string(),
                        ),
                    });
                }

                if tracker.current_progress > 1.0 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' exceeds 100% progress",
                            tracker.character_arc_id
                        ),
                        entity_ids: vec![tracker.id.clone()],
                        suggested_action: Some(
                            "recalculate progress or mark the arc complete".to_string(),
                        ),
                    });
                }

                if tracker.status == "behind" || tracker.status == "stalled" {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' tracker status is '{}'",
                            tracker.character_arc_id, tracker.status
                        ),
                        entity_ids: vec![tracker.id.clone()],
                        suggested_action: Some(
                            "accelerate the arc or update pacing constraints".to_string(),
                        ),
                    });
                }

                if tracker.status == "ahead" {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' is ahead of its planned pacing",
                            tracker.character_arc_id
                        ),
                        entity_ids: vec![tracker.id.clone()],
                        suggested_action: Some(
                            "consider cooling the arc before the next milestone".to_string(),
                        ),
                    });
                }
            }

            if !pacing_configs.is_empty() {
                let planned_books: BTreeSet<i32> = self
                    .repository
                    .list_books_by_project(&project_id)
                    .await?
                    .into_iter()
                    .map(|book| book.book_number)
                    .collect();
                let curve_books: BTreeSet<i32> = pacing_curves
                    .into_iter()
                    .map(|curve| curve.book_number)
                    .collect();
                for missing_book in planned_books.difference(&curve_books) {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!("book {} has no pacing curve", missing_book),
                        entity_ids: vec![],
                        suggested_action: Some("create_pacing_curve for this book".to_string()),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "timeline_continuity") {
            let mut previous: Option<&crate::sqlite::records::Scene> = None;
            for scene in &scenes {
                if let Some(prev) = previous
                    && scene.book_number == prev.book_number
                    && scene.chapter_number == prev.chapter_number
                    && scene.scene_order != prev.scene_order + 1
                {
                    issues.push(ConsistencyIssue {
                        severity: "error".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "scene order jumps from {} to {} in book {}, chapter {}",
                            prev.scene_order,
                            scene.scene_order,
                            scene.book_number,
                            scene.chapter_number
                        ),
                        entity_ids: vec![prev.id.clone(), scene.id.clone()],
                        suggested_action: Some(
                            "fill the missing scene order or renumber the chapter".to_string(),
                        ),
                    });
                }
                previous = Some(scene);
            }

            let summary_keys = scoped_summaries
                .iter()
                .map(|summary| (summary.book_number, summary.chapter_number))
                .collect::<BTreeSet<_>>();
            for chapter in chapter_keys_from_scenes(&scenes) {
                if !summary_keys.contains(&chapter) {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "book {}, chapter {} has scenes but no chapter summary",
                            chapter.0, chapter.1
                        ),
                        entity_ids: vec![],
                        suggested_action: Some(
                            "save_summary after the chapter draft stabilizes".to_string(),
                        ),
                    });
                }
            }

            let timeline_events = self
                .repository
                .list_timeline_events_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|event| {
                    scope_contains_position(
                        &scope,
                        event.placement.book_number,
                        event.placement.chapter_number,
                        event.placement.scene_order.unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>();
            let event_ids = timeline_events
                .iter()
                .map(|event| event.id.clone())
                .collect::<BTreeSet<_>>();

            let mut previous_event: Option<&crate::sqlite::records::TimelineEvent> = None;
            for event in &timeline_events {
                if let Some(prev) = previous_event
                    && story_index_from_placement(&event.placement)
                        == story_index_from_placement(&prev.placement)
                    && spindle_core::models::normalize_name(&event.title)
                        == spindle_core::models::normalize_name(&prev.title)
                {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "timeline event '{}' overlaps another event at the same story position",
                            event.title
                        ),
                        entity_ids: vec![prev.id.clone(), event.id.clone()],
                        suggested_action: Some(
                            "merge duplicate events or differentiate their placement".to_string(),
                        ),
                    });
                }
                previous_event = Some(event);
            }

            let interventions = self
                .repository
                .list_temporal_interventions_by_project(&project_id)
                .await?;
            for intervention in &interventions {
                let Some(source_event_id) = intervention.source_event_id.as_ref() else {
                    continue;
                };
                let Some(target_event_id) = intervention.target_event_id.as_ref() else {
                    continue;
                };
                let source_event = self.repository.get_timeline_event(source_event_id).await?;
                let target_event = self.repository.get_timeline_event(target_event_id).await?;
                if !event_ids.contains(&source_event.id) && !event_ids.contains(&target_event.id) {
                    continue;
                }

                let source_index = story_index_from_placement(&source_event.placement);
                let target_index = story_index_from_placement(&target_event.placement);
                if source_index < target_index {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "temporal intervention '{}' points from an earlier event to a later one",
                            intervention.title
                        ),
                        entity_ids: vec![
                            intervention.id.clone(),
                            source_event.id.clone(),
                            target_event.id.clone(),
                        ],
                        suggested_action: Some(
                            "verify source and target events are oriented correctly for time travel"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        // ── Chronology: in-world time monotonicity ───────────────────
        // When a project declares a calendar and scene clocks, flag any scene
        // set earlier in story time than its predecessor on the same timeline
        // thread that is not marked as a flashback. Deterministic; a no-op for
        // projects that never declare story-time.
        if should_run_check(&requested_checks_set, "chronology")
            && let Some(calendar) = self.repository.get_project_calendar(&project_id).await?
        {
            let clocks = self
                .repository
                .list_scene_clocks_by_project_and_branch(&project_id, &active_branch.id)
                .await?;
            let clock_by_scene: BTreeMap<String, &crate::sqlite::records::StoredSceneClock> =
                clocks
                    .iter()
                    .map(|clock| (clock.scene_id.clone(), clock))
                    .collect();
            let mut last_linear: BTreeMap<String, (i64, String)> = BTreeMap::new();
            for scene in &scenes {
                let Some(scene_clock) = clock_by_scene.get(&scene.id) else {
                    continue;
                };
                let Some(index) = scene_clock.clock.total_index(&calendar.calendar) else {
                    continue;
                };
                let thread = scene_clock
                    .thread_key
                    .clone()
                    .unwrap_or_else(|| "main".to_string());
                let is_out_of_line = matches!(
                    scene_clock.temporal_mode.as_deref(),
                    Some("flashback") | Some("flashforward") | Some("concurrent")
                );
                if !is_out_of_line
                    && let Some((prev_index, prev_scene_id)) = last_linear.get(&thread)
                    && index < *prev_index
                {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "chronology".to_string(),
                        message: format!(
                            "scene is set earlier in story time (day {}) than the previous scene on thread '{}' but is not marked as a flashback",
                            scene_clock.clock.day_index.unwrap_or_default(),
                            thread
                        ),
                        entity_ids: vec![prev_scene_id.clone(), scene.id.clone()],
                        suggested_action: Some(
                            "set temporal_mode to 'flashback'/'concurrent' or correct the scene's day_index"
                                .to_string(),
                        ),
                    });
                }
                if !is_out_of_line {
                    last_linear.insert(thread, (index, scene.id.clone()));
                }
            }
        }

        // ── Temporal coherence: intra-scene time jumps ─────────────────
        // The within-scene, forward-looking analog of the chronology check:
        // scan each scene's prose for an unsignaled time-of-day skip (morning →
        // night with no transition), an internal time-of-day contradiction, or
        // a declared multi-day span rendered as one unbroken block. Prose-only
        // and calendar-free; a no-op for scenes with no detectable problem. The
        // scene clock (when set) only suppresses: a flashback/coarse-precision
        // scene, or the declared `duration_days`.
        if should_run_check(&requested_checks_set, "temporal_coherence") {
            let clocks = self
                .repository
                .list_scene_clocks_by_project_and_branch(&project_id, &active_branch.id)
                .await?;
            let clock_by_scene: BTreeMap<String, &crate::sqlite::records::StoredSceneClock> =
                clocks
                    .iter()
                    .map(|clock| (clock.scene_id.clone(), clock))
                    .collect();
            for scene in &scenes {
                issues.extend(scan_temporal_findings(
                    &scene.id,
                    &scene.full_text,
                    clock_by_scene.get(&scene.id).copied(),
                ));
            }
            // Tier 2 (opt-in): a model-backed semantic pass that catches the
            // idiomatic time jumps the deterministic lexicon cannot (meals,
            // light, errands, "three cigarettes later"). Falls back to nothing
            // when no review model is configured, so the deterministic scan
            // above stands alone.
            if deep_check {
                let mut coverage = spindle_core::models::AuditCoverage::default();
                issues.extend(
                    self.deep_temporal_coherence_issues(&scenes, &mut coverage)
                        .await?,
                );
                audit_coverage.push(coverage);
            }
        }

        // ── Promise-payoff candidate detection (T-106, Tier 2) ──────────
        // Deep-only, opt-in. Propose (never write) that an unresolved promise the
        // scoped prose already delivered be confirmed with update_promise_status.
        // Runs ONLY when deep_check is set AND the check is requested, mirroring
        // the temporal Tier-2 gating shape.
        if deep_check && should_run_check(&requested_checks_set, "promise_payoff_detection") {
            let mut candidates = scoped_narrative_promises(
                self.repository
                    .list_narrative_promises_by_project(&project_id)
                    .await?,
                &scope,
            )
            .into_iter()
            .filter(|promise| {
                promise.archived_at.is_none()
                    && promise.status != "paid_off"
                    && promise.status != "abandoned"
            })
            .collect::<Vec<_>>();

            // Rank by urgency (overdue > due > soon > watch), tiebreak id asc, so
            // the most pressing unresolved threads are audited first under the cap.
            if let Some(current_index) = end_scope_index(&scope, &scenes) {
                candidates.sort_by(|a, b| {
                    let ua = crate::format::promise_timing_verdict(a, current_index)
                        .urgency
                        .rank();
                    let ub = crate::format::promise_timing_verdict(b, current_index)
                        .urgency
                        .rank();
                    ub.cmp(&ua).then_with(|| a.id.cmp(&b.id))
                });
            } else {
                candidates.sort_by(|a, b| a.id.cmp(&b.id));
            }

            const PROMISE_PAYOFF_SCAN_CAP: usize = 10;
            let mut coverage = spindle_core::models::AuditCoverage::new(
                "promise_payoff_detection",
                candidates.iter().map(|p| p.id.clone()),
            );
            coverage.window(deep_offset, PROMISE_PAYOFF_SCAN_CAP);
            candidates = candidates
                .into_iter()
                .skip(deep_offset)
                .take(PROMISE_PAYOFF_SCAN_CAP)
                .collect();
            let unscanned = coverage.eligible.saturating_sub(candidates.len());

            issues.extend(
                self.deep_promise_payoff_issues(&candidates, &scenes, unscanned, &mut coverage)
                    .await?,
            );
            audit_coverage.push(coverage);
        }

        // ── Scene-purpose fulfillment (P3.3, deep tier) ─────────────────
        // Deep-only, opt-in advisory: did the drafted scene accomplish its
        // chapter-plan `purpose`? For each scoped persisted scene that has BOTH
        // non-empty prose AND a plan scene entry with a non-empty `purpose`, one
        // model call on `review` carries THAT scene's rating (per-scene pattern,
        // mirroring the behavioral secret-leak audit). A `fulfilled:false`
        // verdict emits one `info` finding; silence is the pass. Skipped
        // silently for scenes with no plan/purpose (nothing to audit).
        if deep_check && should_run_check(&requested_checks_set, "scene_purpose_fulfillment") {
            // Story order is preserved by `scenes`; join each to its planned
            // purpose (find the plan by (book, chapter), then the planned scene
            // by scene_order — the same join agency_tracking / research use).
            let mut units: Vec<ScenePurposeScanUnit> = Vec::new();
            for scene in &scenes {
                if scene.full_text.trim().is_empty() {
                    continue;
                }
                let Some(purpose) = scoped_plans
                    .iter()
                    .find(|plan| {
                        plan.book_number == scene.book_number
                            && plan.chapter_number == scene.chapter_number
                    })
                    .and_then(|plan| {
                        plan.scenes
                            .iter()
                            .find(|s| s.scene_order == scene.scene_order)
                    })
                    .map(|planned| planned.purpose.trim())
                    .filter(|purpose| !purpose.is_empty())
                else {
                    continue;
                };
                units.push(ScenePurposeScanUnit {
                    scene_id: scene.id.clone(),
                    scene_ref: format!(
                        "book {}, chapter {} scene {}",
                        scene.book_number, scene.chapter_number, scene.scene_order
                    ),
                    scene_rating: scene.content_rating.clone(),
                    scene_text: scene.full_text.clone(),
                    purpose: purpose.to_string(),
                });
            }
            let mut coverage = spindle_core::models::AuditCoverage::new(
                "scene_purpose_fulfillment",
                units.iter().map(|u| u.scene_id.clone()),
            );
            coverage.window(deep_offset, SCENE_PURPOSE_SCAN_CAP);
            units = units
                .into_iter()
                .skip(deep_offset)
                .take(SCENE_PURPOSE_SCAN_CAP)
                .collect();
            let unscanned = coverage.eligible.saturating_sub(units.len());
            issues.extend(
                self.deep_scene_purpose_issues(&units, unscanned, &mut coverage)
                    .await?,
            );
            audit_coverage.push(coverage);
        }

        // ── Quantity drift: an unexplained multi-band jump ──────────────
        // For subjects with a declared scheme, flag when a measure's band moves
        // more than `max_band_jump` ordered tiers between consecutive stamps with
        // no `change_reason`. No scheme / no bands → no-op (fully additive).
        if should_run_check(&requested_checks_set, "quantity_drift") {
            let schemes = self
                .repository
                .list_project_quantity_schemes(&project_id, &active_branch.id)
                .await?;
            if !schemes.is_empty() {
                let scheme_by_measure: BTreeMap<String, &spindle_core::models::QuantityScheme> =
                    schemes
                        .iter()
                        .map(|stored| (stored.scheme.measure.clone(), &stored.scheme))
                        .collect();
                // Rows arrive grouped by (subject_table, subject_id, measure) in
                // position order, so the previous row in the same group is the
                // prior stamp for that subject's measure.
                let states = self
                    .repository
                    .list_quantity_states_by_project_and_branch(&project_id, &active_branch.id)
                    .await?;
                let mut prev: Option<&crate::sqlite::records::StoredQuantityState> = None;
                for state in &states {
                    if let Some(previous) = prev
                        && previous.subject_table == state.subject_table
                        && previous.subject_id == state.subject_id
                        && previous.measure == state.measure
                        && state.state.change_reason.is_none()
                        && let Some(scheme) = scheme_by_measure.get(state.measure.as_str())
                        && let (Some(from), Some(to)) =
                            (previous.state.band.as_deref(), state.state.band.as_deref())
                        && let Some(jump) = scheme.band_jump(from, to)
                        && jump > scheme.max_band_jump_or_default()
                    {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "quantity_drift".to_string(),
                            message: format!(
                                "{} for '{}' jumped {jump} tiers ({from} → {to}) without a change_reason",
                                state.measure, state.subject_id
                            ),
                            entity_ids: vec![state.subject_id.clone()],
                            suggested_action: Some(
                                "record a change_reason for the jump or stage it across intermediate bands"
                                    .to_string(),
                            ),
                        });
                    }
                    prev = Some(state);
                }
            }
        }

        // ── Currency consistency: price facts that disagree once converted ──
        // Two numeric price facts for the same good in different denominations
        // must reduce to the same base value under the scheme. Deterministic.
        if should_run_check(&requested_checks_set, "currency_consistency") {
            let mut unit_base: BTreeMap<String, f64> = BTreeMap::new();
            for stored in &self
                .repository
                .list_project_quantity_schemes(&project_id, &active_branch.id)
                .await?
            {
                for denom in &stored.scheme.denominations {
                    unit_base.insert(denom.name.to_ascii_lowercase(), denom.per_base as f64);
                }
            }
            if !unit_base.is_empty() {
                let facts = self
                    .repository
                    .list_active_canonical_facts_by_project_and_branch(
                        &project_id,
                        &active_branch.id,
                    )
                    .await?;
                let mut groups: PriceFactGroups = BTreeMap::new();
                for fact in &facts {
                    if fact.value_kind != "number" {
                        continue;
                    }
                    let (Some(number), Some(unit)) = (fact.value_number, fact.unit.as_deref())
                    else {
                        continue;
                    };
                    let Some(per_base) = unit_base.get(&unit.to_ascii_lowercase()) else {
                        continue;
                    };
                    let key = (
                        fact.subject_table.clone(),
                        fact.subject_id.clone().unwrap_or_default(),
                        fact.predicate.clone(),
                    );
                    groups.entry(key).or_default().push((
                        number * per_base,
                        unit.to_string(),
                        fact.id.clone(),
                    ));
                }
                for ((_table, _id, predicate), entries) in &groups {
                    if entries.len() < 2 {
                        continue;
                    }
                    let first = entries[0].0;
                    if entries
                        .iter()
                        .any(|(base, _, _)| (base - first).abs() > f64::EPSILON)
                    {
                        let detail = entries
                            .iter()
                            .map(|(base, unit, _)| format!("{base} base (as {unit})"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "currency_consistency".to_string(),
                            message: format!(
                                "'{predicate}' has prices that disagree once converted to base units: {detail}"
                            ),
                            entity_ids: entries.iter().map(|(_, _, id)| id.clone()).collect(),
                            suggested_action: Some(
                                "reconcile the prices or correct the denomination used".to_string(),
                            ),
                        });
                    }
                }
            }
        }

        // ── Affordability: a priced purchase a present character can't cover ──
        // Advisory (Info, high-precision/low-recall): if a scene names a price
        // above a present character's tracked wealth (converted to base), flag.
        if should_run_check(&requested_checks_set, "affordability") {
            let schemes = self
                .repository
                .list_project_quantity_schemes(&project_id, &active_branch.id)
                .await?;
            let mut unit_base: BTreeMap<String, f64> = BTreeMap::new();
            for stored in &schemes {
                for denom in &stored.scheme.denominations {
                    unit_base.insert(denom.name.to_ascii_lowercase(), denom.per_base as f64);
                }
            }
            if !unit_base.is_empty() {
                let units: Vec<String> = unit_base.keys().cloned().collect();
                let characters = self
                    .repository
                    .list_characters_by_project_and_branch(&project_id, &active_branch.id)
                    .await?;
                for scene in &scenes {
                    let mentions =
                        spindle_core::models::extract_price_mentions(&scene.full_text, &units);
                    let Some(max_price_base) = mentions
                        .iter()
                        .filter_map(|m| {
                            unit_base
                                .get(&m.unit.to_ascii_lowercase())
                                .map(|per_base| m.amount * per_base)
                        })
                        .fold(None, |acc: Option<f64>, value| {
                            Some(acc.map_or(value, |a| a.max(value)))
                        })
                    else {
                        continue;
                    };
                    let cursor = scene.book_number as i64 * 1_000_000
                        + scene.chapter_number as i64 * 1_000
                        + scene.scene_order as i64;
                    for character in &characters {
                        if !contains_case_insensitive_word_boundary(
                            &scene.full_text,
                            &character.name,
                        ) {
                            continue;
                        }
                        for stored in &schemes {
                            let measure = stored.scheme.measure.as_str();
                            let Some(state) = self
                                .repository
                                .latest_quantity_state_at_or_before(
                                    &project_id,
                                    &active_branch.id,
                                    "character",
                                    &character.id,
                                    measure,
                                    cursor,
                                )
                                .await?
                            else {
                                continue;
                            };
                            let (Some(amount), Some(unit)) =
                                (state.state.amount, state.state.unit.as_deref())
                            else {
                                continue;
                            };
                            let Some(per_base) = unit_base.get(&unit.to_ascii_lowercase()) else {
                                continue;
                            };
                            let wealth_base = amount * per_base;
                            if max_price_base > wealth_base {
                                issues.push(ConsistencyIssue {
                                    severity: "info".to_string(),
                                    check_type: "affordability".to_string(),
                                    message: format!(
                                        "scene names a price of {max_price_base} base units, above {}'s tracked {measure} of {wealth_base} base units",
                                        character.name
                                    ),
                                    entity_ids: vec![scene.id.clone(), character.id.clone()],
                                    suggested_action: Some(
                                        "confirm the character can afford this, or update their tracked amount"
                                            .to_string(),
                                    ),
                                });
                                break; // one advisory per character per scene
                            }
                        }
                    }
                }
            }
        }

        // ── Knowledge timing: prose referencing not-yet-learned facts ──
        // A present character who references, in prose, a knowledge_fact they do
        // not learn until a later story position. High-precision/low-recall, so
        // an advisory warning. Mirrors the write-time premature-knowledge scan.
        if should_run_check(&requested_checks_set, "knowledge_timing") {
            let knowledge_facts = self
                .repository
                .list_knowledge_facts_by_project_and_branch(&project_id, &active_branch.id)
                .await?;
            if !knowledge_facts.is_empty() {
                let characters = self
                    .repository
                    .list_characters_by_project_and_branch(&project_id, &active_branch.id)
                    .await?;
                let character_by_id: BTreeMap<String, &crate::sqlite::records::Character> =
                    characters.iter().map(|c| (c.id.clone(), c)).collect();
                let mention_records: Vec<spindle_core::character_mentions::CharacterMentionRecord> =
                    characters
                        .iter()
                        .map(
                            |character| spindle_core::character_mentions::CharacterMentionRecord {
                                id: character.id.clone(),
                                name: character.name.clone(),
                                aliases: character.aliases.clone(),
                            },
                        )
                        .collect();
                let mention_resolver =
                    spindle_core::character_mentions::CharacterMentionResolver::new(
                        &mention_records,
                    );
                let single_protagonist = {
                    let protagonists: Vec<&str> = characters
                        .iter()
                        .filter(|character| character.role.eq_ignore_ascii_case("protagonist"))
                        .map(|character| character.id.as_str())
                        .collect();
                    (protagonists.len() == 1).then(|| protagonists[0].to_string())
                };
                let pov_by_chapter: BTreeMap<(i32, i32), String> = scoped_plans
                    .iter()
                    .filter_map(|plan| {
                        plan.pov_character_id
                            .clone()
                            .map(|pov| ((plan.book_number, plan.chapter_number), pov))
                    })
                    .collect();
                for scene in &scenes {
                    let scene_pov = pov_by_chapter
                        .get(&(scene.book_number, scene.chapter_number))
                        .cloned()
                        .or_else(|| single_protagonist.clone());
                    let scene_position =
                        (scene.book_number, scene.chapter_number, scene.scene_order);
                    for fact in &knowledge_facts {
                        let Some(learned_at) = fact.learned_at.as_ref() else {
                            continue;
                        };
                        let learned_position = (
                            learned_at.book_number,
                            learned_at.chapter_number,
                            learned_at.scene_order.unwrap_or(0),
                        );
                        if !position_gt(learned_position, scene_position) {
                            continue;
                        }
                        let Some(character) = character_by_id.get(&fact.character_id) else {
                            continue;
                        };
                        if !mention_resolver.mentions_with_pov(
                            &scene.full_text,
                            &character.id,
                            scene_pov.as_deref(),
                        ) {
                            continue;
                        }
                        let exact_hit = contains_case_insensitive_phrase(
                            &scene.full_text,
                            &fact.normalized_fact,
                        );
                        let paraphrase = if exact_hit {
                            None
                        } else {
                            let scene_input = SceneValidationInputV1 {
                                scene_id: &scene.id,
                                book_number: scene.book_number,
                                chapter_number: scene.chapter_number,
                                scene_order: scene.scene_order,
                                text: &scene.full_text,
                            };
                            let core_fact = KnowledgeValidationFactV1 {
                                fact_id: fact.id.clone(),
                                character_id: fact.character_id.clone(),
                                character_names: vec![character.name.clone()],
                                character_aliases: mention_resolver
                                    .aliases_for(&character.id)
                                    .to_vec(),
                                first_person_subject: scene_pov.as_deref()
                                    == Some(character.id.as_str()),
                                fact: fact.fact.clone(),
                                learned_book_number: learned_at.book_number,
                                learned_chapter_number: learned_at.chapter_number,
                                learned_scene_order: learned_at.scene_order,
                            };
                            detect_knowledge_paraphrase_leak(&scene_input, &core_fact)
                        };
                        if !exact_hit && paraphrase.is_none() {
                            continue;
                        }
                        if let Some(finding) = paraphrase {
                            issues.push(ConsistencyIssue {
                                severity: finding.severity.as_str().to_string(),
                                check_type: finding.check_type,
                                message: finding.message,
                                entity_ids: finding.entity_ids,
                                suggested_action: Some(
                                    "move the reveal, or record the character learning this earlier"
                                        .to_string(),
                                ),
                            });
                            continue;
                        }
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "knowledge_timing".to_string(),
                            message: format!(
                                "scene has '{}' reference knowledge they do not learn until book {} chapter {}",
                                character.name, learned_at.book_number, learned_at.chapter_number
                            ),
                            entity_ids: vec![scene.id.clone(), fact.character_id.clone()],
                            suggested_action: Some(
                                "move the reveal, or record the character learning this earlier"
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
        }

        // ── Pacing drift: a sustained sag in realized intensity ─────────
        // For books with a planned pacing curve, flag when realized per-chapter
        // intensity falls across several consecutive chapters. Transparent
        // heuristic: a single dip is intentional, a sustained slide is drift.
        if should_run_check(&requested_checks_set, "pacing_drift") {
            let books_with_curves: BTreeSet<i32> = self
                .repository
                .list_pacing_curves_by_project(&project_id)
                .await?
                .into_iter()
                .map(|curve| curve.book_number)
                .collect();
            if !books_with_curves.is_empty() {
                let annotations = self
                    .repository
                    .list_scene_beat_annotations_by_project(&project_id)
                    .await?;
                // Shared per-chapter mean-intensity computation, also driving the
                // realized-intensity feed-forward in `get_scene_context`.
                let chapter_means = crate::format::chapter_mean_intensities(&scenes, &annotations);
                let mut per_book: BTreeMap<i32, Vec<(i32, f64)>> = BTreeMap::new();
                for ((book, chapter), mean) in &chapter_means {
                    if !books_with_curves.contains(book) {
                        continue;
                    }
                    per_book.entry(*book).or_default().push((*chapter, *mean));
                }
                for (book, mut chapters) in per_book {
                    chapters.sort_by_key(|(chapter, _)| *chapter);
                    if chapters.len() < 4 {
                        continue;
                    }
                    let mut run = 1usize;
                    let mut max_run = 1usize;
                    for window in chapters.windows(2) {
                        if window[1].1 < window[0].1 - 0.01 {
                            run += 1;
                            max_run = max_run.max(run);
                        } else {
                            run = 1;
                        }
                    }
                    if max_run >= 3 {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "pacing_drift".to_string(),
                            message: format!(
                                "realized intensity falls across {max_run} consecutive chapters in book {book} (a sustained sag)"
                            ),
                            entity_ids: Vec::new(),
                            suggested_action: Some(
                                "raise the stakes or add a turn; a sustained intensity drop reads as a sagging middle".to_string(),
                            ),
                        });
                    }
                }
            }
        }

        if should_run_check(&requested_checks_set, "tone_consistency") {
            let allowed_boundaries = self
                .repository
                .get_project(&project_id)
                .await?
                .reader_contract
                .boundaries
                .iter()
                .map(|boundary| boundary.to_lowercase())
                .collect::<Vec<_>>();
            for scene in &scenes {
                if let Some(tone) = scene.tone.as_ref() {
                    let tone_lower = tone.to_lowercase();
                    if allowed_boundaries.iter().any(|boundary| {
                        boundary.contains("tone:") && !boundary.contains(&tone_lower)
                    }) {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "tone_consistency".to_string(),
                            message: format!(
                                "scene {} uses tone '{}' outside declared tone boundaries",
                                scene.id, tone
                            ),
                            entity_ids: vec![scene.id.clone()],
                            suggested_action: Some(
                                "revise scene tone or update the reader contract boundary"
                                    .to_string(),
                            ),
                        });
                    }
                } else {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "tone_consistency".to_string(),
                        message: format!("scene {} has no tone metadata", scene.id),
                        entity_ids: vec![scene.id.clone()],
                        suggested_action: Some(
                            "save_scene_draft with a tone value for continuity review".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "world_rule_compliance") {
            let rules = self
                .repository
                .list_world_rules_by_project(&project_id)
                .await?;
            if !rules.is_empty() && scenes.is_empty() {
                issues.push(ConsistencyIssue {
                    severity: "info".to_string(),
                    check_type: "world_rule_compliance".to_string(),
                    message: "world rules exist but there are no scenes in the requested scope"
                        .to_string(),
                    entity_ids: rules.iter().map(|rule| rule.id.clone()).collect(),
                    suggested_action: Some(
                        "write scenes before expecting rule compliance checks".to_string(),
                    ),
                });
            }

            for rule in &rules {
                if let Some(established_in) = rule.established_in.as_ref() {
                    if !scope_contains_chapter(
                        &scope,
                        established_in.book_number,
                        established_in.chapter_number,
                    ) {
                        continue;
                    }
                    let chapter_scenes = scenes
                        .iter()
                        .filter(|scene| {
                            scene.book_number == established_in.book_number
                                && scene.chapter_number == established_in.chapter_number
                        })
                        .collect::<Vec<_>>();
                    if chapter_scenes.is_empty() {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "world_rule_compliance".to_string(),
                            message: format!(
                                "world rule '{}' is established in a chapter with no scoped scene evidence",
                                rule.rule_name
                            ),
                            entity_ids: vec![rule.id.clone()],
                            suggested_action: Some(
                                "add or restore the establishing scene for this rule".to_string(),
                            ),
                        });
                        continue;
                    }

                    if !chapter_scenes
                        .into_iter()
                        .any(|scene| crate::format::scene_mentions_rule(scene, rule))
                    {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "world_rule_compliance".to_string(),
                            message: format!(
                                "world rule '{}' is not clearly demonstrated in its establishing chapter",
                                rule.rule_name
                            ),
                            entity_ids: vec![rule.id.clone()],
                            suggested_action: Some(
                                "revise the establishing scenes so the rule is shown on page"
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
            if deep_check {
                // Model-router-driven semantic compliance audit on top of
                // the pattern-based path above. Falls back to an in-process
                // heuristic when the configured route is local-only or
                // unavailable. Mirrors `services/mod.rs:6344` in 705b835^.
                let mut coverage = spindle_core::models::AuditCoverage::default();
                let semantic_issues = self
                    .deep_world_rule_compliance_issues(&scenes, &rules, &mut coverage)
                    .await?;
                issues.extend(semantic_issues);
                audit_coverage.push(coverage);
            }
        }

        if should_run_check(&requested_checks_set, "character_consistency") {
            let characters = self
                .repository
                .list_characters_by_project(&project_id)
                .await?;
            // Project-wide character states fetched once; filter per character below.
            let all_states = self
                .repository
                .list_character_states_by_project_and_branch(&project_id, &active_branch.id)
                .await?;
            for character in characters {
                let voice_profile = self
                    .repository
                    .get_character_voice_profile(&character.id)
                    .await?;
                let emotional_profile = self
                    .repository
                    .get_character_emotional_profile(&character.id)
                    .await?;

                if voice_profile.example_lines.is_empty() {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "character_consistency".to_string(),
                        message: format!(
                            "character '{}' has no example dialogue lines",
                            character.name
                        ),
                        entity_ids: vec![character.id.clone(), voice_profile.id.clone()],
                        suggested_action: Some(
                            "add example_lines so dialogue checks have a baseline".to_string(),
                        ),
                    });
                }

                if emotional_profile.base_emotions.is_empty()
                    && emotional_profile.suppressed.is_empty()
                    && emotional_profile.triggers.is_empty()
                {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "character_consistency".to_string(),
                        message: format!(
                            "character '{}' has a very thin emotional profile",
                            character.name
                        ),
                        entity_ids: vec![character.id.clone(), emotional_profile.id.clone()],
                        suggested_action: Some(
                            "expand the emotional profile before relying on consistency checks"
                                .to_string(),
                        ),
                    });
                }

                let scoped_states = all_states
                    .iter()
                    .filter(|state| state.character_id == character.id)
                    .filter(|state| {
                        scope_contains_position(
                            &scope,
                            state.book_number,
                            state.chapter_number,
                            state.scene_order,
                        )
                    })
                    .collect::<Vec<_>>();
                for window in scoped_states.windows(2) {
                    let previous = window[0];
                    let current = window[1];
                    if current.book_number == previous.book_number
                        && current.chapter_number == previous.chapter_number
                        && current.scene_order == previous.scene_order
                    {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "character_consistency".to_string(),
                            message: format!(
                                "character '{}' has multiple state snapshots at book {}, chapter {}, scene {}",
                                character.name, current.book_number, current.chapter_number, current.scene_order
                            ),
                            entity_ids: vec![current.id.clone()],
                            suggested_action: Some("consolidate duplicate state commits for this scene".to_string()),
                        });
                    }
                }
            }
        }

        if should_run_check(&requested_checks_set, "agency_tracking") {
            for plan in &scoped_plans {
                let low_purpose_count = plan
                    .scenes
                    .iter()
                    .filter(|scene| scene.purpose.trim().len() < 12)
                    .count();
                if low_purpose_count > 0 {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "agency_tracking".to_string(),
                        message: format!(
                            "chapter plan {}:{} has {} scene purposes that are too vague to audit agency",
                            plan.book_number, plan.chapter_number, low_purpose_count
                        ),
                        entity_ids: vec![plan.id.clone()],
                        suggested_action: Some(
                            "rewrite scene purposes as concrete choices or pressures".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "try_fail_cycle_tracking") {
            let conflicts = self
                .repository
                .list_conflicts_by_project(&project_id)
                .await?;
            for conflict in conflicts {
                let steps = conflict.try_fail_cycles.len();
                if conflict.expected_total_cycles.unwrap_or(steps as i32) >= 2 && steps < 2 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "try_fail_cycle_tracking".to_string(),
                        message: format!(
                            "conflict '{}' does not yet have enough try-fail steps",
                            conflict.name
                        ),
                        entity_ids: vec![conflict.id.clone()],
                        suggested_action: Some(
                            "add more failed attempts before resolution".to_string(),
                        ),
                    });
                }
                if conflict.resolution_summary.is_some() && steps == 0 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "try_fail_cycle_tracking".to_string(),
                        message: format!(
                            "conflict '{}' resolves without recorded failed attempts",
                            conflict.name
                        ),
                        entity_ids: vec![conflict.id.clone()],
                        suggested_action: Some(
                            "record the failed attempts or simplify the conflict model".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "consequence_delivery_audit") {
            let conflicts = self
                .repository
                .list_conflicts_by_project(&project_id)
                .await?;
            for conflict in conflicts {
                for consequence in conflict.stated_consequences {
                    if consequence.delivered {
                        continue;
                    }
                    issues.push(ConsistencyIssue {
                        severity: if consequence.must_demonstrate_by.is_some() {
                            "warning".to_string()
                        } else {
                            "info".to_string()
                        },
                        check_type: "consequence_delivery_audit".to_string(),
                        message: format!(
                            "conflict '{}' has an undelivered consequence: {}",
                            conflict.name, consequence.description
                        ),
                        entity_ids: vec![conflict.id.clone()],
                        suggested_action: Some(
                            "show the consequence on page or mark it intentionally deferred"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "content_boundary_compliance") {
            for scene in &scenes {
                if scene.content_rating.eq_ignore_ascii_case("Explicit") {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "content_boundary_compliance".to_string(),
                        message: format!(
                            "scene {} is explicit and should be reviewed against reader boundaries manually",
                            scene.id
                        ),
                        entity_ids: vec![scene.id.clone()],
                        suggested_action: Some("review scene tags and boundaries before finalizing".to_string()),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "knowledge_contradiction_detection")
            && !scoped_annotations.is_empty()
            && scenes.is_empty()
        {
            issues.push(ConsistencyIssue {
                severity: "info".to_string(),
                check_type: "knowledge_contradiction_detection".to_string(),
                message:
                    "scene annotations exist without scoped scenes; knowledge checks are incomplete"
                        .to_string(),
                entity_ids: scoped_annotations
                    .iter()
                    .map(|annotation| annotation.id.clone())
                    .collect(),
                suggested_action: Some(
                    "restore or rescope the related scenes before running contradiction checks"
                        .to_string(),
                ),
            });
        }

        if should_run_check(&requested_checks_set, "knowledge_contradiction_detection") {
            let future_knowledge = self
                .repository
                .list_future_knowledge_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|knowledge| {
                    scope_contains_position(
                        &scope,
                        knowledge.learned_at.book_number,
                        knowledge.learned_at.chapter_number,
                        knowledge.learned_at.scene_order.unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>();
            let timeline_events = self
                .repository
                .list_timeline_events_by_project(&project_id)
                .await?;
            let timeline_event_ids = timeline_events
                .iter()
                .map(|event| (event.id.clone(), event))
                .collect::<BTreeMap<_, _>>();
            let interventions = self
                .repository
                .list_temporal_interventions_by_project(&project_id)
                .await?;

            for knowledge in &future_knowledge {
                let learned_index = story_index_from_placement(&knowledge.learned_at);

                if let Some(expires_at) = knowledge.expires_at.as_ref() {
                    let expires_index = story_index_from_placement(expires_at);
                    if expires_index < learned_index {
                        issues.push(ConsistencyIssue {
                            severity: "error".to_string(),
                            check_type: "knowledge_contradiction_detection".to_string(),
                            message: format!(
                                "future knowledge '{}' expires before it is learned",
                                knowledge.knowledge_summary
                            ),
                            entity_ids: vec![knowledge.id.clone()],
                            suggested_action: Some(
                                "move the expiry later or revise the learned-at placement"
                                    .to_string(),
                            ),
                        });
                    }
                }

                let invalidating_interventions = interventions
                    .iter()
                    .filter(|intervention| {
                        intervention
                            .consequences
                            .iter()
                            .any(|consequence| consequence.to_lowercase().contains("invalidate"))
                    })
                    .filter_map(|intervention| {
                        intervention
                            .target_event_id
                            .as_ref()
                            .and_then(|id| timeline_event_ids.get(id).copied())
                            .map(|event| (intervention, event))
                    })
                    .filter(|(_, event)| {
                        story_index_from_placement(&event.placement) >= learned_index
                    })
                    .collect::<Vec<_>>();

                if !invalidating_interventions.is_empty() && knowledge.expires_at.is_none() {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "knowledge_contradiction_detection".to_string(),
                        message: format!(
                            "future knowledge '{}' may be invalidated by a later intervention but has no expiry",
                            knowledge.knowledge_summary
                        ),
                        entity_ids: std::iter::once(knowledge.id.clone())
                            .chain(
                                invalidating_interventions
                                    .iter()
                                    .map(|(intervention, _)| intervention.id.clone()),
                            )
                            .collect(),
                        suggested_action: Some(
                            "set an expiry placement or clarify why the knowledge remains valid"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        // The write-time retcon scanner also sees character state (the Phase-4
        // registry does not). Resolve inherited scenes against the active branch.
        if should_run_check(&requested_checks_set, "retcon_reachability") {
            let resolved = self
                .repository
                .list_scenes_resolved_for_branch(&project_id, &active_branch.id)
                .await?;
            let retcon_scenes = self
                .narrow_scenes_by_subjects(
                    &project_id,
                    &active_branch.id,
                    scoped_scenes(resolved, &scope),
                    &input.subjects,
                )
                .await?;
            for scene in &retcon_scenes {
                let mut effective_scene = scene.clone();
                effective_scene.branch_id = active_branch.id.clone();
                for finding in self
                    .scan_retcon_findings(&project_id, &effective_scene)
                    .await?
                {
                    let mut issue = retcon_finding_to_consistency_issue(&finding);
                    issue.entity_ids.push(scene.id.clone());
                    if !issues.iter().any(|existing| {
                        existing.check_type == issue.check_type
                            && existing.message == issue.message
                            && existing.entity_ids == issue.entity_ids
                    }) {
                        issues.push(issue.clone());
                    }
                    // Stable detector-name alias: dead-character acting is a
                    // first-class contradiction type for consumers that do not
                    // know the historical `retcon_reachability` bucket.
                    if matches!(
                        finding,
                        spindle_core::models::RetconFinding::DeadCharacterAct { .. }
                    ) && !issues.iter().any(|existing| {
                        existing.check_type == "dead_character_acting"
                            && existing.message == issue.message
                            && existing.entity_ids == issue.entity_ids
                    }) {
                        let mut alias = issue;
                        alias.check_type = "dead_character_acting".to_string();
                        issues.push(alias);
                    }
                }
            }
        }

        // ── Phase-4 validator fan-out ───────────────────────────────
        // Runs canonical_fact_prose_drift / world_rule_semantic_drift /
        // voice_drift / retcon_reachability across the scoped scenes. The
        // resulting issues are appended to `issues` (and surface in
        // `report_sections` via `build_consistency_report_sections` below).
        // Mirrors `services/mod.rs:6233` in 705b835^.
        let phase_four_checks = requested_phase_four_validator_checks(&requested_checks_set);
        let phase_four_issues = self
            .run_phase_four_validator_checks_for_scenes(
                &project_id,
                &active_branch.id,
                &scenes,
                &phase_four_checks,
                true,
            )
            .await?;
        issues.extend(phase_four_issues);

        // ── Gate 1 + 3: scene divergence detection ──────────────────
        if should_run_check(&requested_checks_set, "scene_divergence") {
            let source_links = self
                .repository
                .list_scene_source_links_by_project(&project_id)
                .await?;
            for link in &source_links {
                let link_scene_id = link.scene_id.clone();
                if !scene_ids.contains(&link_scene_id) {
                    continue; // outside the requested scope
                }
                let Some(scene) = scenes.iter().find(|scene| scene.id == link_scene_id) else {
                    continue;
                };
                if let Some(observation) =
                    crate::sqlite::source_bridge::evaluate_scene_divergence(link, scene)
                {
                    issues.push(
                        crate::sqlite::source_bridge::divergence_observation_to_consistency_issue(
                            &observation,
                            &link.source_path,
                            &link_scene_id,
                        ),
                    );
                }
            }
        }

        // ── Gate 2: scope-aware canonical fact contradiction detection ──
        // Two active facts conflict only when they share a subject+predicate key,
        // carry different values, AND have overlapping validity windows. Facts
        // with disjoint windows are legitimate evolving state, not contradictions.
        if should_run_check(&requested_checks_set, "canonical_fact_consistency") {
            let facts = self
                .repository
                .list_active_canonical_facts_by_project(&project_id)
                .await?;
            let candidates: Vec<crate::format::ContradictionCandidate> = facts
                .iter()
                .map(crate::format::candidate_from_canonical_fact)
                .collect();
            for contradiction in crate::format::detect_fact_contradictions(&candidates) {
                issues.push(ConsistencyIssue {
                    severity: "error".to_string(),
                    check_type: "canonical_fact_consistency".to_string(),
                    message: format!(
                        "canonical fact '{}' has conflicting active values: {}",
                        contradiction.composite_key,
                        contradiction
                            .values
                            .iter()
                            .map(|value| format!("'{value}'"))
                            .collect::<Vec<_>>()
                            .join(" vs ")
                    ),
                    entity_ids: contradiction.conflicting_ids,
                    suggested_action: Some(
                        "supersede the outdated fact (register_canonical_fact with supersedes_fact_id) or set distinct validity windows".to_string(),
                    ),
                });
            }
        }

        if should_run_check(&requested_checks_set, "research_accuracy") {
            // Check 1: Claims without note or source provenance
            let claims = self
                .repository
                .list_research_claims_by_project(&project_id)
                .await?;
            for claim in claims {
                if claim.note_id.is_none() && claim.source_id.is_none() {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "research_accuracy".to_string(),
                        message: format!(
                            "Research claim '{}' ({}) has no note or source provenance",
                            claim.claim, claim.id
                        ),
                        entity_ids: vec![claim.id.clone()],
                        suggested_action: Some(
                            "Link this claim to a parent note or source".to_string(),
                        ),
                    });
                }
            }

            // Check 2: Missing usage for scenes requiring research
            for scene in &scenes {
                let plan_scene = scoped_plans
                    .iter()
                    .find(|plan| {
                        plan.book_number == scene.book_number
                            && plan.chapter_number == scene.chapter_number
                    })
                    .and_then(|plan| {
                        plan.scenes
                            .iter()
                            .find(|s| s.scene_order == scene.scene_order)
                    });

                let is_required = plan_scene
                    .and_then(|ps| ps.research_required)
                    .unwrap_or(false);
                let has_required_tags = plan_scene
                    .map(|ps| !ps.research_tags.is_empty())
                    .unwrap_or(false);

                if is_required || has_required_tags {
                    let usages = self
                        .list_research_usages_for_scene(&project_id, &scene.id)
                        .await?;
                    let total_usages: usize = usages
                        .iter()
                        .map(|u| u.source_ids.len() + u.note_ids.len() + u.claim_ids.len())
                        .sum();
                    if total_usages == 0 {
                        let reason = if is_required {
                            "is marked as research_required"
                        } else {
                            "has required research tags"
                        };
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "research_accuracy".to_string(),
                            message: format!(
                                "Scene {} (book {}, chapter {} scene {}) {} but has no recorded research usage",
                                scene.id, scene.book_number, scene.chapter_number, scene.scene_order, reason
                            ),
                            entity_ids: vec![scene.id.clone()],
                            suggested_action: Some("Draft or save the scene with research references".to_string()),
                        });
                    }
                }
            }
        }

        // Chapter lookup for the metadata-only motif/theme audits below: each
        // scoped scene id maps to its (book, chapter) key, and the scoped
        // chapter set feeds the ≥3-chapter noise gate on unused motifs.
        let scene_chapter_by_id: BTreeMap<String, (i32, i32)> = scenes
            .iter()
            .map(|scene| (scene.id.clone(), (scene.book_number, scene.chapter_number)))
            .collect();
        let scoped_chapter_keys: BTreeSet<(i32, i32)> =
            scene_chapter_by_id.values().copied().collect();

        if should_run_check(&requested_checks_set, "motif_usage_audit") {
            let mut motifs = self
                .repository
                .list_motifs_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|motif| motif.archived_at.is_none())
                .collect::<Vec<_>>();
            // Deterministic finding order: entity id then chapter (chapter
            // order falls out of the BTreeMap iteration below).
            motifs.sort_by(|a, b| a.id.cmp(&b.id));

            // Count beat-annotation motif links per (motif, chapter) within
            // scope, plus whether the motif was linked anywhere in scope at all.
            let mut links_by_motif_chapter: BTreeMap<(String, (i32, i32)), i64> = BTreeMap::new();
            let mut linked_motifs: BTreeSet<String> = BTreeSet::new();
            for annotation in &scoped_annotations {
                let Some(chapter) = scene_chapter_by_id.get(&annotation.scene_id).copied() else {
                    continue;
                };
                for motif_id in &annotation.motif_ids {
                    linked_motifs.insert(motif_id.clone());
                    *links_by_motif_chapter
                        .entry((motif_id.clone(), chapter))
                        .or_insert(0) += 1;
                }
            }

            for motif in &motifs {
                if let Some(limit) = motif.max_uses_per_chapter {
                    // Overuse: strictly above the limit fires; exactly at the
                    // limit is clean.
                    for ((motif_id, chapter), count) in &links_by_motif_chapter {
                        if motif_id != &motif.id {
                            continue;
                        }
                        if *count > limit as i64 {
                            issues.push(ConsistencyIssue {
                                severity: "warning".to_string(),
                                check_type: "motif_usage_audit".to_string(),
                                message: format!(
                                    "motif '{}' is linked {} time(s) in book {}, chapter {}, over its limit of {}",
                                    motif.name, count, chapter.0, chapter.1, limit
                                ),
                                entity_ids: vec![motif.id.clone()],
                                suggested_action: Some(
                                    "thin the motif in this chapter or raise max_uses_per_chapter"
                                        .to_string(),
                                ),
                            });
                        }
                    }
                }

                // Declared-but-unused: only when the scope covers ≥3 chapters,
                // to avoid noise on small scopes.
                if scoped_chapter_keys.len() >= 3 && !linked_motifs.contains(&motif.id) {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "motif_usage_audit".to_string(),
                        message: format!(
                            "motif '{}' is declared but never linked by any beat annotation in scope",
                            motif.name
                        ),
                        entity_ids: vec![motif.id.clone()],
                        suggested_action: Some(
                            "annotate a scene with this motif or archive it".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "theme_placement_audit") {
            let mut themes = self
                .repository
                .list_themes_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|theme| theme.archived_at.is_none())
                .collect::<Vec<_>>();
            // Deterministic finding order: entity id (introduction before
            // resolution within a theme).
            themes.sort_by(|a, b| a.id.cmp(&b.id));

            // Chapters in which each theme is linked by a beat annotation.
            let mut theme_link_chapters: BTreeMap<String, BTreeSet<(i32, i32)>> = BTreeMap::new();
            for annotation in &scoped_annotations {
                let Some(chapter) = scene_chapter_by_id.get(&annotation.scene_id).copied() else {
                    continue;
                };
                for theme_id in &annotation.theme_ids {
                    theme_link_chapters
                        .entry(theme_id.clone())
                        .or_default()
                        .insert(chapter);
                }
            }

            for theme in &themes {
                let linked_chapters = theme_link_chapters.get(&theme.id);
                for (label, placement) in [
                    ("introduction", theme.introduction_point.as_ref()),
                    ("resolution", theme.resolution_point.as_ref()),
                ] {
                    let Some(placement) = placement else {
                        continue;
                    };
                    let chapter = (placement.book_number, placement.chapter_number);
                    if !scope_contains_chapter(&scope, chapter.0, chapter.1) {
                        continue;
                    }
                    let linked_here = linked_chapters
                        .map(|chapters| chapters.contains(&chapter))
                        .unwrap_or(false);
                    if !linked_here {
                        issues.push(ConsistencyIssue {
                            severity: "info".to_string(),
                            check_type: "theme_placement_audit".to_string(),
                            message: format!(
                                "theme '{}' declares its {} point at book {}, chapter {} but no beat annotation links it there",
                                theme.theme_statement, label, chapter.0, chapter.1
                            ),
                            entity_ids: vec![theme.id.clone()],
                            suggested_action: Some(
                                "annotate a scene in that chapter with this theme or move the placement"
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
        }

        // plot_line_convergence_audit (V0022): a plot line whose convergence
        // point resolves to a chapter in scope, that declares connected
        // conflicts and/or themes, but whose convergence chapter carries no beat
        // annotation linking any of those connected ids → info. A convergence
        // outside scope, or a plot line that declares no connected ids at all,
        // is silent.
        if should_run_check(&requested_checks_set, "plot_line_convergence_audit") {
            let mut plot_lines = self
                .repository
                .list_plot_lines_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|plot_line| plot_line.archived_at.is_none())
                .collect::<Vec<_>>();
            // Deterministic finding order: entity id then chapter.
            plot_lines.sort_by(|a, b| a.id.cmp(&b.id));

            // Conflicts and themes linked by a beat annotation, keyed by chapter.
            let mut conflicts_by_chapter: BTreeMap<(i32, i32), BTreeSet<String>> = BTreeMap::new();
            let mut themes_by_chapter: BTreeMap<(i32, i32), BTreeSet<String>> = BTreeMap::new();
            for annotation in &scoped_annotations {
                let Some(chapter) = scene_chapter_by_id.get(&annotation.scene_id).copied() else {
                    continue;
                };
                for conflict_id in &annotation.conflict_ids {
                    conflicts_by_chapter
                        .entry(chapter)
                        .or_default()
                        .insert(conflict_id.clone());
                }
                for theme_id in &annotation.theme_ids {
                    themes_by_chapter
                        .entry(chapter)
                        .or_default()
                        .insert(theme_id.clone());
                }
            }

            for plot_line in &plot_lines {
                if plot_line.connected_conflict_ids.is_empty()
                    && plot_line.connected_theme_ids.is_empty()
                {
                    // No declared connections → nothing to check against.
                    continue;
                }
                // Convergence chapters in scope, ascending for deterministic order.
                let mut convergence_chapters: Vec<(i32, i32)> = plot_line
                    .convergence_points
                    .iter()
                    .map(|point| (point.book_number, point.chapter_number))
                    .filter(|(book, chapter)| scope_contains_chapter(&scope, *book, *chapter))
                    .collect();
                convergence_chapters.sort_unstable();
                convergence_chapters.dedup();

                for chapter in convergence_chapters {
                    let conflict_here = conflicts_by_chapter.get(&chapter);
                    let theme_here = themes_by_chapter.get(&chapter);
                    let linked = plot_line
                        .connected_conflict_ids
                        .iter()
                        .any(|id| conflict_here.map(|set| set.contains(id)).unwrap_or(false))
                        || plot_line
                            .connected_theme_ids
                            .iter()
                            .any(|id| theme_here.map(|set| set.contains(id)).unwrap_or(false));
                    if !linked {
                        issues.push(ConsistencyIssue {
                            severity: "info".to_string(),
                            check_type: "plot_line_convergence_audit".to_string(),
                            message: format!(
                                "plot line '{}' converges at book {}, chapter {} but no beat annotation there links any of its connected conflicts or themes",
                                plot_line.name, chapter.0, chapter.1
                            ),
                            entity_ids: vec![plot_line.id.clone()],
                            suggested_action: Some(
                                "annotate a scene in that chapter with a connected conflict/theme, or revise the convergence point"
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
        }

        // arc_milestone_audit (V0022): a milestone placed at chapter P (in or
        // before the scope end) that has not been reached. If the manuscript
        // already runs past P (max scoped chapter ≥ P+1) → warning with the
        // overdue count; if the scope has only reached P (max == P) → info "due
        // now". A milestone with a `reached_at` marker, or with no placement, is
        // silent.
        if should_run_check(&requested_checks_set, "arc_milestone_audit") {
            let mut arcs = self
                .repository
                .list_character_arcs_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|arc| arc.archived_at.is_none())
                .collect::<Vec<_>>();
            // Deterministic finding order: entity id then chapter.
            arcs.sort_by(|a, b| a.id.cmp(&b.id));

            // Max scoped chapter per book, from the scoped scene set.
            let mut max_chapter_by_book: BTreeMap<i32, i32> = BTreeMap::new();
            for (book, chapter) in &scoped_chapter_keys {
                let entry = max_chapter_by_book.entry(*book).or_insert(*chapter);
                if *chapter > *entry {
                    *entry = *chapter;
                }
            }

            for arc in &arcs {
                // Milestones in ascending placement order for deterministic finds.
                let mut pending: Vec<(&str, i32, i32)> = arc
                    .milestones
                    .iter()
                    .filter(|milestone| milestone.reached_at.is_none())
                    .filter_map(|milestone| {
                        milestone.placement.as_ref().map(|placement| {
                            (
                                milestone.label.as_str(),
                                placement.book_number,
                                placement.chapter_number,
                            )
                        })
                    })
                    .collect();
                pending.sort_by(|a, b| (a.1, a.2, a.0).cmp(&(b.1, b.2, b.0)));

                for (label, book, milestone_chapter) in pending {
                    if !scope_contains_chapter(&scope, book, milestone_chapter) {
                        continue;
                    }
                    let Some(max_chapter) = max_chapter_by_book.get(&book).copied() else {
                        continue;
                    };
                    if max_chapter < milestone_chapter {
                        // Manuscript hasn't reached the milestone chapter yet.
                        continue;
                    }
                    // max_chapter >= milestone_chapter here; strictly greater
                    // means the manuscript has run past the due chapter.
                    if max_chapter > milestone_chapter {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "arc_milestone_audit".to_string(),
                            message: format!(
                                "arc milestone '{}' is due at book {}, chapter {} but is not marked reached, and the manuscript is {} chapter(s) past it",
                                label, book, milestone_chapter, max_chapter - milestone_chapter
                            ),
                            entity_ids: vec![arc.id.clone()],
                            suggested_action: Some(
                                "call update_arc_milestone to stamp reached_at where the \
                                 milestone lands, or to move its placement"
                                    .to_string(),
                            ),
                        });
                    } else {
                        // max_chapter == milestone_chapter.
                        issues.push(ConsistencyIssue {
                            severity: "info".to_string(),
                            check_type: "arc_milestone_audit".to_string(),
                            message: format!(
                                "arc milestone '{}' is due now at book {}, chapter {} and not yet marked reached",
                                label, book, milestone_chapter
                            ),
                            entity_ids: vec![arc.id.clone()],
                            suggested_action: Some(
                                "demonstrate the milestone in this chapter and set reached_at \
                                 via update_arc_milestone"
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
        }

        // conflict_escalation_audit (V0022): a conflict whose per-stage
        // demonstration markers are out of order — any pair (i < j) where stage
        // j is demonstrated (Some placement) while an earlier stage i is not →
        // warning naming the conflict and the out-of-order stage labels. All
        // demonstrated stages in order, or no markers at all, is silent.
        if should_run_check(&requested_checks_set, "conflict_escalation_audit") {
            let mut conflicts = self
                .repository
                .list_conflicts_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|conflict| conflict.archived_at.is_none())
                .collect::<Vec<_>>();
            // Deterministic finding order: entity id.
            conflicts.sort_by(|a, b| a.id.cmp(&b.id));

            for conflict in &conflicts {
                let stages = &conflict.escalation_stages;
                let markers = &conflict.escalation_demonstrated;
                // Absent markers beyond the vector length read as None.
                let demonstrated = |index: usize| -> bool {
                    markers.get(index).map(|m| m.is_some()).unwrap_or(false)
                };
                let stage_label = |index: usize| -> String {
                    stages
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("stage {}", index + 1))
                };
                // First out-of-order pair (earliest undemonstrated i, earliest
                // later demonstrated j) is enough to flag the conflict once.
                let mut flagged: Option<(usize, usize)> = None;
                let stage_count = stages.len().max(markers.len());
                'outer: for i in 0..stage_count {
                    if demonstrated(i) {
                        continue;
                    }
                    for j in (i + 1)..stage_count {
                        if demonstrated(j) {
                            flagged = Some((i, j));
                            break 'outer;
                        }
                    }
                }
                if let Some((i, j)) = flagged {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "conflict_escalation_audit".to_string(),
                        message: format!(
                            "conflict '{}' demonstrates escalation stage '{}' before the earlier stage '{}' is demonstrated",
                            conflict.name, stage_label(j), stage_label(i)
                        ),
                        entity_ids: vec![conflict.id.clone()],
                        suggested_action: Some(
                            "demonstrate the earlier escalation stage first, or correct the demonstration markers"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        // secret_leak (design §2.4, deterministic tier): the audience-direction
        // complement to knowledge_timing. For each scoped scene, for each secret
        // fact whose circle-at-cursor does NOT cover a PRESENT character, scan
        // that out-of-circle speaker's ATTRIBUTED DIALOGUE (reusing voice_drift's
        // speaker attribution) for the secret's value lexeme (reusing
        // canonical_fact_prose_drift's whole-word matcher). A hit is the leak.
        // Narration/behavior scanning is deliberately out of v1 scope (deep tier,
        // next wave). Silent for projects with no secret facts.
        if should_run_check(&requested_checks_set, "secret_leak") {
            let secret_facts = self
                .repository
                .list_active_canonical_facts_by_project_and_branch(&project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|fact| fact.secret)
                .collect::<Vec<_>>();

            if !secret_facts.is_empty() {
                let characters = self
                    .repository
                    .list_characters_by_project(&project_id)
                    .await?;
                let mention_records: Vec<spindle_core::character_mentions::CharacterMentionRecord> =
                    characters
                        .iter()
                        .map(
                            |character| spindle_core::character_mentions::CharacterMentionRecord {
                                id: character.id.clone(),
                                name: character.name.clone(),
                                aliases: character.aliases.clone(),
                            },
                        )
                        .collect();
                let mention_resolver =
                    spindle_core::character_mentions::CharacterMentionResolver::new(
                        &mention_records,
                    );

                // Circle-at-cursor per fact is cursor-dependent, so compute it
                // per (scene, fact). Cache the raw circle rows per fact to avoid
                // repeated DB reads across scenes.
                let mut circle_cache: BTreeMap<String, Vec<(String, Option<i64>)>> =
                    BTreeMap::new();

                // Deterministic order: scenes are already ordered; secret facts
                // by id; leaking characters by id.
                let mut ordered_facts = secret_facts;
                ordered_facts.sort_by(|a, b| a.id.cmp(&b.id));

                for scene in &scenes {
                    let cursor = crate::format::story_index(
                        scene.book_number,
                        scene.chapter_number,
                        scene.scene_order,
                    );
                    for fact in &ordered_facts {
                        let value = crate::format::canonical_fact_value(fact);
                        if value.trim().is_empty() || value == "<unset>" {
                            continue;
                        }
                        // Circle rows (cached), narrowed to the scene cursor.
                        let rows = if let Some(rows) = circle_cache.get(&fact.id) {
                            rows.clone()
                        } else {
                            let rows = self
                                .repository
                                .secret_circle_members(&project_id, &active_branch.id, &fact.id)
                                .await?
                                .into_iter()
                                .map(|(character_id, learned_at)| {
                                    let idx = learned_at
                                        .as_ref()
                                        .map(crate::format::story_index_from_placement);
                                    (character_id, idx)
                                })
                                .collect::<Vec<_>>();
                            circle_cache.insert(fact.id.clone(), rows.clone());
                            rows
                        };
                        let circle_at_cursor: BTreeSet<&str> = rows
                            .iter()
                            .filter(|(_, idx)| idx.is_none_or(|i| i <= cursor))
                            .map(|(id, _)| id.as_str())
                            .collect();

                        // Present, out-of-circle characters (deterministic by id).
                        let mut leakers: Vec<&crate::sqlite::records::Character> = characters
                            .iter()
                            .filter(|character| !circle_at_cursor.contains(character.id.as_str()))
                            .filter(|character| {
                                mention_resolver.mentions(&scene.full_text, &character.id)
                            })
                            .collect();
                        leakers.sort_by(|a, b| a.id.cmp(&b.id));

                        for character in leakers {
                            let dialogue_leak =
                                spindle_core::voice::attributed_dialogue_ranges_for_aliases(
                                    &scene.full_text,
                                    mention_resolver.aliases_for(&character.id),
                                )
                                .into_iter()
                                .filter(|range| {
                                    range.end > range.start && range.end <= scene.full_text.len()
                                })
                                .any(|range| {
                                    crate::sqlite::validators::contains_case_insensitive_word(
                                        &scene.full_text[range.start..range.end],
                                        &value,
                                    )
                                });
                            let suggested_action = |medium: &str| {
                                format!(
                                    "record the reveal (record_knowledge with secret_of_fact_id) if intended, or revise the {medium}"
                                )
                            };
                            if dialogue_leak {
                                issues.push(ConsistencyIssue {
                                    severity: "warning".to_string(),
                                    check_type: "secret_leak".to_string(),
                                    message: format!(
                                        "scene (book {}, chapter {} scene {}): {} speaks secret fact '{}' ('{}') — no recorded reveal to {} before this scene",
                                        scene.book_number,
                                        scene.chapter_number,
                                        scene.scene_order,
                                        character.name,
                                        fact.predicate,
                                        value,
                                        character.name,
                                    ),
                                    entity_ids: vec![
                                        fact.id.clone(),
                                        character.id.clone(),
                                        scene.id.clone(),
                                    ],
                                    suggested_action: Some(suggested_action("dialogue")),
                                });
                                continue;
                            }
                            let scene_input = SceneValidationInputV1 {
                                scene_id: &scene.id,
                                book_number: scene.book_number,
                                chapter_number: scene.chapter_number,
                                scene_order: scene.scene_order,
                                text: &scene.full_text,
                            };
                            let candidate = SecretValidationCandidateV1 {
                                fact_id: fact.id.clone(),
                                predicate: fact.predicate.clone(),
                                value: value.clone(),
                                character_id: character.id.clone(),
                                character_name: character.name.clone(),
                                character_aliases: mention_resolver
                                    .aliases_for(&character.id)
                                    .to_vec(),
                                other_present_character_names: characters
                                    .iter()
                                    .filter(|other| other.id != character.id)
                                    .map(|other| other.name.clone())
                                    .collect(),
                                other_present_character_aliases: characters
                                    .iter()
                                    .filter(|other| other.id != character.id)
                                    .flat_map(|other| {
                                        mention_resolver.aliases_for(&other.id).iter().cloned()
                                    })
                                    .collect(),
                            };
                            if let Some(finding) =
                                detect_secret_narration_leak(&scene_input, &candidate)
                            {
                                issues.push(ConsistencyIssue {
                                    severity: finding.severity.as_str().to_string(),
                                    check_type: finding.check_type,
                                    message: finding.message,
                                    entity_ids: finding.entity_ids,
                                    suggested_action: Some(suggested_action("narration")),
                                });
                            }
                        }
                        let already_disclosed = issues.iter().any(|issue| {
                            issue.check_type == "secret_leak"
                                && issue.entity_ids.contains(&fact.id)
                                && issue.entity_ids.contains(&scene.id)
                        });
                        if !already_disclosed {
                            let holder_names: Vec<String> = fact
                                .subject_id
                                .as_ref()
                                .and_then(|holder_id| {
                                    characters
                                        .iter()
                                        .find(|character| &character.id == holder_id)
                                })
                                .map(|holder| {
                                    let mut names = vec![holder.name.clone()];
                                    names.extend(holder.aliases.iter().cloned());
                                    names
                                })
                                .unwrap_or_default();
                            let in_circle_names: Vec<String> = characters
                                .iter()
                                .filter(|character| {
                                    circle_at_cursor.contains(character.id.as_str())
                                })
                                .flat_map(|character| {
                                    let mut names = vec![character.name.clone()];
                                    names.extend(character.aliases.iter().cloned());
                                    names
                                })
                                .collect();
                            let generic_scene = SceneValidationInputV1 {
                                scene_id: &scene.id,
                                book_number: scene.book_number,
                                chapter_number: scene.chapter_number,
                                scene_order: scene.scene_order,
                                text: &scene.full_text,
                            };
                            if let Some(finding) = detect_secret_generic_disclosure(
                                &generic_scene,
                                &fact.id,
                                &value,
                                &holder_names,
                                &in_circle_names,
                            ) {
                                issues.push(ConsistencyIssue {
                                    severity: finding.severity.as_str().to_string(),
                                    check_type: finding.check_type,
                                    message: finding.message,
                                    entity_ids: finding.entity_ids,
                                    suggested_action: Some(
                                        "record the reveal (record_knowledge with secret_of_fact_id) if intended, or revise the disclosure"
                                            .to_string(),
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        // secret_leak DEEP tier (design §2.4, Item 5): model-backed behavioral
        // leak detection. Complements the deterministic (dialogue-lexeme) tier by
        // catching an out-of-circle character who ACTS ON, implies knowledge of,
        // or conspicuously avoids a secret. Runs ONLY when deep_check is set AND
        // the check is requested, one model call per (scene, secret-set), capped
        // and rating-gated. Silent for projects with no secret facts.
        if deep_check && should_run_check(&requested_checks_set, "secret_leak") {
            use spindle_core::voice::{VoiceDriftCharacter, character_present_in_scene};

            let secret_facts = self
                .repository
                .list_active_canonical_facts_by_project_and_branch(&project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|fact| fact.secret)
                .collect::<Vec<_>>();

            if !secret_facts.is_empty() {
                let characters = self
                    .repository
                    .list_characters_by_project(&project_id)
                    .await?;
                let name_of: BTreeMap<&str, &str> = characters
                    .iter()
                    .map(|c| (c.id.as_str(), c.name.as_str()))
                    .collect();

                // Cache raw circle rows per fact to avoid repeated DB reads.
                let mut circle_cache: BTreeMap<String, Vec<(String, Option<i64>)>> =
                    BTreeMap::new();
                let mut ordered_facts = secret_facts;
                ordered_facts.sort_by(|a, b| a.id.cmp(&b.id));

                // Scenes are already in story order; build one scan unit per scene
                // that has at least one in-play secret (an out-of-circle present
                // character), preserving that order for the cap ranking.
                let mut units: Vec<SecretLeakScanUnit> = Vec::new();
                for scene in &scenes {
                    let cursor = crate::format::story_index(
                        scene.book_number,
                        scene.chapter_number,
                        scene.scene_order,
                    );
                    // Present cast for this scene (deterministic by id).
                    let present: Vec<&crate::sqlite::records::Character> = characters
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
                    if present.is_empty() {
                        continue;
                    }

                    let mut secrets_in_play: Vec<SecretInPlay> = Vec::new();
                    let mut unaware_present: BTreeMap<String, String> = BTreeMap::new();
                    for fact in &ordered_facts {
                        let value = crate::format::canonical_fact_value(fact);
                        if value.trim().is_empty() || value == "<unset>" {
                            continue;
                        }
                        let rows = if let Some(rows) = circle_cache.get(&fact.id) {
                            rows.clone()
                        } else {
                            let rows = self
                                .repository
                                .secret_circle_members(&project_id, &active_branch.id, &fact.id)
                                .await?
                                .into_iter()
                                .map(|(character_id, learned_at)| {
                                    let idx = learned_at
                                        .as_ref()
                                        .map(crate::format::story_index_from_placement);
                                    (character_id, idx)
                                })
                                .collect::<Vec<_>>();
                            circle_cache.insert(fact.id.clone(), rows.clone());
                            rows
                        };
                        let circle_at_cursor: BTreeSet<&str> = rows
                            .iter()
                            .filter(|(_, idx)| idx.is_none_or(|i| i <= cursor))
                            .map(|(id, _)| id.as_str())
                            .collect();

                        let known: Vec<String> = present
                            .iter()
                            .filter(|c| circle_at_cursor.contains(c.id.as_str()))
                            .map(|c| c.id.clone())
                            .collect();
                        let unaware: Vec<String> = present
                            .iter()
                            .filter(|c| !circle_at_cursor.contains(c.id.as_str()))
                            .map(|c| c.id.clone())
                            .collect();

                        // "In play" = the circle does NOT cover the whole present
                        // cast (at least one out-of-circle present character).
                        if unaware.is_empty() {
                            continue;
                        }
                        for id in &unaware {
                            unaware_present.insert(
                                id.clone(),
                                name_of.get(id.as_str()).unwrap_or(&id.as_str()).to_string(),
                            );
                        }
                        secrets_in_play.push(SecretInPlay {
                            fact_value: value,
                            known_present: known,
                            unaware_present: unaware,
                        });
                    }

                    if secrets_in_play.is_empty() {
                        continue;
                    }
                    units.push(SecretLeakScanUnit {
                        scene_id: scene.id.clone(),
                        scene_ref: format!(
                            "book {}, chapter {} scene {}",
                            scene.book_number, scene.chapter_number, scene.scene_order
                        ),
                        scene_rating: scene.content_rating.clone(),
                        scene_text: scene.full_text.clone(),
                        secrets: secrets_in_play,
                        unaware_present,
                    });
                }
                let mut coverage = spindle_core::models::AuditCoverage::new(
                    "secret_leak",
                    units.iter().map(|u| u.scene_id.clone()),
                );
                coverage.window(deep_offset, SECRET_LEAK_DEEP_SCAN_CAP);
                units = units
                    .into_iter()
                    .skip(deep_offset)
                    .take(SECRET_LEAK_DEEP_SCAN_CAP)
                    .collect();
                let unscanned = coverage.eligible.saturating_sub(units.len());
                issues.extend(
                    self.deep_secret_behavioral_leak_issues(&units, unscanned, &mut coverage)
                        .await?,
                );
                audit_coverage.push(coverage);
            }
        }

        if should_run_check(&requested_checks_set, "canon_consistency") {
            issues.extend(
                self.canon_consistency_issues(&project_id, &active_branch.id)
                    .await?,
            );
        }

        if let Some(requested_severities) = requested_severities_set.as_ref() {
            issues.retain(|issue| requested_severities.contains(issue.severity.as_str()));
        }

        // Build per-validator scene groups for the Phase-4 issues so MCP
        // callers can render them under their own headings.
        let report_sections = build_consistency_report_sections(&issues, &scenes);

        let summary = ConsistencySummary {
            error_count: issues
                .iter()
                .filter(|issue| issue.severity == "error")
                .count(),
            warning_count: issues
                .iter()
                .filter(|issue| issue.severity == "warning")
                .count(),
            info_count: issues
                .iter()
                .filter(|issue| issue.severity == "info")
                .count(),
        };

        for coverage in &mut audit_coverage {
            coverage.finish();
        }
        let format = input.format.unwrap_or(ContextFormat::Json);
        let markdown = if format == ContextFormat::Markdown {
            let budget = input
                .budget_tokens
                .unwrap_or(DEFAULT_CHECK_CONSISTENCY_BUDGET_TOKENS);
            let mut report = format_consistency_markdown(&issues, &report_sections, budget);
            if !audit_coverage.is_empty() {
                // Coverage is protected from prose-budget trimming, like hard findings.
                report.push_str("\n## Audit coverage\n");
                for coverage in &audit_coverage {
                    report.push_str(&format!("- {}: {} eligible; {} model-reviewed; {} heuristic-only; {} not model-reviewed. Next offset: {:?}.\n",
                        coverage.check_type, coverage.eligible, coverage.evaluated_ids.len(), coverage.heuristic_ids.len(), coverage.not_evaluated_ids.len(), coverage.next_offset));
                }
            }
            Some(report)
        } else {
            None
        };

        Ok(spindle_core::models::CheckConsistencyOutput {
            audit_coverage,
            issues,
            summary,
            report_sections,
            markdown,
        })
    }
}
