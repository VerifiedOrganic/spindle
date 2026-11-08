use super::*;

impl SqliteSpindleService {
    /// Aggregate the writer's current state on a project/branch: cursor scene,
    /// hard constraints, subject snapshots, recent scenes, due promises,
    /// active system overlays, outlines, and recent session activity. Pure
    /// orchestration over `Repository` reads plus the writer-state helpers in
    /// [`crate::format`].
    ///
    /// Per-project main branches (Phase 6 reconciliation): the SurrealDB
    /// reference compared against the global `bible_branch:main` singleton.
    /// Here, the "fall back to main" semantics resolve per project — the
    /// project's branch named "main" is the fallback target. When that branch
    /// IS the requested branch, no fallback happens.
    ///
    /// `unsynced_local_files` and `drift_warnings` are populated by walking
    /// `list_scene_source_links_by_project` and running each link through
    /// `SourceBridge::evaluate_scene_divergence` — same semantics as the
    /// SurrealDB reference. Risk #7 (see migration plan) is now closed.
    pub async fn get_writer_state(
        &self,
        input: spindle_core::models::GetWriterStateInput,
    ) -> Result<spindle_core::models::WriterState> {
        use crate::format::{
            self, DEFAULT_WRITER_STATE_BUDGET_TOKENS, DEFAULT_WRITER_STATE_RECENT_ACTIVITY_LIMIT,
            DEFAULT_WRITER_STATE_RECENT_SCENE_LIMIT, DEFAULT_WRITER_STATE_SUGGESTED_SUBJECT_LIMIT,
        };
        use serde_json::json;
        use spindle_core::models::{
            BookSummary, ChapterPositionSummary, ContextBundleSummary, ContextFormat,
            HardConstraint, OutlineRef, OverlayWithTrajectory, ProjectSummary, Provenance,
            RelationshipSummary, ScenePositionSummary, SessionActivitySummary, SubjectRef,
            WriterIntent, WriterState, WriterStateCurrent, WriterStateNarrativePromiseSummary,
            WriterStateNext, WriterStateSubjectSnapshot,
        };

        let format_fmt = input.format.unwrap_or(ContextFormat::Markdown);
        let token_budget = input
            .budget_tokens
            .unwrap_or(DEFAULT_WRITER_STATE_BUDGET_TOKENS);
        let include_subjects = input.include_subjects.unwrap_or(true);
        let include_recent_activity = input.include_recent_activity.unwrap_or(true);
        let recent_activity_limit = input
            .recent_activity_limit
            .unwrap_or(DEFAULT_WRITER_STATE_RECENT_ACTIVITY_LIMIT);

        let project = self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&project.id).await?;

        let branch = match input.branch_id.as_deref() {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(project.id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        project.id
                    );
                }
                branch
            }
            None => active_branch.clone(),
        };
        let branch_id = branch.id.clone();

        let writer_position = self
            .repository
            .get_writer_position(&project.id, &branch_id)
            .await?;

        let scenes = self
            .repository
            .list_scenes_resolved_for_branch(&project.id, &branch_id)
            .await?;

        let cursor_scene = match input.at_scene_id.as_deref() {
            Some(scene_id) => {
                let scene = self.repository.get_scene(scene_id).await?;
                if scene.project_id != project.id
                    || !scenes.iter().any(|candidate| candidate.id == scene.id)
                {
                    anyhow::bail!(
                        "scene {scene_id} does not belong to the requested project branch"
                    );
                }
                Some(scene)
            }
            None => {
                if let Some(position_scene_id) =
                    writer_position.as_ref().and_then(|p| p.scene_id.clone())
                {
                    match self.repository.get_scene(&position_scene_id).await {
                        Ok(scene)
                            if scene.project_id == project.id
                                && (scene.branch_id == branch_id
                                    || scenes.iter().any(|candidate| candidate.id == scene.id)) =>
                        {
                            Some(scene)
                        }
                        _ => scenes.last().cloned(),
                    }
                } else {
                    scenes.last().cloned()
                }
            }
        };
        let cursor_index = cursor_scene
            .as_ref()
            .and_then(|scene| scenes.iter().position(|candidate| candidate.id == scene.id));
        let next_scene = cursor_index
            .and_then(|index| scenes.get(index + 1))
            .cloned();

        let mut book_summary = None;
        let mut chapter_summary = None;
        let mut scene_summary = None;
        let mut last_completed_scene_summary = None;
        if let Some(scene) = cursor_scene.as_ref() {
            let book = self.repository.get_book(&scene.book_id).await?;
            let chapter = self.repository.get_chapter(&scene.chapter_id).await?;
            book_summary = Some(BookSummary {
                book_id: book.id.clone(),
                book_number: book.book_number,
                title: book.title.clone(),
            });
            chapter_summary = Some(ChapterPositionSummary {
                chapter_id: chapter.id.clone(),
                book_number: chapter.book_number,
                chapter_number: chapter.chapter_number,
                title: chapter.title.clone(),
            });
            scene_summary = Some(ScenePositionSummary {
                scene_id: scene.id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                summary: scene.summary.clone(),
                tone: scene.tone.clone(),
            });
            last_completed_scene_summary = cursor_index
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| scenes.get(index))
                .map(|s| s.summary.clone());
        }
        if book_summary.is_none()
            && let Some(position_book_id) =
                writer_position.as_ref().and_then(|p| p.book_id.as_ref())
            && let Ok(book) = self.repository.get_book(position_book_id).await
            && book.project_id == project.id
        {
            book_summary = Some(BookSummary {
                book_id: book.id.clone(),
                book_number: book.book_number,
                title: book.title.clone(),
            });
        }
        if chapter_summary.is_none()
            && let Some(position_chapter_id) =
                writer_position.as_ref().and_then(|p| p.chapter_id.as_ref())
            && let Ok(chapter) = self.repository.get_chapter(position_chapter_id).await
            && chapter.project_id == project.id
        {
            chapter_summary = Some(ChapterPositionSummary {
                chapter_id: chapter.id.clone(),
                book_number: chapter.book_number,
                chapter_number: chapter.chapter_number,
                title: chapter.title.clone(),
            });
        }
        if scene_summary.is_none()
            && let Some(position_scene_id) =
                writer_position.as_ref().and_then(|p| p.scene_id.as_ref())
            && let Ok(scene) = self.repository.get_scene(position_scene_id).await
            && scene.project_id == project.id
            && (scene.branch_id == branch_id
                || scenes.iter().any(|candidate| candidate.id == scene.id))
        {
            scene_summary = Some(ScenePositionSummary {
                scene_id: scene.id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                summary: scene.summary.clone(),
                tone: scene.tone.clone(),
            });
        }

        let world_rules = self
            .repository
            .list_world_rules_by_project_and_branch(&project.id, &branch_id)
            .await?;
        let mut hard_constraints = world_rules
            .into_iter()
            .map(|rule| HardConstraint {
                id: rule.rule_name,
                statement: rule.description,
            })
            .collect::<Vec<_>>();

        // Economy price facts: surface current named prices in the re-anchor
        // packet so the writer (and the authoring loop) stay consistent with the
        // economy. No-op for projects with no economies / no price facts.
        {
            use spindle_core::models::StoryPlacement;
            use spindle_core::subject::{Subject, SubjectTable};

            let economy_placement = cursor_scene
                .as_ref()
                .map(|scene| StoryPlacement {
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: Some(scene.scene_order),
                    note: None,
                })
                .unwrap_or(StoryPlacement {
                    book_number: i32::MAX,
                    chapter_number: i32::MAX,
                    scene_order: Some(i32::MAX),
                    note: None,
                });
            let economy_subjects: Vec<Subject> = self
                .repository
                .list_economies_by_project_and_branch(&project.id, &branch_id)
                .await?
                .into_iter()
                .filter(|economy| economy.archived_at.is_none())
                .filter_map(|economy| Subject::new(SubjectTable::Economy, economy.id).ok())
                .collect();
            if !economy_subjects.is_empty() {
                let economy_facts = self
                    .repository
                    .list_canonical_facts_for_subjects(
                        &project.id,
                        &branch_id,
                        &economy_subjects,
                        &economy_placement,
                    )
                    .await?;
                hard_constraints.extend(
                    economy_facts
                        .iter()
                        .map(format::canonical_fact_hard_constraint),
                );
            }
        }

        let characters = self
            .repository
            .list_characters_by_project_and_branch(&project.id, &branch_id)
            .await?;
        let character_ids: Vec<String> = characters.iter().map(|c| c.id.clone()).collect();
        let relationships = if character_ids.is_empty() {
            Vec::new()
        } else {
            self.repository
                .list_relationships_for_characters(&branch_id, &character_ids)
                .await?
        };

        let subjects = if include_subjects {
            let cursor_position = cursor_scene
                .as_ref()
                .map(|scene| (scene.book_number, scene.chapter_number, scene.scene_order));
            let mut subject_snapshots = Vec::new();
            for character in &characters {
                let state =
                    if let Some((book_number, chapter_number, scene_order)) = cursor_position {
                        self.repository
                            .resolve_character_state_for_branch(
                                &branch_id,
                                &character.id,
                                book_number,
                                chapter_number,
                                scene_order + 1,
                            )
                            .await
                            .ok()
                            .flatten()
                    } else {
                        None
                    };
                let relationship_summaries = relationships
                    .iter()
                    .filter(|rel| rel.in_id == character.id || rel.out_id == character.id)
                    .map(|rel| RelationshipSummary {
                        // The SQLite RelatesTo has no surrogate id (composite
                        // primary key (in, out, branch_id)). Build a stable
                        // synthetic id so callers can refer to the edge.
                        relationship_id: format!(
                            "relationship:{}:{}:{}",
                            rel.in_id, rel.out_id, rel.branch_id
                        ),
                        source_character_id: rel.in_id.clone(),
                        target_character_id: rel.out_id.clone(),
                        relationship_type: rel.relationship_type.clone(),
                        trust: rel.trust,
                        tension: rel.tension,
                        dynamics: rel.dynamics.clone(),
                    })
                    .collect::<Vec<_>>();
                subject_snapshots.push(WriterStateSubjectSnapshot {
                    subject: SubjectRef {
                        subject_id: character.id.clone(),
                        kind: "character".to_string(),
                        name: character.name.clone(),
                    },
                    summary: character.summary.clone(),
                    role: Some(character.role.clone()),
                    status: state.map(|s| s.status).unwrap_or_default(),
                    relationships: relationship_summaries,
                });
            }
            subject_snapshots
        } else {
            Vec::new()
        };

        let suggested_subjects = subjects
            .iter()
            .take(DEFAULT_WRITER_STATE_SUGGESTED_SUBJECT_LIMIT)
            .map(|s| s.subject.clone())
            .collect::<Vec<_>>();
        let persisted_next_focus = writer_position.as_ref().and_then(|p| p.next_focus.clone());
        let next = WriterStateNext {
            intended_focus: persisted_next_focus.or_else(|| {
                next_scene
                    .as_ref()
                    .map(|scene| {
                        format!(
                            "Draft Book {} Chapter {} Scene {}: {}",
                            scene.book_number,
                            scene.chapter_number,
                            scene.scene_order,
                            scene.summary
                        )
                    })
                    .or_else(|| {
                        cursor_scene.as_ref().map(|scene| {
                            format!(
                                "Continue Book {} Chapter {} from Scene {}",
                                scene.book_number, scene.chapter_number, scene.scene_order
                            )
                        })
                    })
            }),
            outline_section_ref: next_scene.as_ref().map(|scene| OutlineRef {
                chapter_id: Some(scene.chapter_id.clone()),
                scene_order: Some(scene.scene_order),
                label: format!(
                    "book {} chapter {} scene {}",
                    scene.book_number, scene.chapter_number, scene.scene_order
                ),
            }),
            suggested_subjects,
        };

        let recent_scenes = if let Some(scene) = cursor_scene.as_ref() {
            let mut recent = self
                .repository
                .list_recent_scenes_by_project_and_branch(
                    &project.id,
                    &branch_id,
                    scene.book_number,
                    scene.chapter_number,
                    scene.scene_order + 1,
                    DEFAULT_WRITER_STATE_RECENT_SCENE_LIMIT,
                )
                .await?;
            if recent
                .last()
                .map(|candidate| candidate.id != scene.id)
                .unwrap_or(true)
            {
                recent.push(scene.clone());
            }
            recent
                .into_iter()
                .rev()
                .take(DEFAULT_WRITER_STATE_RECENT_SCENE_LIMIT)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(format::writer_state_recent_scene_summary)
                .collect()
        } else {
            Vec::new()
        };

        let open_promises_due_now = if let Some(scene) = cursor_scene.as_ref() {
            self.repository
                .list_narrative_promises_by_project_and_branch(&project.id, &branch_id)
                .await?
                .into_iter()
                .filter(|promise| promise.status != "paid_off")
                .filter(|promise| {
                    format::story_index_from_placement(&promise.planted_at)
                        <= format::story_index(
                            scene.book_number,
                            scene.chapter_number,
                            scene.scene_order,
                        )
                })
                .map(|promise| WriterStateNarrativePromiseSummary {
                    narrative_promise_id: promise.id,
                    promise_type: promise.promise_type,
                    description: promise.description,
                    status: promise.status,
                    planted_at: promise.planted_at.into_core(),
                    planned_payoff: promise.planned_payoff.map(|p| p.into_core()),
                    notes: promise.notes,
                })
                .collect()
        } else {
            Vec::new()
        };

        let active_overlays = self
            .repository
            .list_system_overlays_by_project_and_branch(&project.id, &branch_id)
            .await?
            .into_iter()
            .map(|overlay| OverlayWithTrajectory {
                overlay_id: overlay.id.clone(),
                name: overlay.system_name.clone(),
                current_value: json!({
                    "system_type": overlay.system_type,
                    "visibility": overlay.visibility,
                    "progression_currency": overlay.progression_currency,
                    "stats": overlay.stats,
                    "advancement_tiers": overlay.advancement_tiers,
                    "rules": overlay.rules,
                }),
                trajectory_delta_since_last_chapter: json!({}),
                recent_events: Vec::new(),
                provenance: Provenance {
                    source: "system_overlay".to_string(),
                    updated_at: Some(overlay.updated_at.to_rfc3339()),
                },
            })
            .collect::<Vec<_>>();

        // Risk #7 (closed): walk scene_source_link rows for any scene in
        // the cursor scene's chapter and emit divergence entries against
        // the on-disk file. Mirrors services/mod.rs:8466 in 705b835^.
        let unsynced_local_files = if let Some(scene) = cursor_scene.as_ref() {
            use spindle_core::models::UnsyncedFileEntry;
            let links = self
                .repository
                .list_scene_source_links_by_project(&project.id)
                .await?;
            let chapter_scene_map: std::collections::BTreeMap<
                String,
                &crate::sqlite::records::Scene,
            > = scenes
                .iter()
                .filter(|candidate| {
                    candidate.book_number == scene.book_number
                        && candidate.chapter_number == scene.chapter_number
                })
                .map(|s| (s.id.clone(), s))
                .collect();
            links
                .into_iter()
                .filter_map(|link| {
                    let scene = chapter_scene_map.get(&link.scene_id)?;
                    let observation =
                        crate::sqlite::source_bridge::evaluate_scene_divergence(&link, scene)?;
                    Some(UnsyncedFileEntry {
                        scene_id: scene.id.clone(),
                        source_path: link.source_path,
                        kind: observation.kind,
                        detail: observation.detail,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let drift_warnings = unsynced_local_files
            .iter()
            .map(|entry| spindle_core::models::DriftWarning {
                code: "source_divergence".to_string(),
                message: format!("{}: {}", entry.source_path, entry.detail),
            })
            .collect::<Vec<_>>();

        let recent_session_activity = if include_recent_activity {
            self.repository
                .list_recent_session_activity(&project.id, &branch_id, recent_activity_limit as i64)
                .await?
                .into_iter()
                .map(|row| SessionActivitySummary {
                    kind: row.kind,
                    subject_table: row.subject_table,
                    subject_id: row.subject_id,
                    summary: row.summary,
                    created_at: row.created_at.to_rfc3339(),
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let chapter_outline = if let Some(scene) = cursor_scene.as_ref() {
            self.repository
                .get_chapter_outline(&scene.chapter_id, &branch_id)
                .await?
                .map(|outline| spindle_core::models::ChapterOutline {
                    chapter_id: outline.chapter_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    beats: outline.beats.into_iter().map(|b| b.into_core()).collect(),
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };
        let book_outline = if let Some(scene) = cursor_scene.as_ref() {
            self.repository
                .get_book_outline(&scene.book_id, &branch_id)
                .await?
                .map(|outline| spindle_core::models::BookOutline {
                    book_id: outline.book_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };
        let has_chapter_outline = chapter_outline.is_some();
        let has_book_outline = book_outline.is_some();

        let current = WriterStateCurrent {
            project: ProjectSummary {
                project_id: project.id.clone(),
                name: project.name,
                project_type: project.project_type,
                genre: project.genre,
                active_style_profile_id: project.active_style_profile_id.clone(),
            },
            branch: format::branch_summary(&branch, Some(active_branch.id.as_str())),
            book: book_summary,
            chapter: chapter_summary,
            scene: scene_summary,
            last_completed_scene_summary,
            intent: writer_position
                .as_ref()
                .map(|p| format::parse_writer_intent(&p.intent))
                .unwrap_or_else(|| {
                    if cursor_scene.is_some() {
                        WriterIntent::Drafting
                    } else {
                        WriterIntent::Idle
                    }
                }),
        };

        let mut state = WriterState {
            current,
            next,
            hard_constraints,
            subjects,
            recent_scenes,
            open_promises_due_now,
            active_overlays,
            drift_warnings,
            unsynced_local_files,
            recent_session_activity,
            chapter_outline,
            book_outline,
            bundle_summary: ContextBundleSummary {
                format: format_fmt,
                estimated_tokens: 0,
                token_budget: Some(token_budget),
                truncated: false,
                included_sections: format::writer_state_included_sections(
                    include_subjects,
                    include_recent_activity,
                    has_chapter_outline,
                    has_book_outline,
                ),
            },
        };

        format::enforce_writer_state_budget(format_fmt, token_budget, &mut state)?;

        Ok(state)
    }

    /// Build the public `get_writer_state` tool envelope while keeping
    /// markdown rendering with the application service instead of the MCP
    /// transport adapter.
    pub async fn get_writer_state_envelope(
        &self,
        input: spindle_core::models::GetWriterStateInput,
    ) -> Result<spindle_core::models::WriterStateEnvelope> {
        let format = input
            .format
            .unwrap_or(spindle_core::models::ContextFormat::Markdown);
        let payload = self.get_writer_state(input).await?;
        let writer_state_markdown = (format == spindle_core::models::ContextFormat::Markdown)
            .then(|| crate::format::format_writer_state_markdown(&payload));

        Ok(spindle_core::models::WriterStateEnvelope {
            current: payload.current,
            next: payload.next,
            hard_constraints: payload.hard_constraints,
            subjects: payload.subjects,
            recent_scenes: payload.recent_scenes,
            open_promises_due_now: payload.open_promises_due_now,
            active_overlays: payload.active_overlays,
            drift_warnings: payload.drift_warnings,
            unsynced_local_files: payload.unsynced_local_files,
            recent_session_activity: payload.recent_session_activity,
            chapter_outline: payload.chapter_outline,
            book_outline: payload.book_outline,
            bundle_summary: payload.bundle_summary,
            writer_state_markdown,
        })
    }

    /// Assemble a budget-aware chapter briefing for the requested chapter,
    /// folding in the scene-context slice for the first/requested scene
    /// when enough fields are pinned down. Mirrors services/mod.rs:
    /// 9524..9890 in 705b835^. Pure projections live in `crate::format::*`;
    /// only the per-call DB orchestration (continuity sheets, scene-context
    /// callback, branch lookup) lives here.
    ///
    /// Build the pre-draft in-world-time hard constraint for a scene position:
    /// the previous scene's end clock (the expected start for this scene), this
    /// scene's own clock and elapsed gap when already stamped, and the grounding
    /// instruction to signal elapsed time. `None` when the project declares no
    /// calendar or no nearby clock carries a day. Shared by `get_scene_context`
    /// and `get_chapter_briefing` so both drafting surfaces agree.
    pub(super) async fn temporal_anchor_constraint(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<spindle_core::models::HardConstraint>> {
        use crate::format::story_index;
        use spindle_core::models::HardConstraint;

        let Some(project_calendar) = self.repository.get_project_calendar(project_id).await? else {
            return Ok(None);
        };
        let cursor_index = story_index(book_number, chapter_number, scene_order);
        let current_clock = self
            .repository
            .latest_scene_clock_at_or_before(project_id, branch_id, cursor_index)
            .await?;
        let prior_clock = if cursor_index > 0 {
            self.repository
                .latest_scene_clock_at_or_before(project_id, branch_id, cursor_index - 1)
                .await?
        } else {
            None
        };
        // Resolve the previous scene's location name (the spatial half of the
        // anchor). Best-effort: a missing scene/location degrades to a clock-only
        // statement rather than failing context assembly.
        let prior_location = match prior_clock.as_ref() {
            Some(prior) => {
                let location_id = self
                    .repository
                    .get_scene(&prior.scene_id)
                    .await
                    .ok()
                    .and_then(|scene| scene.location_id);
                match location_id {
                    Some(id) => self.repository.get_location(&id).await.ok().map(|l| l.name),
                    None => None,
                }
            }
            None => None,
        };
        Ok(temporal_anchor_statement(
            &project_calendar.calendar,
            prior_clock.as_ref(),
            current_clock.as_ref(),
            prior_location.as_deref(),
        )
        .map(|statement| HardConstraint {
            id: "[IN-WORLD TIME]".to_string(),
            statement,
        }))
    }

    /// Divergences from the reference:
    ///   * The bundled scene-context slice inherits the same
    ///     semantic-references / explicit-draft constraint gaps already
    ///     called out on `get_scene_context`.
    pub async fn get_chapter_briefing(
        &self,
        input: spindle_core::models::GetChapterBriefingInput,
    ) -> Result<spindle_core::models::GetChapterBriefingOutput> {
        use crate::format::{
            CHAPTER_BRIEFING_SCENE_CONTEXT_SECTIONS, DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS,
            DEFAULT_CHAPTER_BRIEFING_RECENT_LIMIT, MAX_CHAPTER_BRIEFING_RECENT_LIMIT,
            apply_chapter_briefing_bundle_trims, build_chapter_briefing_bundle,
            canonical_fact_hard_constraint, canonical_fact_read_model,
            fit_chapter_briefing_hard_constraints, format_chapter_briefing_markdown,
            is_hard_constraint_budget_error, recent_chapter_summaries_for_briefing,
            truncate_markdown_at_line_boundary,
        };
        use spindle_core::context_bundle::estimate_text_tokens;
        use spindle_core::models::{
            CanonicalFactReadModel, ChapterBriefingSceneSeed, ChapterPlanBriefing,
            ChapterSummaryBriefing, ContextFormat, GetChapterBriefingOutput, GetSceneContextInput,
            HardConstraint, StoryPlacement,
        };
        use spindle_core::subject::{Subject, SubjectTable};

        let format_fmt = input.format.unwrap_or(ContextFormat::Markdown);
        let budget_tokens = input
            .budget_tokens
            .or(input.token_budget)
            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS);

        self.repository.get_project(&input.project_id).await?;

        let recent_limit = input
            .recent_chapter_limit
            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_RECENT_LIMIT)
            .min(MAX_CHAPTER_BRIEFING_RECENT_LIMIT);
        let recent_chapter_summaries = recent_chapter_summaries_for_briefing(
            self.repository
                .list_chapter_summaries_by_project(&input.project_id)
                .await?,
            input.book_number,
            input.chapter_number,
            recent_limit,
        )
        .into_iter()
        .map(|summary| ChapterSummaryBriefing {
            book_number: summary.book_number,
            chapter_number: summary.chapter_number,
            summary: summary.summary,
            key_events: summary.key_events,
            character_changes: summary.character_changes,
            relationship_shifts: summary.relationship_shifts,
            arc_advances: summary.arc_advances,
            promise_events: summary.promise_events,
        })
        .collect::<Vec<_>>();

        let chapter_plan = self
            .repository
            .list_chapter_plans_by_project(&input.project_id)
            .await?
            .into_iter()
            .find(|plan| {
                plan.book_number == input.book_number && plan.chapter_number == input.chapter_number
            })
            .map(|plan| ChapterPlanBriefing {
                synopsis: plan.synopsis,
                pov_character_id: plan.pov_character_id,
                target_theme_ids: plan.target_theme_ids,
                target_conflict_ids: plan.target_conflict_ids,
                target_plot_line_ids: plan.target_plot_line_ids,
                scenes: plan
                    .scenes
                    .into_iter()
                    .map(|scene| scene.into_core())
                    .collect(),
            });

        let active_branch = self.repository.get_active_branch(&input.project_id).await?;

        // Hydrate the plan-targeted narrative threads (themes, conflicts, plot
        // lines, theme-connected motifs) at the briefing top level, mirroring
        // the scene-context novel layer. Empty when the chapter has no plan.
        let mut active_threads = if let Some(plan) = chapter_plan.as_ref() {
            self.assemble_active_threads(
                &input.project_id,
                &active_branch.id,
                &plan.target_theme_ids,
                &plan.target_conflict_ids,
                &plan.target_plot_line_ids,
            )
            .await?
        } else {
            Vec::new()
        };

        active_threads.extend(
            self.editorial_threads(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
            )
            .await?,
        );

        let chapter_scene_orders = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?
            .into_iter()
            .filter(|scene| {
                scene.book_number == input.book_number
                    && scene.chapter_number == input.chapter_number
            })
            .map(|scene| scene.scene_order)
            .collect::<Vec<_>>();

        let resolved_scene_order = input
            .scene_order
            .or_else(|| {
                chapter_plan
                    .as_ref()
                    .and_then(|plan| plan.scenes.first().map(|scene| scene.scene_order))
            })
            .or_else(|| chapter_scene_orders.first().copied());

        let resolved_character_ids = if !input.character_ids.is_empty() {
            input.character_ids.clone()
        } else if let Some(scene_order) = resolved_scene_order {
            chapter_plan
                .as_ref()
                .and_then(|plan| {
                    plan.scenes
                        .iter()
                        .find(|scene| scene.scene_order == scene_order)
                        .map(|scene| scene.character_ids.clone())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let resolved_location_id = input.location_id.clone();

        let mut missing_fields = Vec::new();
        if resolved_scene_order.is_none() {
            missing_fields.push("scene_order".to_string());
        }
        if resolved_character_ids.is_empty() {
            missing_fields.push("character_ids".to_string());
        }
        if resolved_location_id.is_none() {
            missing_fields.push("location_id".to_string());
        }

        let scene_context = if missing_fields.is_empty() {
            match self
                .get_scene_context(GetSceneContextInput {
                    project_id: input.project_id.clone(),
                    book_number: input.book_number,
                    chapter_number: input.chapter_number,
                    chapter_id: None,
                    scene_order: resolved_scene_order.expect("checked above"),
                    character_ids: resolved_character_ids.clone(),
                    max_character_count: None,
                    location_id: resolved_location_id.clone().expect("checked above"),
                    format: Some(ContextFormat::Json),
                    budget_tokens: Some(
                        input
                            .budget_tokens
                            .or(input.token_budget)
                            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS),
                    ),
                    token_budget: Some(
                        input
                            .budget_tokens
                            .or(input.token_budget)
                            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS),
                    ),
                    sections: Some(
                        CHAPTER_BRIEFING_SCENE_CONTEXT_SECTIONS
                            .iter()
                            .map(|section| section.to_string())
                            .collect(),
                    ),
                })
                .await
            {
                Ok(scene_context) => Some(scene_context),
                Err(error) if is_hard_constraint_budget_error(&error) => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };

        let scene_seed = ChapterBriefingSceneSeed {
            scene_order: resolved_scene_order,
            character_ids: resolved_character_ids.clone(),
            location_id: resolved_location_id.clone(),
            missing_fields,
            scene_context_available: scene_context.is_some(),
        };

        // Continuity-sheet subjects: POV + requested characters + location.
        let continuity_sheets = {
            let placement = StoryPlacement {
                book_number: input.book_number,
                chapter_number: input.chapter_number,
                scene_order: resolved_scene_order,
                note: None,
            };
            let mut seen = std::collections::BTreeSet::new();
            let mut subjects: Vec<Subject> = Vec::new();
            let mut push_subject = |table: SubjectTable, subject_id: String| -> Result<()> {
                let key = format!("{}:{subject_id}", table.as_str());
                if seen.insert(key) {
                    subjects.push(
                        Subject::new(table, subject_id)
                            .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                    );
                }
                Ok(())
            };

            if let Some(pov_character_id) = chapter_plan
                .as_ref()
                .and_then(|plan| plan.pov_character_id.clone())
            {
                push_subject(SubjectTable::Character, pov_character_id)?;
            }
            for character_id in &resolved_character_ids {
                push_subject(SubjectTable::Character, character_id.clone())?;
            }
            if let Some(location_id) = resolved_location_id.as_deref() {
                push_subject(SubjectTable::Location, location_id.to_string())?;
            }

            if subjects.is_empty() {
                Vec::new()
            } else {
                self.repository
                    .assemble_subject_snapshots(
                        &input.project_id,
                        &active_branch.id,
                        &subjects,
                        &placement,
                    )
                    .await?
            }
        };

        let chapter_record = self
            .repository
            .get_chapter_by_number(&input.project_id, input.book_number, input.chapter_number)
            .await
            .ok();
        let book_record = self
            .repository
            .get_book_by_number(&input.project_id, input.book_number)
            .await
            .ok();
        let chapter_outline = if let Some(chapter) = chapter_record.as_ref() {
            self.repository
                .get_chapter_outline(&chapter.id, &active_branch.id)
                .await?
                .map(|outline| spindle_core::models::ChapterOutline {
                    chapter_id: outline.chapter_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    beats: outline
                        .beats
                        .into_iter()
                        .map(|beat| beat.into_core())
                        .collect(),
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };
        let book_outline = if let Some(book) = book_record.as_ref() {
            self.repository
                .get_book_outline(&book.id, &active_branch.id)
                .await?
                .map(|outline| spindle_core::models::BookOutline {
                    book_id: outline.book_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };

        let hard_constraints_from_scene_context: Option<Vec<HardConstraint>> = scene_context
            .as_ref()
            .map(|ctx| ctx.hard_constraints.clone());
        let canonical_facts_from_scene_context: Option<Vec<CanonicalFactReadModel>> = scene_context
            .as_ref()
            .map(|ctx| ctx.canonical_facts.clone());

        let (hard_constraints, canonical_facts): (
            Vec<HardConstraint>,
            Vec<CanonicalFactReadModel>,
        ) = match (
            hard_constraints_from_scene_context,
            canonical_facts_from_scene_context,
        ) {
            (Some(constraints), Some(canonical_facts)) => (constraints, canonical_facts),
            _ => {
                let rules = self
                    .repository
                    .list_world_rules_by_project_and_branch(&input.project_id, &active_branch.id)
                    .await?;
                let placement = StoryPlacement {
                    book_number: input.book_number,
                    chapter_number: input.chapter_number,
                    scene_order: resolved_scene_order,
                    note: None,
                };
                let project_wide_facts = self
                    .repository
                    .list_canonical_facts_for_project_wide(
                        &input.project_id,
                        &active_branch.id,
                        &placement,
                    )
                    .await?;
                let canonical_fact_models = project_wide_facts
                    .iter()
                    .map(canonical_fact_read_model)
                    .collect::<Vec<_>>();
                let constraints = rules
                    .into_iter()
                    .map(|rule| HardConstraint {
                        id: rule.rule_name,
                        statement: rule.description,
                    })
                    .chain(
                        project_wide_facts
                            .iter()
                            .map(canonical_fact_hard_constraint),
                    )
                    .collect::<Vec<_>>();
                (constraints, canonical_fact_models)
            }
        };

        // Ensure the temporal anchor reaches the briefing even on the fallback
        // path (no bundled scene context — e.g. no characters resolved), where
        // hard_constraints are rebuilt from world rules + facts only and would
        // otherwise drop it. The inherited path already carries it, so only add
        // when absent.
        let mut hard_constraints = hard_constraints;
        if !hard_constraints
            .iter()
            .any(|constraint| constraint.id.contains("IN-WORLD TIME"))
            && let Some(constraint) = self
                .temporal_anchor_constraint(
                    &input.project_id,
                    &active_branch.id,
                    input.book_number,
                    input.chapter_number,
                    resolved_scene_order.unwrap_or(1),
                )
                .await?
        {
            hard_constraints.insert(0, constraint);
        }

        let (hard_constraints, hard_constraints_compacted) = fit_chapter_briefing_hard_constraints(
            format_fmt,
            budget_tokens,
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &hard_constraints,
        )?;

        let briefing_markdown = format_chapter_briefing_markdown(
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &hard_constraints,
            &continuity_sheets,
            &recent_chapter_summaries,
            chapter_outline.as_ref(),
            book_outline.as_ref(),
            chapter_plan.as_ref(),
            &active_threads,
            scene_context.as_ref(),
            &scene_seed,
        );
        let compact_shelf_digest = scene_context
            .as_ref()
            .and_then(|context| context.compact_shelf_digest.clone());
        let mut output = GetChapterBriefingOutput {
            hard_constraints: hard_constraints.clone(),
            canonical_facts: if hard_constraints_compacted {
                Vec::new()
            } else {
                canonical_facts
            },
            continuity_sheets,
            briefing_markdown,
            recent_chapter_summaries,
            chapter_outline,
            book_outline,
            chapter_plan,
            active_threads,
            scene_seed,
            scene_context,
            compact_shelf_digest,
        };

        let mut bundle = build_chapter_briefing_bundle(
            format_fmt,
            budget_tokens,
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &output.hard_constraints,
            &output.canonical_facts,
            &output.continuity_sheets,
            &output.recent_chapter_summaries,
            output.chapter_outline.as_ref(),
            output.book_outline.as_ref(),
            output.chapter_plan.as_ref(),
            &output.active_threads,
            output.scene_context.as_ref(),
            &output.scene_seed,
        );
        let budget_report = bundle
            .enforce_budget()
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        apply_chapter_briefing_bundle_trims(
            &budget_report.truncated_section_ids,
            &mut output.canonical_facts,
            &mut output.continuity_sheets,
            &mut output.recent_chapter_summaries,
            &mut output.chapter_outline,
            &mut output.book_outline,
            &mut output.chapter_plan,
            &mut output.active_threads,
            &mut output.scene_context,
        );
        output.briefing_markdown = format_chapter_briefing_markdown(
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &output.hard_constraints,
            &output.continuity_sheets,
            &output.recent_chapter_summaries,
            output.chapter_outline.as_ref(),
            output.book_outline.as_ref(),
            output.chapter_plan.as_ref(),
            &output.active_threads,
            output.scene_context.as_ref(),
            &output.scene_seed,
        );
        if format_fmt == ContextFormat::Markdown
            && estimate_text_tokens(&output.briefing_markdown) > budget_tokens
        {
            // Final fallback: keep markdown whole-line boundaries instead
            // of cutting headers mid-line when the trim loop still exceeds
            // budget.
            output.briefing_markdown =
                truncate_markdown_at_line_boundary(&output.briefing_markdown, budget_tokens);
        }

        Ok(output)
    }

    /// Hydrate the narrative threads a chapter plan explicitly targets into
    /// [`ActiveThreadSummary`] rows so the drafting agent sees the themes,
    /// conflicts, plot lines, and theme-connected motifs it is meant to advance.
    ///
    /// Rules (T-102): stale/nonexistent and archived ids are skipped silently;
    /// each statement is truncated char-boundary-safe; at most five entries per
    /// kind survive; ordering is deterministic — kind order
    /// theme→conflict→plot_line→motif, then id ascending within kind.
    pub(super) async fn assemble_active_threads(
        &self,
        project_id: &str,
        branch_id: &str,
        target_theme_ids: &[String],
        target_conflict_ids: &[String],
        target_plot_line_ids: &[String],
    ) -> Result<Vec<spindle_core::models::ActiveThreadSummary>> {
        use crate::format::truncate_active_thread_statement;
        use spindle_core::models::ActiveThreadSummary;

        const PER_KIND_CAP: usize = 5;

        let mut threads: Vec<ActiveThreadSummary> = Vec::new();

        // Themes: name/label doubles as the statement (the entity has no
        // separate name field); status renders the introduction/resolution
        // placement pair when either is set.
        let mut themes: Vec<ActiveThreadSummary> = Vec::new();
        for id in target_theme_ids {
            let Some(theme) = self.repository.get_theme(id).await.ok() else {
                continue;
            };
            if theme.archived_at.is_some() {
                continue;
            }
            let status = active_thread_placement_status(
                theme.introduction_point.as_ref().map(|p| p.chapter_number),
                theme.resolution_point.as_ref().map(|p| p.chapter_number),
            );
            themes.push(ActiveThreadSummary {
                id: theme.id,
                kind: "theme".to_string(),
                name: theme.theme_statement.clone(),
                statement: truncate_active_thread_statement(&theme.theme_statement),
                status,
                next_expectation: None,
            });
        }
        finalize_active_thread_kind(&mut themes, PER_KIND_CAP);
        threads.extend(themes);

        // Conflicts: statement = stakes; status = conflict_type; next
        // expectation = the first undelivered stated consequence.
        let mut conflicts: Vec<ActiveThreadSummary> = Vec::new();
        for id in target_conflict_ids {
            let Some(conflict) = self.repository.get_conflict(id).await.ok() else {
                continue;
            };
            if conflict.archived_at.is_some() {
                continue;
            }
            let next_expectation = conflict
                .stated_consequences
                .iter()
                .find(|consequence| !consequence.delivered)
                .map(|consequence| consequence.description.clone());
            conflicts.push(ActiveThreadSummary {
                id: conflict.id,
                kind: "conflict".to_string(),
                name: conflict.name.clone(),
                statement: truncate_active_thread_statement(&conflict.stakes),
                status: conflict.conflict_type.clone(),
                next_expectation,
            });
        }
        finalize_active_thread_kind(&mut conflicts, PER_KIND_CAP);
        threads.extend(conflicts);

        // Plot lines: statement = summary; status = the plot line's status.
        let mut plot_lines: Vec<ActiveThreadSummary> = Vec::new();
        for id in target_plot_line_ids {
            let Some(plot_line) = self.repository.get_plot_line(id).await.ok() else {
                continue;
            };
            if plot_line.archived_at.is_some() {
                continue;
            }
            plot_lines.push(ActiveThreadSummary {
                id: plot_line.id,
                kind: "plot_line".to_string(),
                name: plot_line.name.clone(),
                statement: truncate_active_thread_statement(&plot_line.summary),
                status: plot_line.status.clone(),
                next_expectation: None,
            });
        }
        finalize_active_thread_kind(&mut plot_lines, PER_KIND_CAP);
        threads.extend(plot_lines);

        // Motifs: theme-connected only — any motif whose connected_theme_ids
        // intersect the plan's target themes. statement = description; next
        // expectation = the per-chapter usage cap when one is set.
        if !target_theme_ids.is_empty() {
            let target_set: std::collections::HashSet<&str> =
                target_theme_ids.iter().map(String::as_str).collect();
            let mut motifs: Vec<ActiveThreadSummary> = self
                .repository
                .list_motifs_by_project_and_branch(project_id, branch_id)
                .await?
                .into_iter()
                .filter(|motif| motif.archived_at.is_none())
                .filter(|motif| {
                    motif
                        .connected_theme_ids
                        .iter()
                        .any(|theme_id| target_set.contains(theme_id.as_str()))
                })
                .map(|motif| {
                    let next_expectation = motif
                        .max_uses_per_chapter
                        .map(|n| format!("use sparingly: ≤{n} per chapter"));
                    ActiveThreadSummary {
                        id: motif.id,
                        kind: "motif".to_string(),
                        name: motif.name.clone(),
                        statement: truncate_active_thread_statement(&motif.description),
                        status: String::new(),
                        next_expectation,
                    }
                })
                .collect();
            finalize_active_thread_kind(&mut motifs, PER_KIND_CAP);
            threads.extend(motifs);
        }

        Ok(threads)
    }

    /// Assemble a budget-aware scene-context bundle for the requested scene
    /// position. Mirrors services/mod.rs:8612..9134 in 705b835^. Validates
    /// project + chapter ownership, derives the scene's location / characters
    /// / relationships / pacing / agency directly off the SQLite repository,
    /// then projects the result through the bundle helpers in
    /// `crate::format::*` to enforce `budget_tokens` (Markdown or JSON).
    ///
    /// Divergences from the reference:
    ///   * Semantic recall (`semantic_references`) is now ported: the present
    ///     cast + location form a query that is embedded and kNN-searched over
    ///     the project's embedding mirror (the same path as `search_bible`),
    ///     excluding the present-scene entities already rendered in the scene
    ///     layer. Subjects are still gathered from POV + character_ids +
    ///     location.
    ///   * No explicit-draft hard constraint is appended (the SQLite service
    ///     does not own a `ModelRouter`); the reference appended one when
    ///     the `draft` route had an explicit-rating override.
    pub(super) async fn assemble_fiction_writing_packet(
        &self,
        project_id: &str,
        active_style_profile_id: Option<&str>,
        style_notes: &[String],
        style_rules: &[spindle_core::style::StyleRule],
    ) -> (
        Option<spindle_core::style::antislop::CompactShelfDigest>,
        Vec<spindle_core::style::antislop::VoiceSample>,
        Vec<spindle_core::style::antislop::SceneNegative>,
    ) {
        let Ok(pack) = spindle_core::style::antislop::ShelfPack::load_default() else {
            return (None, Vec::new(), Vec::new());
        };
        let profile = match active_style_profile_id {
            Some(profile_id) => self
                .repository
                .get_style_profile(project_id, profile_id)
                .await
                .ok()
                .flatten(),
            None => None,
        };
        let mut style_texts = style_notes.to_vec();
        for rule in style_rules {
            style_texts.push(rule.rule_name.clone());
            style_texts.push(rule.description.clone());
        }
        let (digest, hooks) = spindle_core::style::antislop::assemble_writing_packet(
            &pack,
            active_style_profile_id,
            &style_texts,
            profile.as_ref().map(|card| &card.guidance),
        );
        (Some(digest), hooks.voice_samples, hooks.scene_negatives)
    }

    pub async fn get_scene_context(
        &self,
        input: spindle_core::models::GetSceneContextInput,
    ) -> Result<spindle_core::models::SceneContextOutput> {
        Ok(self
            .get_scene_context_for_branch(input, None, false)
            .await?
            .output)
    }

    pub(crate) async fn get_scene_context_for_branch(
        &self,
        input: spindle_core::models::GetSceneContextInput,
        branch_id_override: Option<&str>,
        for_prepare_scene: bool,
    ) -> Result<SceneContextWithCoverage> {
        use crate::format::{
            DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS, SCENE_CONTEXT_HARD_CONSTRAINT_HEADROOM_TOKENS,
            WorldRuleContextCharacter, agency_check_from_scene_history,
            apply_scene_context_bundle_trims, build_scene_context_bundle,
            canonical_fact_hard_constraint, canonical_fact_read_model, economy_summary,
            empty_agency_check_summary, empty_location_summary, empty_reader_contract,
            empty_world_state_summary, estimate_scene_context_tokens, filter_relevant_world_rules,
            future_knowledge_briefing_item, future_knowledge_summary, knowledge_fact_briefing_item,
            narrative_promise_due_summary, non_truncatable_prefix_tokens_scene_context,
            pacing_directives_for_characters, story_index, story_index_from_placement,
            system_overlay_summary, timeline_event_summary_at_or_before,
        };
        use spindle_core::models::{
            CharacterStateSummary, ContextFormat, HardConstraint, LocationSummary,
            RelationshipSummary, SceneContextBudgetMeta, SceneContextNovelLayer,
            SceneContextOutput, SceneContextSceneLayer, StoryPlacement, WorldRuleSummary,
            WorldStateSummary,
        };
        use spindle_core::subject::{Subject, SubjectTable};

        let format_fmt = input.format.unwrap_or(ContextFormat::Markdown);
        let mut budget_tokens = input
            .budget_tokens
            .or(input.token_budget)
            .unwrap_or(DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS);

        let want_reader_contract = input.wants_novel_section("reader_contract");
        let want_style_directive = input.wants_novel_section("style_directive");
        let want_world_rules = input.wants_novel_section("world_rules");
        let want_system_overlays = input.wants_novel_section("system_overlays");
        let want_timeline_briefing = input.wants_novel_section("timeline_briefing");
        let want_future_knowledge_briefing = input.wants_novel_section("future_knowledge_briefing");
        let want_pacing_directives = input.wants_novel_section("pacing_directives");
        let want_narrative_promises_due = input.wants_novel_section("narrative_promises_due");
        let want_knowledge_briefing = input.wants_novel_section("knowledge_briefing");
        let want_semantic_references = input.wants_novel_section("semantic_references");
        let want_economy_briefing = input.wants_novel_section("economy_briefing");
        let want_subjects = input.wants_novel_section("subjects");
        let want_previous_scene_tail = input.wants_novel_section("previous_scene_tail");

        let want_location = input.wants_scene_section("location");
        let want_world_state = input.wants_scene_section("world_state");
        let want_characters = input.wants_scene_section("characters");
        let want_relationships = input.wants_scene_section("relationships");
        let want_agency_check = input.wants_scene_section("agency_check");

        let project = self.repository.get_project(&input.project_id).await?;
        let active_branch = match branch_id_override
            .map(str::trim)
            .filter(|branch_id| !branch_id.is_empty())
        {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(input.project_id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        input.project_id
                    );
                }
                branch
            }
            None => self.repository.get_active_branch(&input.project_id).await?,
        };

        // Reconcile chapter_id vs book_number/chapter_number.
        let mut normalized = input.clone();
        if let Some(chapter_id) = input.chapter_id.as_deref() {
            let chapter = self.repository.get_chapter(chapter_id).await?;
            if chapter.project_id != input.project_id {
                anyhow::bail!("chapter does not belong to the requested project");
            }
            if input.book_number > 0 && input.book_number != chapter.book_number {
                anyhow::bail!(
                    "chapter_id {} does not match book_number {}",
                    chapter_id,
                    input.book_number
                );
            }
            if input.chapter_number > 0 && input.chapter_number != chapter.chapter_number {
                anyhow::bail!(
                    "chapter_id {} does not match chapter_number {}",
                    chapter_id,
                    input.chapter_number
                );
            }
            normalized.book_number = chapter.book_number;
            normalized.chapter_number = chapter.chapter_number;
        } else if input.book_number <= 0 || input.chapter_number <= 0 {
            anyhow::bail!(
                "get_scene_context requires chapter_id or both book_number and chapter_number"
            );
        }
        let input = normalized;

        let location = self.repository.get_location(&input.location_id).await?;
        if location.project_id != input.project_id {
            anyhow::bail!("location does not belong to the requested project");
        }

        let world_state = if want_world_state {
            self.repository
                .get_world_state_for_location(&active_branch.id, &input.location_id)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        let system_overlays = if want_system_overlays {
            self.repository
                .list_system_overlays_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
        } else {
            Vec::new()
        };

        let timeline_briefing = if want_timeline_briefing {
            self.repository
                .list_timeline_events_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter_map(|event| {
                    timeline_event_summary_at_or_before(
                        event,
                        input.book_number,
                        input.chapter_number,
                        input.scene_order,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        // Economies in play: surfaced as a briefing section (lore) and, more
        // importantly, added to the canonical-fact subject set below so any
        // economy-scoped price facts ride the never-trimmed hard-constraint path.
        let economies = self
            .repository
            .list_economies_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?
            .into_iter()
            .filter(|economy| economy.archived_at.is_none())
            .collect::<Vec<_>>();

        let economy_briefing = if want_economy_briefing {
            economies
                .iter()
                .cloned()
                .map(economy_summary)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        // Validate every requested character belongs to the project.
        let need_resolved_characters =
            want_characters || want_agency_check || want_semantic_references;
        let mut validated_characters = Vec::new();
        for character_id in &input.character_ids {
            let character = self.repository.get_character(character_id).await?;
            if character.project_id != input.project_id {
                anyhow::bail!(
                    "character {} does not belong to the requested project",
                    character.name
                );
            }
            validated_characters.push(character);
        }
        let included_characters = if let Some(max_character_count) = input.max_character_count {
            validated_characters
                .into_iter()
                .take(max_character_count)
                .collect::<Vec<_>>()
        } else {
            validated_characters
        };

        let ids = included_characters
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>();
        let world_rule_context_characters = included_characters
            .iter()
            .map(|character| WorldRuleContextCharacter {
                name: character.name.clone(),
                role: character.role.clone(),
                summary: character.summary.clone(),
            })
            .collect::<Vec<_>>();

        let mut resolved_characters: Vec<CharacterStateSummary> = Vec::new();
        if need_resolved_characters {
            for character in &included_characters {
                let state = self
                    .repository
                    .resolve_character_state_for_branch(
                        &active_branch.id,
                        &character.id,
                        input.book_number,
                        input.chapter_number,
                        input.scene_order,
                    )
                    .await?;
                resolved_characters.push(CharacterStateSummary {
                    character_id: character.id.clone(),
                    name: character.name.clone(),
                    summary: character.summary.clone(),
                    role: character.role.clone(),
                    emotional_state: state
                        .as_ref()
                        .map(|s| s.emotional_state.clone())
                        .unwrap_or_default(),
                    goals: state.as_ref().map(|s| s.goals.clone()).unwrap_or_default(),
                    status: state.as_ref().map(|s| s.status.clone()).unwrap_or_default(),
                    notes: state.as_ref().map(|s| s.notes.clone()).unwrap_or_default(),
                });
            }
        }

        let all_world_rules = self
            .repository
            .list_world_rules_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;

        // Style-typed world rules are prose-level directives, not lore. Pull
        // them regardless of relevance filtering so they always reach the
        // style directive (the brief's "untitled / buried" failure mode).
        let style_rules: Vec<spindle_core::style::StyleRule> = all_world_rules
            .iter()
            .filter(|rule| rule.rule_type.eq_ignore_ascii_case("style"))
            .map(|rule| spindle_core::style::StyleRule {
                rule_name: rule.rule_name.clone(),
                description: rule.description.clone(),
            })
            .collect();

        // Single source of truth for genre-voice enforcement: reader contract
        // + style world rules + narrator voice. Read by scene context, the
        // save gate, the style_compliance validator, and the review persona.
        let style_directive = spindle_core::style::StyleDirective::assemble(
            project.genre.clone(),
            project.project_type.clone(),
            project.reader_contract.promise.clone(),
            project.reader_contract.style_notes.clone(),
            project.reader_contract.boundaries.clone(),
            style_rules,
            project
                .narrator_voice
                .clone()
                .map(|stored| stored.into_core()),
        );

        let relevant_world_rules = filter_relevant_world_rules(
            &all_world_rules,
            &location,
            &world_rule_context_characters,
        );

        let relationships = if want_relationships && !ids.is_empty() {
            self.repository
                .list_relationships_for_characters(&active_branch.id, &ids)
                .await?
        } else {
            Vec::new()
        };

        // Story-position cursor for this scene; used to gate time-sensitive
        // knowledge so the drafting model never sees facts a POV character has
        // not reached yet.
        let cursor = story_index(input.book_number, input.chapter_number, input.scene_order);
        let raw_future_knowledge = if (want_future_knowledge_briefing || want_knowledge_briefing)
            && !ids.is_empty()
        {
            self.repository
                .list_future_knowledge_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|knowledge| ids.contains(&knowledge.character_id))
                // Position gate: only surface future knowledge the character has
                // actually reached (learned_at <= cursor) and that has not expired
                // (expires_at >= cursor), so the generator is not primed with
                // premature reveals across long arcs.
                .filter(|knowledge| {
                    story_index_from_placement(&knowledge.learned_at) <= cursor
                        && knowledge
                            .expires_at
                            .as_ref()
                            .is_none_or(|expires| story_index_from_placement(expires) >= cursor)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let future_knowledge_briefing = if want_future_knowledge_briefing {
            raw_future_knowledge
                .iter()
                .map(future_knowledge_summary)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let pacing_directives = if want_pacing_directives && !ids.is_empty() {
            pacing_directives_for_characters(
                &self
                    .repository
                    .list_character_arcs_by_project_and_branch(&input.project_id, &active_branch.id)
                    .await?,
                &self
                    .repository
                    .list_pacing_trackers_by_project_and_branch(
                        &input.project_id,
                        &active_branch.id,
                    )
                    .await?,
                &ids,
            )
        } else {
            Vec::new()
        };

        // Realized-intensity feed-forward (T-109): summarize the trend across the
        // last up-to-3 annotated chapters strictly before this one so the drafting
        // agent sees the pacing trajectory it is about to extend. Book/chapter
        // level, independent of the per-arc pacing directives above; rides the
        // same pacing_directives render/budget/trim path.
        let realized_intensity_trend = if want_pacing_directives {
            let scenes = self
                .repository
                .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?;
            let annotations = self
                .repository
                .list_scene_beat_annotations_by_project_and_branch(
                    &input.project_id,
                    &active_branch.id,
                )
                .await?;
            let chapter_means = crate::format::chapter_mean_intensities(&scenes, &annotations);
            // Curve expectation clause (V0022): pull the book's pacing curve
            // intensity_points and derive the chapter denominator from the max
            // persisted chapter number in the same book. Empty points / no
            // denominator → the without-expectation form.
            let intensity_points = self
                .repository
                .list_pacing_curves_by_project(&input.project_id)
                .await?
                .into_iter()
                .find(|curve| {
                    curve.branch_id == active_branch.id && curve.book_number == input.book_number
                })
                .map(|curve| curve.intensity_points)
                .unwrap_or_default();
            let max_chapter = scenes
                .iter()
                .filter(|scene| scene.book_number == input.book_number)
                .map(|scene| scene.chapter_number)
                .max();
            crate::format::realized_intensity_trend_directive(
                &chapter_means,
                input.book_number,
                input.chapter_number,
                &intensity_points,
                max_chapter,
            )
        } else {
            None
        };

        let narrative_promises_due = if want_narrative_promises_due {
            self.repository
                .list_narrative_promises_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|promise| promise.status != "paid_off")
                .map(|promise| {
                    narrative_promise_due_summary(
                        &promise,
                        input.book_number,
                        input.chapter_number,
                        input.scene_order,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        // Raw, cursor-gated knowledge rows for the present cast. Kept as rows
        // (not yet mapped to briefing items) so the secret-knowledge gate can
        // strip secret-linked entries by `secret_of_fact_id` before rendering —
        // the briefing item drops that link. Filtered + mapped after the gate.
        let knowledge_rows = if want_knowledge_briefing && !ids.is_empty() {
            self.repository
                .list_knowledge_facts_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|fact| ids.contains(&fact.character_id))
                .filter(|fact| {
                    fact.learned_at
                        .as_ref()
                        .is_none_or(|placement| story_index_from_placement(placement) <= cursor)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let agency_check = if want_agency_check {
            let recent_scenes = self
                .repository
                .list_recent_scenes_by_project_and_branch(
                    &input.project_id,
                    &active_branch.id,
                    input.book_number,
                    input.chapter_number,
                    input.scene_order,
                    50,
                )
                .await?;
            agency_check_from_scene_history(
                &recent_scenes,
                &resolved_characters,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
        } else {
            empty_agency_check_summary()
        };

        // Semantic recall: surface canon that is *topically* related to the
        // scene's present cast and setting but may be structurally distant
        // (a different chapter or book), so relevance — not just adjacency —
        // drives what resurfaces. Reuses the exact embed→kNN path as
        // `search_bible`; the query is the present characters + location.
        let mut unavailable_passes = Vec::new();
        let semantic_references: Vec<spindle_core::models::SearchBibleResultItem> =
            if want_semantic_references {
                const SEMANTIC_REFERENCE_LIMIT: usize = 5;
                let mut query_parts: Vec<&str> = Vec::new();
                for character in &included_characters {
                    query_parts.push(character.name.as_str());
                    if !character.summary.is_empty() {
                        query_parts.push(character.summary.as_str());
                    }
                }
                query_parts.push(location.name.as_str());
                if !location.summary.is_empty() {
                    query_parts.push(location.summary.as_str());
                }
                let query = query_parts.join(". ");
                if query.trim().is_empty() {
                    Vec::new()
                } else {
                    let recall_result: Result<Vec<spindle_core::models::SearchBibleResultItem>> =
                        async {
                            let embedding_session =
                                self.repository.model_router().embedding_session()?;
                            let query_embedding = embedding_session.embed_text(&query).await?;
                            // Over-fetch so excluding present-scene entities (which are
                            // already rendered in the scene layer) can't starve the slice.
                            let k = SEMANTIC_REFERENCE_LIMIT + ids.len() + 1;
                            let hits = self
                                .repository
                                .knn_search_embeddings(
                                    &input.project_id,
                                    Some(&active_branch.id),
                                    &query_embedding,
                                    k,
                                )
                                .await?;
                            let present: std::collections::HashSet<&str> = ids
                                .iter()
                                .map(String::as_str)
                                .chain(std::iter::once(input.location_id.as_str()))
                                .collect();
                            Ok(hits
                                .into_iter()
                                .filter(|(se, _)| !present.contains(se.entity_id.as_str()))
                                .take(SEMANTIC_REFERENCE_LIMIT)
                                .map(|(se, distance)| {
                                    spindle_core::models::SearchBibleResultItem {
                                        entity_type: se.entity_table,
                                        entity_id: se.entity_id,
                                        title: se.title,
                                        excerpt: se.excerpt,
                                        // Lower distance = more similar; report as a [0, 1]
                                        // score where higher is better, mirroring search_bible.
                                        score: 1.0 / (1.0 + distance),
                                    }
                                })
                                .collect())
                        }
                        .await;
                    match recall_result {
                        Ok(items) => items,
                        Err(err) if for_prepare_scene => {
                            unavailable_passes.push(
                                spindle_core::models::PrepareSceneUnavailablePass {
                                    pass: "semantic_references".to_string(),
                                    reason: err.to_string(),
                                },
                            );
                            Vec::new()
                        }
                        Err(err) => return Err(err),
                    }
                }
            } else {
                Vec::new()
            };

        let placement = StoryPlacement {
            book_number: input.book_number,
            chapter_number: input.chapter_number,
            scene_order: Some(input.scene_order),
            note: None,
        };
        let scene_id = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?
            .map(|scene| scene.id);

        // Subjects for canonical-fact lookup: characters + location + scene.
        let mut canonical_subjects: Vec<Subject> = Vec::new();
        let mut seen_subjects = std::collections::BTreeSet::new();
        for character_id in &input.character_ids {
            if seen_subjects.insert(format!("character:{character_id}")) {
                canonical_subjects.push(
                    Subject::new(SubjectTable::Character, character_id.clone())
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                );
            }
        }
        if seen_subjects.insert(format!("location:{}", input.location_id)) {
            canonical_subjects.push(
                Subject::new(SubjectTable::Location, input.location_id.clone())
                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
            );
        }
        if let Some(scene_id) = scene_id.as_ref()
            && seen_subjects.insert(format!("scene:{scene_id}"))
        {
            canonical_subjects.push(
                Subject::new(SubjectTable::Scene, scene_id.clone())
                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
            );
        }
        // Economy subjects so economy-scoped canonical facts (e.g. named prices)
        // are loaded and rendered as hard constraints alongside character facts.
        for economy in &economies {
            if seen_subjects.insert(format!("economy:{}", economy.id)) {
                canonical_subjects.push(
                    Subject::new(SubjectTable::Economy, economy.id.clone())
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                );
            }
        }

        let mut canonical_facts = self
            .repository
            .list_canonical_facts_for_subjects(
                &input.project_id,
                &active_branch.id,
                &canonical_subjects,
                &placement,
            )
            .await?;
        canonical_facts.extend(
            self.repository
                .list_canonical_facts_for_project_wide(
                    &input.project_id,
                    &active_branch.id,
                    &placement,
                )
                .await?,
        );

        // ── Secret-knowledge context gate (design §2.2) ──────────────────
        // The single chokepoint: resolve every secret fact's visibility for
        // this scene (cast = scene character ids; pov = chapter-plan POV;
        // cursor = the same story index the knowledge/temporal filters use),
        // then strip gated secrets from EVERY carrier via one helper. No-op
        // when the project declares no secret facts (byte-identical output).
        let secret_facts: Vec<crate::sqlite::records::CanonicalFact> = canonical_facts
            .iter()
            .filter(|fact| fact.secret)
            .cloned()
            .collect();
        let secret_gate = if secret_facts.is_empty() {
            SceneSecretGate::default()
        } else {
            // POV: the chapter plan's pov_character_id, when one exists.
            let pov_character_id = self
                .repository
                .list_chapter_plans_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .find(|plan| {
                    plan.book_number == input.book_number
                        && plan.chapter_number == input.chapter_number
                })
                .and_then(|plan| plan.pov_character_id);
            // Name map for the envelope rosters. Covers every project character
            // so circle members outside the present cast still resolve.
            let name_of: std::collections::BTreeMap<String, String> = self
                .repository
                .list_characters_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .map(|character| (character.id, character.name))
                .collect();
            self.resolve_scene_secret_gate(
                &input.project_id,
                &active_branch.id,
                &secret_facts,
                &SceneSecretGateParams {
                    scene_cast: &input.character_ids,
                    pov: pov_character_id.as_deref(),
                    cursor,
                    name_of: &name_of,
                },
            )
            .await?
        };

        // Strip gated secrets from the loaded fact list BEFORE deriving read
        // models and hard-constraint fact lines — every gated secret leaves the
        // normal carriers and lives only in the [SECRETS IN PLAY] block.
        if !secret_gate.is_empty() {
            canonical_facts.retain(|fact| !secret_gate.gated_fact_ids.contains(&fact.id));
        }

        let canonical_fact_read_models = canonical_facts
            .iter()
            .map(canonical_fact_read_model)
            .collect::<Vec<_>>();

        let subjects_snapshots = if want_subjects {
            let mut subject_keys = std::collections::BTreeSet::new();
            let mut subjects = Vec::new();
            let mut push_subject = |subject: Subject| -> anyhow::Result<()> {
                let key = subject.to_string();
                if subject_keys.insert(key) {
                    subjects.push(subject);
                }
                Ok(())
            };
            if let Some(pov_character_id) = self
                .repository
                .list_chapter_plans_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .find(|plan| {
                    plan.book_number == input.book_number
                        && plan.chapter_number == input.chapter_number
                })
                .and_then(|plan| plan.pov_character_id)
            {
                push_subject(
                    Subject::new(SubjectTable::Character, pov_character_id)
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                )?;
            }
            for character_id in &input.character_ids {
                push_subject(
                    Subject::new(SubjectTable::Character, character_id.clone())
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                )?;
            }
            push_subject(
                Subject::new(SubjectTable::Location, input.location_id.clone())
                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
            )?;
            // Controlling-faction subject (best-effort name match).
            if let Some(name) = world_state
                .as_ref()
                .and_then(|state| state.controlling_faction.as_deref())
            {
                let normalized_name = name.trim().to_ascii_lowercase();
                if !normalized_name.is_empty() {
                    for faction in self
                        .repository
                        .list_factions_by_project_and_branch(&input.project_id, &active_branch.id)
                        .await?
                    {
                        if faction.name.trim().to_ascii_lowercase() == normalized_name {
                            push_subject(
                                Subject::new(SubjectTable::Faction, faction.id)
                                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                            )?;
                            break;
                        }
                    }
                }
            }

            self.repository
                .assemble_subject_snapshots(
                    &input.project_id,
                    &active_branch.id,
                    &subjects,
                    &placement,
                )
                .await?
        } else {
            Vec::new()
        };

        // Secret gate: strip gated secrets from every subject snapshot's
        // canonical_facts (one of the carriers §2.2 requires covering).
        let mut subjects_snapshots = subjects_snapshots;
        apply_secret_visibility(&secret_gate, &mut subjects_snapshots);

        // Knowledge briefing: build from the cursor-gated rows, dropping any row
        // linked to a gated secret (secret_of_fact_id ∈ gate) so a secret never
        // leaks through the per-character briefing either.
        let knowledge_briefing = if want_knowledge_briefing && !ids.is_empty() {
            let mut briefing = knowledge_rows
                .into_iter()
                .filter(|fact| {
                    fact.secret_of_fact_id
                        .as_ref()
                        .is_none_or(|fact_id| !secret_gate.gated_fact_ids.contains(fact_id))
                })
                .map(knowledge_fact_briefing_item)
                .collect::<Vec<_>>();
            briefing.extend(
                raw_future_knowledge
                    .iter()
                    .map(future_knowledge_briefing_item),
            );
            briefing
        } else {
            Vec::new()
        };

        // Hydrate the narrative threads this chapter's plan explicitly targets
        // (themes, conflicts, plot lines, theme-connected motifs) so the
        // drafting agent can advance them. No-op when the chapter has no plan.
        let mut active_threads = if let Some(plan) = self
            .repository
            .list_chapter_plans_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?
            .into_iter()
            .find(|plan| {
                plan.book_number == input.book_number && plan.chapter_number == input.chapter_number
            }) {
            self.assemble_active_threads(
                &input.project_id,
                &active_branch.id,
                &plan.target_theme_ids,
                &plan.target_conflict_ids,
                &plan.target_plot_line_ids,
            )
            .await?
        } else {
            Vec::new()
        };

        active_threads.extend(
            self.editorial_threads(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
            )
            .await?,
        );

        // Resolve the immediately preceding scene's closing prose so the drafting
        // agent can hand off cleanly from its exit beat. Prefer the same-chapter
        // predecessor (scene_order - 1); when the current scene is the chapter's
        // first, reach back to the highest-order scene of the previous chapter in
        // the same book. Book start (chapter 1, no earlier scene) yields None.
        let previous_scene_tail = if want_previous_scene_tail {
            let previous_scene = if input.scene_order > 1
                && let Some(scene) = self
                    .repository
                    .find_scene_by_natural_key(
                        &input.project_id,
                        &active_branch.id,
                        input.book_number,
                        input.chapter_number,
                        input.scene_order - 1,
                    )
                    .await?
            {
                Some(scene)
            } else if input.chapter_number > 1 {
                // First scene of a later chapter: pull the highest-order scene of
                // the previous chapter in the same book (active branch).
                self.repository
                    .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
                    .await?
                    .into_iter()
                    .filter(|scene| {
                        scene.book_number == input.book_number
                            && scene.chapter_number == input.chapter_number - 1
                    })
                    .max_by_key(|scene| scene.scene_order)
            } else {
                None
            };

            // Cross-rating elision. This context is assembled into a draft
            // prompt that is dispatched to whichever route serves the TARGET
            // scene's rating. Handing an explicit neighbour's prose to a
            // general-rated target would therefore transmit explicit material
            // to an agent the operator never cleared for it — routing the
            // dispatch correctly is worthless if the payload smuggles the prose
            // across anyway. So when the neighbour is explicit and the target is
            // not, substitute the neighbour's (non-explicit) summary and say so.
            //
            // Fail closed on an unknown target rating: an undrafted, unplanned
            // scene gives us nothing to prove clearance with, and guessing
            // "probably fine" is exactly the wrong default for this invariant.
            //
            // The target rating is resolved ONLY when the neighbour is actually
            // explicit. Resolving it eagerly would put a scene lookup plus a
            // full chapter-plan scan on every scene-context call in every
            // project, to answer a question that only matters for the rare
            // explicit neighbour.
            match previous_scene {
                Some(scene) if scene.content_rating.trim().eq_ignore_ascii_case("explicit") => {
                    let target_rating = self
                        .resolve_target_scene_rating(
                            &input.project_id,
                            &active_branch.id,
                            input.book_number,
                            input.chapter_number,
                            input.scene_order,
                        )
                        .await?;
                    let target_is_explicit = target_rating
                        .as_deref()
                        .is_some_and(|rating| rating.eq_ignore_ascii_case("explicit"));
                    if target_is_explicit {
                        // Same clearance on both sides: the real prose goes over.
                        crate::format::scene_closing_excerpt(&scene.full_text).map(|excerpt| {
                            spindle_core::models::PreviousSceneTail {
                                scene_id: scene.id,
                                chapter_number: scene.chapter_number,
                                scene_order: scene.scene_order,
                                excerpt,
                                elided_reason: None,
                            }
                        })
                    } else {
                        let reason = match target_rating.as_deref() {
                            Some(rating) => format!(
                                "previous scene is explicit; this scene is rated {rating} and \
                                 drafts on a route that may not be cleared for explicit content, \
                                 so its closing prose is replaced by the scene summary"
                            ),
                            None => "previous scene is explicit and this scene has no planned \
                                     content rating, so its closing prose is replaced by the \
                                     scene summary until the rating is known"
                                .to_string(),
                        };
                        // The summary is authored at a non-explicit register and
                        // is what preserves the hand-off; an empty one degrades
                        // to no tail rather than falling back to the prose.
                        let summary = scene.summary.trim();
                        if summary.is_empty() {
                            None
                        } else {
                            Some(spindle_core::models::PreviousSceneTail {
                                scene_id: scene.id,
                                chapter_number: scene.chapter_number,
                                scene_order: scene.scene_order,
                                excerpt: summary.to_string(),
                                elided_reason: Some(reason),
                            })
                        }
                    }
                }
                Some(scene) => {
                    crate::format::scene_closing_excerpt(&scene.full_text).map(|excerpt| {
                        spindle_core::models::PreviousSceneTail {
                            scene_id: scene.id,
                            chapter_number: scene.chapter_number,
                            scene_order: scene.scene_order,
                            excerpt,
                            elided_reason: None,
                        }
                    })
                }
                None => None,
            }
        } else {
            None
        };

        let mut novel_layer = SceneContextNovelLayer {
            reader_contract: if want_reader_contract {
                project.reader_contract.into_core()
            } else {
                empty_reader_contract()
            },
            style_directive: if want_style_directive && !style_directive.is_empty() {
                Some(style_directive)
            } else {
                None
            },
            active_style_profile_id: project.active_style_profile_id.clone(),
            world_rules: if want_world_rules {
                let mut summaries: Vec<WorldRuleSummary> = relevant_world_rules
                    .clone()
                    .into_iter()
                    .map(|rule| WorldRuleSummary {
                        rule_name: rule.rule_name,
                        rule_type: rule.rule_type,
                        description: rule.description,
                    })
                    .collect();
                // Surface style-typed rules first so they read as prose-level
                // directives, not lore buried in a long list.
                summaries.sort_by_key(|summary| {
                    if summary.rule_type.eq_ignore_ascii_case("style") {
                        0
                    } else {
                        1
                    }
                });
                summaries
            } else {
                Vec::new()
            },
            subjects: subjects_snapshots,
            system_overlays: if want_system_overlays {
                system_overlays
                    .into_iter()
                    .map(system_overlay_summary)
                    .collect()
            } else {
                Vec::new()
            },
            timeline_briefing,
            future_knowledge_briefing,
            pacing_directives,
            realized_intensity_trend,
            narrative_promises_due,
            knowledge_briefing,
            semantic_references,
            economy_briefing,
            active_threads,
            previous_scene_tail,
        };

        let mut scene_layer = SceneContextSceneLayer {
            location: if want_location {
                LocationSummary {
                    location_id: location.id.clone(),
                    name: location.name.clone(),
                    kind: location.kind.clone(),
                    realm: location.realm.clone(),
                    summary: location.summary.clone(),
                }
            } else {
                empty_location_summary()
            },
            world_state: if let Some(world_state) = world_state {
                WorldStateSummary {
                    controlling_faction: world_state.controlling_faction,
                    status: world_state.status,
                    prosperity: world_state.prosperity,
                    stability: world_state.stability,
                    threat_level: world_state.threat_level,
                    sensory_details: world_state.sensory_details,
                }
            } else {
                empty_world_state_summary()
            },
            characters: if want_characters {
                resolved_characters
            } else {
                Vec::new()
            },
            relationships: if want_relationships {
                relationships
                    .into_iter()
                    .map(|relationship| RelationshipSummary {
                        // SQLite stores relationships as a junction table with
                        // composite PK (in_id, out_id, branch_id); synthesize
                        // a deterministic relationship_id from the pair.
                        relationship_id: format!("{}|{}", relationship.in_id, relationship.out_id),
                        source_character_id: relationship.in_id,
                        target_character_id: relationship.out_id,
                        relationship_type: relationship.relationship_type,
                        trust: relationship.trust,
                        tension: relationship.tension,
                        dynamics: relationship.dynamics,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            agency_check,
        };

        let mut hard_constraints: Vec<HardConstraint> = relevant_world_rules
            .iter()
            .map(|rule| HardConstraint {
                // Mark style-typed rules so they read as mandatory prose-level
                // directives even in the flat hard-constraints list.
                id: if rule.rule_type.eq_ignore_ascii_case("style") {
                    format!("[STYLE DIRECTIVE] {}", rule.rule_name)
                } else {
                    rule.rule_name.clone()
                },
                statement: rule.description.clone(),
            })
            .chain(canonical_facts.iter().map(canonical_fact_hard_constraint))
            .collect();

        // Secret gate: append the [SECRETS IN PLAY] block(s) as non-truncatable
        // hard constraints (same tier as STORY SO FAR / IN-WORLD TIME, appended
        // after the existing constraints per §2.2). Empty for no-secrets scenes.
        hard_constraints.extend(secret_gate.envelope_blocks.iter().cloned());

        // Surface the in-world time as a non-truncatable hard constraint so the
        // drafting model anchors WHEN this scene takes place relative to the
        // previous one and signals any elapsed time. This is the temporal twin
        // of the spatial grounding rule, and the prevention half of the
        // temporal-coherence subsystem (the `temporal_coherence` check is the
        // detection half). No-op for projects that never declare a calendar /
        // scene clocks.
        if let Some(constraint) = self
            .temporal_anchor_constraint(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?
        {
            hard_constraints.insert(0, constraint);
        }

        // Quantity state: surface each scene character's current band/amount per
        // declared measure as a hard constraint, so drafting stays consistent with
        // established wealth/progression. No-op when the project declares no scheme.
        let quantity_schemes = self
            .repository
            .list_project_quantity_schemes(&input.project_id, &active_branch.id)
            .await?;
        if !quantity_schemes.is_empty() {
            let cursor_index =
                story_index(input.book_number, input.chapter_number, input.scene_order);
            let mut lines: Vec<String> = Vec::new();
            for character_id in &input.character_ids {
                for stored_scheme in &quantity_schemes {
                    let measure = stored_scheme.scheme.measure.as_str();
                    let Some(state) = self
                        .repository
                        .latest_quantity_state_at_or_before(
                            &input.project_id,
                            &active_branch.id,
                            "character",
                            character_id,
                            measure,
                            cursor_index,
                        )
                        .await?
                    else {
                        continue;
                    };
                    let mut parts: Vec<String> = Vec::new();
                    if let Some(band) = state.state.band.as_deref() {
                        parts.push(band.to_string());
                    }
                    if let Some(amount) = state.state.amount {
                        match state.state.unit.as_deref() {
                            Some(unit) => parts.push(format!("{amount} {unit}")),
                            None => parts.push(format!("{amount}")),
                        }
                    }
                    if !parts.is_empty() {
                        let mut line = format!("- {character_id} {measure}: {}", parts.join(", "));
                        // Per-book trajectory: note where this measure started the
                        // book so the model keeps the arc consistent, not just the
                        // current value (a pure read; no digest table).
                        if let Some(current_band) = state.state.band.as_deref()
                            && let Some(first) = self
                                .repository
                                .earliest_quantity_state_in_book(
                                    &input.project_id,
                                    &active_branch.id,
                                    "character",
                                    character_id,
                                    measure,
                                    cursor_index,
                                )
                                .await?
                            && let Some(first_band) = first.state.band.as_deref()
                            && first_band != current_band
                        {
                            line.push_str(&format!(
                                " (book {} so far: {first_band} → {current_band})",
                                input.book_number
                            ));
                        }
                        lines.push(line);
                    }
                }
            }
            if !lines.is_empty() {
                hard_constraints.insert(
                    0,
                    HardConstraint {
                        id: "[WEALTH/STATE]".to_string(),
                        statement: format!(
                            "Current tracked quantities (do not contradict these amounts or this scale):\n{}",
                            lines.join("\n")
                        ),
                    },
                );
            }
        }

        let story_so_far = self
            .story_so_far(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?;
        if !story_so_far.is_empty() {
            hard_constraints.insert(
                0,
                HardConstraint {
                    id: "[STORY SO FAR]".into(),
                    statement: story_so_far,
                },
            );
        }

        let non_truncatable_cost =
            non_truncatable_prefix_tokens_scene_context(format_fmt, &hard_constraints);
        if non_truncatable_cost > budget_tokens {
            if for_prepare_scene {
                anyhow::bail!(
                    "budget_tokens ({budget_tokens}) too small to fit hard constraints (minimum required: {non_truncatable_cost} tokens). Increase budget_tokens or reduce world rules."
                );
            }
            budget_tokens =
                non_truncatable_cost.saturating_add(SCENE_CONTEXT_HARD_CONSTRAINT_HEADROOM_TOKENS);
        }

        let mut bundle = build_scene_context_bundle(
            format_fmt,
            budget_tokens,
            &hard_constraints,
            &novel_layer.subjects,
            &novel_layer,
            &scene_layer,
        );
        let budget_report = bundle
            .enforce_budget()
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        apply_scene_context_bundle_trims(
            &budget_report.truncated_section_ids,
            &mut novel_layer,
            &mut scene_layer,
        );

        let estimated_tokens = estimate_scene_context_tokens(
            format_fmt,
            &hard_constraints,
            &novel_layer,
            &scene_layer,
        );
        let novel_layer_truncated = !budget_report.truncated_section_ids.is_empty();

        let style_notes = novel_layer
            .style_directive
            .as_ref()
            .map(|directive| directive.style_notes.as_slice())
            .unwrap_or(novel_layer.reader_contract.style_notes.as_slice());
        let style_rules = novel_layer
            .style_directive
            .as_ref()
            .map(|directive| directive.style_rules.as_slice())
            .unwrap_or(&[]);
        let (compact_shelf_digest, voice_samples, scene_negatives) = self
            .assemble_fiction_writing_packet(
                &input.project_id,
                novel_layer.active_style_profile_id.as_deref(),
                style_notes,
                style_rules,
            )
            .await;

        Ok(SceneContextWithCoverage {
            output: SceneContextOutput {
                hard_constraints,
                canonical_facts: canonical_fact_read_models,
                novel: novel_layer,
                scene: scene_layer,
                budget: SceneContextBudgetMeta {
                    estimated_tokens,
                    token_budget: Some(budget_tokens),
                    novel_layer_truncated,
                },
                compact_shelf_digest,
                voice_samples,
                scene_negatives,
            },
            bundle_estimated_tokens: budget_report.estimated_tokens,
            hard_constraint_tokens: non_truncatable_cost,
            truncated_section_ids: budget_report.truncated_section_ids,
            unavailable_passes,
        })
    }

    /// Secret-knowledge gating (design §2.2): resolve, for one scene, which
    /// secret canonical facts must be withheld or enveloped, and produce the
    /// `[SECRETS IN PLAY]` hard-constraint block(s). This is the single
    /// chokepoint the context-assembly path calls; the per-carrier filtering
    /// (`apply_secret_visibility`) consumes the returned `SceneSecretGate` so no
    /// section decides secrecy on its own.
    ///
    /// `secret_facts` are the loaded canonical facts already flagged
    /// `secret = true`. The scene-scoping inputs (cast, POV, cursor, name map)
    /// ride in [`SceneSecretGateParams`].
    pub(super) async fn resolve_scene_secret_gate(
        &self,
        project_id: &str,
        branch_id: &str,
        secret_facts: &[crate::sqlite::records::CanonicalFact],
        params: &SceneSecretGateParams<'_>,
    ) -> Result<SceneSecretGate> {
        use crate::format::{
            SECRETS_IN_PLAY_CONSTRAINT_ID, SecretDecision, canonical_fact_value_display,
            resolve_secret_visibility, secret_in_play_block, story_index_from_placement,
        };
        use spindle_core::models::HardConstraint;

        let scene_cast = params.scene_cast;
        let pov = params.pov;
        let cursor = params.cursor;
        let name_of = params.name_of;

        // Resolve a roster of character ids to display names (unknown ids fall
        // back to the raw id so a roster line is never empty/misleading).
        let names = |ids: &[String]| -> Vec<String> {
            ids.iter()
                .map(|id| name_of.get(id).cloned().unwrap_or_else(|| id.clone()))
                .collect()
        };

        let mut gate = SceneSecretGate::default();
        for fact in secret_facts {
            if !fact.secret {
                continue;
            }
            // Circle-at-cursor: derived from the linked knowledge rows.
            let circle: Vec<(String, Option<i64>)> = self
                .repository
                .secret_circle_members(project_id, branch_id, &fact.id)
                .await?
                .into_iter()
                .map(|(character_id, learned_at)| {
                    let idx = learned_at.as_ref().map(story_index_from_placement);
                    (character_id, idx)
                })
                .collect();

            let decision = resolve_secret_visibility(
                &circle,
                scene_cast,
                pov,
                cursor,
                fact.concealment_note.as_deref(),
            );

            // Every gated secret leaves the normal carriers: it is withheld, or
            // it moves into the envelope block. In both cases it is stripped
            // from hard-constraint fact lines, read models, snapshots, and the
            // knowledge briefing — it only ever appears in [SECRETS IN PLAY].
            gate.gated_fact_ids.insert(fact.id.clone());
            gate.withheld_snapshot_facts
                .insert(snapshot_fact_string(fact));

            // Map the resolver's *id* rosters to display *names* so the block is
            // human-facing (design: "resolve character names from ids like other
            // blocks do"), keeping the pure formatter id-agnostic.
            let named_decision = match &decision {
                SecretDecision::Withhold => SecretDecision::Withhold,
                SecretDecision::Envelope {
                    known_to,
                    unaware_present,
                    concealment_note,
                } => SecretDecision::Envelope {
                    known_to: names(known_to),
                    unaware_present: names(unaware_present),
                    concealment_note: concealment_note.clone(),
                },
                SecretDecision::PovEnvelope {
                    known_to,
                    concealment_note,
                } => SecretDecision::PovEnvelope {
                    known_to: names(known_to),
                    concealment_note: concealment_note.clone(),
                },
            };

            if let Some(statement) = secret_in_play_block(
                &canonical_fact_value_display(fact),
                &named_decision,
                fact.concealment_note.as_deref(),
            ) {
                gate.envelope_blocks.push(HardConstraint {
                    id: SECRETS_IN_PLAY_CONSTRAINT_ID.to_string(),
                    statement,
                });
            }
        }
        Ok(gate)
    }

    /// Build the public `get_scene_context` tool envelope while keeping
    /// standards inclusion and markdown rendering in the application service
    /// boundary instead of the MCP transport adapter.
    pub async fn get_scene_context_envelope(
        &self,
        input: spindle_core::models::GetSceneContextInput,
    ) -> Result<spindle_core::models::SceneContextEnvelope> {
        let format = input
            .format
            .unwrap_or(spindle_core::models::ContextFormat::Markdown);
        let include_standards = input.wants_standards();
        let payload = self.get_scene_context(input).await?;
        let packet_markdown = crate::format::format_writing_packet_markdown(
            payload.compact_shelf_digest.as_ref(),
            &payload.voice_samples,
            &payload.scene_negatives,
        );
        let context_markdown =
            (format == spindle_core::models::ContextFormat::Markdown).then(|| {
                crate::format::format_scene_context_markdown_with_packet(
                    None,
                    &payload.hard_constraints,
                    &payload.novel,
                    &payload.scene,
                    payload.compact_shelf_digest.as_ref(),
                    &payload.voice_samples,
                    &payload.scene_negatives,
                )
            });

        Ok(spindle_core::models::SceneContextEnvelope {
            hard_constraints: payload.hard_constraints,
            standards: if include_standards {
                // Lead the standards block with the project-specific style
                // requirements so the genre-voice contract sits in the same
                // instruction the model reads first, not in a separate payload
                // it may deprioritize (the brief's Change 8).
                let mut standards = String::new();
                if let Some(directive) = payload
                    .novel
                    .style_directive
                    .as_ref()
                    .and_then(|directive| directive.render_markdown())
                {
                    standards.push_str(directive.trim_start());
                    standards.push_str("\n\n");
                }
                if !packet_markdown.is_empty() {
                    standards.push_str(&packet_markdown);
                    standards.push_str("\n\n");
                }
                standards.push_str(crate::guidance::standards_text());
                standards
            } else {
                String::new()
            },
            novel: payload.novel,
            scene: payload.scene,
            budget: payload.budget,
            compact_shelf_digest: payload.compact_shelf_digest,
            voice_samples: payload.voice_samples,
            scene_negatives: payload.scene_negatives,
            context_markdown,
        })
    }

    /// Specialised character-focused snapshot: same backbone as
    /// `get_entity` for the Character subject, but unpacks the voice
    /// profile / current_state / recent_appearances off the
    /// SubjectSnapshot for direct caller consumption. Mirrors
    /// services/mod.rs:9245..9310 in 705b835^.
    pub async fn get_character_snapshot(
        &self,
        input: spindle_core::models::GetCharacterSnapshotInput,
    ) -> Result<spindle_core::models::CharacterSnapshotOutput> {
        use spindle_core::models::{CharacterSnapshotOutput, StoryPlacement};
        use spindle_core::subject::{Subject, SubjectTable};

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let branch = match input.branch_id.as_deref() {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(input.project_id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        input.project_id
                    );
                }
                branch
            }
            None => active_branch,
        };

        let character = self.repository.get_character(&input.character_id).await?;
        if character.project_id != input.project_id {
            anyhow::bail!(
                "character {} does not belong to project {}",
                input.character_id,
                input.project_id
            );
        }

        let placement = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &branch.id)
            .await?
            .last()
            .map(|scene| StoryPlacement {
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: Some(scene.scene_order),
                note: None,
            })
            .unwrap_or(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            });

        let subject = Subject::new(SubjectTable::Character, character.id.clone())
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        let snapshot = self
            .repository
            .assemble_subject_snapshot(&input.project_id, &branch.id, &subject, &placement)
            .await?;

        Ok(CharacterSnapshotOutput {
            voice_profile: snapshot.voice_profile().cloned(),
            current_state: snapshot.current_state().cloned(),
            recent_appearances: snapshot.recent_appearances().to_vec(),
            snapshot,
        })
    }

    /// Assemble a deep `SubjectSnapshot` for an arbitrary entity on the
    /// requested branch (or the project's active branch when omitted).
    /// Mirrors services/mod.rs:9135..9184 in 705b835^. Backed by the
    /// freshly-ported `Repository::assemble_subject_snapshot` (commit
    /// d0e721d), which loads the entire `SnapshotBatchContext` for the
    /// branch and projects the requested subject through pure helpers.
    pub async fn get_entity(
        &self,
        input: spindle_core::models::GetEntityInput,
    ) -> Result<spindle_core::subject_snapshot::SubjectSnapshot> {
        use spindle_core::models::StoryPlacement;
        use spindle_core::subject::{Subject, SubjectTable};

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let branch = match input.branch_id.as_deref() {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(input.project_id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        input.project_id
                    );
                }
                branch
            }
            None => active_branch,
        };

        if input.table == SubjectTable::Project {
            anyhow::bail!("get_entity does not support table=project");
        }

        let subject = Subject::new(input.table, input.entity_id)
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;

        // Use the latest scene on the branch as the placement anchor;
        // fall back to (1,1,1) when the branch has no scenes yet so the
        // assembly's "as-of" filters still have a position to compare
        // against. Mirrors the reference behaviour.
        let placement = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &branch.id)
            .await?
            .last()
            .map(|scene| StoryPlacement {
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: Some(scene.scene_order),
                note: None,
            })
            .unwrap_or(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            });

        self.repository
            .assemble_subject_snapshot(&input.project_id, &branch.id, &subject, &placement)
            .await
    }

    /// Assemble a bounded, read-only scene preparation packet composing writer cursor
    /// state, chapter/scene purpose, branch-visible context, secret-gated canon,
    /// style guidance, optional research recall, and `DraftingPacketV1`.
    pub async fn prepare_scene(
        &self,
        input: spindle_core::models::PrepareSceneInput,
    ) -> Result<spindle_core::models::PrepareSceneOutput> {
        use crate::format::DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS;
        use spindle_core::models::{
            ContentRating, ContextFormat, GetSceneContextInput, PrepareSceneCoverage,
            PrepareSceneCursor, PrepareSceneEvidenceRef, PrepareSceneIntent, PrepareSceneOutput,
            PrepareScenePurpose, PrepareSceneResearchItem, PrepareSceneUnavailablePass,
            ResearchSearchInput, StoryPlacement,
        };

        let project = self.repository.get_project(&input.project_id).await?;
        let branch = match input
            .branch_id
            .as_deref()
            .map(str::trim)
            .filter(|branch_id| !branch_id.is_empty())
        {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(project.id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        project.id
                    );
                }
                branch
            }
            None => self.repository.get_active_branch(&project.id).await?,
        };

        let intent = input.intent.unwrap_or_default();
        let scenes = self
            .repository
            .list_scenes_resolved_for_branch(&project.id, &branch.id)
            .await?;
        let writer_position = self
            .repository
            .get_writer_position(&project.id, &branch.id)
            .await?;

        let cursor_scene = if let Some(position_scene_id) = writer_position
            .as_ref()
            .and_then(|pos| pos.scene_id.clone())
        {
            scenes
                .iter()
                .find(|candidate| candidate.id == position_scene_id)
                .cloned()
                .or_else(|| scenes.last().cloned())
        } else {
            scenes.last().cloned()
        };
        let cursor_index = cursor_scene
            .as_ref()
            .and_then(|scene| scenes.iter().position(|candidate| candidate.id == scene.id));
        let next_scene = cursor_index
            .and_then(|index| scenes.get(index + 1))
            .cloned();

        let writer_cursor = PrepareSceneCursor {
            cursor_scene_id: cursor_scene.as_ref().map(|scene| scene.id.clone()),
            cursor_placement: cursor_scene.as_ref().map(|scene| StoryPlacement {
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: Some(scene.scene_order),
                note: None,
            }),
            next_scene_id: next_scene.as_ref().map(|scene| scene.id.clone()),
            writer_intent: writer_position
                .as_ref()
                .map(|pos| crate::format::parse_writer_intent(&pos.intent)),
            notes: writer_position
                .as_ref()
                .and_then(|pos| pos.next_focus.clone())
                .or_else(|| project.notes.clone()),
        };

        // Resolve target scene / placement without mutating writer position.
        let mut target_scene: Option<crate::sqlite::records::Scene> = None;
        let mut resolved_chapter_id: Option<String> = None;
        let (book_number, chapter_number, scene_order) = if let Some(scene_id) = input
            .scene_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let scene = if let Some(found) = scenes.iter().find(|s| s.id == scene_id).cloned() {
                found
            } else {
                let raw = self.repository.get_scene(scene_id).await?;
                if raw.project_id != project.id || raw.branch_id != branch.id {
                    anyhow::bail!(
                        "scene {scene_id} does not belong to branch {} of project {}",
                        branch.id,
                        project.id
                    );
                }
                raw
            };
            let b = input.book_number.unwrap_or(scene.book_number);
            let c = input.chapter_number.unwrap_or(scene.chapter_number);
            let s = input.scene_order.unwrap_or(scene.scene_order);
            if b != scene.book_number || c != scene.chapter_number || s != scene.scene_order {
                anyhow::bail!(
                    "scene_id {} placement ({},{},{}) does not match requested placement ({},{},{})",
                    scene.id,
                    scene.book_number,
                    scene.chapter_number,
                    scene.scene_order,
                    b,
                    c,
                    s
                );
            }
            resolved_chapter_id = Some(scene.chapter_id.clone());
            target_scene = Some(scene);
            (b, c, s)
        } else if let Some(chapter_id) = input
            .chapter_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let chapter = self.repository.get_chapter(chapter_id).await?;
            if chapter.project_id != project.id {
                anyhow::bail!("chapter does not belong to the requested project");
            }
            if let Some(b) = input.book_number
                && b > 0
                && b != chapter.book_number
            {
                anyhow::bail!("chapter_id {} does not match book_number {}", chapter_id, b);
            }
            if let Some(c) = input.chapter_number
                && c > 0
                && c != chapter.chapter_number
            {
                anyhow::bail!(
                    "chapter_id {} does not match chapter_number {}",
                    chapter_id,
                    c
                );
            }
            let b = chapter.book_number;
            let c = chapter.chapter_number;
            let s = input.scene_order.unwrap_or(1).max(1);
            resolved_chapter_id = Some(chapter.id);
            target_scene = scenes
                .iter()
                .find(|scene| {
                    scene.book_number == b && scene.chapter_number == c && scene.scene_order == s
                })
                .cloned();
            (b, c, s)
        } else if input.book_number.is_some() || input.chapter_number.is_some() {
            let b = input.book_number.unwrap_or(1);
            let c = input.chapter_number.unwrap_or(1);
            if b <= 0 || c <= 0 {
                anyhow::bail!("prepare_scene book_number and chapter_number must be positive");
            }
            let s = input.scene_order.unwrap_or(1).max(1);
            target_scene = scenes
                .iter()
                .find(|scene| {
                    scene.book_number == b && scene.chapter_number == c && scene.scene_order == s
                })
                .cloned();
            if let Some(scene) = target_scene.as_ref() {
                resolved_chapter_id = Some(scene.chapter_id.clone());
            }
            (b, c, s)
        } else if let Some(scene) = match intent {
            PrepareSceneIntent::Continue => next_scene.clone().or_else(|| cursor_scene.clone()),
            PrepareSceneIntent::Draft | PrepareSceneIntent::Revise => cursor_scene.clone(),
        } {
            let s = input.scene_order.unwrap_or(scene.scene_order).max(1);
            resolved_chapter_id = Some(scene.chapter_id.clone());
            if s == scene.scene_order {
                target_scene = Some(scene.clone());
            } else {
                target_scene = scenes
                    .iter()
                    .find(|candidate| {
                        candidate.book_number == scene.book_number
                            && candidate.chapter_number == scene.chapter_number
                            && candidate.scene_order == s
                    })
                    .cloned();
            }
            (scene.book_number, scene.chapter_number, s)
        } else {
            let s = input.scene_order.unwrap_or(1).max(1);
            (1, 1, s)
        };

        let chapters = self
            .repository
            .list_chapters_by_project(&project.id)
            .await?;
        let chapter_record = if let Some(chapter_id) = resolved_chapter_id.as_deref() {
            chapters.iter().find(|ch| ch.id == chapter_id).cloned()
        } else {
            chapters
                .iter()
                .find(|ch| ch.book_number == book_number && ch.chapter_number == chapter_number)
                .cloned()
        };
        if resolved_chapter_id.is_none() {
            resolved_chapter_id = chapter_record.as_ref().map(|ch| ch.id.clone());
        }

        let chapter_plan = self
            .repository
            .list_chapter_plans_by_project_and_branch(&project.id, &branch.id)
            .await?
            .into_iter()
            .find(|plan| plan.book_number == book_number && plan.chapter_number == chapter_number);

        let planned_scene = chapter_plan
            .as_ref()
            .and_then(|plan| plan.scenes.iter().find(|s| s.scene_order == scene_order));

        // When target_scene is inherited from an ancestor branch while chapter_plan was
        // authored on the requested branch (or when intent is Draft), prefer the branch
        // plan's cast/location/summary over the ancestor scene's defaults.
        let prefer_branch_plan = intent == PrepareSceneIntent::Draft
            || (target_scene
                .as_ref()
                .is_some_and(|s| s.branch_id != branch.id)
                && chapter_plan
                    .as_ref()
                    .is_some_and(|p| p.branch_id == branch.id));

        // Resolve character_ids and location_id from input -> chapter_plan -> branch defaults.
        let mut resolved_character_ids = input.character_ids.clone();
        if resolved_character_ids.is_empty() {
            if let Some(plan) = chapter_plan.as_ref() {
                if let Some(pov) = plan
                    .pov_character_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                {
                    resolved_character_ids.push(pov.to_string());
                }
                if let Some(ps) = planned_scene {
                    for cid in &ps.character_ids {
                        if !resolved_character_ids.contains(cid) {
                            resolved_character_ids.push(cid.clone());
                        }
                    }
                }
            }
            if resolved_character_ids.is_empty() {
                for character in self
                    .repository
                    .list_characters_by_project_and_branch(&project.id, &branch.id)
                    .await?
                {
                    resolved_character_ids.push(character.id);
                }
            }
        }

        let resolved_location_id = if let Some(loc_id) = input
            .location_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            loc_id.to_string()
        } else if prefer_branch_plan
            && let Some(loc_id) = planned_scene
                .and_then(|ps| ps.location_id.as_deref())
                .map(str::trim)
                .filter(|id| !id.is_empty())
        {
            loc_id.to_string()
        } else if let Some(loc_id) = target_scene
            .as_ref()
            .and_then(|s| s.location_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            loc_id.to_string()
        } else if let Some(loc_id) = planned_scene
            .and_then(|ps| ps.location_id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            loc_id.to_string()
        } else if let Some(loc_id) = scenes
            .iter()
            .filter(|s| s.book_number == book_number && s.chapter_number == chapter_number)
            .find_map(|s| {
                s.location_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
            })
        {
            loc_id.to_string()
        } else {
            let locations = self
                .repository
                .list_locations_by_project_and_branch(&project.id, &branch.id)
                .await?;
            locations.first().map(|loc| loc.id.clone()).ok_or_else(|| {
                anyhow::anyhow!(
                    "prepare_scene requires location_id or at least one location in the project"
                )
            })?
        };

        let pov_character_id = chapter_plan
            .as_ref()
            .and_then(|p| p.pov_character_id.clone())
            .or_else(|| resolved_character_ids.first().cloned());

        let content_rating = if prefer_branch_plan {
            planned_scene
                .and_then(|ps| ps.content_rating.clone())
                .or_else(|| {
                    target_scene.as_ref().and_then(|s| {
                        serde_json::from_value::<ContentRating>(serde_json::Value::String(
                            s.content_rating.trim().to_ascii_lowercase(),
                        ))
                        .ok()
                    })
                })
                .unwrap_or(ContentRating::General)
        } else {
            target_scene
                .as_ref()
                .and_then(|s| {
                    serde_json::from_value::<ContentRating>(serde_json::Value::String(
                        s.content_rating.trim().to_ascii_lowercase(),
                    ))
                    .ok()
                })
                .or_else(|| planned_scene.and_then(|ps| ps.content_rating.clone()))
                .unwrap_or(ContentRating::General)
        };

        let mut expected_beats = planned_scene
            .map(|ps| ps.beat_structure.clone())
            .unwrap_or_default();
        if expected_beats.is_empty()
            && let Some(chapter_id) = resolved_chapter_id.as_deref()
            && let Some(outline) = self
                .repository
                .get_chapter_outline(chapter_id, &branch.id)
                .await?
        {
            expected_beats = outline
                .beats
                .into_iter()
                .filter(|beat| beat.order == scene_order)
                .map(|beat| beat.summary)
                .collect();
        }

        let chapter_synopsis = chapter_plan
            .as_ref()
            .map(|plan| plan.synopsis.clone())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                chapter_record
                    .as_ref()
                    .and_then(|ch| ch.title.clone())
                    .filter(|s| !s.trim().is_empty())
            });

        let scene_version = if let Some(scene) = target_scene.as_ref() {
            let versions = self.repository.list_scene_versions(&scene.id).await?;
            Some(versions.last().map(|v| v.version_number).unwrap_or(1))
        } else {
            None
        };

        let scene_summary = if prefer_branch_plan {
            planned_scene
                .map(|ps| ps.summary.clone())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    target_scene
                        .as_ref()
                        .map(|s| s.summary.clone())
                        .filter(|s| !s.trim().is_empty())
                })
        } else {
            target_scene
                .as_ref()
                .map(|s| s.summary.clone())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| planned_scene.map(|ps| ps.summary.clone()))
        };

        let mut scene_purpose = PrepareScenePurpose {
            chapter_synopsis,
            scene_title: chapter_record.as_ref().and_then(|ch| ch.title.clone()),
            scene_summary,
            pov_character_id,
            location_id: resolved_location_id.clone(),
            character_ids: resolved_character_ids.clone(),
            content_rating,
            expected_beats,
            active_threads: Vec::new(),
        };

        let effective_budget = input
            .budget_tokens
            .unwrap_or(DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS);
        let purpose_cursor_tokens = purpose_and_cursor_tokens(&scene_purpose, &writer_cursor);

        let scene_context_with_cov = self
            .get_scene_context_for_branch(
                GetSceneContextInput {
                    project_id: project.id.clone(),
                    chapter_id: resolved_chapter_id.clone(),
                    book_number,
                    chapter_number,
                    scene_order,
                    character_ids: resolved_character_ids.clone(),
                    location_id: resolved_location_id.clone(),
                    sections: None,
                    format: Some(ContextFormat::Markdown),
                    budget_tokens: Some(usize::MAX),
                    token_budget: None,
                    max_character_count: None,
                },
                Some(&branch.id),
                true,
            )
            .await?;

        let hard_constraint_tokens = scene_context_with_cov
            .hard_constraint_tokens
            .saturating_add(purpose_cursor_tokens);
        if hard_constraint_tokens > effective_budget {
            anyhow::bail!(
                "budget_tokens ({effective_budget}) too small to fit hard constraints (minimum required: {hard_constraint_tokens} tokens). Increase budget_tokens or reduce world rules."
            );
        }

        let mut unavailable_passes = scene_context_with_cov.unavailable_passes;
        let mut scene_context = scene_context_with_cov.output;
        let untruncated_section_tokens = scene_context_section_token_map(&scene_context);

        // Eliminate duplicated representations across top-level PrepareSceneOutput and novel.
        let mut reader_contract = std::mem::replace(
            &mut scene_context.novel.reader_contract,
            crate::format::empty_reader_contract(),
        );
        let mut style_directive = scene_context.novel.style_directive.take();
        scene_purpose.active_threads = std::mem::take(&mut scene_context.novel.active_threads);
        let world_rules_for_evidence = std::mem::take(&mut scene_context.novel.world_rules);

        // Optional research recall pass with bounded summaries.
        let mut research = Vec::new();
        if let Some(query) = input
            .research_query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
        {
            match self
                .research_search(ResearchSearchInput {
                    project_id: project.id.clone(),
                    branch_id: Some(branch.id.clone()),
                    query: query.to_string(),
                    tags: Vec::new(),
                    time_period: None,
                    location: None,
                    limit: Some(5),
                })
                .await
            {
                Ok(found) => {
                    for item in found.results {
                        let raw_summary = if item.preview.trim().is_empty()
                            || item.preview == item.title_or_summary
                        {
                            item.title_or_summary
                        } else {
                            format!("{}: {}", item.title_or_summary, item.preview)
                        };
                        research.push(PrepareSceneResearchItem {
                            id: item.id,
                            kind: item.item_type,
                            summary: truncate_chars(&raw_summary, 240),
                            source_id: item.source_title,
                        });
                    }
                }
                Err(err) => {
                    unavailable_passes.push(PrepareSceneUnavailablePass {
                        pass: "research_recall".to_string(),
                        reason: err.to_string(),
                    });
                }
            }
        }

        let placement = StoryPlacement {
            book_number,
            chapter_number,
            scene_order: Some(scene_order),
            note: None,
        };

        let dependency_tokens = self
            .compute_scene_preparation_dependencies(
                &project.id,
                &branch.id,
                &placement,
                target_scene.as_ref().map(|s| s.id.as_str()),
            )
            .await?;
        let context_revision_token = composite_context_revision_token(&dependency_tokens);
        let preparation_id = format!(
            "prep:{}",
            &generation_sha256_hex(
                format!(
                    "{}:{}:{}:{}:{}:{}:{}",
                    project.id,
                    branch.id,
                    book_number,
                    chapter_number,
                    scene_order,
                    intent.as_str(),
                    context_revision_token
                )
                .as_bytes()
            )[..16]
        );

        let mut existing_prose = match intent {
            PrepareSceneIntent::Revise | PrepareSceneIntent::Continue => target_scene
                .as_ref()
                .map(|s| s.full_text.clone())
                .filter(|text| !text.trim().is_empty()),
            PrepareSceneIntent::Draft => None,
        };

        let mut drafting_packet_v1 = match build_drafting_packet_v1_for_prepare_scene(
            &preparation_id,
            &project.id,
            &branch.id,
            target_scene.as_ref(),
            scene_version,
            &placement,
            &dependency_tokens,
            &scene_purpose,
            &scene_context,
            &research,
        ) {
            Ok(packet) => Some(packet),
            Err(reason) => {
                unavailable_passes.push(PrepareSceneUnavailablePass {
                    pass: "drafting_packet_v1".to_string(),
                    reason,
                });
                None
            }
        };

        let mut evidence_references = Vec::new();
        if let Some(scene) = target_scene.as_ref() {
            evidence_references.push(PrepareSceneEvidenceRef {
                id: scene.id.clone(),
                kind: "scene".to_string(),
                label: truncate_chars(&scene.summary, 80),
            });
        }
        if let Some(plan) = chapter_plan.as_ref() {
            evidence_references.push(PrepareSceneEvidenceRef {
                id: plan.id.clone(),
                kind: "chapter_plan".to_string(),
                label: format!("Book {} Chapter {}", plan.book_number, plan.chapter_number),
            });
        }
        if !scene_context.scene.location.location_id.is_empty() {
            evidence_references.push(PrepareSceneEvidenceRef {
                id: scene_context.scene.location.location_id.clone(),
                kind: "location".to_string(),
                label: truncate_chars(&scene_context.scene.location.name, 80),
            });
        }
        for character in &scene_context.scene.characters {
            evidence_references.push(PrepareSceneEvidenceRef {
                id: character.character_id.clone(),
                kind: "character".to_string(),
                label: truncate_chars(&character.name, 80),
            });
        }
        for rule in &world_rules_for_evidence {
            evidence_references.push(PrepareSceneEvidenceRef {
                id: rule.rule_name.clone(),
                kind: "world_rule".to_string(),
                label: truncate_chars(&rule.rule_type, 80),
            });
        }
        for fact in &scene_context.canonical_facts {
            evidence_references.push(PrepareSceneEvidenceRef {
                id: fact.canonical_fact_id.clone(),
                kind: "canonical_fact".to_string(),
                label: truncate_chars(&fact.predicate, 80),
            });
        }
        for item in &research {
            evidence_references.push(PrepareSceneEvidenceRef {
                id: item.id.clone(),
                kind: "research".to_string(),
                label: truncate_chars(&item.kind, 80),
            });
        }

        let mut compact_shelf_digest = scene_context.compact_shelf_digest;
        let mut voice_samples = scene_context.voice_samples;
        let mut scene_negatives = scene_context.scene_negatives;
        let mut truncated_sections = scene_context_with_cov.truncated_section_ids;
        let push_truncated = |list: &mut Vec<String>, id: &str| {
            if !list.iter().any(|existing| existing == id) {
                list.push(id.to_string());
            }
        };

        let had_existing_prose = existing_prose.is_some();
        let had_research = !research.is_empty();
        let had_drafting_packet = drafting_packet_v1.is_some();
        let had_shelf_digest = compact_shelf_digest.is_some();
        let had_voice_samples = !voice_samples.is_empty();
        let had_scene_negatives = !scene_negatives.is_empty();
        let had_evidence_references = !evidence_references.is_empty();

        let mut style_directive_tokens = style_directive
            .as_ref()
            .and_then(|d| d.render_markdown())
            .map(|m| spindle_core::context_bundle::estimate_text_tokens(&m))
            .unwrap_or(0);
        let mut shelf_digest_tokens = compact_shelf_digest
            .as_ref()
            .map(estimate_value_tokens)
            .unwrap_or(0);
        let mut voice_samples_tokens = if voice_samples.is_empty() {
            0
        } else {
            estimate_value_tokens(&voice_samples)
        };
        let mut scene_negatives_tokens = if scene_negatives.is_empty() {
            0
        } else {
            estimate_value_tokens(&scene_negatives)
        };
        let mut evidence_tokens = if evidence_references.is_empty() {
            0
        } else {
            estimate_value_tokens(&evidence_references)
        };
        let mut packet_tokens = drafting_packet_v1
            .as_ref()
            .map(estimate_value_tokens)
            .unwrap_or(0);
        let mut research_tokens = if research.is_empty() {
            0
        } else {
            estimate_value_tokens(&research)
        };
        let mut existing_prose_tokens = existing_prose
            .as_deref()
            .map(spindle_core::context_bundle::estimate_text_tokens)
            .unwrap_or(0);

        let mut estimated_tokens = scene_context_with_cov
            .bundle_estimated_tokens
            .saturating_add(purpose_cursor_tokens)
            .saturating_add(style_directive_tokens)
            .saturating_add(shelf_digest_tokens)
            .saturating_add(voice_samples_tokens)
            .saturating_add(scene_negatives_tokens)
            .saturating_add(evidence_tokens)
            .saturating_add(packet_tokens)
            .saturating_add(research_tokens)
            .saturating_add(existing_prose_tokens);

        if estimated_tokens > effective_budget && drafting_packet_v1.is_some() {
            drafting_packet_v1 = None;
            estimated_tokens = estimated_tokens.saturating_sub(packet_tokens);
            packet_tokens = 0;
            push_truncated(&mut truncated_sections, "drafting_packet_v1");
        }
        if estimated_tokens > effective_budget && !evidence_references.is_empty() {
            evidence_references.clear();
            estimated_tokens = estimated_tokens.saturating_sub(evidence_tokens);
            evidence_tokens = 0;
            push_truncated(&mut truncated_sections, "evidence_references");
        }
        if estimated_tokens > effective_budget && !voice_samples.is_empty() {
            voice_samples.clear();
            estimated_tokens = estimated_tokens.saturating_sub(voice_samples_tokens);
            voice_samples_tokens = 0;
            push_truncated(&mut truncated_sections, "voice_samples");
        }
        if estimated_tokens > effective_budget && !scene_negatives.is_empty() {
            scene_negatives.clear();
            estimated_tokens = estimated_tokens.saturating_sub(scene_negatives_tokens);
            scene_negatives_tokens = 0;
            push_truncated(&mut truncated_sections, "scene_negatives");
        }
        if estimated_tokens > effective_budget && compact_shelf_digest.is_some() {
            compact_shelf_digest = None;
            estimated_tokens = estimated_tokens.saturating_sub(shelf_digest_tokens);
            shelf_digest_tokens = 0;
            push_truncated(&mut truncated_sections, "compact_shelf_digest");
        }
        if estimated_tokens > effective_budget && !research.is_empty() {
            while estimated_tokens > effective_budget && !research.is_empty() {
                let removed = research.pop();
                if let Some(removed_item) = removed {
                    evidence_references
                        .retain(|r| !(r.kind == "research" && r.id == removed_item.id));
                }
                let new_research_tokens = if research.is_empty() {
                    0
                } else {
                    estimate_value_tokens(&research)
                };
                estimated_tokens = estimated_tokens
                    .saturating_sub(research_tokens)
                    .saturating_add(new_research_tokens);
                research_tokens = new_research_tokens;
            }
            push_truncated(&mut truncated_sections, "research");
        }

        const LOWER_SCENE_CONTEXT_SECTIONS: &[&str] = &[
            "semantic_references",
            "knowledge_briefing",
            "timeline_briefing",
            "economy_briefing",
            "future_knowledge_briefing",
            "pacing_directives",
            "narrative_promises_due",
            "system_overlays",
            "active_threads",
            "previous_scene_tail",
            "subjects",
            "agency_check",
            "relationships",
        ];
        for &sec_id in LOWER_SCENE_CONTEXT_SECTIONS {
            if estimated_tokens <= effective_budget {
                break;
            }
            if !truncated_sections.iter().any(|s| s == sec_id) {
                let sec_cost = untruncated_section_tokens.get(sec_id).copied().unwrap_or(0);
                if sec_id == "active_threads" {
                    scene_purpose.active_threads.clear();
                } else {
                    crate::format::apply_scene_context_bundle_trims(
                        &[sec_id.to_string()],
                        &mut scene_context.novel,
                        &mut scene_context.scene,
                    );
                }
                estimated_tokens = estimated_tokens.saturating_sub(sec_cost);
                push_truncated(&mut truncated_sections, sec_id);
            }
        }

        if estimated_tokens > effective_budget && existing_prose.is_some() {
            let other_tokens = estimated_tokens
                .saturating_sub(existing_prose_tokens)
                .max(hard_constraint_tokens);
            if effective_budget > other_tokens && effective_budget - other_tokens >= 8 {
                let allowed_tokens = effective_budget - other_tokens;
                if let Some(prose) = existing_prose.as_mut() {
                    *prose = truncate_chars(prose, allowed_tokens * 4);
                }
                let new_prose_tokens = existing_prose
                    .as_deref()
                    .map(spindle_core::context_bundle::estimate_text_tokens)
                    .unwrap_or(0);
                estimated_tokens = other_tokens.saturating_add(new_prose_tokens);
                existing_prose_tokens = new_prose_tokens;
                push_truncated(&mut truncated_sections, "existing_prose");
            }
        }

        if estimated_tokens > effective_budget && style_directive.is_some() {
            style_directive = None;
            estimated_tokens = estimated_tokens.saturating_sub(style_directive_tokens);
            style_directive_tokens = 0;
            push_truncated(&mut truncated_sections, "style_directive");
        }

        const UPPER_SCENE_CONTEXT_SECTIONS: &[&str] =
            &["characters", "world_state", "location", "reader_contract"];
        for &sec_id in UPPER_SCENE_CONTEXT_SECTIONS {
            if estimated_tokens <= effective_budget {
                break;
            }
            if !truncated_sections.iter().any(|s| s == sec_id) {
                let sec_cost = untruncated_section_tokens.get(sec_id).copied().unwrap_or(0);
                if sec_id == "reader_contract" {
                    reader_contract = crate::format::empty_reader_contract();
                } else {
                    crate::format::apply_scene_context_bundle_trims(
                        &[sec_id.to_string()],
                        &mut scene_context.novel,
                        &mut scene_context.scene,
                    );
                }
                estimated_tokens = estimated_tokens.saturating_sub(sec_cost);
                push_truncated(&mut truncated_sections, sec_id);
            }
        }

        if estimated_tokens > effective_budget && existing_prose.is_some() {
            let other_tokens = estimated_tokens
                .saturating_sub(existing_prose_tokens)
                .max(hard_constraint_tokens);
            if effective_budget > other_tokens && effective_budget - other_tokens >= 4 {
                let allowed_tokens = effective_budget - other_tokens;
                if let Some(prose) = existing_prose.as_mut() {
                    *prose = truncate_chars(prose, allowed_tokens * 4);
                }
                let new_prose_tokens = existing_prose
                    .as_deref()
                    .map(spindle_core::context_bundle::estimate_text_tokens)
                    .unwrap_or(0);
                estimated_tokens = other_tokens.saturating_add(new_prose_tokens);
                let _ = (
                    packet_tokens,
                    evidence_tokens,
                    voice_samples_tokens,
                    scene_negatives_tokens,
                    shelf_digest_tokens,
                    style_directive_tokens,
                    new_prose_tokens,
                );
            } else {
                existing_prose = None;
                estimated_tokens = other_tokens;
            }
            push_truncated(&mut truncated_sections, "existing_prose");
        }
        estimated_tokens = estimated_tokens.max(hard_constraint_tokens);

        // Compute included vs truncated sections.
        const PREPARE_SCENE_SECTIONS: &[&str] = &[
            "reader_contract",
            "style_directive",
            "world_rules",
            "subjects",
            "system_overlays",
            "timeline_briefing",
            "future_knowledge_briefing",
            "pacing_directives",
            "narrative_promises_due",
            "knowledge_briefing",
            "semantic_references",
            "economy_briefing",
            "active_threads",
            "previous_scene_tail",
            "location",
            "world_state",
            "characters",
            "relationships",
            "agency_check",
        ];
        let truncated_set: std::collections::HashSet<&str> =
            truncated_sections.iter().map(String::as_str).collect();
        let mut included_sections = vec!["hard_constraints".to_string()];
        for section in PREPARE_SCENE_SECTIONS {
            if !truncated_set.contains(*section) {
                included_sections.push((*section).to_string());
            }
        }
        for (section, was_present) in [
            ("compact_shelf_digest", had_shelf_digest),
            ("voice_samples", had_voice_samples),
            ("scene_negatives", had_scene_negatives),
            ("existing_prose", had_existing_prose),
            ("research", had_research),
            ("drafting_packet_v1", had_drafting_packet),
            ("evidence_references", had_evidence_references),
        ] {
            if was_present && !truncated_set.contains(section) {
                included_sections.push(section.to_string());
            }
        }

        let coverage = PrepareSceneCoverage {
            budget_tokens: effective_budget,
            estimated_tokens,
            hard_constraint_tokens,
            included_sections,
            truncated_sections,
            evidence_references,
            unavailable_passes,
        };

        Ok(PrepareSceneOutput {
            preparation_id,
            project_id: project.id,
            branch_id: branch.id,
            branch_name: branch.name,
            intent,
            placement,
            chapter_id: resolved_chapter_id,
            scene_id: target_scene.as_ref().map(|s| s.id.clone()),
            scene_version,
            context_revision_token,
            dependency_tokens,
            writer_cursor,
            scene_purpose,
            hard_constraints: scene_context.hard_constraints,
            reader_contract,
            style_directive,
            compact_shelf_digest,
            voice_samples,
            scene_negatives,
            novel: scene_context.novel,
            scene: scene_context.scene,
            existing_prose,
            research,
            drafting_packet_v1,
            coverage,
        })
    }

    /// Evaluate whether a previously prepared scene packet's dependencies are still fresh
    /// against current project state on the given branch and placement.
    pub async fn check_prepare_scene_freshness(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: &spindle_core::models::StoryPlacement,
        scene_id: Option<&str>,
        expected_dependencies: &spindle_core::models::ScenePreparationDependencies,
    ) -> Result<spindle_core::models::ScenePreparationFreshness> {
        let current_dependencies = self
            .compute_scene_preparation_dependencies(project_id, branch_id, placement, scene_id)
            .await?;
        let current_context_revision_token =
            composite_context_revision_token(&current_dependencies);

        let mut stale_components = Vec::new();
        if current_dependencies.scene_text_hash != expected_dependencies.scene_text_hash {
            stale_components.push("scene_text".to_string());
        }
        if current_dependencies.canon_snapshot_hash != expected_dependencies.canon_snapshot_hash {
            stale_components.push("canon".to_string());
        }
        if current_dependencies.style_revision_hash != expected_dependencies.style_revision_hash {
            stale_components.push("style".to_string());
        }
        if current_dependencies.plan_revision_hash != expected_dependencies.plan_revision_hash {
            stale_components.push("plan".to_string());
        }

        Ok(spindle_core::models::ScenePreparationFreshness {
            fresh: stale_components.is_empty(),
            current_context_revision_token,
            current_dependencies,
            stale_components,
        })
    }

    pub(crate) async fn compute_scene_preparation_dependencies(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: &spindle_core::models::StoryPlacement,
        scene_id: Option<&str>,
    ) -> Result<spindle_core::models::ScenePreparationDependencies> {
        let project = self.repository.get_project(project_id).await?;
        let scenes = self
            .repository
            .list_scenes_resolved_for_branch(project_id, branch_id)
            .await?;
        let scene_order = placement.scene_order.unwrap_or(1);

        let target_scene = if let Some(id) = scene_id {
            scenes.iter().find(|s| s.id == id).cloned()
        } else {
            scenes
                .iter()
                .find(|s| {
                    s.book_number == placement.book_number
                        && s.chapter_number == placement.chapter_number
                        && s.scene_order == scene_order
                })
                .cloned()
        };

        let scene_text_hash = if let Some(scene) = target_scene.as_ref() {
            let versions = self.repository.list_scene_versions(&scene.id).await?;
            let version_number = versions.last().map(|v| v.version_number).unwrap_or(1);
            let payload = serde_json::json!({
                "id": scene.id,
                "version_number": version_number,
                "summary": scene.summary,
                "full_text": scene.full_text,
                "tone": scene.tone,
                "location_id": scene.location_id,
                "content_rating": scene.content_rating,
            });
            Some(format!(
                "scene:{}",
                &generation_sha256_hex(&serde_json::to_vec(&payload).unwrap_or_default())[..16]
            ))
        } else {
            None
        };

        let mut characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        characters.sort_by(|a, b| a.id.cmp(&b.id));
        let character_ids: Vec<String> = characters.iter().map(|c| c.id.clone()).collect();
        let mut character_states = Vec::new();
        for character in &characters {
            let state = self
                .repository
                .resolve_character_state_for_branch(
                    branch_id,
                    &character.id,
                    placement.book_number,
                    placement.chapter_number,
                    scene_order,
                )
                .await?;
            character_states.push(serde_json::json!({
                "id": character.id,
                "name": character.name,
                "role": character.role,
                "summary": character.summary,
                "aliases": character.aliases,
                "state": state.map(|s| serde_json::json!({
                    "emotional_state": s.emotional_state,
                    "goals": s.goals,
                    "status": s.status,
                    "notes": s.notes,
                })),
            }));
        }

        let mut relationships = if character_ids.is_empty() {
            Vec::new()
        } else {
            self.repository
                .list_relationships_for_characters(branch_id, &character_ids)
                .await?
        };
        relationships.sort_by(|a, b| (&a.in_id, &a.out_id).cmp(&(&b.in_id, &b.out_id)));
        let relationship_entries: Vec<_> = relationships
            .iter()
            .map(|r| {
                serde_json::json!({
                    "in_id": r.in_id,
                    "out_id": r.out_id,
                    "relationship_type": r.relationship_type,
                    "trust": r.trust,
                    "tension": r.tension,
                    "dynamics": r.dynamics,
                })
            })
            .collect();

        let mut locations = self
            .repository
            .list_locations_by_project_and_branch(project_id, branch_id)
            .await?;
        locations.sort_by(|a, b| a.id.cmp(&b.id));
        let mut location_entries = Vec::new();
        for loc in &locations {
            let ws = self
                .repository
                .get_world_state_for_location(branch_id, &loc.id)
                .await
                .ok()
                .flatten();
            location_entries.push(serde_json::json!({
                "id": loc.id,
                "name": loc.name,
                "kind": loc.kind,
                "realm": loc.realm,
                "summary": loc.summary,
                "world_state": ws.map(|w| serde_json::json!({
                    "controlling_faction": w.controlling_faction,
                    "status": w.status,
                    "prosperity": w.prosperity,
                    "stability": w.stability,
                    "threat_level": w.threat_level,
                    "sensory_details": w.sensory_details,
                })),
            }));
        }

        let all_world_rules = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let mut lore_rules: Vec<_> = all_world_rules
            .iter()
            .filter(|r| !r.rule_type.eq_ignore_ascii_case("style"))
            .map(|r| (&r.id, &r.rule_name, &r.rule_type, &r.description, &r.notes))
            .collect();
        lore_rules.sort_by(|a, b| a.0.cmp(b.0));

        let mut style_rules: Vec<_> = all_world_rules
            .iter()
            .filter(|r| r.rule_type.eq_ignore_ascii_case("style"))
            .map(|r| (&r.id, &r.rule_name, &r.description))
            .collect();
        style_rules.sort_by(|a, b| a.0.cmp(b.0));

        let mut canonical_facts = self
            .repository
            .list_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        canonical_facts.sort_by(|a, b| a.id.cmp(&b.id));
        let fact_entries: Vec<_> = canonical_facts
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "subject_table": f.subject_table,
                    "subject_id": f.subject_id,
                    "predicate": f.predicate,
                    "value_kind": f.value_kind,
                    "value_text": f.value_text,
                    "value_number": f.value_number,
                    "unit": f.unit,
                    "value_json": f.value_json,
                    "secret": f.secret,
                    "valid_from": f.valid_from,
                    "valid_until": f.valid_until,
                })
            })
            .collect();

        let mut knowledge_facts = self
            .repository
            .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        knowledge_facts.sort_by(|a, b| a.id.cmp(&b.id));
        let knowledge_entries: Vec<_> = knowledge_facts
            .iter()
            .map(|k| {
                (
                    &k.id,
                    &k.character_id,
                    &k.fact,
                    &k.learned_at,
                    &k.secret_of_fact_id,
                )
            })
            .collect();

        let mut future_knowledge = self
            .repository
            .list_future_knowledge_by_project_and_branch(project_id, branch_id)
            .await?;
        future_knowledge.sort_by(|a, b| a.id.cmp(&b.id));
        let future_knowledge_entries: Vec<_> = future_knowledge
            .iter()
            .map(|fk| {
                (
                    &fk.id,
                    &fk.character_id,
                    &fk.knowledge_summary,
                    &fk.learned_at,
                    &fk.expires_at,
                )
            })
            .collect();

        let mut promises = self
            .repository
            .list_narrative_promises_by_project_and_branch(project_id, branch_id)
            .await?;
        promises.sort_by(|a, b| a.id.cmp(&b.id));
        let promise_entries: Vec<_> = promises
            .iter()
            .map(|p| (&p.id, &p.description, &p.status, &p.planned_payoff))
            .collect();

        let mut timeline_events = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, branch_id)
            .await?;
        timeline_events.sort_by(|a, b| a.id.cmp(&b.id));
        let timeline_entries: Vec<_> = timeline_events
            .iter()
            .map(|t| (&t.id, &t.title, &t.event_type, &t.summary, &t.placement))
            .collect();

        let cursor_index = crate::format::story_index(
            placement.book_number,
            placement.chapter_number,
            scene_order,
        );
        let project_calendar = self
            .repository
            .get_project_calendar(project_id)
            .await?
            .map(|c| c.calendar);
        let current_clock = self
            .repository
            .latest_scene_clock_at_or_before(project_id, branch_id, cursor_index)
            .await?;
        let prior_clock = if cursor_index > 0 {
            self.repository
                .latest_scene_clock_at_or_before(project_id, branch_id, cursor_index - 1)
                .await?
        } else {
            None
        };
        let mut scene_clocks = self
            .repository
            .list_scene_clocks_by_project_and_branch(project_id, branch_id)
            .await?;
        scene_clocks.sort_by(|a, b| a.scene_id.cmp(&b.scene_id));
        let clock_entries: Vec<_> = scene_clocks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "scene_id": c.scene_id,
                    "day_index": c.clock.day_index,
                    "time_of_day": c.clock.time_of_day,
                    "duration_days": c.clock.duration_days,
                    "precision": c.clock.precision,
                    "temporal_mode": c.temporal_mode,
                    "thread_key": c.thread_key,
                })
            })
            .collect();

        let quantity_schemes = self
            .repository
            .list_project_quantity_schemes(project_id, branch_id)
            .await?;
        let scheme_entries: Vec<_> = quantity_schemes.iter().map(|s| &s.scheme).collect();
        let quantity_states = self
            .repository
            .list_quantity_states_by_project_and_branch(project_id, branch_id)
            .await?;
        let quantity_state_entries: Vec<_> = quantity_states
            .iter()
            .filter(|q| {
                crate::format::story_index(q.book_number, q.chapter_number, q.scene_order)
                    <= cursor_index
            })
            .map(|q| {
                serde_json::json!({
                    "id": q.id,
                    "subject_table": q.subject_table,
                    "subject_id": q.subject_id,
                    "measure": q.measure,
                    "amount": q.state.amount,
                    "unit": q.state.unit,
                    "band": q.state.band,
                    "book_number": q.book_number,
                    "chapter_number": q.chapter_number,
                    "scene_order": q.scene_order,
                })
            })
            .collect();

        let mut prior_scenes: Vec<_> = scenes
            .iter()
            .filter(|s| {
                (s.book_number, s.chapter_number, s.scene_order)
                    < (placement.book_number, placement.chapter_number, scene_order)
            })
            .collect();
        prior_scenes.sort_by_key(|s| (s.book_number, s.chapter_number, s.scene_order));
        let prior_scene_entries: Vec<_> = prior_scenes
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "book_number": s.book_number,
                    "chapter_number": s.chapter_number,
                    "scene_order": s.scene_order,
                    "summary": s.summary,
                    "full_text": s.full_text,
                    "content_rating": s.content_rating,
                    "location_id": s.location_id,
                })
            })
            .collect();

        let chapter_summaries = self
            .repository
            .list_chapter_summaries_by_project_and_branch(project_id, branch_id)
            .await?;
        let prior_chapter_summaries: Vec<_> = chapter_summaries
            .iter()
            .filter(|s| {
                (s.book_number, s.chapter_number)
                    < (placement.book_number, placement.chapter_number)
            })
            .map(|s| (&s.id, s.book_number, s.chapter_number, &s.summary))
            .collect();

        let canon_payload = serde_json::json!({
            "characters": character_states,
            "relationships": relationship_entries,
            "locations": location_entries,
            "lore_rules": lore_rules,
            "canonical_facts": fact_entries,
            "knowledge_facts": knowledge_entries,
            "future_knowledge": future_knowledge_entries,
            "promises": promise_entries,
            "timeline_events": timeline_entries,
            "project_calendar": project_calendar,
            "current_clock": current_clock.as_ref().map(|c| (&c.scene_id, c.clock.day_index, c.clock.time_of_day, c.clock.duration_days, &c.clock.precision)),
            "prior_clock": prior_clock.as_ref().map(|c| (&c.scene_id, c.clock.day_index, c.clock.time_of_day, c.clock.duration_days, &c.clock.precision)),
            "scene_clocks": clock_entries,
            "quantity_schemes": scheme_entries,
            "quantity_states": quantity_state_entries,
            "prior_scenes": prior_scene_entries,
            "prior_chapter_summaries": prior_chapter_summaries,
        });
        let canon_snapshot_hash = format!(
            "canon:{}",
            &generation_sha256_hex(&serde_json::to_vec(&canon_payload).unwrap_or_default())[..16]
        );

        let active_style_profile =
            if let Some(profile_id) = project.active_style_profile_id.as_deref() {
                self.repository
                    .get_style_profile(project_id, profile_id)
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
        let style_payload = serde_json::json!({
            "genre": project.genre,
            "project_type": project.project_type,
            "reader_contract": project.reader_contract,
            "narrator_voice": project.narrator_voice,
            "active_style_profile_id": project.active_style_profile_id,
            "active_style_profile": active_style_profile.map(|p| serde_json::json!({
                "profile_id": p.profile_id,
                "name": p.name,
                "guidance": p.guidance,
            })),
            "style_rules": style_rules,
        });
        let style_revision_hash = format!(
            "style:{}",
            &generation_sha256_hex(&serde_json::to_vec(&style_payload).unwrap_or_default())[..16]
        );

        let chapters = self.repository.list_chapters_by_project(project_id).await?;
        let chapter_record = chapters.iter().find(|ch| {
            ch.book_number == placement.book_number && ch.chapter_number == placement.chapter_number
        });
        let chapter_outline = if let Some(ch) = chapter_record {
            self.repository
                .get_chapter_outline(&ch.id, branch_id)
                .await?
        } else {
            None
        };
        let chapter_plan = self
            .repository
            .list_chapter_plans_by_project_and_branch(project_id, branch_id)
            .await?
            .into_iter()
            .find(|plan| {
                plan.book_number == placement.book_number
                    && plan.chapter_number == placement.chapter_number
            });
        let plan_payload = serde_json::json!({
            "chapter": chapter_record.map(|ch| serde_json::json!({
                "id": ch.id,
                "title": ch.title,
            })),
            "chapter_plan": chapter_plan.map(|plan| serde_json::json!({
                "id": plan.id,
                "synopsis": plan.synopsis,
                "pov_character_id": plan.pov_character_id,
                "scenes": plan.scenes,
                "plan_revision": plan.plan_revision,
                "target_theme_ids": plan.target_theme_ids,
                "target_conflict_ids": plan.target_conflict_ids,
                "target_plot_line_ids": plan.target_plot_line_ids,
            })),
            "chapter_outline": chapter_outline.map(|outline| serde_json::json!({
                "format": outline.format,
                "content": outline.content,
                "beats": outline.beats,
            })),
        });
        let plan_revision_hash = format!(
            "plan:{}",
            &generation_sha256_hex(&serde_json::to_vec(&plan_payload).unwrap_or_default())[..16]
        );

        Ok(spindle_core::models::ScenePreparationDependencies {
            scene_text_hash,
            canon_snapshot_hash,
            style_revision_hash,
            plan_revision_hash,
        })
    }
}

pub(crate) struct SceneContextWithCoverage {
    pub output: spindle_core::models::SceneContextOutput,
    pub bundle_estimated_tokens: usize,
    pub hard_constraint_tokens: usize,
    pub truncated_section_ids: Vec<String>,
    pub unavailable_passes: Vec<spindle_core::models::PrepareSceneUnavailablePass>,
}

pub(super) fn composite_context_revision_token(
    deps: &spindle_core::models::ScenePreparationDependencies,
) -> String {
    let payload = serde_json::to_vec(deps).unwrap_or_default();
    format!("ctxrev:{}", &generation_sha256_hex(&payload)[..16])
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    if keep == 0 {
        return text.chars().take(max_chars).collect();
    }
    let mut out: String = text.chars().take(keep).collect();
    out.push_str("...");
    out
}

fn estimate_value_tokens(value: &impl serde::Serialize) -> usize {
    serde_json::to_value(value)
        .map(|v| spindle_core::context_bundle::estimate_json_tokens(&v))
        .unwrap_or(0)
}

fn purpose_and_cursor_tokens(
    purpose: &spindle_core::models::PrepareScenePurpose,
    cursor: &spindle_core::models::PrepareSceneCursor,
) -> usize {
    let mut text = String::new();
    if let Some(s) = purpose.chapter_synopsis.as_deref() {
        text.push_str(s);
        text.push('\n');
    }
    if let Some(s) = purpose.scene_title.as_deref() {
        text.push_str(s);
        text.push('\n');
    }
    if let Some(s) = purpose.scene_summary.as_deref() {
        text.push_str(s);
        text.push('\n');
    }
    for beat in &purpose.expected_beats {
        text.push_str(beat);
        text.push('\n');
    }
    if let Some(s) = cursor.notes.as_deref() {
        text.push_str(s);
        text.push('\n');
    }
    spindle_core::context_bundle::estimate_text_tokens(&text)
}

fn scene_context_section_token_map(
    scene_context: &spindle_core::models::SceneContextOutput,
) -> std::collections::BTreeMap<String, usize> {
    use spindle_core::models::ContextFormat;
    let bundle = crate::format::build_scene_context_bundle(
        ContextFormat::Markdown,
        usize::MAX,
        &scene_context.hard_constraints,
        &scene_context.novel.subjects,
        &scene_context.novel,
        &scene_context.scene,
    );
    bundle
        .sections()
        .iter()
        .map(|sec| {
            (
                sec.id().to_string(),
                sec.token_estimate(ContextFormat::Markdown),
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_drafting_packet_v1_for_prepare_scene(
    preparation_id: &str,
    project_id: &str,
    branch_id: &str,
    target_scene: Option<&crate::sqlite::records::Scene>,
    scene_version: Option<i32>,
    placement: &spindle_core::models::StoryPlacement,
    dependencies: &spindle_core::models::ScenePreparationDependencies,
    purpose: &spindle_core::models::PrepareScenePurpose,
    context: &spindle_core::models::SceneContextOutput,
    _research: &[spindle_core::models::PrepareSceneResearchItem],
) -> std::result::Result<spindle_core::drafting::DraftingPacketV1, String> {
    use spindle_core::drafting::{
        CraftRubric, DraftingPacketV1, EditorialMemory, EmotionShift, NarrativePerson,
        NarrativeTense, PacketChapter, PacketContinuity, PacketLocation, PacketParticipant,
        PacketPov, PacketSource, PacketVoice, SceneCardV1, TargetWords, ValueShift,
    };

    let pov_character = if let Some(pov_id) = purpose.pov_character_id.as_deref() {
        context
            .scene
            .characters
            .iter()
            .find(|c| c.character_id == pov_id)
            .or_else(|| context.scene.characters.first())
    } else {
        context.scene.characters.first()
    };

    let Some(pov_char) = pov_character else {
        return Err(
            "no POV or scene character available to populate DraftingPacketV1.scene_card.pov"
                .to_string(),
        );
    };

    let plan_source = PacketSource {
        branch_id: branch_id.to_string(),
        source_id: format!(
            "plan:{}:{}:{}",
            placement.book_number,
            placement.chapter_number,
            placement.scene_order.unwrap_or(1)
        ),
        revision_hash: dependencies.plan_revision_hash.clone(),
    };
    let canon_source = PacketSource {
        branch_id: branch_id.to_string(),
        source_id: format!("canon:{project_id}"),
        revision_hash: dependencies.canon_snapshot_hash.clone(),
    };

    let mut source_versions = vec![plan_source.clone(), canon_source.clone()];
    source_versions.sort();
    source_versions.dedup();

    let raw_goal = purpose
        .scene_summary
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| purpose.expected_beats.first().map(String::as_str))
        .or(purpose.chapter_synopsis.as_deref())
        .unwrap_or("Advance the planned scene beat");
    let goal = truncate_chars(raw_goal, 160);

    let raw_turn = purpose
        .expected_beats
        .last()
        .map(String::as_str)
        .or(purpose.scene_summary.as_deref())
        .unwrap_or("Deliver the scene's narrative turn");
    let turn = truncate_chars(raw_turn, 160);

    let participants = context
        .scene
        .characters
        .iter()
        .map(|c| PacketParticipant {
            id: c.character_id.clone(),
            display_name: c.name.clone(),
            role: c.role.clone(),
        })
        .collect();

    let entry_emotion = if pov_char.emotional_state.is_empty() {
        "focused".to_string()
    } else {
        serde_json::to_string(&pov_char.emotional_state).unwrap_or_else(|_| "focused".to_string())
    };

    Ok(DraftingPacketV1 {
        schema_version: "drafting_packet_v1".to_string(),
        packet_id: preparation_id.to_string(),
        project_id: project_id.to_string(),
        branch_id: branch_id.to_string(),
        run_id: None,
        scene_id: target_scene.map(|s| s.id.clone()),
        scene_revision: scene_version.map(|v| v.max(1).to_string()),
        canon_snapshot_hash: dependencies.canon_snapshot_hash.clone(),
        plan_revision: dependencies.plan_revision_hash.clone(),
        source_versions,
        scene_card: SceneCardV1 {
            source: plan_source,
            pov: PacketPov {
                character_id: pov_char.character_id.clone(),
                display_name: pov_char.name.clone(),
                person: NarrativePerson::Third,
                tense: NarrativeTense::Past,
                source: canon_source,
            },
            goal: goal.clone(),
            opposition: "Scene conflict".to_string(),
            turn,
            intended_outcome: goal,
            value_shift: ValueShift {
                axis: "narrative".to_string(),
                before: "opening".to_string(),
                after: "shifted".to_string(),
            },
            emotion: EmotionShift {
                entry: entry_emotion,
                exit: "changed".to_string(),
            },
            chapter: PacketChapter {
                book_number: placement.book_number.max(1) as u32,
                number: placement.chapter_number.max(1) as u32,
                scene_index: placement.scene_order.unwrap_or(1).max(1) as u32,
                planned_scene_count: None,
                is_closer: false,
            },
            target_words: TargetWords {
                min: 800,
                preferred: 1_500,
                max: 3_000,
            },
            location: PacketLocation {
                id: context.scene.location.location_id.clone(),
                name: context.scene.location.name.clone(),
            },
            story_clock: String::new(),
            rating: purpose.content_rating.clone(),
            tone: None,
            participants,
            required_beats: purpose
                .expected_beats
                .iter()
                .map(|b| truncate_chars(b, 120))
                .collect(),
            promises_due: context
                .novel
                .narrative_promises_due
                .iter()
                .map(|p| truncate_chars(&p.description, 120))
                .collect(),
            prohibited_outcomes: Vec::new(),
        },
        hard_constraints: Vec::new(),
        continuity: PacketContinuity::default(),
        voice: PacketVoice::default(),
        editorial_memory: EditorialMemory::default(),
        craft_rubric: CraftRubric {
            version: crate::drafting::RUBRIC_VERSION.to_string(),
            text: String::new(),
        },
        research_evidence: Vec::new(),
        assembly: None,
    })
}
