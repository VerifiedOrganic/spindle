use super::*;

#[tokio::test]
async fn compile_manuscript_tool_reads_chapters_and_flags_undrafted_scenes() {
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Compile MCP".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "read so far".to_string(),
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
    .expect("project output");

    // One committed scene in chapter 1.
    let save_args = serde_json::to_value(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "The tool assembles this prose.".to_string(),
        summary: "s".to_string(),
        content_rating: ContentRating::General,
        tone: None,
        source_path: None,
        generation_id: None,
        ..Default::default()
    })
    .expect("save args");
    let save_args = save_args.as_object().cloned().expect("save args object");
    router
        .call_tool("save_scene_draft", Some(&save_args))
        .await
        .expect("save scene draft");

    // Chapter 2 has a plan with an undrafted second scene.
    let plan_args = serde_json::to_value(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 2,
        pov_character_id: None,
        synopsis: "planned".to_string(),
        target_theme_ids: vec![],
        target_conflict_ids: vec![],
        target_plot_line_ids: vec![],
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "undrafted".to_string(),
            purpose: "establishing".to_string(),
            ..Default::default()
        }],
    })
    .expect("plan args");
    let plan_args = plan_args.as_object().cloned().expect("plan args object");
    router
        .call_tool("plan_chapter", Some(&plan_args))
        .await
        .expect("plan chapter");

    let compile_args = serde_json::to_value(CompileManuscriptInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        start_chapter: None,
        end_chapter: None,
        write_to_workspace: false,
    })
    .expect("compile args");
    let compile_args = compile_args
        .as_object()
        .cloned()
        .expect("compile args object");
    let out: CompileManuscriptOutput = serde_json::from_value(structured_json(
        router
            .call_tool("compile_manuscript", Some(&compile_args))
            .await
            .expect("compile manuscript"),
    ))
    .expect("compile output");

    assert_eq!(out.scene_count, 1);
    assert!(out.markdown.contains("The tool assembles this prose."));
    assert!(out.markdown.contains("> [scene 2.1 not yet drafted]"));
    assert_eq!(out.missing_scenes, vec!["2.1".to_string()]);
}

#[tokio::test]
async fn export_recap_tool_folds_summary_end_to_end() {
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Recap MCP".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "read so far".to_string(),
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
    .expect("project output");

    let save_summary_args = serde_json::to_value(SaveSummaryInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "The recap tool folds this chapter.".to_string(),
        key_events: vec![],
        character_changes: vec![],
        relationship_shifts: vec![],
        arc_advances: vec![],
        promise_events: vec![],
    })
    .expect("save summary args");
    let save_summary_args = save_summary_args
        .as_object()
        .cloned()
        .expect("save summary object");
    router
        .call_tool("save_summary", Some(&save_summary_args))
        .await
        .expect("save summary");

    let recap_args = serde_json::to_value(ExportRecapInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        through_chapter: 1,
        write_to_workspace: false,
    })
    .expect("recap args");
    let recap_args = recap_args.as_object().cloned().expect("recap args object");
    let out: ExportRecapOutput = serde_json::from_value(structured_json(
        router
            .call_tool("export_recap", Some(&recap_args))
            .await
            .expect("export recap"),
    ))
    .expect("recap output");

    assert_eq!(out.chapter_count, 1);
    assert!(out.markdown.contains("The recap tool folds this chapter."));
    assert!(out.word_count > 0);
}

#[tokio::test]
async fn export_series_bible_tool_lists_character_end_to_end() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput,
    };
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Bible MCP".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "series bible".to_string(),
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
    .expect("project output");

    let char_args = serde_json::to_value(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project.project_id.clone(),
        name: "Bellwether".to_string(),
        summary: "The bell-ringer of the keep.".to_string(),
        role: "protagonist".to_string(),
        realm: None,
        voice_profile: CharacterVoiceProfileData {
            tone: None,
            vocabulary: vec![],
            sentence_structure: vec![],
            tics: vec![],
            forbidden_words: vec![],
            example_lines: vec![],
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: CharacterEmotionalProfileData {
            base_emotions: std::collections::BTreeMap::new(),
            suppressed: vec![],
            triggers: vec![],
            defense_mechanisms: vec![],
            flex_range: None,
        },
        initial_state: Some(CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: None,
            notes: None,
            source_summary: None,
        }),
    })
    .expect("char args");
    let char_args = char_args.as_object().cloned().expect("char args object");
    router
        .call_tool("create_character", Some(&char_args))
        .await
        .expect("create character");

    let bible_args = serde_json::to_value(ExportSeriesBibleInput {
        project_id: project.project_id.clone(),
        through: None,
        write_to_workspace: false,
    })
    .expect("bible args");
    let bible_args = bible_args.as_object().cloned().expect("bible args object");
    let out: ExportSeriesBibleOutput = serde_json::from_value(structured_json(
        router
            .call_tool("export_series_bible", Some(&bible_args))
            .await
            .expect("export series bible"),
    ))
    .expect("bible output");

    assert_eq!(out.chapter_count, 1);
    assert!(out.markdown.contains("Bellwether"));
    assert!(out.word_count > 0);
}
