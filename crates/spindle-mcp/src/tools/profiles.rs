use super::static_tool_names;
pub(super) fn profile_allowed_tool_names(profile: &str) -> Option<&'static [&'static str]> {
    match profile {
        "import" => Some(&[
            "create_project",
            "list_projects",
            "set_active_project",
            "list_skills",
            "get_skill",
            "get_reference",
            "import_manuscript",
            "import_status",
            "import_extract_entities",
            "import_consolidate_entities",
            "import_analyze_character",
            "import_extract_world",
            "import_analyze_narrative",
            "import_compute_final_state",
            "import_hydrate_bible",
            "import_apply_review_decisions",
            "record_knowledge",
            "record_note",
            "update_writer_position",
            "search_bible",
            "find_scenes_referencing",
            "get_chapter_briefing",
            "create_style_profile_from_markdown",
            "list_style_profiles",
            "get_style_profile",
            "apply_style_profile",
            "preview_apply_style_profile",
            "list_style_profile_applications",
            "rollback_style_profile_application",
            "check_style_profile_sources",
            "preview_refresh_style_profile",
            "refresh_style_profile",
            "check_style_against_profile",
            "plan_style_revision",
            "compare_style_profiles",
            "archive_style_profile",
            "preview_style_revision_patch",
            "evaluate_style_revision_patch",
            "list_style_revision_patch_audits",
            "rollback_style_revision_patch",
        ]),
        "write" => Some(&[
            "get_editorial_queue",
            "decide_editorial_item",
            "read_episode",
            "get_model_usage",
            "get_series_status",
            "prepare_episode_release",
            "release_episode",
            "get_episode_release",
            "create_project",
            "list_projects",
            "set_active_project",
            "list_skills",
            "get_skill",
            "get_reference",
            "create_book",
            "create_chapter",
            "create_character",
            "create_location",
            "create_faction",
            "create_religion",
            "create_economy",
            "create_term",
            "batch_create_terms",
            "create_relationship",
            "create_world_rule",
            "update_world_rule",
            "set_character_voice_profile",
            "batch_set_character_voice_profiles",
            "create_save_point",
            "restore_save_point",
            "create_plot_line",
            "create_conflict",
            "create_theme",
            "create_motif",
            "batch_create_motifs",
            "create_narrative_promise",
            "update_promise_status",
            "batch_create_narrative_promises",
            "create_character_arc",
            "update_arc_milestone",
            "create_system_overlay",
            "preflight_book_export",
            "compile_manuscript",
            "export_recap",
            "export_series_bible",
            "mine_scene_canon",
            "list_canon_deltas",
            "decide_canon_deltas",
            "replan_chapter",
            "list_plan_amendments",
            "decide_plan_amendments",
            "get_writer_state",
            "get_scene_context",
            "prepare_scene",
            "commit_scene",
            "get_entity",
            "find_entity",
            "get_character_snapshot",
            "get_chapter_briefing",
            "list_chapter_scenes",
            "list_book_chapters",
            "save_scene_draft",
            "move_scene",
            "delete_scene",
            "operator_delete_scene",
            "commit_scene_changes",
            "commit_character_state",
            "update_relationship",
            "record_note",
            "update_writer_position",
            "save_summary",
            "plan_chapter",
            "annotate_scene_beats",
            "set_book_outline",
            "set_chapter_outline",
            "update_entity",
            "search_bible",
            "find_scenes_referencing",
            "check_consistency",
            "run_dual_persona_review",
            "backfill_scene_source_offsets",
            "register_canonical_fact",
            "extract_canonical_facts_from_scene",
            "migrate_canonical_fact",
            "bind_canonical_fact_to_scene",
            "research_query",
            "research_add_source",
            "research_add_note",
            "research_add_claim",
            "research_search",
            "research_pack_for_scene",
            "research_usage_for_scene",
            "research_ingest_report",
            "research_plan_for_scene",
            "pull_chapter_from_file",
            "push_chapter_to_file",
            "authoring_prepare_run",
            "authoring_start_run",
            "authoring_status",
            "authoring_execute_next",
            "authoring_save_scene_draft",
            "authoring_record_checkpoint_audit",
            "authoring_review_checkpoint",
            "authoring_resolve_block",
            "authoring_cancel_run",
            "create_style_profile_from_markdown",
            "list_style_profiles",
            "get_style_profile",
            "apply_style_profile",
            "preview_apply_style_profile",
            "list_style_profile_applications",
            "rollback_style_profile_application",
            "check_style_profile_sources",
            "preview_refresh_style_profile",
            "refresh_style_profile",
            "check_style_against_profile",
            "plan_style_revision",
            "compare_style_profiles",
            "archive_style_profile",
            "preview_style_revision_patch",
            "evaluate_style_revision_patch",
            "apply_style_revision_patch",
            "list_style_revision_patch_audits",
            "rollback_style_revision_patch",
        ]),
        "minimal" => Some(&[
            "create_project",
            "list_projects",
            "set_active_project",
            "list_skills",
            "get_skill",
            "get_reference",
            "search_bible",
            "find_scenes_referencing",
            "get_writer_state",
            "get_scene_context",
            "prepare_scene",
            "commit_scene",
            "get_entity",
            "find_entity",
            "get_character_snapshot",
            "list_chapter_scenes",
            "list_book_chapters",
            "save_scene_draft",
            "update_writer_position",
            "import_manuscript",
            "import_status",
            "import_hydrate_bible",
        ]),
        "authoring" => Some(&[
            "list_agents",
            "get_character_snapshot",
            "update_writer_position",
            "commit_character_state",
            "register_canonical_fact",
            "record_knowledge",
            "create_future_knowledge",
            "set_project_calendar",
            "set_scene_clock",
            "record_note",
            "update_arc_milestone",
            "create_character_arc",
            "create_motif",
            "create_theme",
            "create_conflict",
            "create_plot_line",
            "annotate_scene_beats",
            "save_scene_draft",
            "set_active_project",
            "get_editorial_queue",
            "decide_editorial_item",
            "read_episode",
            "get_series_status",
            "prepare_episode_release",
            "release_episode",
            "get_episode_release",
            "get_model_usage",
            "create_project",
            "list_projects",
            "list_skills",
            "get_skill",
            "get_reference",
            "create_book",
            "create_chapter",
            "create_character",
            "create_location",
            "create_world_rule",
            "create_narrative_promise",
            "update_promise_status",
            "get_writer_state",
            "get_entity",
            "find_entity",
            "search_bible",
            "get_chapter_briefing",
            "get_scene_context",
            "prepare_scene",
            "commit_scene",
            "list_chapter_scenes",
            "list_book_chapters",
            "plan_chapter",
            "set_book_outline",
            "set_chapter_outline",
            "update_entity",
            "authoring_prepare_run",
            "authoring_start_run",
            "authoring_status",
            "authoring_execute_next",
            "authoring_save_scene_draft",
            "authoring_record_checkpoint_audit",
            "authoring_review_checkpoint",
            "authoring_resolve_block",
            "authoring_cancel_run",
            "commit_scene_changes",
            "save_summary",
            "check_consistency",
            "run_dual_persona_review",
            "mine_scene_canon",
            "list_canon_deltas",
            "decide_canon_deltas",
            "replan_chapter",
            "list_plan_amendments",
            "decide_plan_amendments",
            "preflight_book_export",
            "compile_manuscript",
            "export_recap",
            "export_series_bible",
            "create_save_point",
            "restore_save_point",
        ]),
        _ => spindle_adapters::guidance::list_profiles()
            .iter()
            .find(|p| p.name == profile)
            .map(|p| p.tools),
    }
}

pub(crate) fn profile_names() -> Vec<&'static str> {
    let mut names = vec!["minimal", "write", "import", "authoring", "legacy-all"];
    names.extend(
        spindle_adapters::guidance::list_profiles()
            .iter()
            .map(|p| p.name),
    );
    names.sort_unstable();
    names
}

pub(crate) fn validate_tool_profile(profile: Option<&str>) -> anyhow::Result<()> {
    if let Some(profile) = profile {
        anyhow::ensure!(
            profile_names().contains(&profile),
            "unknown tool profile '{profile}'; choose one of: {}. Unset SPINDLE_TOOL_PROFILE keeps the compatibility catalog",
            profile_names().join(", ")
        );
    }
    Ok(())
}

pub(crate) fn profile_has_tool(profile: Option<&str>, name: &str) -> bool {
    if !static_tool_names().contains(name) {
        return false;
    }
    match profile {
        None | Some("legacy-all") => true,
        Some(profile) => {
            profile_allowed_tool_names(profile).is_some_and(|allowed| allowed.contains(&name))
        }
    }
}

pub(crate) fn profile_help() -> String {
    format!(
        "Select a startup profile with SPINDLE_TOOL_PROFILE. Profiles: {}. Unset and legacy-all keep the full compatibility catalog. Generated profiles retain current schemas; run 'spindle-mcp profiles' for units and capabilities.",
        profile_names().join(", ")
    )
}

pub(crate) fn print_profiles(json: bool) {
    let generated = spindle_adapters::guidance::list_profiles();
    if json {
        let entries: Vec<_> = profile_names().iter().map(|name| {
            if let Some(profile) = generated.iter().find(|p| p.name == *name) {
                serde_json::json!({"name": profile.name, "description": profile.description, "units": profile.units, "tools": profile.tools, "catalog": "generated-transitional"})
            } else {
                let mut names: Vec<_> = if *name == "legacy-all" { static_tool_names().iter().copied().collect() } else { profile_allowed_tool_names(name).unwrap().to_vec() };
                names.sort_unstable();
                serde_json::json!({"name": name, "catalog": "compatibility", "tools": names})
            }
        }).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).expect("profile catalog")
        );
    } else {
        println!(
            "Unset profile: full compatibility catalog ({} tools).",
            static_tool_names().len()
        );
        for name in profile_names() {
            if let Some(p) = generated.iter().find(|p| p.name == name) {
                println!(
                    "{} ({} tools): {} Units: {}",
                    p.name,
                    p.tools.len(),
                    p.description,
                    p.units.join(", ")
                );
            } else {
                println!("{name}: unchanged compatibility catalog.");
            }
        }
    }
}
