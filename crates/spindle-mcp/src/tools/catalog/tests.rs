use super::*;

/// Canonical JSON bytes pin every field and the order/membership of each
/// startup profile. Unset is the generated `workflow-write` profile.
#[tokio::test(flavor = "current_thread")]
async fn tools_list_matches_snapshots_for_every_profile() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/snapshots");
    let mut cases: Vec<(String, Option<String>)> =
        vec![("default".to_string(), Some("workflow-write".to_string()))];
    cases.extend(
        crate::tools::profile_names()
            .into_iter()
            .map(|name| (name.to_string(), Some(name.to_string()))),
    );
    cases.push((
        "unknown-profile".to_string(),
        Some("unknown-profile".to_string()),
    ));

    let mut router = router().await;
    if std::env::var("SPINDLE_UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        for (filename, profile) in &cases {
            router.tool_profile = profile.clone();
            let result = rmcp::model::ListToolsResult::with_all_items(router.list_tools());
            let canonical = serde_json::to_value(result).expect("serialize tools/list");
            let actual = serde_json::to_vec(&canonical).expect("serialize canonical tools/list");
            std::fs::write(dir.join(format!("{filename}.json")), actual).expect("write snapshot");
        }
    }
    for (filename, profile) in cases {
        router.tool_profile = profile;
        let result = rmcp::model::ListToolsResult::with_all_items(router.list_tools());
        let canonical = serde_json::to_value(result).expect("serialize tools/list");
        let actual = serde_json::to_vec(&canonical).expect("serialize canonical tools/list");
        let expected = std::fs::read(dir.join(format!("{filename}.json")))
            .unwrap_or_else(|error| panic!("read snapshot {filename}: {error}"));
        assert!(
            actual == expected,
            "tools/list changed for profile {filename}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn skill_tools_are_listed_in_every_tool_profile() {
    for profile in crate::tools::profile_names() {
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("router.db"))
            .await
            .expect("db init");
        let data_dir = temp.path().to_path_buf();
        let mut router = ToolRouter::with_tool_profile_and_serialization(
            SpindleService::new(SpindleRepository::with_model_router(
                db,
                data_dir,
                ModelRouter::local_only(),
            )),
            Some(profile.to_string()),
            Arc::new(ToolSerializationState::default()),
        );
        router.hold_temp_dir(temp);
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
        let allowed = crate::tools::profiles::profile_allowed_tool_names(profile)
            .expect("generated profile tools");
        for tool_name in allowed {
            assert!(
                router.has_tool(tool_name),
                "profile {profile} cannot dispatch declared tool {tool_name}"
            );
        }
        if profile == "workflow-authoring" {
            for step in [
                "authoring_prepare_run",
                "authoring_start_run",
                "authoring_status",
                "authoring_execute_next",
                "authoring_save_scene_draft",
                "commit_scene_changes",
                "save_scene_draft",
                "annotate_scene_beats",
                "decide_canon_deltas",
                "decide_plan_amendments",
            ] {
                assert!(
                    names.iter().any(|name| name == step),
                    "workflow-authoring cannot complete {step}"
                );
            }
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
