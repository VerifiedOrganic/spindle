use super::*;

/// Full tools/list results captured from unsplit tools.rs at 586d00f37ac0.
/// Canonical JSON bytes pin every field and the order/membership of each profile.
#[tokio::test(flavor = "current_thread")]
async fn tools_list_matches_pre_split_snapshots_for_every_profile() {
    let cases: [(Option<&str>, &[u8]); 6] = [
        (
            None,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/tools/snapshots/default.json"
            )),
        ),
        (
            Some("import"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/tools/snapshots/import.json"
            )),
        ),
        (
            Some("write"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/tools/snapshots/write.json"
            )),
        ),
        (
            Some("minimal"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/tools/snapshots/minimal.json"
            )),
        ),
        (
            Some("authoring"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/tools/snapshots/authoring.json"
            )),
        ),
        (
            Some("unknown-profile"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/tools/snapshots/unknown-profile.json"
            )),
        ),
    ];
    let mut router = router().await;
    for (profile, expected) in cases {
        router.tool_profile = profile.map(str::to_owned);
        let result = rmcp::model::ListToolsResult::with_all_items(router.list_tools());
        let canonical = serde_json::to_value(result).expect("serialize tools/list");
        let actual = serde_json::to_vec(&canonical).expect("serialize canonical tools/list");
        assert!(
            actual == expected,
            "tools/list changed for profile {profile:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn skill_tools_are_listed_in_every_tool_profile() {
    for profile in ["import", "write", "minimal", "authoring"] {
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("router.db"))
            .await
            .expect("db init");
        let data_dir = temp.keep();
        let router = ToolRouter::with_tool_profile_and_serialization(
            SpindleService::new(SpindleRepository::with_model_router(
                db,
                data_dir,
                ModelRouter::local_only(),
            )),
            Some(profile.to_string()),
            Arc::new(ToolSerializationState::default()),
        );
        let names: Vec<String> = router
            .list_tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        for tool_name in [
            "list_skills",
            "get_skill",
            "get_reference",
            "set_active_project",
        ] {
            assert!(
                names.iter().any(|name| name == tool_name),
                "profile {profile} is missing tool {tool_name}"
            );
        }
        if profile == "authoring" {
            for step in [
                "list_agents",
                "get_character_snapshot",
                "update_writer_position",
                "commit_character_state",
                "register_canonical_fact",
                "record_knowledge",
                "create_future_knowledge",
                "set_project_calendar",
                "set_scene_clock",
                "authoring_prepare_run",
                "authoring_start_run",
                "authoring_status",
                "authoring_execute_next",
                "authoring_save_scene_draft",
                "commit_scene_changes",
                "save_summary",
                "authoring_record_checkpoint_audit",
                "authoring_review_checkpoint",
                "authoring_resolve_block",
                "authoring_cancel_run",
                "decide_canon_deltas",
                "decide_plan_amendments",
                "compile_manuscript",
                "set_active_project",
                "save_scene_draft",
                "annotate_scene_beats",
                "get_editorial_queue",
                "decide_editorial_item",
                "read_episode",
                "prepare_episode_release",
                "release_episode",
                "get_episode_release",
                "create_plot_line",
                "create_conflict",
                "create_character_arc",
            ] {
                assert!(
                    names.iter().any(|name| name == step),
                    "authoring profile cannot complete {step}"
                );
            }
            assert!(names.len() < router.all_tools().len() / 2);
        }
    }
}

#[test]
fn static_tool_names_matches_all_tool_schemas() {
    let schemas = all_tool_schemas();
    let names = static_tool_names();
    assert_eq!(
        names.len(),
        schemas.len(),
        "every tool schema must have a unique entry in static_tool_names()"
    );
    for tool in schemas {
        assert!(
            names.contains(tool.name.as_ref()),
            "static_tool_names() missing `{}`",
            tool.name
        );
    }
}
