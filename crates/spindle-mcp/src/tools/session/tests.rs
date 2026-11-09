use super::*;

#[tokio::test(flavor = "current_thread")]
async fn create_project_sets_session_defaults_for_follow_up_tools() {
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Session Defaults".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "Session defaults should remove redundant ids.".to_string(),
            style_notes: vec![],
            boundaries: vec![],
        },
    })
    .expect("create project args");
    let create_project_args = create_project_args
        .as_object()
        .cloned()
        .expect("create project object");
    let project: CreateProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_project", Some(&create_project_args))
            .await
            .expect("create project"),
    ))
    .expect("decode create project");

    let mut create_character_args = serde_json::to_value(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: "project:placeholder".to_string(),
        name: "Liora".to_string(),
        summary: "A courier with a perfect memory.".to_string(),
        role: "protagonist".to_string(),
        realm: None,
        voice_profile: CharacterVoiceProfileData {
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec![],
            example_lines: vec![],
            tone: None,
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: CharacterEmotionalProfileData {
            base_emotions: Default::default(),
            suppressed: vec![],
            triggers: vec![],
            defense_mechanisms: vec![],
            flex_range: None,
        },
        initial_state: None,
    })
    .expect("create character args");
    let create_character_args = create_character_args
        .as_object_mut()
        .expect("create character object");
    create_character_args.remove("project_id");
    let character: CreateCharacterOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_character", Some(create_character_args))
            .await
            .expect("create character with session default"),
    ))
    .expect("decode create character");

    let mut batch_voice_profile_args = serde_json::to_value(BatchSetCharacterVoiceProfilesInput {
        project_id: "project:placeholder".to_string(),
        branch_id: "bible_branch:main".to_string(),
        items: vec![BatchSetCharacterVoiceProfileItem {
            character_id: character.character_id.clone(),
            profile: CharacterVoiceProfileData {
                vocabulary: vec!["ash".to_string()],
                sentence_structure: vec!["short".to_string()],
                tics: vec!["counts exits".to_string()],
                forbidden_words: vec![],
                example_lines: vec!["We move before the gate fails.".to_string()],
                tone: Some("clipped".to_string()),
                established_in_scene_id: None,
                updated_at: None,
            },
        }],
        dry_run: None,
    })
    .expect("batch voice args");
    let batch_voice_profile_args = batch_voice_profile_args
        .as_object_mut()
        .expect("batch voice profile object");
    batch_voice_profile_args.remove("project_id");
    batch_voice_profile_args.remove("branch_id");
    let batch_voice_profiles: BatchSetCharacterVoiceProfilesOutput =
        serde_json::from_value(structured_json(
            router
                .call_tool(
                    "batch_set_character_voice_profiles",
                    Some(batch_voice_profile_args),
                )
                .await
                .expect("batch set voice profiles with session defaults"),
        ))
        .expect("decode batch voice profiles");
    assert_eq!(batch_voice_profiles.updated, 1);
    assert_eq!(
        batch_voice_profiles.profiles[0].character_id,
        character.character_id
    );
    // Per-project main branch (Phase 6): use the project's actual
    // branch_id from create_project rather than the legacy literal.
    assert_eq!(
        batch_voice_profiles.profiles[0].branch_id,
        project.branch_id
    );

    let mut update_writer_position_args = serde_json::to_value(UpdateWriterPositionInput {
        project_id: "project:placeholder".to_string(),
        branch_id: project.branch_id.clone(),
        book_id: None,
        chapter_id: None,
        scene_id: None,
        intent: "planning".to_string(),
        next_focus: Some("Outline the next scene.".to_string()),
    })
    .expect("writer position args");
    let update_writer_position_args = update_writer_position_args
        .as_object_mut()
        .expect("writer position object");
    update_writer_position_args.remove("project_id");
    update_writer_position_args.remove("branch_id");
    let position: WriterPosition = serde_json::from_value(structured_json(
        router
            .call_tool("update_writer_position", Some(update_writer_position_args))
            .await
            .expect("update writer position with session defaults"),
    ))
    .expect("decode writer position");
    assert_eq!(position.project_id, project.project_id);
    assert_eq!(position.branch_id, project.branch_id);
}

#[tokio::test(flavor = "current_thread")]
async fn create_character_infers_single_project_without_active_session_project() {
    let router = router().await;

    let project = router
        .service
        .create_project(CreateProjectInput {
            name: "Implicit Project".to_string(),
            project_type: "novel".to_string(),
            genre: "sports drama".to_string(),
            reader_contract: ReaderContract {
                promise: "Single-project sessions should infer project context.".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .await
        .expect("create project directly");

    let mut create_character_args = serde_json::to_value(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: "project:placeholder".to_string(),
        name: "Mike Petrovic".to_string(),
        summary: "Head coach.".to_string(),
        role: "supporting".to_string(),
        realm: Some("Livonia".to_string()),
        voice_profile: CharacterVoiceProfileData {
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec![],
            example_lines: vec![],
            tone: Some("dry".to_string()),
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: CharacterEmotionalProfileData {
            base_emotions: Default::default(),
            suppressed: vec![],
            triggers: vec![],
            defense_mechanisms: vec![],
            flex_range: None,
        },
        initial_state: None,
    })
    .expect("create character args");
    let create_character_args = create_character_args
        .as_object_mut()
        .expect("create character object");
    create_character_args.remove("project_id");

    let character: CreateCharacterOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_character", Some(create_character_args))
            .await
            .expect("create character with inferred project"),
    ))
    .expect("decode create character");
    assert!(character.character_id.starts_with("character:"));

    let writer_state = structured_json(
        router
            .call_tool("get_writer_state", Some(&serde_json::Map::new()))
            .await
            .expect("writer state after inferred project"),
    );
    assert_eq!(
        writer_state["current"]["project"]["project_id"],
        serde_json::json!(project.project_id)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn create_character_without_project_id_errors_when_session_is_ambiguous() {
    let router = router().await;

    for name in ["First Project", "Second Project"] {
        router
            .service
            .create_project(CreateProjectInput {
                name: name.to_string(),
                project_type: "novel".to_string(),
                genre: "sports drama".to_string(),
                reader_contract: ReaderContract {
                    promise: "Ambiguous sessions should fail clearly.".to_string(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await
            .expect("create project directly");
    }

    let mut create_character_args = serde_json::to_value(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: "project:placeholder".to_string(),
        name: "Danny Voss".to_string(),
        summary: "Undersized center.".to_string(),
        role: "supporting".to_string(),
        realm: Some("Westland".to_string()),
        voice_profile: CharacterVoiceProfileData {
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec![],
            example_lines: vec![],
            tone: Some("quick".to_string()),
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: CharacterEmotionalProfileData {
            base_emotions: Default::default(),
            suppressed: vec![],
            triggers: vec![],
            defense_mechanisms: vec![],
            flex_range: None,
        },
        initial_state: None,
    })
    .expect("create character args");
    let create_character_args = create_character_args
        .as_object_mut()
        .expect("create character object");
    create_character_args.remove("project_id");

    let result = router
        .call_tool("create_character", Some(create_character_args))
        .await
        .expect("tool result");
    assert_eq!(result.is_error, Some(true));
    let text = format!("{:?}", result.content.first().expect("error content"));
    assert!(text.contains("set_active_project"));
    assert!(text.contains("project_id"));
}

#[tokio::test(flavor = "current_thread")]
async fn set_active_project_switches_session_defaults() {
    let router = router().await;

    let create_project = |name: &str| CreateProjectInput {
        name: name.to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "Project switching should be session scoped.".to_string(),
            style_notes: vec![],
            boundaries: vec![],
        },
    };

    let first_args =
        serde_json::to_value(create_project("First Project")).expect("first project args");
    let first_args = first_args
        .as_object()
        .cloned()
        .expect("first project object");
    let first: CreateProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_project", Some(&first_args))
            .await
            .expect("create first project"),
    ))
    .expect("decode first project");

    let second_args =
        serde_json::to_value(create_project("Second Project")).expect("second project args");
    let second_args = second_args
        .as_object()
        .cloned()
        .expect("second project object");
    let second: CreateProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_project", Some(&second_args))
            .await
            .expect("create second project"),
    ))
    .expect("decode second project");

    let set_active_args = serde_json::to_value(SetActiveProjectInput {
        project_id: first.project_id.clone(),
        branch_id: None,
    })
    .expect("set active args");
    let set_active_args = set_active_args
        .as_object()
        .cloned()
        .expect("set active object");
    let active: SetActiveProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("set_active_project", Some(&set_active_args))
            .await
            .expect("set active project"),
    ))
    .expect("decode active project");
    assert_eq!(active.project_id, first.project_id);
    // Per-project main branch (Phase 6): use the project's actual
    // branch_id rather than the legacy singleton literal.
    assert_eq!(active.branch_id, first.branch_id);

    let writer_state_args = serde_json::json!({
        "format": "json",
        "budget_tokens": 2000,
        "include_subjects": false,
        "include_recent_activity": false,
        "recent_activity_limit": 0
    });
    let writer_state_args = writer_state_args
        .as_object()
        .cloned()
        .expect("writer state object");
    let writer_state = structured_json(
        router
            .call_tool("get_writer_state", Some(&writer_state_args))
            .await
            .expect("writer state with session project"),
    );
    assert_eq!(
        writer_state["current"]["project"]["project_id"],
        serde_json::json!(first.project_id)
    );
    assert_ne!(
        writer_state["current"]["project"]["project_id"],
        serde_json::json!(second.project_id)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn project_selection_is_callable_in_every_profile() {
    let mut router = router().await;
    let args = serde_json::json!({
        "name": "Existing book", "project_type": "novel", "genre": "fantasy",
        "reader_contract": {"promise": "An existing story", "style_notes": [], "boundaries": []}
    });
    let project = structured_json(
        router
            .call_tool("create_project", args.as_object())
            .await
            .unwrap(),
    );
    let selection = serde_json::json!({"project_id": project["project_id"]});
    for profile in crate::tools::profile_names() {
        router.tool_profile = Some(profile.to_owned());
        assert!(router.has_tool("set_active_project"), "{profile}");
        let active = structured_json(
            router
                .call_tool("set_active_project", selection.as_object())
                .await
                .unwrap(),
        );
        assert_eq!(active["project_id"], project["project_id"], "{profile}");
        assert_eq!(active["branch_id"], project["branch_id"], "{profile}");
    }
}
