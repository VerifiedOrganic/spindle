use super::*;

/// The sampled scene ids recorded on a checkpoint, for the `checkpoint_created`
/// payload. Recomputed from the after-state the same way the checkpoint report
/// samples them (ids only — no prose).
pub(super) fn authoring_checkpoint_sampled_scene_ids(
    state: &spindle_harness::state::HarnessState,
    checkpoint: &spindle_harness::state::CheckpointRecord,
) -> Vec<String> {
    authoring_sample_checkpoint_scene_ids(state, checkpoint.start_chapter, checkpoint.end_chapter)
        .unwrap_or_default()
}

pub(super) fn authoring_status_after_checkpoint_review(state: &HarnessState) -> &'static str {
    if state.chapters.iter().any(|chapter| {
        chapter
            .scenes
            .iter()
            .any(|scene| scene.blocked_reason.is_some())
    }) {
        return "blocked";
    }
    if state
        .checkpoint_history
        .iter()
        .any(|checkpoint| checkpoint.status == CheckpointStatus::PendingReview)
    {
        return "blocked";
    }
    if state.last_checkpoint_end_chapter >= state.range.end_chapter
        && state.chapters.iter().all(|chapter| {
            matches!(
                chapter.status,
                spindle_harness::state::ChapterStatus::Complete
            )
        })
    {
        return "completed";
    }
    "active"
}

pub(super) fn authoring_unresolved_checkpoint_directive(directives: &[String]) -> Option<String> {
    const UNRESOLVED_MARKERS: &[&str] = &[
        "acknowledged",
        "acknowledge",
        "fix in polish",
        "polish pass",
        "not actually fixed",
        "not fixed",
        "unfixed",
        "still unfixed",
        "defer",
        "deferred",
        "fix later",
        "later pass",
        "carry forward",
        "known issue",
        "known issues",
        "remaining issue",
        "remaining issues",
        "needs fix",
        "needs fixing",
        "todo",
    ];

    directives.iter().find_map(|directive| {
        let normalized = directive.to_ascii_lowercase();
        if UNRESOLVED_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            Some(directive.clone())
        } else {
            None
        }
    })
}

pub(super) fn authoring_build_chapter_summary_package(
    artifact_store: &ArtifactStore,
    chapter: &spindle_harness::state::ChapterState,
) -> anyhow::Result<GeneratedChapterSummaryPackage> {
    let mut scene_summaries = Vec::new();
    let mut key_events = Vec::new();
    let mut character_changes = Vec::new();
    let mut relationship_shifts = Vec::new();
    let mut arc_advances = Vec::new();
    let mut promise_events = Vec::new();

    for scene in &chapter.scenes {
        let artifact_path = scene.scene_artifact_path.as_ref().with_context(|| {
            format!(
                "chapter {} scene {} has no scene artifact path for summary",
                chapter.chapter_number, scene.scene_order
            )
        })?;
        let artifact: SceneGenerationArtifact = artifact_store
            .load_json(artifact_path)
            .with_context(|| format!("failed to load scene artifact {artifact_path}"))?;
        let package = artifact.package.as_ref().with_context(|| {
            format!("scene artifact {artifact_path} has no structured package for summary")
        })?;
        let scene_prefix = format!("Scene {}", scene.scene_order);

        if !package.summary.trim().is_empty() {
            scene_summaries.push(format!("{scene_prefix}: {}", package.summary.trim()));
        }

        for beat in &package.beats {
            if !beat.summary.trim().is_empty() {
                let beat_label = beat.beat_type.trim();
                let entry = if beat_label.is_empty() {
                    format!("{scene_prefix}: {}", beat.summary.trim())
                } else {
                    format!("{scene_prefix} [{beat_label}]: {}", beat.summary.trim())
                };
                if beat_label.eq_ignore_ascii_case("promise")
                    || beat_label.eq_ignore_ascii_case("narrative_promise")
                {
                    promise_events.push(entry.clone());
                }
                key_events.push(entry);
            }
        }

        for fact in &package.canonical_facts {
            let value = fact
                .value
                .as_deref()
                .or(fact.value_text.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    fact.value_json
                        .as_ref()
                        .map(ToString::to_string)
                        .filter(|value| !value.is_empty())
                });
            let label = fact
                .key
                .as_deref()
                .or(fact.predicate.as_deref())
                .unwrap_or("canonical fact")
                .trim();
            if let Some(value) = value {
                key_events.push(format!("{scene_prefix}: {label} = {value}"));
            }
        }

        for state_change in &package.character_states {
            if let Some(summary) = state_change.changes.source_summary.as_deref()
                && !summary.trim().is_empty()
            {
                character_changes.push(format!(
                    "{scene_prefix}: {} - {}",
                    state_change.character_id,
                    summary.trim()
                ));
            }
            if let Some(notes) = state_change.changes.notes.as_ref() {
                for note in notes {
                    if !note.trim().is_empty() {
                        character_changes.push(format!(
                            "{scene_prefix}: {} - {}",
                            state_change.character_id,
                            note.trim()
                        ));
                    }
                }
            }
            if let Some(statuses) = state_change.changes.status.as_ref() {
                for status in statuses {
                    if !status.trim().is_empty() {
                        character_changes.push(format!(
                            "{scene_prefix}: {} status - {}",
                            state_change.character_id,
                            status.trim()
                        ));
                    }
                }
            }
            if let Some(goals) = state_change.changes.goals.as_ref() {
                for goal in goals {
                    if !goal.trim().is_empty() {
                        arc_advances.push(format!(
                            "{scene_prefix}: {} goal - {}",
                            state_change.character_id,
                            goal.trim()
                        ));
                    }
                }
            }
        }

        for relationship in &package.relationship_updates {
            relationship_shifts.push(format!(
                "{scene_prefix}: {} <-> {} trust {:+}, tension {:+}: {}",
                relationship.character_a_id,
                relationship.character_b_id,
                relationship.trust_delta,
                relationship.tension_delta,
                relationship.reason.trim()
            ));
        }

        for note in &package.continuity_notes {
            if !note.trim().is_empty() {
                arc_advances.push(format!("{scene_prefix}: {}", note.trim()));
            }
        }
    }

    if key_events.is_empty() {
        key_events = scene_summaries.clone();
    }

    let summary = if scene_summaries.is_empty() {
        chapter.synopsis.clone()
    } else {
        format!("{} {}", chapter.synopsis.trim(), scene_summaries.join(" "))
            .trim()
            .to_string()
    };

    Ok(GeneratedChapterSummaryPackage {
        summary,
        key_events: authoring_dedup_preserve_order(key_events),
        character_changes: authoring_dedup_preserve_order(character_changes),
        relationship_shifts: authoring_dedup_preserve_order(relationship_shifts),
        arc_advances: authoring_dedup_preserve_order(arc_advances),
        promise_events: authoring_dedup_preserve_order(promise_events),
    })
}

pub(super) fn authoring_dedup_preserve_order(items: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for item in items {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        deduped.push(normalized);
    }
    deduped
}

pub(super) fn authoring_sample_checkpoint_scene_ids(
    state: &HarnessState,
    start_chapter: i32,
    end_chapter: i32,
) -> anyhow::Result<Vec<String>> {
    let selected_chapters = [
        start_chapter,
        start_chapter + ((end_chapter - start_chapter) / 2),
        end_chapter,
    ];
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for chapter_number in selected_chapters {
        if !seen.insert(chapter_number) {
            continue;
        }
        let chapter = state
            .chapters
            .iter()
            .find(|chapter| chapter.chapter_number == chapter_number)
            .with_context(|| format!("checkpoint chapter {} missing from state", chapter_number))?;
        let scene = if chapter_number == end_chapter {
            chapter.scenes.last()
        } else {
            chapter.scenes.first()
        }
        .with_context(|| format!("checkpoint chapter {} has no scenes", chapter_number))?;
        let scene_id = scene.scene_id.clone().with_context(|| {
            format!(
                "checkpoint chapter {} scene {} has no scene_id",
                chapter_number, scene.scene_order
            )
        })?;
        candidates.push(scene_id);
    }
    Ok(candidates)
}

/// Map a run's persisted `checkpoint_policy` to the auto policy in force, or
/// `None` for manual (the default). `None`/`Some("manual")` (never persisted —
/// manual canonicalizes to NULL at start) run the classic flow; only
/// `auto_advisory`/`auto_strict` trigger the in-process automation. The
/// scheduler is deliberately lenient so an out-of-band value never diverts the
/// checkpoint loop into the automation.
pub(super) fn auto_checkpoint_policy(policy: Option<&str>) -> Option<&'static str> {
    match policy {
        Some("auto_advisory") => Some("auto_advisory"),
        Some("auto_strict") => Some("auto_strict"),
        _ => None,
    }
}

/// Mark the pending checkpoint as an auto-block (evolution §3.3): stamp its
/// `auto_outcome` (`blocked` or `manual`) and pending-manual scene ids on the
/// in-memory state (the caller persists), and return the outcome carrying the
/// prose-free `blocked_reason`. Never mutates checkpoint status — a blocked
/// auto-checkpoint stays `pending_review` exactly like a manual one, so the
/// operator's manual escape hatch still works.
pub(super) fn auto_checkpoint_block(
    harness_state: &mut HarnessState,
    start_chapter: i32,
    end_chapter: i32,
    reason: &str,
    pending_manual_scene_ids: Vec<String>,
) -> AutoCheckpointOutcome {
    let outcome = if pending_manual_scene_ids.is_empty() {
        "blocked"
    } else {
        "manual"
    };
    if let Some(checkpoint) = harness_state
        .checkpoint_history
        .iter_mut()
        .find(|cp| cp.start_chapter == start_chapter && cp.end_chapter == end_chapter)
    {
        checkpoint.auto_outcome = Some(outcome.to_string());
        checkpoint.pending_manual_scene_ids = pending_manual_scene_ids.clone();
    }
    AutoCheckpointOutcome {
        approved: false,
        finding_counts: std::collections::BTreeMap::new(),
        blocked_reason: Some(reason.to_string()),
        pending_manual_scene_ids,
    }
}

/// Severity counts of a deep `check_consistency` output, keyed by the
/// lowercase severity word (`error` / `warning` / `info`). Uses the returned
/// `summary` (already computed by the service) so the verdict never re-walks
/// prose. Counts only — no prose.
/// Product lock: `auto_strict` blocks when any hard anti-slop finding is
/// present (`check_type == "anti_slop"`). Soft hits are not issued as
/// consistency findings, so they cannot trip this lock.
pub(super) fn auto_strict_anti_slop_blocks(
    issues: &[spindle_core::models::ConsistencyIssue],
) -> bool {
    issues
        .iter()
        .any(|issue| issue.check_type == spindle_core::style::antislop::ANTI_SLOP_CHECK)
}

pub(super) fn auto_checkpoint_severity_counts(
    output: &spindle_core::models::CheckConsistencyOutput,
) -> std::collections::BTreeMap<String, i64> {
    let mut counts = std::collections::BTreeMap::new();
    counts.insert("error".to_string(), output.summary.error_count as i64);
    counts.insert("warning".to_string(), output.summary.warning_count as i64);
    counts.insert("info".to_string(), output.summary.info_count as i64);
    counts
}

/// Read the compact per-chapter reader-sim engagement summary from a checkpoint
/// report's `reader_sim` section (evolution §3.6, R3), for `authoring_status`.
/// Returns an empty vec when the report is missing/unreadable or carries no
/// reader-sim section (manual policy, or an older report) — a best-effort read
/// that never fails status. Enums/ids only, never prose (I8).
pub(super) fn read_reader_sim_engagement(
    artifacts_root: &Path,
    report_rel: &str,
) -> Vec<spindle_core::models::ReaderSimEngagementSummary> {
    let report_path = artifacts_root.join(report_rel);
    let Ok(raw) = std::fs::read_to_string(&report_path) else {
        return Vec::new();
    };
    let Ok(report) = serde_json::from_str::<CheckpointReportArtifact>(&raw) else {
        return Vec::new();
    };
    report
        .reader_sim
        .map(|section| {
            section
                .chapters
                .into_iter()
                .map(|ch| spindle_core::models::ReaderSimEngagementSummary {
                    chapter: ch.chapter,
                    engagement: ch.engagement,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn read_checkpoint_sampled_review_summary(
    artifacts_root: &Path,
    report_rel: &str,
) -> Option<(Option<String>, Option<usize>)> {
    let report_path = artifacts_root.join(report_rel);
    let raw = std::fs::read_to_string(&report_path).ok()?;
    let report = serde_json::from_str::<CheckpointReportArtifact>(&raw).ok()?;
    Some((
        Some(report.sampled_review_status),
        Some(report.sampled_reviews.len()),
    ))
}

/// Is `error` a rating-not-covered rejection from the dispatch chokepoint
/// (evolution §4 rule 2)? The service surfaces the typed
/// `RouteClearanceError::RatingNotCovered` through anyhow, so downcast to detect
/// the explicit-manual-fallback case (I3) precisely — never string-matching an
/// error message. Names ids only; carries no prose.
pub(super) fn auto_checkpoint_error_is_rating_not_covered(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<spindle_adapters::ai::RouteClearanceError>(),
        Some(spindle_adapters::ai::RouteClearanceError::RatingNotCovered { .. })
    )
}

/// Record a deep-consistency audit into the checkpoint report artifact — the
/// exact write `authoring_record_checkpoint_audit` performs, factored out so the
/// auto-checkpoint automation records the audit through the SAME persistence
/// shape (evolution §3.3 K3.1: "call the service/handler internals, not a new
/// persistence shape"). `deep_consistency` is the serialized deep output.
pub(super) fn authoring_record_checkpoint_deep_audit(
    harness_state: &spindle_harness::state::HarnessState,
    state_path: &Path,
    start_chapter: i32,
    end_chapter: i32,
    deep_consistency: serde_json::Value,
) -> anyhow::Result<()> {
    let report_path =
        checkpoint_report_path(harness_state, state_path, start_chapter, end_chapter)?;
    let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
        format!(
            "failed to read checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
        serde_json::from_str(&raw_report).with_context(|| {
            format!(
                "failed to parse checkpoint report artifact {}",
                report_path.display()
            )
        })?;
    report.deep_consistency = Some(deep_consistency);
    report.deep_consistency_status = "complete".to_string();
    report.deep_consistency_instruction =
        "Deep consistency audit recorded by the auto-checkpoint automation (evolution §3.3)."
            .to_string();
    let report_json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&report_path, report_json).with_context(|| {
        format!(
            "failed to update checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    Ok(())
}

pub(super) fn authoring_record_checkpoint_sampled_reviews(
    harness_state: &spindle_harness::state::HarnessState,
    state_path: &Path,
    start_chapter: i32,
    end_chapter: i32,
    sampled_reviews: Vec<serde_json::Value>,
    pending_manual: &[String],
) -> anyhow::Result<()> {
    let report_path =
        checkpoint_report_path(harness_state, state_path, start_chapter, end_chapter)?;
    let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
        format!(
            "failed to read checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
        serde_json::from_str(&raw_report).with_context(|| {
            format!(
                "failed to parse checkpoint report artifact {}",
                report_path.display()
            )
        })?;
    report.sampled_reviews = sampled_reviews;
    if pending_manual.is_empty() {
        report.sampled_review_status = "complete".to_string();
        report.sampled_review_instruction =
            "Sampled dual-persona reviews recorded by the auto-checkpoint automation (evolution §3.3)."
                .to_string();
    } else {
        report.sampled_review_status = "pending_manual_review".to_string();
        report.sampled_review_instruction = format!(
            "Auto-checkpoint recorded completed sampled reviews; manual dual-persona review still required for: {}.",
            pending_manual.join(", ")
        );
    }
    let report_json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&report_path, report_json).with_context(|| {
        format!(
            "failed to update checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    Ok(())
}

pub(super) fn authoring_record_checkpoint_sampled_reviews_failed(
    harness_state: &spindle_harness::state::HarnessState,
    state_path: &Path,
    start_chapter: i32,
    end_chapter: i32,
    sampled_reviews: Vec<serde_json::Value>,
    unreviewed_scenes: &[String],
) -> anyhow::Result<()> {
    let report_path =
        checkpoint_report_path(harness_state, state_path, start_chapter, end_chapter)?;
    let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
        format!(
            "failed to read checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
        serde_json::from_str(&raw_report).with_context(|| {
            format!(
                "failed to parse checkpoint report artifact {}",
                report_path.display()
            )
        })?;
    report.sampled_reviews = sampled_reviews;
    report.sampled_review_status = "incomplete".to_string();
    report.sampled_review_instruction = format!(
        "Auto-checkpoint sampled dual-persona review failed during dispatch; unreviewed scenes: {}.",
        unreviewed_scenes.join(", ")
    );
    let report_json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&report_path, report_json).with_context(|| {
        format!(
            "failed to update checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    Ok(())
}

pub(super) fn checkpoint_report_path(
    state: &spindle_harness::state::HarnessState,
    state_path: &Path,
    start_chapter: i32,
    end_chapter: i32,
) -> anyhow::Result<PathBuf> {
    let checkpoint = state
        .checkpoint_history
        .iter()
        .find(|checkpoint| {
            checkpoint.start_chapter == start_chapter && checkpoint.end_chapter == end_chapter
        })
        .with_context(|| {
            format!(
                "checkpoint {}-{} not found in authoring run",
                start_chapter, end_chapter
            )
        })?;
    let report_artifact_path = checkpoint.report_artifact_path.clone().with_context(|| {
        format!(
            "checkpoint {}-{} has no report artifact path",
            start_chapter, end_chapter
        )
    })?;
    let artifacts_root = state_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&state.artifacts_dir);
    Ok(artifacts_root.join(report_artifact_path))
}

impl ToolRouter {
    pub(super) async fn execute_authoring_run_checkpoint(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        start_chapter: i32,
        end_chapter: i32,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let consistency = self
            .service
            .check_consistency(CheckConsistencyInput {
                deep_scan_offset: None,
                project_id: state.project_id.clone(),
                scope: ConsistencyScopeInput::chapter_range(
                    state.book_number,
                    start_chapter,
                    state.book_number,
                    end_chapter,
                ),
                checks: Vec::new(),
                severity_filter: vec![],
                deep_check: Some(false),
                subjects: vec![],
                format: None,
                budget_tokens: None,
            })
            .await
            .with_context(|| {
                format!(
                    "failed to run consistency check for chapters {start_chapter}-{end_chapter}"
                )
            })?;

        let sampled_scene_ids =
            authoring_sample_checkpoint_scene_ids(&state, start_chapter, end_chapter)?;

        let pacing_overview = self
            .service
            .read_project_resource(&state.project_id, "pacing/overview")
            .await?;
        let chapter_summaries = self
            .service
            .read_project_resource(&state.project_id, "chapter-summaries")
            .await?;
        let narrative_promises = self
            .service
            .read_project_resource(&state.project_id, "narrative-promises")
            .await?;

        let report_path = ArtifactStore::checkpoint_relative_path(start_chapter, end_chapter);
        let save_point = self
            .service
            .create_save_point(CreateSavePointInput {
                project_id: state.project_id.clone(),
                name: format!(
                    "checkpoint-b{}-ch{}-{}",
                    state.book_number, start_chapter, end_chapter
                ),
                description: Some(format!(
                    "Before editorial decision for book {} chapters {}-{}",
                    state.book_number, start_chapter, end_chapter
                )),
            })
            .await
            .with_context(|| {
                format!("failed to create save point for checkpoint {start_chapter}-{end_chapter}")
            })?;

        state.checkpoint_history.push(CheckpointRecord {
            start_chapter,
            end_chapter,
            save_point_id: save_point.save_point_id.clone(),
            status: CheckpointStatus::PendingReview,
            report_artifact_path: Some(report_path.clone()),
            auto_outcome: None,
            pending_manual_scene_ids: Vec::new(),
        });
        state.last_checkpoint_end_chapter = end_chapter;
        state.save(state_path)?;

        let sampled_review_instruction = format!(
            "Run dual-persona review for sampled scenes [{}], inspect this checkpoint report, \
             revise any fixable local craft/continuity/system-UI findings before approval, \
             then call authoring_review_checkpoint with operator directives. Do not ask \
             'revise or approve?' unless the finding requires an operator plot, canon, \
             content-boundary, relationship-direction, or author-intent decision.",
            sampled_scene_ids.join(", ")
        );
        let deep_consistency_instruction = format!(
            "Run check_consistency for project {} over book {} chapters {}-{} with deep_check=true, \
             then call authoring_record_checkpoint_audit with the returned consistency payload.",
            state.project_id, state.book_number, start_chapter, end_chapter
        );

        artifact_store.save_json(
            &report_path,
            &CheckpointReportArtifact {
                version: 1,
                start_chapter,
                end_chapter,
                save_point: save_point.clone(),
                consistency: serde_json::to_value(consistency)?,
                deep_consistency: None,
                deep_consistency_status: "pending_deep_consistency".to_string(),
                deep_consistency_instruction: deep_consistency_instruction.clone(),
                sampled_reviews: Vec::new(),
                sampled_review_status: "pending_dual_persona_review".to_string(),
                sampled_review_instruction: sampled_review_instruction.clone(),
                pacing_overview,
                chapter_summaries,
                narrative_promises,
                sampled_scene_ids,
                // Reader-sim runs later, inside the auto-checkpoint automation
                // (evolution §3.6); the report is created with no section.
                reader_sim: None,
            },
        )?;

        Ok(ExecutionResult {
            state,
            message: format!(
                "Created checkpoint for chapters {start_chapter}-{end_chapter}; awaiting deep consistency and sampled dual-persona review ({}) before operator checkpoint review. {} {}",
                save_point.save_point_id, deep_consistency_instruction, sampled_review_instruction
            ),
        })
    }

    pub(super) async fn handle_authoring_record_checkpoint_audit(
        &self,
        input: AuthoringRecordCheckpointAuditInput,
    ) -> anyhow::Result<AuthoringRecordCheckpointAuditOutput> {
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
        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, &run_id);

        let report_path = checkpoint_report_path(
            &harness_state,
            &state_path,
            input.start_chapter,
            input.end_chapter,
        )?;
        let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
            format!(
                "failed to read checkpoint report artifact {}",
                report_path.display()
            )
        })?;
        let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
            serde_json::from_str(&raw_report).with_context(|| {
                format!(
                    "failed to parse checkpoint report artifact {}",
                    report_path.display()
                )
            })?;

        report.deep_consistency = Some(input.deep_consistency);
        report.deep_consistency_status = "complete".to_string();
        report.deep_consistency_instruction =
            "Deep consistency audit recorded from a separate check_consistency(deep_check=true) call.".to_string();

        let report_json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&report_path, report_json).with_context(|| {
            format!(
                "failed to update checkpoint report artifact {}",
                report_path.display()
            )
        })?;

        Ok(AuthoringRecordCheckpointAuditOutput {
            run_id,
            message: format!(
                "Recorded deep consistency audit for checkpoint {}-{}.",
                input.start_chapter, input.end_chapter
            ),
            status: run.status,
        })
    }

    pub(super) async fn handle_authoring_review_checkpoint(
        &self,
        input: AuthoringReviewCheckpointInput,
    ) -> anyhow::Result<AuthoringReviewCheckpointOutput> {
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

        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, &run_id);
        let report_path = checkpoint_report_path(
            &harness_state,
            &state_path,
            input.start_chapter,
            input.end_chapter,
        )?;
        let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
            format!(
                "failed to read checkpoint report artifact {}",
                report_path.display()
            )
        })?;
        let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
            serde_json::from_str(&raw_report).with_context(|| {
                format!(
                    "failed to parse checkpoint report artifact {}",
                    report_path.display()
                )
            })?;

        if report.deep_consistency_status == "pending_deep_consistency"
            || report.deep_consistency_status == "pending"
        {
            anyhow::bail!(
                "checkpoint {}-{} cannot be marked reviewed until deep consistency is recorded. \
                 Run check_consistency with deep_check=true for this chapter range, then call \
                 authoring_record_checkpoint_audit with the returned payload.",
                input.start_chapter,
                input.end_chapter
            );
        }

        let mut sampled_reviews = Vec::new();
        let mut missing_reviews = Vec::new();
        for scene_id in &report.sampled_scene_ids {
            match repo
                .get_dual_persona_review(&harness_state.active_branch_id, scene_id)
                .await?
            {
                Some(review) if review.status == "current" && review.rounds_completed >= 2 => {
                    sampled_reviews.push(serde_json::to_value(review)?);
                }
                Some(review) => missing_reviews.push(format!(
                    "{} (status={}, rounds={})",
                    scene_id, review.status, review.rounds_completed
                )),
                None => missing_reviews.push(format!("{scene_id} (missing)")),
            }
        }
        if !missing_reviews.is_empty() {
            anyhow::bail!(
                "checkpoint {}-{} cannot be marked reviewed until sampled dual-persona reviews are current: {}. \
                 Run run_dual_persona_review with rounds=2 for each sampled scene in the checkpoint report, \
                 then call authoring_review_checkpoint again.",
                input.start_chapter,
                input.end_chapter,
                missing_reviews.join(", ")
            );
        }
        if !report.sampled_scene_ids.is_empty() {
            report.sampled_reviews = sampled_reviews;
            report.sampled_review_status = "complete".to_string();
            report.sampled_review_instruction =
                "Sampled dual-persona reviews verified from the local Spindle database before operator checkpoint review.".to_string();
            let report_json = serde_json::to_string_pretty(&report)?;
            std::fs::write(&report_path, report_json).with_context(|| {
                format!(
                    "failed to update checkpoint report artifact {}",
                    report_path.display()
                )
            })?;
        }

        if let Some(unresolved) = authoring_unresolved_checkpoint_directive(&input.directives) {
            anyhow::bail!(
                "checkpoint {}-{} cannot be marked reviewed because a directive appears to leave fixable findings unresolved: {:?}. \
                 Apply the prose/canon fixes first, then call authoring_review_checkpoint with resolved directives.",
                input.start_chapter,
                input.end_chapter,
                unresolved
            );
        }

        harness_state.save(&state_path)?;

        let review_result = spindle_harness::operator::review_checkpoint(
            &mut harness_state,
            &state_path,
            input.start_chapter,
            input.end_chapter,
            &input.directives,
        );

        let _ = std::fs::remove_file(&state_path);

        let message = review_result?;
        let final_status = authoring_status_after_checkpoint_review(&harness_state);

        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, final_status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2): the operator review completed and persisted. Emit
        // `checkpoint_reviewed`, then the actual run-status transition it caused
        // (diff prev vs final): review of the final checkpoint drives the run
        // straight to `completed`; a mid-run checkpoint review unblocks it to
        // `active` (`run_resumed`). Emitted AFTER the save commits (D3.3).
        let journal = RunJournal::new(repo);
        journal
            .emit(
                &run_id,
                "checkpoint_reviewed",
                run_journal::checkpoint_reviewed_payload(
                    input.start_chapter,
                    input.end_chapter,
                    input.directives.len(),
                ),
            )
            .await;
        if run.status != final_status {
            match final_status {
                "completed" => {
                    journal
                        .emit(
                            &run_id,
                            "run_completed",
                            run_journal::run_status_payload(None),
                        )
                        .await;
                }
                "active" if run.status == "blocked" || run.status == "paused" => {
                    journal
                        .emit(
                            &run_id,
                            "run_resumed",
                            run_journal::run_status_payload(None),
                        )
                        .await;
                }
                _ => {}
            }
        }

        Ok(AuthoringReviewCheckpointOutput {
            run_id,
            message,
            status: final_status.to_string(),
        })
    }

    /// In-process auto-checkpoint automation (evolution §3.3, K3). Invoked from
    /// `handle_authoring_execute_next` when a run whose `checkpoint_policy` is
    /// `auto_advisory`/`auto_strict` reaches a pending checkpoint, INSTEAD of
    /// surfacing `AwaitCheckpointReview`. This is pure orchestration of the
    /// existing manual-checkpoint service paths — it introduces NO new review
    /// logic:
    ///
    /// 1. Deep consistency: `check_consistency(deep_check=true)` over the
    ///    checkpoint's chapter range (full default check set, as the manual flow
    ///    instructs operators), recorded into the report artifact the SAME way
    ///    `authoring_record_checkpoint_audit` records it.
    /// 2. Sampled dual-persona reviews (rounds=2) via `run_dual_persona_review`,
    ///    the exact service path the manual flow uses. Explicit-manual-fallback
    ///    (I3): a scene whose review dispatch fails `RatingNotCovered` (surfaced
    ///    from the chokepoint, NOT pre-checked) is marked pending-manual and is
    ///    dispatched NOWHERE else; if any scene is pending-manual the checkpoint
    ///    BLOCKS listing exactly those scenes — the rest of the automation still
    ///    completed and is recorded.
    /// 3. Verdict from the deep-consistency severity counts: `auto_advisory`
    ///    approves iff zero findings ≥ `warning` (info allowed); `auto_strict`
    ///    iff zero findings of ANY severity. Approve takes the SAME path
    ///    `authoring_review_checkpoint` takes (`operator::review_checkpoint` with
    ///    an honest auto-approval directive). Not approved → the checkpoint stays
    ///    pending_review exactly as manual, with a prose-free `blocked_reason`.
    /// 4. Journal (ADR D2): `checkpoint_auto_approved` on approval,
    ///    `checkpoint_blocked` on block — via the existing RunJournal.
    ///
    /// Cost note: there is NO per-checkpoint model-cost ceiling in v1 (open
    /// design per owner decision D-1); every sampled scene's review and the deep
    /// pass run unbudgeted. A future ADR may add a ceiling.
    ///
    /// Transport-level errors during automation (deep-check call fails, a review
    /// call fails for a NON-clearance reason) never fake completion (I8) and
    /// never crash the run: the checkpoint stays pending_review with a
    /// `blocked_reason` naming the failed step, returned as `AutoCheckpointOutcome`.
    pub(super) async fn run_auto_checkpoint(
        &self,
        run_id: &str,
        policy: &str,
        harness_state: &mut HarnessState,
        start_chapter: i32,
        end_chapter: i32,
    ) -> anyhow::Result<AutoCheckpointOutcome> {
        let repo = self.service.repository();
        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, run_id);

        // ── Step 1: deep consistency over the checkpoint range ──
        // Any transport error here blocks the checkpoint honestly (I8) — the
        // automation never fabricates a clean audit.
        let deep = match self
            .service
            .check_consistency(CheckConsistencyInput {
                deep_scan_offset: None,
                project_id: harness_state.project_id.clone(),
                scope: ConsistencyScopeInput::chapter_range(
                    harness_state.book_number,
                    start_chapter,
                    harness_state.book_number,
                    end_chapter,
                ),
                checks: Vec::new(),
                severity_filter: Vec::new(),
                deep_check: Some(true),
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
        {
            Ok(output) => output,
            Err(error) => {
                tracing::warn!(
                    run_id,
                    step = "deep_consistency",
                    error = format!("{error:#}"),
                    "auto-checkpoint deep consistency failed; blocking checkpoint (I8)"
                );
                return Ok(auto_checkpoint_block(
                    harness_state,
                    start_chapter,
                    end_chapter,
                    "deep_consistency_failed",
                    Vec::new(),
                ));
            }
        };

        // Record the deep audit into the report artifact — the same shape
        // `handle_authoring_record_checkpoint_audit` writes.
        let deep_value = serde_json::to_value(&deep)?;
        let finding_counts = auto_checkpoint_severity_counts(&deep);
        if let Err(error) = authoring_record_checkpoint_deep_audit(
            harness_state,
            &state_path,
            start_chapter,
            end_chapter,
            deep_value,
        ) {
            tracing::warn!(
                run_id,
                step = "record_audit",
                error = format!("{error:#}"),
                "auto-checkpoint audit record failed; blocking checkpoint (I8)"
            );
            return Ok(auto_checkpoint_block(
                harness_state,
                start_chapter,
                end_chapter,
                "record_audit_failed",
                Vec::new(),
            ));
        }

        // ── Step 2: sampled dual-persona reviews (rounds=2) ──
        let sampled_scene_ids =
            authoring_sample_checkpoint_scene_ids(harness_state, start_chapter, end_chapter)?;
        let mut sampled_reviews: Vec<serde_json::Value> = Vec::new();
        let mut pending_manual: Vec<String> = Vec::new();
        for (idx, scene_id) in sampled_scene_ids.iter().enumerate() {
            match self
                .service
                .run_dual_persona_review(spindle_core::models::RunDualPersonaReviewInput {
                    project_id: harness_state.project_id.clone(),
                    branch_id: Some(harness_state.active_branch_id.clone()),
                    scene_id: scene_id.clone(),
                    rounds: Some(2),
                })
                .await
            {
                Ok(review_output) => {
                    let review_value = match self
                        .service
                        .repository()
                        .get_dual_persona_review(&harness_state.active_branch_id, scene_id)
                        .await?
                    {
                        Some(stored) => serde_json::to_value(stored)?,
                        None => serde_json::to_value(&review_output)?,
                    };
                    sampled_reviews.push(review_value);
                }
                Err(error) => {
                    // Explicit-manual-fallback (I3): a RatingNotCovered rejection
                    // from the dispatch chokepoint means the `review` route's
                    // agent is not cleared for THIS scene's rating. That scene's
                    // prose was never dispatched anywhere (the chokepoint rejects
                    // before any model call); mark it pending-manual and DO NOT
                    // route it elsewhere. Any OTHER error is a transport failure:
                    // block the checkpoint naming the failed step (I8), never
                    // faking completion.
                    if auto_checkpoint_error_is_rating_not_covered(&error) {
                        pending_manual.push(scene_id.clone());
                    } else {
                        let mut unreviewed_scenes = pending_manual.clone();
                        unreviewed_scenes.extend(sampled_scene_ids[idx..].iter().cloned());
                        let _ = authoring_record_checkpoint_sampled_reviews_failed(
                            harness_state,
                            &state_path,
                            start_chapter,
                            end_chapter,
                            sampled_reviews,
                            &unreviewed_scenes,
                        );
                        tracing::warn!(
                            run_id,
                            step = "dual_persona_review",
                            scene_id,
                            error = format!("{error:#}"),
                            "auto-checkpoint review failed at transport; blocking checkpoint (I8)"
                        );
                        return Ok(auto_checkpoint_block(
                            harness_state,
                            start_chapter,
                            end_chapter,
                            "review_dispatch_failed",
                            pending_manual,
                        ));
                    }
                }
            }
        }
        if let Err(error) = authoring_record_checkpoint_sampled_reviews(
            harness_state,
            &state_path,
            start_chapter,
            end_chapter,
            sampled_reviews,
            &pending_manual,
        ) {
            tracing::warn!(
                run_id,
                step = "record_sampled_reviews",
                error = format!("{error:#}"),
                "auto-checkpoint sampled review record failed; blocking checkpoint (I8)"
            );
            return Ok(auto_checkpoint_block(
                harness_state,
                start_chapter,
                end_chapter,
                "record_sampled_reviews_failed",
                pending_manual,
            ));
        }

        // ── Step 2.5: cumulative reader-simulation pass (evolution §3.6) ──
        // Enrichment, not a gate: it reads each chapter in the range in order
        // with rolling memory, writes a `reader_sim` section into the report and
        // a per-run rolling notes artifact, and NEVER alters the verdict (its
        // concerns are report-only, matching the sampled-review outcomes, which
        // also never fold into the finding counts). A transport error or an
        // uncleared rating skips that chapter honestly and the pass continues;
        // reader-sim skips never mark scenes pending-manual. It runs BEFORE the
        // pending-manual early-return so its enrichment section is always
        // recorded even when a sampled review fell back to manual (a
        // pending-manual block still short-circuits the verdict below). Box::pin
        // keeps this grown path off the auto-checkpoint stack frame
        // (decide_canon_deltas precedent).
        if let Err(error) = Box::pin(self.run_reader_sim_pass(
            run_id,
            &state_path,
            harness_state,
            start_chapter,
            end_chapter,
        ))
        .await
        {
            // A failure to WRITE the report/notes artifact is not a leak and is
            // not a verdict input; log and continue so reader-sim enrichment can
            // never itself block or fabricate a checkpoint outcome (I8).
            tracing::warn!(
                run_id,
                step = "reader_sim",
                error = format!("{error:#}"),
                "auto-checkpoint reader-sim pass failed to record; continuing (enrichment, not a gate)"
            );
        }

        // Any scene awaiting manual review blocks the checkpoint (partial
        // automation, zero leakage): the completed reviews + deep audit are
        // recorded, but the checkpoint stays pending_review listing exactly the
        // scenes the operator must review by hand.
        if !pending_manual.is_empty() {
            return Ok(auto_checkpoint_block(
                harness_state,
                start_chapter,
                end_chapter,
                "pending_manual_review",
                pending_manual,
            ));
        }

        // ── Step 3: verdict from the deep-consistency severity counts ──
        let errors = *finding_counts.get("error").unwrap_or(&0);
        let warnings = *finding_counts.get("warning").unwrap_or(&0);
        let infos = *finding_counts.get("info").unwrap_or(&0);
        let mut approve = match policy {
            "auto_advisory" => errors == 0 && warnings == 0,
            "auto_strict" => errors == 0 && warnings == 0 && infos == 0,
            // Defensive: an unknown policy never auto-approves.
            _ => false,
        };
        // Product lock: auto_strict requires anti_slop.hard_count == 0 even if
        // a later mapping changes hard shelves off warning. Soft-only never
        // invents a hard finding (verify_findings emits hard only).
        if policy == "auto_strict" && auto_strict_anti_slop_blocks(&deep.issues) {
            approve = false;
        }

        if approve {
            // Approve via the SAME path authoring_review_checkpoint takes: an
            // honest auto-approval directive, then operator::review_checkpoint.
            let directives = vec![format!("auto-approved under {policy}: 0 blocking findings")];
            harness_state.save(&state_path)?;
            let review_result = spindle_harness::operator::review_checkpoint(
                harness_state,
                &state_path,
                start_chapter,
                end_chapter,
                &directives,
            );
            let _ = std::fs::remove_file(&state_path);
            review_result?;
            if let Some(checkpoint) = harness_state
                .checkpoint_history
                .iter_mut()
                .find(|cp| cp.start_chapter == start_chapter && cp.end_chapter == end_chapter)
            {
                checkpoint.auto_outcome = Some("approved".to_string());
            }
            Ok(AutoCheckpointOutcome {
                approved: true,
                finding_counts,
                blocked_reason: None,
                pending_manual_scene_ids: Vec::new(),
            })
        } else {
            let _ = std::fs::remove_file(&state_path);
            // Blocked by findings: stay pending_review exactly as manual, with a
            // prose-free severity-count summary (ids/counts, no prose).
            let reason = format!(
                "auto_{policy_tail}_findings: errors={errors} warnings={warnings} info={infos}",
                policy_tail = policy.strip_prefix("auto_").unwrap_or(policy),
            );
            Ok(auto_checkpoint_block(
                harness_state,
                start_chapter,
                end_chapter,
                &reason,
                Vec::new(),
            ))
        }
    }

    /// Cumulative reader-simulation pass over a checkpoint range (evolution
    /// §3.6, P3.4). For each chapter start_chapter..=end_chapter IN ORDER, gather
    /// its scenes in spine order, derive the batch rating (the strictest across
    /// the chapter's scenes — `max_scene_rating`), load the reader's prior notes
    /// (rolling memory, updated BETWEEN chapters so memory accumulates chapter to
    /// chapter within this checkpoint), and call the service reader-sim pass
    /// (route `reader_sim` → `review` fallback, rating-gated). The rolling notes
    /// artifact (`reader-sim-notes.json`, one per run) is updated after each
    /// chapter; a `reader_sim` section is written into the checkpoint report.
    ///
    /// Enrichment, not a gate (I8): a skip (uncleared rating / transport error /
    /// no prose) or an unparsed read records an honest entry and the pass
    /// continues; reader-sim never marks scenes pending-manual and never alters
    /// the verdict.
    pub(super) async fn run_reader_sim_pass(
        &self,
        run_id: &str,
        state_path: &Path,
        harness_state: &HarnessState,
        start_chapter: i32,
        end_chapter: i32,
    ) -> anyhow::Result<()> {
        use spindle_harness::artifacts::{
            CheckpointReaderSimChapter, CheckpointReaderSimSection, READER_SIM_NOTES_PATH,
            ReaderSimConcernEntry, ReaderSimHistoryEntry, ReaderSimNotesArtifact,
        };

        let artifacts_root = authoring_artifacts_root(state_path, harness_state);
        let artifact_store = ArtifactStore::new(artifacts_root);

        // Load (or start) the run's rolling reader-sim notes artifact.
        let mut notes_artifact = artifact_store
            .load_json::<ReaderSimNotesArtifact>(READER_SIM_NOTES_PATH)
            .unwrap_or_default();

        let mut report_chapters: Vec<CheckpointReaderSimChapter> = Vec::new();

        for chapter_number in start_chapter..=end_chapter {
            let Some(_chapter) = harness_state
                .chapters
                .iter()
                .find(|ch| ch.chapter_number == chapter_number)
            else {
                continue;
            };

            let read = self
                .service
                .read_episode(spindle_core::models::ReadEpisodeInput {
                    project_id: harness_state.project_id.clone(),
                    book_number: harness_state.book_number,
                    chapter_number,
                    branch_id: Some(harness_state.active_branch_id.clone()),
                    force: false,
                })
                .await?;
            let outcome = read.outcome;

            let concerns_count = outcome.concerns.len();
            let skipped_reason = if outcome.status == "skipped" {
                outcome.skip_reason.clone()
            } else {
                None
            };

            // Update the rolling notes artifact after THIS chapter so the next
            // chapter's prompt carries the accumulated memory. On a read, adopt
            // the model's replacement notes and advance the watermark; on an
            // unparsed read or a skip, keep the prior notes (the reader loses no
            // memory) and do NOT advance the watermark.
            if outcome.status == "read" {
                notes_artifact.notes = outcome.notes.clone();
                notes_artifact.updated_through_chapter = chapter_number;
            }
            notes_artifact.history.push(ReaderSimHistoryEntry {
                range: format!("{chapter_number}..{chapter_number}"),
                engagement: outcome.engagement.clone(),
                concerns_count,
            });
            // Persist the rolling artifact each chapter (crash-safe memory).
            artifact_store.save_json(READER_SIM_NOTES_PATH, &notes_artifact)?;

            report_chapters.push(CheckpointReaderSimChapter {
                memory: Some(read.memory),
                chapter: chapter_number,
                engagement: outcome.engagement.clone(),
                concerns: outcome
                    .concerns
                    .iter()
                    .map(|c| ReaderSimConcernEntry {
                        severity: c.severity.clone(),
                        description: c.description.clone(),
                    })
                    .collect(),
                skipped_reason,
            });

            tracing::debug!(
                run_id,
                chapter = chapter_number,
                engagement = outcome.engagement.as_str(),
                status = outcome.status.as_str(),
                concerns = concerns_count,
                "reader-sim chapter recorded"
            );
        }

        // Fold the section into the checkpoint report (additive; report-only).
        let report_path =
            checkpoint_report_path(harness_state, state_path, start_chapter, end_chapter)?;
        let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
            format!(
                "failed to read checkpoint report artifact {}",
                report_path.display()
            )
        })?;
        let mut report: CheckpointReportArtifact =
            serde_json::from_str(&raw_report).with_context(|| {
                format!(
                    "failed to parse checkpoint report artifact {}",
                    report_path.display()
                )
            })?;
        report.reader_sim = Some(CheckpointReaderSimSection {
            chapters: report_chapters,
            notes_artifact_path: READER_SIM_NOTES_PATH.to_string(),
        });
        let report_json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&report_path, report_json).with_context(|| {
            format!(
                "failed to update checkpoint report artifact {}",
                report_path.display()
            )
        })?;

        Ok(())
    }
}
