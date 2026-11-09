use super::*;

pub(super) fn log_tool_call_timing(name: &str, lock_wait_ms: u64, total_ms: u64, is_error: bool) {
    let slow_threshold_ms = std::env::var("SPINDLE_SLOW_TOOL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000);
    if matches!(std::env::var("SPINDLE_PERF_LOG").as_deref(), Ok("1"))
        || lock_wait_ms > 0
        || total_ms >= slow_threshold_ms
    {
        tracing::info!(
            tool = name,
            lock_wait_ms,
            total_ms,
            is_error,
            "mcp_tool_call_timing"
        );
    }
}

impl ToolRouter {
    #[cfg(test)]
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
    ) -> anyhow::Result<CallToolResult> {
        self.call_tool_with_internal_token(name, arguments, None)
            .await
    }

    pub async fn call_tool_with_internal_token(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
        internal_token: Option<&str>,
    ) -> anyhow::Result<CallToolResult> {
        Box::pin(self.call_tool_with_internal_token_inner(name, arguments, internal_token)).await
    }

    pub(super) async fn call_tool_with_internal_token_inner(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
        internal_token: Option<&str>,
    ) -> anyhow::Result<CallToolResult> {
        let call_started = Instant::now();
        // Operator-only tools stay hidden from model transports. A server-issued
        // internal token (used by the authoring harness back-dispatch) may reach
        // the internal-only arms below; external callers cannot.
        let internal_call = self.serialization_state.is_internal_token(internal_token);
        if !self.has_tool(name) && !(internal_call && static_tool_names().contains(name)) {
            return Ok(structured_error_result(&anyhow::anyhow!(
                "unknown tool: {name}"
            )));
        }
        let resolved_arguments = match self.resolve_arguments(name, arguments).await {
            Ok(arguments) => arguments,
            Err(error) => return Ok(structured_error_result(&error)),
        };
        let lock_started = Instant::now();
        let scope = match self
            .tool_serialization_scope(name, &resolved_arguments)
            .await
        {
            Ok(scope) => scope,
            Err(error) => return Ok(structured_error_result(&error)),
        };

        let reentrant_lock = scope.as_ref().and_then(|s| {
            self.serialization_state
                .reentrant_lock_for_token(internal_token, s)
        });

        // Observability under a long write. Tool calls for a project are
        // serialized, and a drafting/review step can hold that lock for MINUTES
        // when the route points at a reasoning model. `authoring_status` then
        // queued behind the very run it reports on and timed out — the operator
        // lost the one tool that could tell them whether the slow step was still
        // alive, exactly when they needed it.
        //
        // So status degrades instead of blocking: if the project lock is not
        // free promptly, serve the non-persisting `authoring_status_readonly`
        // reconcile. The answer is identical; it just does not write the
        // reconcile back, which is precisely what makes it safe to run
        // alongside the in-flight writer.
        if name == "authoring_status"
            && reentrant_lock.is_none()
            && let Some(scope) = scope.as_ref()
            && !self.scope_is_free(scope).await
        {
            return match parse_arguments::<AuthoringStatusInput>(Some(&resolved_arguments)) {
                Ok(input) => match self.authoring_status_readonly(input).await {
                    Ok(Some(output)) => structured_result(&output)
                        .or_else(|error| Ok(structured_error_result(&error))),
                    Ok(None) => Ok(structured_error_result(&anyhow::anyhow!(
                        "No active or latest authoring run found for project"
                    ))),
                    Err(error) => Ok(structured_error_result(&error)),
                },
                Err(error) => Ok(structured_error_result(&error)),
            };
        }

        let _serialization_guard = match (scope, reentrant_lock) {
            (Some(_), Some(reentrant)) => Some(ToolSerializationGuard::Reentrant {
                _reentrant: reentrant.lock_owned().await,
            }),
            (Some(scope), None) => Some(self.lock_tool_scope(scope).await),
            (None, _) => None,
        };
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;
        let arguments = Some(&resolved_arguments);
        let result = match name {
            "create_project" => match parse_arguments::<CreateProjectInput>(arguments) {
                Ok(input) => match self.service.create_project(input).await {
                    Ok(output) => {
                        // Per-project main branch (Phase 6): use the
                        // service-returned branch_id rather than the legacy
                        // hardcoded singleton.
                        self.set_session_context(
                            output.project_id.clone(),
                            Some(output.branch_id.clone()),
                        )
                        .await;
                        structured_result(&output)
                    }
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            "list_projects" => structured_result(&self.service.list_projects().await?),
            "set_active_project" => match parse_arguments::<SetActiveProjectInput>(arguments) {
                Ok(input) => match self.set_active_project(input).await {
                    Ok(output) => structured_result(&output),
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            "create_book" => {
                self.invoke(arguments, |input| self.service.create_book(input))
                    .await
            }
            "create_chapter" => {
                self.invoke(arguments, |input| self.service.create_chapter(input))
                    .await
            }
            "create_branch" => {
                self.invoke(arguments, |input| self.service.create_branch(input))
                    .await
            }
            "switch_branch" => match parse_arguments::<SwitchBranchInput>(arguments) {
                Ok(input) => {
                    let project_id = input.project_id.clone();
                    match self.service.switch_branch(input).await {
                        Ok(output) => {
                            self.set_session_context(project_id, Some(output.branch_id.clone()))
                                .await;
                            structured_result(&output)
                        }
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            },
            "set_narrator_voice" => {
                self.invoke(arguments, |input| self.service.set_narrator_voice(input))
                    .await
            }
            "set_project_calendar" => {
                self.invoke(arguments, |input| self.service.set_project_calendar(input))
                    .await
            }
            "set_scene_clock" => {
                self.invoke(arguments, |input| self.service.set_scene_clock(input))
                    .await
            }
            "set_timeline_event_clock" => {
                self.invoke(arguments, |input| {
                    self.service.set_timeline_event_clock(input)
                })
                .await
            }
            "set_character_birth" => {
                self.invoke(arguments, |input| self.service.set_character_birth(input))
                    .await
            }
            "set_project_quantity_scheme" => {
                self.invoke(arguments, |input| {
                    self.service.set_project_quantity_scheme(input)
                })
                .await
            }
            "commit_quantity_state" => {
                self.invoke(arguments, |input| self.service.commit_quantity_state(input))
                    .await
            }
            "derive_quantity_scheme_from_system_overlay" => {
                self.invoke(arguments, |input| {
                    self.service
                        .derive_quantity_scheme_from_system_overlay(input)
                })
                .await
            }
            "scan_scene_prices" => {
                self.invoke(arguments, |input| self.service.scan_scene_prices(input))
                    .await
            }
            "create_save_point" => {
                self.invoke(arguments, |input| self.service.create_save_point(input))
                    .await
            }
            "restore_save_point" => {
                self.invoke(arguments, |input| self.service.restore_save_point(input))
                    .await
            }
            "diff_branches" => {
                self.invoke(arguments, |input| self.service.diff_branches(input))
                    .await
            }
            "create_style_profile_from_markdown" => {
                self.invoke(arguments, |input| {
                    self.service.create_style_profile_from_markdown(input)
                })
                .await
            }
            "check_style_profile_sources" => {
                self.invoke(arguments, |input| {
                    self.service.check_style_profile_sources(input)
                })
                .await
            }
            "preview_refresh_style_profile" => {
                self.invoke(arguments, |input| {
                    self.service.preview_refresh_style_profile(input)
                })
                .await
            }
            "refresh_style_profile" => {
                self.invoke(arguments, |input| self.service.refresh_style_profile(input))
                    .await
            }
            "list_style_profiles" => {
                self.invoke(arguments, |input| self.service.list_style_profiles(input))
                    .await
            }
            "get_style_profile" => {
                self.invoke(arguments, |input| self.service.get_style_profile(input))
                    .await
            }
            "apply_style_profile" => {
                self.invoke(arguments, |input| self.service.apply_style_profile(input))
                    .await
            }
            "preview_apply_style_profile" => {
                self.invoke(arguments, |input| {
                    self.service.preview_apply_style_profile(input)
                })
                .await
            }
            "list_style_profile_applications" => {
                self.invoke(arguments, |input| {
                    self.service.list_style_profile_applications(input)
                })
                .await
            }
            "rollback_style_profile_application" => {
                self.invoke(arguments, |input| {
                    self.service.rollback_style_profile_application(input)
                })
                .await
            }
            "check_style_against_profile" => {
                self.invoke(arguments, |input| {
                    self.service.check_style_against_profile(input)
                })
                .await
            }
            "plan_style_revision" => {
                self.invoke(arguments, |input| self.service.plan_style_revision(input))
                    .await
            }
            "compare_style_profiles" => {
                self.invoke(arguments, |input| {
                    self.service.compare_style_profiles(input)
                })
                .await
            }
            "archive_style_profile" => {
                self.invoke(arguments, |input| self.service.archive_style_profile(input))
                    .await
            }
            "create_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.create_style_revision_patch(input)
                })
                .await
            }
            "preview_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.preview_style_revision_patch(input)
                })
                .await
            }
            "evaluate_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.evaluate_style_revision_patch(input)
                })
                .await
            }
            "apply_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.apply_style_revision_patch(input)
                })
                .await
            }
            "list_style_revision_patch_audits" => {
                self.invoke(arguments, |input| {
                    self.service.list_style_revision_patch_audits(input)
                })
                .await
            }
            "rollback_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.rollback_style_revision_patch(input)
                })
                .await
            }
            "merge_branch" => {
                self.invoke(arguments, |input| self.service.merge_branch(input))
                    .await
            }
            "revise_scene" => {
                self.invoke(arguments, |input| self.service.revise_scene(input))
                    .await
            }
            "generate_alternatives" => {
                self.invoke(arguments, |input| self.service.generate_alternatives(input))
                    .await
            }
            "compare_alternatives" => {
                self.invoke(arguments, |input| self.service.compare_alternatives(input))
                    .await
            }
            "select_alternative" => {
                self.invoke(arguments, |input| self.service.select_alternative(input))
                    .await
            }
            "list_revision_markers" => {
                self.invoke(arguments, |input| self.service.list_revision_markers(input))
                    .await
            }
            "resolve_revision_marker" => {
                self.invoke(arguments, |input| {
                    self.service.resolve_revision_marker(input)
                })
                .await
            }
            "create_character" => {
                self.invoke(arguments, |input| self.service.create_character(input))
                    .await
            }
            "create_location" => {
                self.invoke(arguments, |input| self.service.create_location(input))
                    .await
            }
            "create_faction" => {
                self.invoke(arguments, |input| self.service.create_faction(input))
                    .await
            }
            "create_religion" => {
                self.invoke(arguments, |input| self.service.create_religion(input))
                    .await
            }
            "create_economy" => {
                self.invoke(arguments, |input| self.service.create_economy(input))
                    .await
            }
            "create_term" => {
                self.invoke(arguments, |input| self.service.create_term(input))
                    .await
            }
            "create_terms" => {
                self.invoke(arguments, |input| self.service.create_terms(input))
                    .await
            }
            "batch_create_terms" => {
                self.invoke(arguments, |input| self.service.batch_create_terms(input))
                    .await
            }
            "create_relationship" => {
                self.invoke(arguments, |input| self.service.create_relationship(input))
                    .await
            }
            "create_world_rule" => {
                self.invoke(arguments, |input| self.service.create_world_rule(input))
                    .await
            }
            "update_character" => {
                self.invoke(arguments, |input| self.service.update_character(input))
                    .await
            }
            "update_location" => {
                self.invoke(arguments, |input| self.service.update_location(input))
                    .await
            }
            "update_conflict" => {
                self.invoke(arguments, |input| self.service.update_conflict(input))
                    .await
            }
            "update_character_arc" => {
                self.invoke(arguments, |input| self.service.update_character_arc(input))
                    .await
            }
            "update_entity" => {
                self.invoke(arguments, |input| self.service.update_entity(input))
                    .await
            }
            "archive_entity" => {
                self.invoke(arguments, |input| self.service.archive_entity(input))
                    .await
            }
            "create_plot_line" => {
                self.invoke(arguments, |input| self.service.create_plot_line(input))
                    .await
            }
            "create_conflict" => {
                self.invoke(arguments, |input| self.service.create_conflict(input))
                    .await
            }
            "create_theme" => {
                self.invoke(arguments, |input| self.service.create_theme(input))
                    .await
            }
            "create_motif" => {
                self.invoke(arguments, |input| self.service.create_motif(input))
                    .await
            }
            "create_motifs" => {
                self.invoke(arguments, |input| self.service.create_motifs(input))
                    .await
            }
            "batch_create_motifs" => {
                self.invoke(arguments, |input| self.service.batch_create_motifs(input))
                    .await
            }
            "create_narrative_promise" => {
                self.invoke(arguments, |input| {
                    self.service.create_narrative_promise(input)
                })
                .await
            }
            "create_narrative_promises" => {
                self.invoke(arguments, |input| {
                    self.service.create_narrative_promises(input)
                })
                .await
            }
            "batch_create_narrative_promises" => {
                self.invoke(arguments, |input| {
                    self.service.batch_create_narrative_promises(input)
                })
                .await
            }
            "update_promise_status" => {
                self.invoke(arguments, |input| self.service.update_promise_status(input))
                    .await
            }
            "create_character_arc" => {
                self.invoke(arguments, |input| self.service.create_character_arc(input))
                    .await
            }
            "update_arc_milestone" => {
                self.invoke(arguments, |input| self.service.update_arc_milestone(input))
                    .await
            }
            "create_future_knowledge" => {
                self.invoke(arguments, |input| {
                    self.service.create_future_knowledge(input)
                })
                .await
            }
            "create_timeline_event" => {
                self.invoke(arguments, |input| self.service.create_timeline_event(input))
                    .await
            }
            "create_temporal_intervention" => {
                self.invoke(arguments, |input| {
                    self.service.create_temporal_intervention(input)
                })
                .await
            }
            "create_system_overlay" => {
                self.invoke(arguments, |input| self.service.create_system_overlay(input))
                    .await
            }
            "run_dual_persona_review" => {
                self.invoke(arguments, |input| {
                    self.service.run_dual_persona_review(input)
                })
                .await
            }
            "create_pacing_config" => {
                self.invoke(arguments, |input| self.service.create_pacing_config(input))
                    .await
            }
            "create_pacing_curve" => {
                self.invoke(arguments, |input| self.service.create_pacing_curve(input))
                    .await
            }
            "set_arc_pacing_constraints" => {
                self.invoke(arguments, |input| {
                    self.service.set_arc_pacing_constraints(input)
                })
                .await
            }
            "plan_chapter" => {
                self.invoke(arguments, |input| self.service.plan_chapter(input))
                    .await
            }
            "annotate_scene_beats" => {
                self.invoke(arguments, |input| self.service.annotate_scene_beats(input))
                    .await
            }
            "save_summary" => {
                self.invoke(arguments, |input| self.service.save_summary(input))
                    .await
            }
            "set_book_outline" => {
                self.invoke(arguments, |input| self.service.set_book_outline(input))
                    .await
            }
            "set_chapter_outline" => {
                self.invoke(arguments, |input| self.service.set_chapter_outline(input))
                    .await
            }
            "check_consistency" => {
                self.invoke(arguments, |input| self.service.check_consistency(input))
                    .await
            }
            "register_canonical_fact" => {
                self.invoke(arguments, |input| {
                    self.service.register_canonical_fact(input)
                })
                .await
            }
            "extract_canonical_facts_from_scene" => {
                self.invoke(arguments, |input| {
                    self.service.extract_canonical_facts_from_scene(input)
                })
                .await
            }
            "migrate_canonical_fact" => {
                self.invoke(arguments, |input| {
                    self.service.migrate_canonical_fact(input)
                })
                .await
            }
            "bind_canonical_fact_to_scene" => {
                self.invoke(arguments, |input| {
                    self.service.bind_canonical_fact_to_scene(input)
                })
                .await
            }
            "update_world_rule" => {
                self.invoke(arguments, |input| self.service.update_world_rule(input))
                    .await
            }
            "set_character_voice_profile" => {
                self.invoke(arguments, |input| {
                    self.service.set_character_voice_profile(input)
                })
                .await
            }
            "set_character_voice_profiles" => {
                self.invoke(arguments, |input| {
                    self.service.set_character_voice_profiles(input)
                })
                .await
            }
            "batch_set_character_voice_profiles" => {
                self.invoke(arguments, |input| {
                    self.service.batch_set_character_voice_profiles(input)
                })
                .await
            }
            "search_bible" => {
                self.invoke(arguments, |input| self.service.search_bible(input))
                    .await
            }
            "find_scenes_referencing" => {
                self.invoke(arguments, |input| {
                    self.service.find_scenes_referencing(input)
                })
                .await
            }
            "pull_chapter_from_file" => {
                self.invoke(arguments, |input| {
                    self.service.pull_chapter_from_file(input)
                })
                .await
            }
            "push_chapter_to_file" => {
                self.invoke(arguments, |input| self.service.push_chapter_to_file(input))
                    .await
            }
            "planning_begin" => {
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_planning_begin(input))
                })
                .await
            }
            "planning_advance" => {
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_planning_advance(input))
                })
                .await
            }
            "planning_status" => {
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_planning_status(input))
                })
                .await
            }
            "planning_decide" => {
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_planning_decide(input))
                })
                .await
            }
            "authoring_prepare_run" => {
                // Box::pin: the authoring handlers' futures are large enough
                // that un-boxed they push call_tool past the 2MB tokio worker
                // stack (same class as the decide_canon_deltas fan-out fix).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_prepare_run(input))
                })
                .await
            }
            "authoring_start_run" => {
                if let Some(rejection) = authoring_start_run_policy_rejection(arguments) {
                    structured_result(&rejection)
                } else {
                    self.invoke(arguments, |input| {
                        Box::pin(self.handle_authoring_start_run(input))
                    })
                    .await
                }
            }
            "authoring_status" => {
                self.invoke(arguments, |input| self.handle_authoring_status(input))
                    .await
            }
            "authoring_execute_next" => {
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_execute_next(input))
                })
                .await
            }
            "authoring_save_scene_draft" => {
                // Box::pin: grown by the journal emitters; keep call_tool below
                // the 2MB tokio worker stack (same precedent as the other
                // authoring handlers).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_save_scene_draft(input))
                })
                .await
            }
            "authoring_record_checkpoint_audit" => {
                self.invoke(arguments, |input| {
                    self.handle_authoring_record_checkpoint_audit(input)
                })
                .await
            }
            "authoring_review_checkpoint" => {
                // Box::pin: grown by the journal emitters (see above).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_review_checkpoint(input))
                })
                .await
            }
            "authoring_resolve_block" => {
                self.invoke(arguments, |input| {
                    self.handle_authoring_resolve_block(input)
                })
                .await
            }
            "authoring_cancel_run" => {
                // Box::pin: grown by the journal emitter (see above).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_cancel_run(input))
                })
                .await
            }
            "get_model_usage" => {
                self.invoke(arguments, |input| self.service.get_model_usage(input))
                    .await
            }
            "get_series_status" => {
                self.invoke(arguments, |input| self.service.get_series_status(input))
                    .await
            }
            "prepare_episode_release" => {
                self.invoke(arguments, |input| {
                    self.service.prepare_episode_release(input)
                })
                .await
            }
            "release_episode" => {
                self.invoke(arguments, |input| self.service.release_episode(input))
                    .await
            }
            "get_episode_release" => {
                self.invoke(arguments, |input| self.service.get_episode_release(input))
                    .await
            }
            "get_editorial_queue" => {
                self.invoke(arguments, |input| self.service.get_editorial_queue(input))
                    .await
            }
            "decide_editorial_item" => {
                self.invoke(arguments, |input| self.service.decide_editorial_item(input))
                    .await
            }
            "read_episode" => {
                self.invoke(arguments, |input| self.service.read_episode(input))
                    .await
            }
            "list_agents" => structured_result(&self.service.list_agents()?),
            "list_skills" => structured_result(&ListSkillsOutput {
                skills: spindle_adapters::list_skills()
                    .iter()
                    .map(|skill| SkillSummary {
                        name: skill.name.to_string(),
                        kind: "skill".to_string(),
                    })
                    .collect(),
                references: spindle_adapters::list_references()
                    .iter()
                    .map(|reference| SkillSummary {
                        name: reference.name.to_string(),
                        kind: "reference".to_string(),
                    })
                    .chain(std::iter::once(SkillSummary {
                        name: "contracts/tools/v1".to_owned(),
                        kind: "reference".to_owned(),
                    }))
                    .collect(),
            }),
            "get_skill" => match parse_arguments::<GetSkillInput>(arguments) {
                Ok(input) => match spindle_adapters::get_skill(&input.name) {
                    Some(skill) => structured_result(&self.resolve_skill_output(skill)),
                    None => Ok(structured_error_result(&anyhow::anyhow!(
                        "unknown skill: {} (use list_skills to see available skills)",
                        input.name
                    ))),
                },
                Err(error) => Err(error),
            },
            "get_reference" => match parse_arguments::<GetReferenceInput>(arguments) {
                Ok(input) if input.name.starts_with("contracts/") => {
                    match response_contract_reference(self, &input.name) {
                        Ok(output) => structured_result(&output),
                        Err(error) => Ok(structured_error_result(&error)),
                    }
                }
                Ok(input) => match spindle_adapters::get_reference(&input.name) {
                    Some(reference) => structured_result(&self.resolve_reference_output(reference)),
                    None => Ok(structured_error_result(&anyhow::anyhow!(
                        "unknown reference: {} (use list_skills to see available references)",
                        input.name
                    ))),
                },
                Err(error) => Err(error),
            },
            "test_agent" => {
                self.invoke(arguments, |input| self.service.test_agent(input))
                    .await
            }
            "continue_generation" => {
                self.invoke(arguments, |input| self.service.continue_generation(input))
                    .await
            }
            "revise_generation" => {
                self.invoke(arguments, |input| self.service.revise_generation(input))
                    .await
            }
            "research_query" => {
                self.invoke(arguments, |input| self.service.research_query(input))
                    .await
            }
            "research_add_source" => {
                self.invoke(arguments, |input| self.service.research_add_source(input))
                    .await
            }
            "research_add_note" => {
                self.invoke(arguments, |input| self.service.research_add_note(input))
                    .await
            }
            "research_add_claim" => {
                self.invoke(arguments, |input| self.service.research_add_claim(input))
                    .await
            }
            "research_search" => {
                self.invoke(arguments, |input| self.service.research_search(input))
                    .await
            }
            "research_pack_for_scene" => {
                self.invoke(arguments, |input| {
                    self.service.research_pack_for_scene(input)
                })
                .await
            }
            "research_usage_for_scene" => {
                self.invoke(arguments, |input| {
                    self.service.research_usage_for_scene(input)
                })
                .await
            }
            "research_ingest_report" => {
                self.invoke(arguments, |input| {
                    self.service.research_ingest_report(input)
                })
                .await
            }
            "research_plan_for_scene" => {
                self.invoke(arguments, |input| {
                    self.service.research_plan_for_scene(input)
                })
                .await
            }
            "export_epub" => {
                self.invoke(arguments, |input| self.service.export_epub(input))
                    .await
            }
            "preflight_book_export" => {
                self.invoke(arguments, |input| self.service.preflight_book_export(input))
                    .await
            }
            "compile_manuscript" => {
                self.invoke(arguments, |input| self.service.compile_manuscript(input))
                    .await
            }
            "export_recap" => {
                self.invoke(arguments, |input| self.service.export_recap(input))
                    .await
            }
            "export_series_bible" => {
                self.invoke(arguments, |input| self.service.export_series_bible(input))
                    .await
            }
            "mine_scene_canon" => {
                self.invoke(arguments, |input| self.service.mine_scene_canon(input))
                    .await
            }
            "list_canon_deltas" => {
                self.invoke(arguments, |input| self.service.list_canon_deltas(input))
                    .await
            }
            "decide_canon_deltas" => {
                self.invoke(arguments, |input| self.service.decide_canon_deltas(input))
                    .await
            }
            "replan_chapter" => {
                // Box::pin: the differ builds the realized/target digests + a
                // model dispatch; un-boxed it inflates the call_tool future past
                // the worker stack (same class as the mine/decide fan-outs).
                self.invoke(arguments, |input| {
                    Box::pin(self.service.replan_chapter(input))
                })
                .await
            }
            "list_plan_amendments" => {
                self.invoke(arguments, |input| self.service.list_plan_amendments(input))
                    .await
            }
            "decide_plan_amendments" => {
                // Box::pin: the apply dispatcher fans out per-class plan replays
                // + the immutability guard; un-boxed it inflates the call_tool
                // future past the worker stack (same class as decide_canon_deltas).
                self.invoke(arguments, |input| {
                    Box::pin(self.service.decide_plan_amendments(input))
                })
                .await
            }
            "export_bible" => {
                self.invoke(arguments, |input| self.service.export_bible(input))
                    .await
            }
            "get_writer_state" => {
                self.invoke(arguments, |input| {
                    self.service.get_writer_state_envelope(input)
                })
                .await
            }
            "get_scene_context" => {
                self.invoke(arguments, |input| {
                    self.service.get_scene_context_envelope(input)
                })
                .await
            }
            "prepare_scene" => {
                self.invoke(arguments, |input| self.service.prepare_scene(input))
                    .await
            }
            "commit_scene" => {
                self.invoke(arguments, |input| self.handle_commit_scene(input))
                    .await
            }
            "get_entity" => {
                self.invoke(arguments, |input| self.service.get_entity(input))
                    .await
            }
            "find_entity" => {
                self.invoke(arguments, |input| self.service.find_entity(input))
                    .await
            }
            "get_character_snapshot" => {
                self.invoke(arguments, |input| {
                    self.service.get_character_snapshot(input)
                })
                .await
            }
            "get_chapter_briefing" => {
                self.invoke(arguments, |input| self.service.get_chapter_briefing(input))
                    .await
            }
            "list_chapter_scenes" => {
                self.invoke(arguments, |input| self.service.list_chapter_scenes(input))
                    .await
            }
            "list_book_chapters" => {
                self.invoke(arguments, |input| self.service.list_book_chapters(input))
                    .await
            }
            "save_scene_draft" => {
                self.invoke(arguments, |input| self.service.save_scene_draft(input))
                    .await
            }
            "move_scene" => {
                self.invoke(arguments, |input| self.service.move_scene(input))
                    .await
            }
            "delete_scene" => {
                self.invoke(arguments, |input| self.service.delete_scene(input))
                    .await
            }
            "list_changes" => {
                self.invoke(arguments, |input| self.service.list_changes(input))
                    .await
            }
            "undo_change" => {
                self.invoke(arguments, |input| self.service.undo_change(input))
                    .await
            }
            "list_scene_versions" => {
                self.invoke(arguments, |input| self.service.list_scene_versions(input))
                    .await
            }
            "restore_scene_version" => {
                self.invoke(arguments, |input| self.service.restore_scene_version(input))
                    .await
            }
            "commit_scene_changes" => {
                self.invoke(arguments, |input| self.service.commit_scene_changes(input))
                    .await
            }
            "commit_character_state" => {
                self.invoke(arguments, |input| {
                    self.service.commit_character_state(input)
                })
                .await
            }
            "update_relationship" => {
                self.invoke(arguments, |input| self.service.update_relationship(input))
                    .await
            }
            "import_manuscript" => {
                self.invoke(arguments, |input| self.service.import_manuscript(input))
                    .await
            }
            "import_status" => {
                self.invoke(arguments, |input| self.service.import_status(input))
                    .await
            }
            "import_extract_entities" => {
                self.invoke(arguments, |input| {
                    self.service.import_extract_entities(input)
                })
                .await
            }
            "import_consolidate_entities" => {
                self.invoke(arguments, |input| {
                    self.service.import_consolidate_entities(input)
                })
                .await
            }
            "import_analyze_character" => {
                self.invoke(arguments, |input| {
                    self.service.import_analyze_character(input)
                })
                .await
            }
            "import_extract_world" => {
                self.invoke(arguments, |input| self.service.import_extract_world(input))
                    .await
            }
            "import_analyze_narrative" => {
                self.invoke(arguments, |input| {
                    self.service.import_analyze_narrative(input)
                })
                .await
            }
            "import_compute_final_state" => {
                self.invoke(arguments, |input| {
                    self.service.import_compute_final_state(input)
                })
                .await
            }
            "import_hydrate_bible" => {
                self.invoke(arguments, |input| self.service.import_hydrate_bible(input))
                    .await
            }
            "import_apply_review_decisions" => {
                self.invoke(arguments, |input| {
                    self.service.import_apply_review_decisions(input)
                })
                .await
            }
            "record_knowledge" => {
                self.invoke(arguments, |input| self.service.record_knowledge(input))
                    .await
            }
            "record_note" => {
                self.invoke(arguments, |input| self.service.record_note(input))
                    .await
            }
            "update_writer_position" => {
                self.invoke(arguments, |input| {
                    self.service.update_writer_position(input)
                })
                .await
            }
            _ => Err(anyhow::anyhow!("unknown tool: {name}")),
        };

        let is_error = result.is_err();
        log_tool_call_timing(
            name,
            lock_wait_ms,
            call_started.elapsed().as_millis() as u64,
            is_error,
        );

        Ok(match result {
            Ok(value) => value,
            Err(error) => structured_error_result(&error),
        })
    }

    pub(super) async fn invoke<I, O, F, Fut>(
        &self,
        arguments: Option<&rmcp::model::JsonObject>,
        f: F,
    ) -> anyhow::Result<CallToolResult>
    where
        I: DeserializeOwned + schemars::JsonSchema,
        O: Serialize,
        F: FnOnce(I) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<O>>,
    {
        let input: I = parse_arguments(arguments)?;
        let output = f(input).await?;
        structured_result(&output)
    }
}

fn authoring_start_run_policy_rejection(
    arguments: Option<&rmcp::model::JsonObject>,
) -> Option<AuthoringStartRunOutput> {
    let args = arguments?;
    if let Some(raw) = args.get("mode").and_then(|value| value.as_str())
        && let Err(rejected) = raw.parse::<AuthoringMode>()
    {
        return Some(AuthoringStartRunOutput {
            run_id: String::new(),
            status: "blocked".to_string(),
            message: format!(
                "Cannot start authoring run: mode must be one of {:?}, got {:?}",
                AUTHORING_MODES, rejected
            ),
        });
    }
    if let Some(raw) = args.get("mining_policy").and_then(|value| value.as_str())
        && let Err(rejected) = raw.parse::<AuthoringMiningPolicy>()
    {
        return Some(AuthoringStartRunOutput {
            run_id: String::new(),
            status: "blocked".to_string(),
            message: format!(
                "Cannot start authoring run: mining_policy must be one of {:?}, got {:?}",
                AUTHORING_MINING_POLICIES, rejected
            ),
        });
    }
    if let Some(raw) = args
        .get("checkpoint_policy")
        .and_then(|value| value.as_str())
        && let Err(rejected) = raw.parse::<AuthoringCheckpointPolicy>()
    {
        return Some(AuthoringStartRunOutput {
            run_id: String::new(),
            status: "blocked".to_string(),
            message: format!(
                "Cannot start authoring run: checkpoint_policy must be one of {:?}, got {:?}",
                AUTHORING_CHECKPOINT_POLICIES, rejected
            ),
        });
    }
    if let Some(raw) = args.get("replan_policy").and_then(|value| value.as_str())
        && let Err(rejected) = raw.parse::<AuthoringReplanPolicy>()
    {
        return Some(AuthoringStartRunOutput {
            run_id: String::new(),
            status: "blocked".to_string(),
            message: format!(
                "Cannot start authoring run: replan_policy must be one of {:?}, got {:?}",
                AUTHORING_REPLAN_POLICIES, rejected
            ),
        });
    }
    None
}
