use super::*;

#[tokio::test]
async fn timeline_graph_mermaid_resource_is_deterministic_and_escapes_labels() {
    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "timeline-graph".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "timeline graph visibility".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let first_event = svc
        .create_timeline_event(CreateTimelineEventInput {
            project_id: project.project_id.clone(),
            title: "Arrival \"north\" [gate] {oath}|line\nbreak".to_string(),
            event_type: "anchor".to_string(),
            placement: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            summary: "The gate oath is established.".to_string(),
            related_entity_ids: Vec::new(),
        })
        .await
        .unwrap();
    let second_event = svc
        .create_timeline_event(CreateTimelineEventInput {
            project_id: project.project_id.clone(),
            title: "Discovery".to_string(),
            event_type: "anchor".to_string(),
            placement: StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(1),
                note: None,
            },
            summary: "The discovery follows the arrival.".to_string(),
            related_entity_ids: Vec::new(),
        })
        .await
        .unwrap();

    svc.create_temporal_intervention(CreateTemporalInterventionInput {
        project_id: project.project_id.clone(),
        title: "Echo [closed]".to_string(),
        intervention_type: "message".to_string(),
        source_event_id: Some(second_event.timeline_event_id),
        target_event_id: Some(first_event.timeline_event_id),
        summary: "The later event echoes backward.".to_string(),
        consequences: vec!["closed loop".to_string()],
        status: Some("planned".to_string()),
    })
    .await
    .unwrap();
    svc.create_temporal_intervention(CreateTemporalInterventionInput {
        project_id: project.project_id.clone(),
        title: "Loose {thread}|alert".to_string(),
        intervention_type: "message".to_string(),
        source_event_id: None,
        target_event_id: None,
        summary: "A loose intervention is missing endpoints.".to_string(),
        consequences: Vec::new(),
        status: Some("draft".to_string()),
    })
    .await
    .unwrap();

    let graph = svc
        .timeline_graph_mermaid_resource(&project.project_id)
        .await
        .unwrap();
    let graph_again = svc
        .timeline_graph_mermaid_resource(&project.project_id)
        .await
        .unwrap();

    assert_eq!(graph, graph_again);
    assert!(graph.starts_with("```mermaid\nflowchart LR\n"));
    assert!(graph.ends_with("```\n"));
    assert_eq!(graph.matches("```").count(), 2);
    assert!(
        graph.contains("Arrival \\\"north\\\" &#91;gate&#93; &#123;oath&#125;&#124;line\\nbreak")
    );
    assert!(graph.contains("-. \"intervention: Echo &#91;closed&#93;\" .->"));
    assert!(graph.contains("missing source and target: Loose &#123;thread&#125;&#124;alert"));
    assert!(graph.contains(":::warning"));
}

/// Exercises the three core dispatch shapes of `read_project_resource`:
/// a simple list (`characters` — empty), a paginated bare list
/// (`research-log` — empty, full envelope), and an explicit page request
/// (`conflicts/0/50` — empty, full envelope with `next_resource: null`).
/// Asserts the pagination envelope structure so the MCP layer's
/// `bible://projects/...` URI fingerprint stays stable.
#[tokio::test]
async fn read_project_resource_dispatches_simple_and_paginated_shapes() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "rpr".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // 1) Simple list: characters — empty array on a fresh project.
    let characters = svc
        .read_project_resource(&proj.project_id, "characters")
        .await
        .unwrap();
    assert_eq!(characters, serde_json::json!([]));

    // 2) Paginated bare path: research-log defaults to offset 0,
    //    limit = DEFAULT_PROJECT_RESOURCE_PAGE_SIZE (50).
    let research = svc
        .read_project_resource(&proj.project_id, "research-log")
        .await
        .unwrap();
    let pagination = research
        .get("pagination")
        .expect("pagination envelope must be present");
    assert_eq!(pagination["offset"], 0);
    assert_eq!(pagination["limit"], 50);
    assert_eq!(pagination["returned"], 0);
    assert_eq!(pagination["total"], 0);
    assert_eq!(pagination["has_more"], false);
    assert_eq!(pagination["order"], "newest_first");
    assert!(pagination["next_resource"].is_null());
    assert!(pagination["previous_resource"].is_null());
    assert_eq!(research["entries"], serde_json::json!([]));

    // 3) Explicit page form: conflicts/0/50 has the same envelope shape
    //    as the bare form, just with the parsed offset/limit echoed back.
    let conflicts = svc
        .read_project_resource(&proj.project_id, "conflicts/0/50")
        .await
        .unwrap();
    let pagination = conflicts
        .get("pagination")
        .expect("pagination envelope must be present");
    assert_eq!(pagination["offset"], 0);
    assert_eq!(pagination["limit"], 50);
    assert_eq!(pagination["total"], 0);
    assert_eq!(pagination["order"], "normalized_name");
    assert!(pagination["next_resource"].is_null());
    assert_eq!(conflicts["entries"], serde_json::json!([]));

    // 4) Unknown resource path bails cleanly.
    let err = svc
        .read_project_resource(&proj.project_id, "totally-fake")
        .await
        .expect_err("unknown resource paths must error");
    assert!(
        err.to_string().contains("unknown project resource path"),
        "got: {err}"
    );

    // 5) Over-size page limit on a paginated resource is rejected.
    let err = svc
        .read_project_resource(&proj.project_id, "research-log/0/500")
        .await
        .expect_err("page size > MAX must error");
    assert!(
        err.to_string().contains("page size must be <="),
        "got: {err}"
    );
}

#[tokio::test]
async fn search_bible_returns_results_in_both_modes() {
    use spindle_core::models::{
        CharacterStatePatch, CharacterVoiceProfileData, CreateCharacterInput, SearchBibleInput,
        SearchBibleMode,
    };
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "P".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: proj.project_id.clone(),
        name: "Mara Oathkeeper".into(),
        summary: "Warden of the Ash Gate.".into(),
        role: "protagonist".into(),
        realm: None,
        voice_profile: CharacterVoiceProfileData {
            tone: None,
            vocabulary: Vec::new(),
            sentence_structure: Vec::new(),
            tics: Vec::new(),
            forbidden_words: Vec::new(),
            example_lines: Vec::new(),
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
            base_emotions: std::collections::BTreeMap::new(),
            suppressed: Vec::new(),
            triggers: Vec::new(),
            defense_mechanisms: Vec::new(),
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
    .await
    .unwrap();

    // Exact (FTS5) mode.
    let exact = svc
        .search_bible(SearchBibleInput {
            project_id: proj.project_id.clone(),
            query: "warden".into(),
            limit: Some(10),
            mode: Some(SearchBibleMode::Exact),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        exact.results.iter().any(|r| r.entity_type == "character"),
        "exact mode must return the character via FTS5"
    );

    // Semantic (vec0) mode — TokenHash embedding is deterministic so
    // searching with the character's own content returns it as a hit.
    let semantic = svc
        .search_bible(SearchBibleInput {
            project_id: proj.project_id.clone(),
            query: "Mara Oathkeeper\n\nWarden of the Ash Gate.".into(),
            limit: Some(10),
            mode: Some(SearchBibleMode::Semantic),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        !semantic.results.is_empty(),
        "semantic mode must return at least one result"
    );
}

#[tokio::test]
async fn full_mcp_priority_flow_through_service() {
    use spindle_core::models::{
        CharacterStatePatch, CharacterVoiceProfileData, ContentRating, CreateBranchInput,
        CreateCharacterInput, CreateLocationInput, CreateRelationshipInput, SaveSceneDraftInput,
        SwitchBranchInput, WorldStateInput,
    };
    let (_tmp, svc) = fresh_service().await;

    // 1. Create the project.
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Marches".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Oathbound wardens fail and a city falls.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // 2. Create a character.
    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Oathbound warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
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
        .await
        .unwrap();
    assert!(mara.character_id.starts_with("character:"));

    let aldric = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Aldric".into(),
            summary: "Scribe of the eastern marches.".into(),
            role: "supporting".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    // 3. Create a location (+ paired world state).
    let gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A blackened wall holding back the dark.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: Some("tense".into()),
                prosperity: None,
                stability: Some("fragile".into()),
                threat_level: Some("high".into()),
                sensory_details: vec!["smell of ash".into()],
            },
        })
        .await
        .unwrap();
    assert!(gate.location_id.starts_with("location:"));

    // 4. Create a relationship.
    let rel = svc
        .create_relationship(CreateRelationshipInput {
            character_a_id: mara.character_id.clone(),
            character_b_id: aldric.character_id.clone(),
            relationship_type: "ally".into(),
            initial_trust: 60,
            initial_tension: 20,
            dynamics: vec!["wary".into()],
        })
        .await
        .unwrap();
    assert!(rel.relationship_id.starts_with("relates_to:"));

    // 5. Save a scene through the service. This insert lands the row,
    //    runs the scene_version snapshot trigger plus the FTS sync.
    let saved = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "Mara holds the gate".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(saved.scene_id.starts_with("scene:"));
    assert_eq!(saved.status, "saved");

    // 6. Branch off main + switch.
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            name: "feature-arc-2".into(),
            branch_type: "feature".into(),
            description: None,
            parent_branch_id: None,
        })
        .await
        .unwrap();
    assert!(feature.branch_id.starts_with("bible_branch:"));

    let switched = svc
        .switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(switched.branch_id, feature.branch_id);
    assert_eq!(switched.branch_name, "feature-arc-2");
}

/// A comprehensive end-to-end test that walks through a realistic
/// MCP-priority flow: create project, create characters with relationships,
/// plan a chapter, save scenes with version snapshots, commit character
/// state, register canonical facts, save a chapter summary, exercise the
/// search index in both modes, and branch off main. This proves the
/// SQLite migration is architecturally sound for the integration test
/// golden path the original plan calls out as the Phase 6 exit criterion.
///
/// Note: this test exercises ~25 of the translated service methods in one
/// run. It is the closest proxy to MCP integration_tests.rs that lives at
/// the spindle-adapters level (before the Phase 6 MCP layer swap).
#[tokio::test]
async fn end_to_end_full_flow_against_sqlite() {
    use spindle_core::models::{
        CanonicalFactScope, CharacterStatePatch, CharacterVoiceProfileData,
        CommitCharacterStateInput, ContentRating, CreateBranchInput, CreateCharacterInput,
        CreateLocationInput, CreateProjectInput, CreateRelationshipInput, PlanChapterInput,
        PlanChapterSceneInput, RecordKnowledgeInput, RegisterCanonicalFactInput,
        SaveSceneDraftInput, SaveSummaryInput, SearchBibleInput, SearchBibleMode, StoryPlacement,
        SwitchBranchInput, UpdateRelationshipInput, WorldStateInput,
    };
    let (_tmp, svc) = fresh_service().await;

    // ----- Phase 1: project + Bible setup -------------------------------
    let proj = svc
        .create_project(CreateProjectInput {
            name: "End-to-End Test".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Oathbound wardens fail and a city falls.".into(),
                style_notes: vec!["sparse prose".into()],
                boundaries: vec!["no second-person".into()],
            },
        })
        .await
        .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Oathbound warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
                vocabulary: vec!["oath".into(), "ash".into()],
                sentence_structure: vec!["clipped".into()],
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: vec!["I know the cost.".into()],
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: vec!["fear".into()],
                triggers: Vec::new(),
                defense_mechanisms: vec!["silence".into()],
                flex_range: None,
            },
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: Some(vec!["hold the gate".into()]),
                status: Some(vec!["wary".into()]),
                notes: None,
                source_summary: Some("introduction".into()),
            }),
        })
        .await
        .unwrap();

    let aldric = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Aldric".into(),
            summary: "Scribe of the marches.".into(),
            role: "supporting".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let _gate = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "A blackened wall holding back the dark.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: Some("watchful".into()),
                prosperity: None,
                stability: Some("fragile".into()),
                threat_level: Some("high".into()),
                sensory_details: vec!["smell of ash".into()],
            },
        })
        .await
        .unwrap();

    let _rel = svc
        .create_relationship(CreateRelationshipInput {
            character_a_id: mara.character_id.clone(),
            character_b_id: aldric.character_id.clone(),
            relationship_type: "ally".into(),
            initial_trust: 60,
            initial_tension: 20,
            dynamics: vec!["wary mutual respect".into()],
        })
        .await
        .unwrap();

    // ----- Phase 2: chapter planning ------------------------------------
    let _plan = svc
        .plan_chapter(PlanChapterInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            pov_character_id: Some(mara.character_id.clone()),
            synopsis: "Mara holds the gate against the first dark.".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![
                PlanChapterSceneInput {
                    scene_order: 1,
                    summary: "Mara takes the watch".into(),
                    beat_structure: vec!["arrival".into(), "first dark".into()],
                    character_ids: vec![mara.character_id.clone()],
                    purpose: "establishing".into(),
                    ..Default::default()
                },
                PlanChapterSceneInput {
                    scene_order: 2,
                    summary: "Aldric brings news".into(),
                    beat_structure: vec!["arrival".into()],
                    character_ids: vec![mara.character_id.clone(), aldric.character_id.clone()],
                    purpose: "complication".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    // ----- Phase 3: scene drafting ---------------------------------------
    let scene1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate. Her oath weighed heavier than her sword."
                .into(),
            summary: "Mara holds the gate at first dark".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(scene1.scene_id.starts_with("scene:"));
    assert_eq!(scene1.status, "saved");

    // Save the scene again with revised prose — version snapshot fires.
    let scene1_v2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate. Behind her, the city slept.".into(),
            summary: "Mara holds the gate at first dark".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(scene1_v2.scene_id, scene1.scene_id);

    // Verify the version count via list_scene_versions.
    let versions = svc
        .list_scene_versions(spindle_core::models::ListSceneVersionsInput {
            project_id: proj.project_id.clone(),
            scene_id: scene1.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        versions.versions.len(),
        1,
        "one snapshot of the prior prose"
    );

    // ----- Phase 4: state + knowledge + canonical fact -------------------
    let state = svc
        .commit_character_state(CommitCharacterStateInput {
            character_id: mara.character_id.clone(),
            scene_id: Some(scene1.scene_id.clone()),
            anchor: None,
            changes: CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: Some(vec!["hold the gate at all costs".into()]),
                status: Some(vec!["determined".into()]),
                notes: Some(vec!["sword across knees".into()]),
                source_summary: Some("first watch".into()),
            },
        })
        .await
        .unwrap();
    assert!(state.state_id.starts_with("character_state:"));

    let _knowledge = svc
        .record_knowledge(RecordKnowledgeInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            character_id: mara.character_id.clone(),
            fact: "The dark advances from the north.".into(),
            source_summary: "scout report at first watch".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            }),
            confidence: Some(0.8),
            tags: vec!["intel".into()],
            reader_visible: true,
            secret_of_fact_id: None,
        })
        .await
        .unwrap();

    let fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(scene1.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(mara.character_id.clone()),
            predicate: Some("oath".into()),
            value_kind: Some("string".into()),
            value_text: Some("ash gate warden".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: vec!["oathbound".into()],
            scope: Some(CanonicalFactScope::Invariant),
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();
    assert!(fact.canonical_fact_id.starts_with("canonical_fact:"));

    // ----- Phase 5: search both modes -----------------------------------
    let lexical = svc
        .search_bible(SearchBibleInput {
            project_id: proj.project_id.clone(),
            query: "warden".into(),
            limit: Some(10),
            mode: Some(SearchBibleMode::Exact),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        lexical.results.iter().any(|r| r.entity_type == "character"),
        "FTS5 lexical search should find the character"
    );

    let _semantic = svc
        .search_bible(SearchBibleInput {
            project_id: proj.project_id.clone(),
            query: "Mara stood at the gate".into(),
            limit: Some(10),
            mode: Some(SearchBibleMode::Semantic),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    // ----- Phase 6: relationship update + summary -----------------------
    let updated_rel = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: mara.character_id.clone(),
            character_b_id: aldric.character_id.clone(),
            trust_delta: 10,
            tension_delta: -5,
            reason: "Aldric's scouting saved Mara's watch".into(),
            scene_id: scene1.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(updated_rel.trust, 70);
    assert_eq!(updated_rel.tension, 15);

    let summary = svc
        .save_summary(SaveSummaryInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            entity_type: None,
            entity_id: None,
            summary: "Mara held the gate; Aldric brought intel.".into(),
            key_events: vec!["first dark approached".into()],
            character_changes: vec!["Mara hardened".into()],
            relationship_shifts: vec!["Mara-Aldric trust deepened".into()],
            arc_advances: Vec::new(),
            promise_events: Vec::new(),
        })
        .await
        .unwrap();
    assert!(summary.chapter_summary_id.starts_with("chapter_summary:"));

    // ----- Phase 7: branch off ------------------------------------------
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            name: "feature-alt-ending".into(),
            branch_type: "feature".into(),
            description: Some("alternate fall-of-the-gate ending".into()),
            parent_branch_id: None,
        })
        .await
        .unwrap();
    let switched = svc
        .switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(switched.branch_id, feature.branch_id);
    assert_eq!(switched.branch_name, "feature-alt-ending");
}

#[tokio::test]
async fn create_project_returns_expected_output_shape() {
    let (_tmp, svc) = fresh_service().await;
    let out = svc
        .create_project(CreateProjectInput {
            name: "Spindle".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    assert!(out.project_id.starts_with("project:"));
    assert!(out.book_id.starts_with("book:"));
    assert!(out.chapter_id.starts_with("chapter:"));

    let list = svc.list_projects().await.unwrap();
    assert_eq!(list.projects.len(), 1);
    assert_eq!(list.projects[0].project_id, out.project_id);
    assert_eq!(list.projects[0].name, "Spindle");
}

#[tokio::test]
async fn migrate_canonical_fact_supersedes_old_fact() {
    use spindle_core::models::{
        CanonicalFactUpgradeSpec, ContentRating, MigrateCanonicalFactInput,
        RegisterCanonicalFactInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Canon".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "A scene.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Seed an untyped canonical fact via register_canonical_fact.
    let untyped = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(scene.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("note".into()),
            key: Some("Kael:gear".into()),
            value: Some("worn cloak".into()),
            context: None,
            subject_table: None,
            subject_id: None,
            predicate: None,
            value_kind: None,
            value_text: None,
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: Some(true),
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();

    let migrated = svc
        .migrate_canonical_fact(MigrateCanonicalFactInput {
            fact_id: untyped.canonical_fact_id.clone(),
            upgrade_spec: CanonicalFactUpgradeSpec {
                subject_table: "character".into(),
                subject_id: Some("character:kael".into()),
                predicate: "gear".into(),
                value_kind: "string".into(),
                value_text: Some("worn cloak".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: vec!["cloak".into()],
                scope: None,
                valid_from: None,
                valid_until: None,
            },
        })
        .await
        .unwrap();

    assert_eq!(migrated.superseded_fact_id, untyped.canonical_fact_id);
    assert_ne!(
        migrated.canonical_fact_id, untyped.canonical_fact_id,
        "migrate must create a new fact id"
    );

    // The old fact is now superseded by the new typed fact.
    let old = svc
        .repository()
        .get_canonical_fact(&untyped.canonical_fact_id)
        .await
        .unwrap();
    assert_eq!(
        old.superseded_by.as_deref(),
        Some(migrated.canonical_fact_id.as_str())
    );
}

/// Live-run bug: `create_chapter` accepted a `title` and returned success,
/// but never persisted it — a later `preflight_book_export` then reported
/// `chapter_missing_title` for the very chapter that was created with one.
#[tokio::test]
async fn create_chapter_persists_title() {
    use spindle_core::models::{ContentRating, PreflightBookExportInput, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Title Persist".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let out = svc
        .create_chapter(spindle_core::models::CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(2),
            title: Some("Prologue: Sol System, Off-Season".to_string()),
        })
        .await
        .unwrap();

    let chapter = svc.repository.get_chapter(&out.chapter_id).await.unwrap();
    assert_eq!(
        chapter.title.as_deref(),
        Some("Prologue: Sol System, Off-Season"),
        "create_chapter must persist the title it accepts"
    );
    assert_eq!(
        out.title.as_deref(),
        Some("Prologue: Sol System, Off-Season"),
        "the success response must echo the persisted title"
    );

    // The preflight that surfaced the live bug must no longer flag the
    // titled chapter.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 2,
        chapter_id: None,
        scene_order: 1,
        full_text: "The station lights hummed over empty docks.".into(),
        summary: "prologue".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    let preflight = svc
        .preflight_book_export(PreflightBookExportInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            start_chapter_number: Some(2),
            end_chapter_number: Some(2),
        })
        .await
        .unwrap();
    assert!(
        preflight
            .issues
            .iter()
            .all(|issue| issue.code != "chapter_missing_title"),
        "a chapter created with a title must not be flagged chapter_missing_title, got {:?}",
        preflight.issues
    );
}

/// A chapter that ALREADY has a title keeps it when `create_chapter` is
/// called again for the same slot with a different one — the ensure path
/// fills in a missing title, it never clobbers an existing one.
#[tokio::test]
async fn create_chapter_does_not_clobber_an_existing_title() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Title Keep".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    svc.create_chapter(spindle_core::models::CreateChapterInput {
        project_id: proj.project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(2),
        title: Some("First Title".to_string()),
    })
    .await
    .unwrap();
    let again = svc
        .create_chapter(spindle_core::models::CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(2),
            title: Some("Second Title".to_string()),
        })
        .await
        .unwrap();

    let chapter = svc.repository.get_chapter(&again.chapter_id).await.unwrap();
    assert_eq!(
        chapter.title.as_deref(),
        Some("First Title"),
        "create_chapter must not overwrite an existing chapter title"
    );
}

/// `check_consistency` must surface stub scenes too — the live run
/// reported 0 errors on a book containing literal placeholders.
#[tokio::test]
async fn check_consistency_flags_stub_scenes() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Stub Consistency".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    chapter_with_stub_scenes(&svc, &proj.project_id).await;

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::book(1),
            checks: vec!["scene_stub_text".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    let stub_issues: Vec<_> = out
        .issues
        .iter()
        .filter(|issue| issue.check_type == "scene_stub_text")
        .collect();
    assert_eq!(
        stub_issues.len(),
        2,
        "the placeholder and thin scenes must be flagged, got: {:?}",
        out.issues
    );
    // Info, not warning/error: stubbiness is a manuscript-readiness
    // observation, and the authoring loop's checkpoint policies gate on
    // errors+warnings. The hard gate is the Blocking preflight issue.
    assert!(
        stub_issues.iter().all(|issue| issue.severity == "info"),
        "stub findings are info: {stub_issues:?}"
    );
}

/// The word floor is a per-project setting (`project.min_scene_word_count`
/// via update_entity), defaulting to 20 words. Placeholder bodies stay
/// flagged regardless of any floor.
#[tokio::test]
async fn project_min_scene_word_count_floor_is_configurable() {
    use spindle_core::models::{PreflightBookExportInput, UpdateEntityInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Stub Floor".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    chapter_with_stub_scenes(&svc, &proj.project_id).await;

    // Lower the floor to 3 words: the 5-word scene is now legitimate,
    // the literal placeholder is still a placeholder.
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "project".to_string(),
        entity_id: proj.project_id.clone(),
        changes: serde_json::json!({ "min_scene_word_count": 3 }),
    })
    .await
    .unwrap();

    let out = svc
        .preflight_book_export(PreflightBookExportInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            start_chapter_number: None,
            end_chapter_number: None,
        })
        .await
        .unwrap();
    let flagged_orders: Vec<i32> = out
        .issues
        .iter()
        .filter(|issue| issue.code == "scene_stub_text")
        .map(|issue| issue.scene_order.unwrap_or(-1))
        .collect();
    assert_eq!(
        flagged_orders,
        vec![1],
        "only the placeholder scene stays flagged at a 3-word floor, got {:?}",
        out.issues
    );
}

#[tokio::test]
async fn compare_alternatives_ranks_by_quality_score() {
    use spindle_core::models::{
        CompareAlternativesInput, ContentRating, CreateBranchInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Alt".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Two feature branches off main, each with one scene draft.
    let alt_a = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "alt-a".into(),
            branch_type: "alternative".into(),
            description: None,
        })
        .await
        .unwrap();
    let alt_b = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "alt-b".into(),
            branch_type: "alternative".into(),
            description: None,
        })
        .await
        .unwrap();

    // Switch into each branch to write its scene. Sparse text →
    // low score; rich text + measured tone → higher score.
    svc.switch_branch(spindle_core::models::SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: alt_a.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Short.".into(),
        summary: "thin".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    svc.switch_branch(spindle_core::models::SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: alt_b.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "A long, rich scene draft with many words to push the text-length \
                        contribution to its maximum and pull the rank ahead of the thin \
                        alternative branch. Repeating: a long, rich, sustained, descriptive \
                        passage that the scoring helper will treat as a heavyweight contender."
            .into(),
        summary: "rich measured beat that opens a quiet pacing slot".into(),
        content_rating: ContentRating::General,
        tone: Some("measured".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .compare_alternatives(CompareAlternativesInput {
            project_id: proj.project_id.clone(),
            branch_ids: vec![alt_a.branch_id.clone(), alt_b.branch_id.clone()],
        })
        .await
        .unwrap();

    assert_eq!(out.alternatives.len(), 2);
    assert_eq!(
        out.recommended_branch_id.as_deref(),
        Some(alt_b.branch_id.as_str())
    );
    // strongest_trait reflects the "measured" tone branch.
    let b_entry = out
        .alternatives
        .iter()
        .find(|c| c.branch_id == alt_b.branch_id)
        .unwrap();
    assert_eq!(b_entry.strongest_trait, "best pacing balance");
}

/// End-to-end genre-voice enforcement (the brief's test case): an
/// NSFW Comedy Webnovel must surface its style contract prominently, flag a
/// literary scene at the save gate, the validator, and the review — and let
/// an on-genre scene through clean.
#[tokio::test]
async fn style_enforcement_pipeline_flags_offgenre_comedy_scene() {
    use spindle_core::models::{
        CharacterStatePatch, CharacterVoiceProfileData, CheckConsistencyInput,
        ConsistencyScopeInput, ContentRating, ContextFormat, CreateCharacterInput,
        CreateLocationInput, CreateWorldRuleInput, GetSceneContextInput, RunDualPersonaReviewInput,
        SaveSceneDraftInput, SetNarratorVoiceInput, WorldStateInput,
    };

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Vegas Pull".into(),
            project_type: "NSFW Comedy Webnovel".into(),
            genre: "Comedy".into(),
            reader_contract: ReaderContract {
                promise: "A raunchy, funny gacha power-fantasy romp.".into(),
                style_notes: vec![
                    "Raunchy modern comedy tone".into(),
                    "Webnovel pacing with clear progression".into(),
                ],
                boundaries: vec!["Focus on raunchy comedy and fun over dark themes".into()],
            },
        })
        .await
        .unwrap();

    svc.set_narrator_voice(SetNarratorVoiceInput {
        project_id: proj.project_id.clone(),
        narrator_voice: spindle_core::style::NarratorVoice {
            emotional_register: Some("funny-and-sarcastic".into()),
            chapter_ending_style: Some("hook".into()),
            ..Default::default()
        },
    })
    .await
    .unwrap();

    svc.create_world_rule(CreateWorldRuleInput {
        project_id: proj.project_id.clone(),
        rule_name: "Prose Style Bible — Webnovel-First, Comedy-First".into(),
        rule_type: "style".into(),
        description: "No grief-beat endings; no contemplative literary pacing.".into(),
        scan_pattern: Some("*".into()),
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    let jason = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Jason".into(),
            summary: "37-year-old mind in a 19-year-old body; sarcastic narrator.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("sarcastic".into()),
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: Some(vec!["win the gacha".into()]),
                status: Some(vec!["broke".into()]),
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

    let casino = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "The Strip".into(),
            kind: "casino".into(),
            realm: None,
            summary: "Neon, noise, bad decisions.".into(),
            initial_state: WorldStateInput {
                controlling_faction: None,
                status: Some("loud".into()),
                prosperity: None,
                stability: None,
                threat_level: None,
                sensory_details: vec!["cigarette smoke".into()],
            },
        })
        .await
        .unwrap();

    // (5) Scene context surfaces the style contract prominently.
    let envelope = svc
        .get_scene_context_envelope(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            character_ids: vec![jason.character_id.clone()],
            max_character_count: None,
            location_id: casino.location_id.clone(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(8000),
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();

    let directive = envelope
        .novel
        .style_directive
        .as_ref()
        .expect("style directive present for a project with a style contract");
    assert!(
        directive
            .style_rules
            .iter()
            .any(|rule| rule.rule_name.contains("Prose Style Bible")),
        "style world rule must reach the directive regardless of relevance filtering"
    );
    assert!(envelope.standards.contains("Project Style Requirements"));
    assert!(envelope.standards.contains("[STYLE DIRECTIVE]"));
    assert!(
        envelope.standards.contains("not funny"),
        "comedy projects get the forceful 'not funny → failed' enforcement line"
    );
    let context_md = envelope.context_markdown.as_deref().unwrap_or_default();
    assert!(context_md.contains("Project Style Requirements"));

    // (6) Save gate flags a literary tone on a comedy project. This scene
    // is the only one in chapter 1, so it is also a chapter end.
    let literary = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Jason sat alone after the others left. The grief of the day pressed \
                            in, a hollow ache, a quiet sorrow, the melancholy of everything he \
                            had lost settling over him like ash."
                .into(),
            summary: "Jason sits with his losses.".into(),
            content_rating: ContentRating::General,
            tone: Some("Quiet, contained, grief beat".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        literary.tone_deviation,
        "a grief-beat tone on a comedy project is a tone deviation"
    );
    assert!(
        literary
            .style_warnings
            .iter()
            .any(|w| w.to_lowercase().contains("comedy")),
        "style_warnings should name the comedy-contract conflict: {:?}",
        literary.style_warnings
    );

    // On-genre scene in a different chapter passes clean. Chapter 2 must
    // exist first; this also keeps the literary scene the sole (ending)
    // scene of chapter 1 so the validator's chapter-end check applies.
    svc.create_chapter(spindle_core::models::CreateChapterInput {
        project_id: proj.project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(2),
        title: None,
    })
    .await
    .unwrap();
    let comedic = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 2,
            chapter_id: None,
            scene_order: 1,
            full_text: "The System pinged like a slot machine hitting jackpot. \"Congrats, \
                            loser,\" it chirped. Jason flipped it off and yanked the lever again."
                .into(),
            summary: "Jason pulls again.".into(),
            content_rating: ContentRating::General,
            tone: Some("manic, raunchy".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        !comedic.tone_deviation,
        "an on-genre scene is not a deviation"
    );
    assert!(
        comedic.style_warnings.is_empty(),
        "on-genre scene should not produce style warnings: {:?}",
        comedic.style_warnings
    );

    // (7) Dual-persona review evaluates genre compliance via Target Reader.
    let review = svc
        .run_dual_persona_review(RunDualPersonaReviewInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            scene_id: literary.scene_id.clone(),
            rounds: Some(1),
        })
        .await
        .unwrap();
    let round = &review.review_rounds[0];
    assert_eq!(round.genre_reader.persona, "target_reader");
    assert!(
        !round.genre_reader.concerns.is_empty(),
        "Target Reader should raise genre concerns on a literary comedy scene"
    );
    assert!(
        round
            .priority_actions
            .iter()
            .any(|a| a.starts_with("Genre fix:")),
        "genre concerns should lead the priority actions: {:?}",
        round.priority_actions
    );

    // (8) check_consistency surfaces a style_compliance finding for the
    // contemplative chapter-ending scene.
    let consistency = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["style_compliance".into()],
            severity_filter: Vec::new(),
            deep_check: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        consistency
            .issues
            .iter()
            .any(|issue| issue.check_type == "style_compliance"),
        "style_compliance validator should flag the off-genre chapter ending: {:?}",
        consistency.issues
    );
}

/// Narrator voice round-trips through the new project column and clears
/// back to unset when an empty voice is submitted.
#[tokio::test]
async fn set_narrator_voice_round_trips_and_clears() {
    use spindle_core::models::SetNarratorVoiceInput;

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "NV".into(),
            project_type: "novel".into(),
            genre: "Comedy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let set = svc
        .set_narrator_voice(SetNarratorVoiceInput {
            project_id: proj.project_id.clone(),
            narrator_voice: spindle_core::style::NarratorVoice {
                emotional_register: Some("funny-and-sarcastic".into()),
                chapter_ending_style: Some("hook".into()),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    assert!(!set.cleared);
    assert_eq!(
        set.narrator_voice.emotional_register.as_deref(),
        Some("funny-and-sarcastic")
    );

    // It must persist into the directive the rest of the pipeline reads.
    let branch_id = svc
        .repository
        .active_branch_id_public(&proj.project_id)
        .await
        .unwrap();
    let directive = svc
        .style_directive_for(&proj.project_id, &branch_id)
        .await
        .unwrap();
    assert_eq!(
        directive
            .narrator_voice
            .as_ref()
            .and_then(|voice| voice.chapter_ending_style.as_deref()),
        Some("hook")
    );

    // Submitting an empty voice clears the column.
    let cleared = svc
        .set_narrator_voice(SetNarratorVoiceInput {
            project_id: proj.project_id.clone(),
            narrator_voice: spindle_core::style::NarratorVoice::default(),
        })
        .await
        .unwrap();
    assert!(cleared.cleared);
    let directive = svc
        .style_directive_for(&proj.project_id, &branch_id)
        .await
        .unwrap();
    assert!(directive.narrator_voice.is_none());
}

/// The plan gate catches genre-incompatible beats before any prose exists.
#[tokio::test]
async fn plan_chapter_flags_offgenre_beats_on_comedy_project() {
    use spindle_core::models::{PlanChapterInput, PlanChapterSceneInput};

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Vegas Pull".into(),
            project_type: "NSFW Comedy Webnovel".into(),
            genre: "Comedy".into(),
            reader_contract: ReaderContract {
                promise: "A raunchy funny gacha romp.".into(),
                style_notes: vec!["Raunchy modern comedy tone; webnovel pacing".into()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let out = svc
        .plan_chapter(PlanChapterInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            pov_character_id: None,
            synopsis: "Jason loses everything and reflects.".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![
                PlanChapterSceneInput {
                    scene_order: 1,
                    summary: "Jason wins big at the slots.".into(),
                    beat_structure: vec!["escalation".into()],
                    character_ids: Vec::new(),
                    purpose: "Open with a manic high.".into(),
                    ..Default::default()
                },
                PlanChapterSceneInput {
                    scene_order: 2,
                    summary: "Jason sits alone with his losses.".into(),
                    beat_structure: vec!["quiet_reflection".into()],
                    character_ids: Vec::new(),
                    purpose: "Quiet, contained. The grief beat of the chapter.".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert!(
        out.style_warnings.iter().any(|w| w.starts_with("Scene 2:")),
        "the grief-beat final scene should be flagged at planning time: {:?}",
        out.style_warnings
    );
}

#[tokio::test]
async fn extract_canonical_facts_from_scene_proposes_sentence_chunks() {
    use spindle_core::models::{
        CanonicalFactScope, ContentRating, ExtractCanonicalFactsFromSceneInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Extract".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let saved = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara held the gate while the bell tolled twice. \
                            Aldric watched the ash settle on the cobbles below. \
                            Short."
                .into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let out = svc
        .extract_canonical_facts_from_scene(ExtractCanonicalFactsFromSceneInput {
            scene_id: saved.scene_id.clone(),
        })
        .await
        .unwrap();

    assert_eq!(out.scene_id, saved.scene_id);
    // Two long sentences should pass the >=24 char filter; the third
    // ("Short.") is below the threshold and must be dropped.
    assert_eq!(out.proposals.len(), 2);
    for (idx, prop) in out.proposals.iter().enumerate() {
        assert_eq!(prop.subject_table, "scene");
        assert_eq!(prop.subject_id.as_deref(), Some(saved.scene_id.as_str()));
        assert_eq!(prop.value_kind, "string");
        assert!(matches!(prop.scope, Some(CanonicalFactScope::Invariant)));
        assert!(
            prop.predicate
                .starts_with(&format!("scene.extracted_{}.", idx + 1))
        );
    }
}

/// Planned-and-pending canonical facts (V0038): register without a scene
/// when the decision predates the dramatising scene, then bind it once
/// the scene exists. book/chapter placement anchors the fact meanwhile.
#[tokio::test]
async fn register_canonical_fact_without_scene_binds_later() {
    use spindle_core::models::{
        BindCanonicalFactToSceneInput, ContentRating, RegisterCanonicalFactInput,
        SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Pending Facts".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let character_id = create_character_named(&svc, &proj.project_id, "Unnamed Foundling").await;

    // The name is locked by author decision in planning — the naming
    // scene does not exist yet (the chapter does; scenes are what a fact
    // anchors to).
    let pending = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: proj.project_id.clone(),
            scene_id: None,
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".to_string()),
            subject_id: Some(character_id.clone()),
            predicate: Some("true_name".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("Kaelen Voss".to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .expect("register without scene_id must succeed");

    let fact = svc
        .repository()
        .get_canonical_fact(&pending.canonical_fact_id)
        .await
        .unwrap();
    assert!(
        fact.scene_id.is_none(),
        "planned-and-pending fact must carry no scene"
    );
    assert_eq!(fact.book_number, 1);
    assert_eq!(fact.chapter_number, 1);

    // The naming scene is drafted later, then the fact is bound to it.
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "\"Kaelen,\" she said. \"That is your name.\"".into(),
            summary: "The foundling is named.".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    svc.bind_canonical_fact_to_scene(BindCanonicalFactToSceneInput {
        canonical_fact_id: pending.canonical_fact_id.clone(),
        scene_id: scene.scene_id.clone(),
    })
    .await
    .expect("bind must succeed");
    let bound = svc
        .repository()
        .get_canonical_fact(&pending.canonical_fact_id)
        .await
        .unwrap();
    assert_eq!(bound.scene_id.as_deref(), Some(scene.scene_id.as_str()));

    // Binding a superseded fact is rejected (history is not re-anchored).
    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: proj.project_id.clone(),
        scene_id: Some(scene.scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: None,
        key: None,
        value: None,
        context: None,
        subject_table: Some("character".to_string()),
        subject_id: Some(character_id.clone()),
        predicate: Some("true_name".to_string()),
        value_kind: Some("string".to_string()),
        value_text: Some("Kaelen Voss of the Flats".to_string()),
        value_number: None,
        value_unit: None,
        value_json: None,
        aliases: Vec::new(),
        scope: None,
        valid_from: None,
        valid_until: None,
        legacy_untyped: None,
        supersedes_fact_id: Some(pending.canonical_fact_id.clone()),
        secrecy: None,
    })
    .await
    .unwrap();
    let err = svc
        .bind_canonical_fact_to_scene(BindCanonicalFactToSceneInput {
            canonical_fact_id: pending.canonical_fact_id.clone(),
            scene_id: scene.scene_id.clone(),
        })
        .await
        .expect_err("binding a superseded fact must fail");
    assert!(format!("{err:#}").contains("superseded"));
}

/// Bug 3: day_index is a signed offset from the story's opening, and
/// every adult character was born before day 1 — the old >= 0 bound made
/// set_character_birth unusable for essentially all contemporary fiction,
/// forcing ages into prose summaries where nothing can validate them.
#[tokio::test]
async fn set_character_birth_accepts_negative_day_index() {
    use spindle_core::models::{SetCharacterBirthInput, StoryClock};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Birthdays".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let character_id = create_character_named(&svc, &proj.project_id, "Mara").await;

    svc.set_character_birth(SetCharacterBirthInput {
        project_id: proj.project_id.clone(),
        character_id: character_id.clone(),
        clock: StoryClock {
            day_index: Some(-8913),
            precision: Some("day".into()),
            ..Default::default()
        },
    })
    .await
    .expect("birth before the epoch must be accepted");

    let stored = svc
        .repository()
        .get_character_birth(&character_id)
        .await
        .unwrap()
        .expect("birth stored");
    assert_eq!(stored.clock.day_index, Some(-8913));
    assert_eq!(stored.clock.precision.as_deref(), Some("day"));

    // Durations are magnitudes and stay non-negative.
    let err = svc
        .set_character_birth(SetCharacterBirthInput {
            project_id: proj.project_id.clone(),
            character_id,
            clock: StoryClock {
                day_index: Some(-8913),
                duration_days: Some(-1.0),
                ..Default::default()
            },
        })
        .await
        .expect_err("negative duration must still be rejected");
    assert!(format!("{err:#}").contains("duration_days"));
}

/// A style world rule carrying a word-count target is cross-checked
/// against reader_contract.style_notes when it is updated.
#[tokio::test]
async fn update_world_rule_style_contradicts_reader_contract_warns() {
    use spindle_core::models::{CreateWorldRuleInput, UpdateWorldRuleInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Rule Contradiction".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: vec!["Chapters are 1,200–2,000 words.".into()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let rule = svc
        .create_world_rule(CreateWorldRuleInput {
            project_id: proj.project_id.clone(),
            rule_name: "Chapter Length".into(),
            rule_type: "style".into(),
            description: "Chapters are 1,200–2,000 words.".into(),
            scan_pattern: None,
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();

    // Updating the rule to a disjoint target warns (reader_contract still
    // carries the old figure — exactly the reported corruption).
    let out = svc
        .update_world_rule(UpdateWorldRuleInput {
            world_rule_id: rule.world_rule_id.clone(),
            changes: serde_json::json!({
                "description": "Chapters are 3,500–5,000 words."
            }),
        })
        .await
        .unwrap();
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("style-surface contradiction"));
}

/// The arc snapshot must project the full milestone array; without it the
/// milestones are write-once (the audit advice to "move its placement"
/// required resending descriptions the caller could not read).
#[tokio::test]
async fn get_entity_character_arc_projects_milestones() {
    use spindle_core::models::{
        CharacterArcMilestone, CharacterVoiceProfileData, CreateCharacterArcInput,
        CreateCharacterInput, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Arc Projection".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let character = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara Oathkeeper".into(),
            summary: "Warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData::default(),
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData::default(),
            initial_state: None,
        })
        .await
        .unwrap();
    let arc = svc
        .create_character_arc(CreateCharacterArcInput {
            project_id: proj.project_id.clone(),
            character_id: character.character_id.clone(),
            arc_type: "belief_shift".into(),
            starting_state: "obeys".into(),
            ending_state: "defies".into(),
            milestones: vec![CharacterArcMilestone {
                label: "first doubt".into(),
                placement: Some(StoryPlacement {
                    book_number: 1,
                    chapter_number: 10,
                    scene_order: None,
                    note: None,
                }),
                description: "Mara questions the oath.".into(),
                unlocks: vec!["inner monologue".into()],
                reached_at: None,
            }],
            thematic_purpose: "Claim agency.".into(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();

    let snapshot = svc
        .get_entity(spindle_core::models::GetEntityInput {
            project_id: proj.project_id.clone(),
            table: spindle_core::subject::SubjectTable::CharacterArc,
            entity_id: arc.character_arc_id.clone(),
            branch_id: None,
        })
        .await
        .unwrap();
    let spindle_core::subject_snapshot::SubjectKindSpecific::CharacterArc(details) =
        snapshot.kind_specific()
    else {
        panic!("expected character_arc kind-specific details");
    };
    assert_eq!(details.milestones.len(), 1);
    assert_eq!(details.milestones[0].label, "first doubt");
    assert_eq!(
        details.milestones[0].description,
        "Mara questions the oath."
    );
    assert_eq!(
        details.milestones[0].unlocks,
        vec!["inner monologue".to_string()]
    );
    assert_eq!(
        details.milestones[0]
            .placement
            .as_ref()
            .map(|p| p.chapter_number),
        Some(10)
    );
}

#[tokio::test]
async fn get_entity_returns_snapshot_for_character_subject() {
    use spindle_core::models::{
        CharacterStatePatch, CharacterVoiceProfileData, CreateCharacterInput, GetEntityInput,
    };
    use spindle_core::subject::SubjectTable;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Entity".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
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
        .await
        .unwrap();

    let snap = svc
        .get_entity(GetEntityInput {
            project_id: proj.project_id.clone(),
            table: SubjectTable::Character,
            entity_id: mara.character_id.clone(),
            branch_id: None,
        })
        .await
        .unwrap();
    assert_eq!(snap.subject().id(), Some(mara.character_id.as_str()));
}

#[tokio::test]
async fn get_entity_rejects_project_subject_table() {
    use spindle_core::models::GetEntityInput;
    use spindle_core::subject::SubjectTable;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "X".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let err = svc
        .get_entity(GetEntityInput {
            project_id: proj.project_id.clone(),
            table: SubjectTable::Project,
            entity_id: proj.project_id.clone(),
            branch_id: None,
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("table=project"));
}

#[tokio::test]
async fn get_character_snapshot_unpacks_into_output_fields() {
    use spindle_core::models::{
        CharacterStatePatch, CharacterVoiceProfileData, CreateCharacterInput,
        GetCharacterSnapshotInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Snap".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: Some(vec!["watching the gate".into()]),
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

    let out = svc
        .get_character_snapshot(GetCharacterSnapshotInput {
            project_id: proj.project_id.clone(),
            character_id: mara.character_id.clone(),
            branch_id: None,
        })
        .await
        .unwrap();
    assert_eq!(
        out.snapshot.subject().id(),
        Some(mara.character_id.as_str())
    );
    assert!(
        out.voice_profile.is_some(),
        "voice profile must be unpacked off the snapshot"
    );
}

/// The mixed-rating chapter contract: one Explicit scene sitting among
/// General ones must be the ONLY scene that reaches the explicit-capable
/// agent. Its neighbours stay on the default agent.
///
/// This is the acceptance test for per-scene routing. It drives the real
/// seam — the planned scene's `content_rating` read back out of the stored
/// chapter plan, handed to the router exactly as the drafting path hands it
/// over — and asserts on the router's own dispatch recorder, so it proves
/// which agent each scene actually reached rather than restating routing
/// logic. A chapter-level or run-level rating collapse (drafting the whole
/// chapter as Explicit because one scene is, or refusing the Explicit scene
/// because the chapter reads General) fails it in one direction or the other.
///
/// Hermetic: `fresh_service_local` + a TempDir agent config. The `local`
/// adapter cannot serve a continuation, so the model calls error — but the
/// dispatch is recorded at the clearance gate BEFORE the adapter runs, which
/// is the seam under test.
mod mixed_rating_chapter_routing {
    use super::*;
    use crate::ai::DispatchRecord;
    use spindle_core::models::{
        ConfigureAgentsInput, ContentRating, ContinueGenerationInput, PlanChapterInput,
        PlanChapterSceneInput,
    };

    /// Two drafting agents, the shape the task describes: a default agent
    /// serving general/teen/mature, and a separate uncensored agent serving
    /// only explicit, bound to `draft` via a per-rating override.
    fn install_split_rating_agents(tmp: &TempDir, svc: &SqliteSpindleService) {
        let config_path = tmp.path().join("split-rating.toml");
        std::fs::write(
            &config_path,
            r#"
[health_check]
enabled = false

[[agents]]
id = "default-agent"
name = "Default Agent"
provider = "local"
endpoint = "local"
model = "default-agent"
ratings = ["general", "teen", "mature"]

[[agents]]
id = "explicit-agent"
name = "Uncensored Agent"
provider = "local"
endpoint = "local"
model = "explicit-agent"
ratings = ["explicit"]

[[routing]]
route = "draft"
agent = "default-agent"

[[routing]]
route = "draft"
agent = "explicit-agent"
rating = "explicit"
"#,
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.display().to_string()),
        })
        .unwrap();
    }

    #[tokio::test]
    async fn each_scene_drafts_through_the_agent_for_its_own_rating() {
        let (tmp, svc) = fresh_service_local().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "mixed-rating".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "One hot scene, the rest on the wall.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        install_split_rating_agents(&tmp, &svc);

        // General, General, Explicit — the chapter shape from the brief.
        let planned = [
            (1, ContentRating::General, "Muster on the wall."),
            (2, ContentRating::General, "The war council argues."),
            (3, ContentRating::Explicit, "Mara and the smith, after."),
        ];
        svc.plan_chapter(PlanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            pov_character_id: None,
            synopsis: "Siege prep, then the night before.".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: planned
                .iter()
                .map(|(order, rating, summary)| PlanChapterSceneInput {
                    scene_order: *order,
                    summary: (*summary).into(),
                    beat_structure: Vec::new(),
                    character_ids: Vec::new(),
                    location_id: None,
                    content_rating: Some(rating.clone()),
                    purpose: "Advance the siege.".into(),
                    research_required: None,
                    research_tags: Vec::new(),
                    explicit_query: None,
                })
                .collect(),
        })
        .await
        .unwrap();

        // Read the ratings back out of the STORED plan rather than reusing
        // the input: the plan round-trip is part of what's under test.
        let stored = svc
            .repository
            .list_chapter_plans_by_project(&project.project_id)
            .await
            .unwrap()
            .into_iter()
            .find(|plan| plan.book_number == 1 && plan.chapter_number == 1)
            .expect("chapter plan should persist");

        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();
        for scene in &stored.scenes {
            let rating = scene
                .content_rating
                .as_ref()
                .map(|rating| rating.as_str().to_ascii_lowercase())
                .expect("every planned scene carries its own rating");
            // Errors are expected — `local` cannot continue a generation.
            // The dispatch is recorded at the clearance gate regardless.
            let _ = svc
                .continue_generation(ContinueGenerationInput {
                    route: "draft".into(),
                    original_prompt: format!("Draft scene {}", scene.scene_order),
                    prior_output: String::new(),
                    rating: Some(rating),
                    project_id: Some(project.project_id.clone()),
                    book_id: None,
                    chapter_id: None,
                    scene_id: None,
                    ..Default::default()
                })
                .await;
        }

        let dispatched: Vec<(String, String)> = log
            .lock()
            .unwrap()
            .iter()
            .filter_map(|record| match record {
                DispatchRecord::Dispatch { agent, rating, .. } => {
                    Some((agent.clone(), rating.clone().unwrap_or_default()))
                }
                DispatchRecord::Rejection { .. } => None,
            })
            .collect();

        assert_eq!(
            dispatched,
            vec![
                ("default-agent".to_string(), "general".to_string()),
                ("default-agent".to_string(), "general".to_string()),
                ("explicit-agent".to_string(), "explicit".to_string()),
            ],
            "each scene must dispatch to the agent serving its OWN rating; \
                 got {dispatched:?}"
        );

        // No clearance rejection: the explicit scene found its agent, and
        // the general scenes were never pushed at one that would refuse.
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .all(|record| !matches!(record, DispatchRecord::Rejection { .. })),
            "a correctly-routed mixed chapter should produce no clearance rejections"
        );
    }
}

/// `select_alternative` must refuse to switch when the underlying
/// merge has conflicts — the caller is expected to resolve them and
/// retry rather than silently leave the project's active branch
/// half-promoted.
#[tokio::test]
async fn select_alternative_bails_when_merge_has_conflicts() {
    use spindle_core::models::{
        ContentRating, CreateBranchInput, SaveSceneDraftInput, SelectAlternativeInput,
        SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Select Conflict".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Same conflict setup as merge_branch_reports_conflict_*: main
    // baseline, then feature branch, then main edits the position,
    // then feature edits the position. Conflict at (1,1,1).
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "main v1".into(),
        summary: "main v1".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "feature-conf".into(),
            branch_type: "feature".into(),
            description: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "main v2".into(),
        summary: "main v2".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: feature.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "feature edit".into(),
        summary: "feature edit".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let err = svc
        .select_alternative(SelectAlternativeInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("scene conflict"),
        "error should mention the conflict; got: {err}"
    );

    // Active branch is still feature — bail-out did not flip it to main.
    let active = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    assert_eq!(active.id, feature.branch_id);
}

/// Exercises the happy path: a project with one scene runs all checks
/// and reports zero blocking issues (no errors). The summary still
/// reports info findings for things like the missing chapter summary
/// and tone metadata, which we assert positively to keep the SQLite
/// port's dispatch behaviour observable.
#[tokio::test]
async fn check_consistency_happy_path_no_blocking_issues() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, ContextFormat,
        SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "chk".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "A quiet scene.".into(),
        summary: "stage".into(),
        content_rating: ContentRating::General,
        tone: Some("calm".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: None,
            subjects: Vec::new(),
            format: Some(ContextFormat::Markdown),
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert_eq!(
        out.summary.error_count, 0,
        "happy path should produce no blocking errors, got issues: {:?}",
        out.issues
    );
    assert!(
        out.markdown.is_some(),
        "markdown should be populated when format=Markdown"
    );
    assert!(
        out.report_sections.is_empty(),
        "happy path has no canon → Phase-4 fan-out produces no sections"
    );
}

#[tokio::test]
async fn commit_surfaces_premature_knowledge_retcon() {
    use spindle_core::models::{
        CharacterVoiceProfileData, CommitSceneChangesInput, ContentRating, CreateCharacterInput,
        RecordKnowledgeInput, RetconFinding, SaveSceneDraftInput, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "pk".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    // Mara only learns the vault password at chapter 40.
    svc.record_knowledge(RecordKnowledgeInput {
        project_id: proj.project_id.clone(),
        branch_id: None,
        character_id: mara.character_id.clone(),
        fact: "knows the vault password is raven".into(),
        source_summary: "told by the archivist".into(),
        learned_at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 40,
            scene_order: Some(1),
            note: None,
        }),
        confidence: None,
        tags: Vec::new(),
        reader_visible: true,
        secret_of_fact_id: None,
    })
    .await
    .unwrap();

    // But an early scene already has her reference it (long before ch40).
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara knows the vault password is raven, somehow.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let out = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: proj.project_id.clone(),
            scene_id: scene.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: None,
        })
        .await
        .unwrap();

    assert!(
        out.retcon_findings
            .iter()
            .any(|f| matches!(f, RetconFinding::PrematureKnowledge { .. })),
        "premature knowledge should be surfaced: {:?}",
        out.retcon_findings
    );
}

#[tokio::test]
async fn serial_context_keeps_recent_books_and_old_commitments_without_future_summaries() {
    let (_tmp, svc) = fresh_service_local().await;
    let project_id = recap_project(&svc, "Long serial").await;
    let branch = svc.repository.get_active_branch(&project_id).await.unwrap();
    for book in 1..=6 {
        for chapter in 1..=40 {
            svc.repository
                .save_summary(&SaveSummaryInput {
                    project_id: project_id.clone(),
                    book_number: book,
                    chapter_number: chapter,
                    entity_type: None,
                    entity_id: None,
                    summary: format!("BOOK{book}CH{chapter} {}", "物語".repeat(300)),
                    key_events: vec![],
                    character_changes: vec![],
                    relationship_shifts: vec![],
                    arc_advances: vec![],
                    promise_events: vec![],
                })
                .await
                .unwrap();
        }
    }
    svc.create_narrative_promise(CreateNarrativePromiseInput {
        project_id: project_id.clone(),
        promise_type: "mystery".into(),
        description: "ANCIENTKEY still missing".into(),
        planted_at: StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        planned_payoff: None,
        notes: vec![],
    })
    .await
    .unwrap();
    let context = svc
        .story_so_far(&project_id, &branch.id, 6, 20, 1)
        .await
        .unwrap();
    assert!(context.contains("BOOK6CH19"), "{context}");
    assert!(context.contains("BOOK5CH40"), "{context}");
    assert!(context.contains("ANCIENTKEY"), "{context}");
    assert!(!context.contains("BOOK6CH20") && !context.contains("BOOK6CH40"));
    assert!(context.contains("earlier books omitted"));
    assert!(context.len() <= 2000, "{} bytes", context.len());
    assert!(
        !context.contains("BOOK1CH"),
        "early books must not crowd out recent ones"
    );
}

#[tokio::test]
async fn save_summary_maintains_book_digest_idempotently() {
    use spindle_core::models::SaveSummaryInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "digest".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let repo = svc.repository();
    let branch = repo.get_active_branch(&proj.project_id).await.unwrap();
    let mk = |chapter: i32, summary: &str| SaveSummaryInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: chapter,
        entity_type: None,
        entity_id: None,
        summary: summary.into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    };

    repo.save_summary(&mk(1, "Mara reaches the Ash Gate"))
        .await
        .unwrap();
    repo.save_summary(&mk(2, "Mara duels the warden"))
        .await
        .unwrap();

    let digests = repo
        .list_book_digests_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap();
    assert_eq!(digests.len(), 1, "one digest per book");
    assert_eq!(digests[0].book_number, 1);
    assert_eq!(digests[0].last_chapter_covered, 2);
    assert!(digests[0].synopsis.contains("Ash Gate"));
    assert!(digests[0].synopsis.contains("duels the warden"));

    // Re-saving a chapter must NOT double-count (idempotent re-derive).
    repo.save_summary(&mk(2, "Mara duels the warden"))
        .await
        .unwrap();
    let digests = repo
        .list_book_digests_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap();
    assert_eq!(digests.len(), 1);
    assert_eq!(digests[0].last_chapter_covered, 2);
    assert_eq!(
        digests[0].synopsis.matches("duels the warden").count(),
        1,
        "chapter must appear exactly once after re-save"
    );
}

#[tokio::test]
async fn save_summary_populates_open_threads_from_unresolved_threads() {
    use spindle_core::models::{
        CreateConflictInput, CreateNarrativePromiseInput, CreatePlotLineInput, SaveSummaryInput,
        StatedConsequence, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "threads".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // An active promise (not paid_off/abandoned) with placements.
    let promise = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: proj.project_id.clone(),
            promise_type: "setup".into(),
            description: "the buried crown will be reclaimed".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 10,
                scene_order: Some(0),
                note: None,
            }),
            notes: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_promise_status(spindle_core::models::UpdatePromiseStatusInput {
        narrative_promise_id: promise.narrative_promise_id.clone(),
        status: "active".into(),
        note: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // A conflict with an undelivered stated consequence.
    svc.create_conflict(CreateConflictInput {
        project_id: proj.project_id.clone(),
        name: "Siege of the Ash Gate".into(),
        conflict_type: "external".into(),
        stakes: "the last free city".into(),
        escalation_stages: Vec::new(),
        expected_total_cycles: None,
        try_fail_cycles: Vec::new(),
        stated_consequences: vec![StatedConsequence {
            description: "the wall will crack".into(),
            stated_at: None,
            must_demonstrate_by: None,
            delivered: false,
        }],
    })
    .await
    .unwrap();

    // A plot line not yet complete.
    svc.create_plot_line(CreatePlotLineInput {
        project_id: proj.project_id.clone(),
        name: "The Reclamation".into(),
        plot_type: "main".into(),
        summary: "the rightful heir returns".into(),
        status: Some("developing".into()),
        convergence_points: Vec::new(),
        connected_conflict_ids: Vec::new(),
        connected_theme_ids: Vec::new(),
    })
    .await
    .unwrap();

    let repo = svc.repository();
    let branch = repo.get_active_branch(&proj.project_id).await.unwrap();
    repo.save_summary(&SaveSummaryInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "The heir flees the burning capital".into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();

    let digests = repo
        .list_book_digests_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap();
    assert_eq!(digests.len(), 1);
    let threads = &digests[0].open_threads;
    assert!(!threads.is_empty(), "open threads persisted: {threads:?}");
    assert!(
        threads
            .iter()
            .any(|t| t.contains("the buried crown will be reclaimed")),
        "promise carried into open threads: {threads:?}"
    );
    assert!(
        threads.iter().any(|t| t.contains("Siege of the Ash Gate")),
        "conflict carried into open threads: {threads:?}"
    );
    assert!(
        threads.iter().any(|t| t.contains("The Reclamation")),
        "plot line carried into open threads: {threads:?}"
    );
}

#[tokio::test]
async fn save_summary_open_threads_deterministic_across_rebuilds() {
    use spindle_core::models::{
        CreateConflictInput, CreateNarrativePromiseInput, CreatePlotLineInput, SaveSummaryInput,
        StatedConsequence, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "detThreads".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    for n in 0..4 {
        svc.create_narrative_promise(CreateNarrativePromiseInput {
            project_id: proj.project_id.clone(),
            promise_type: "setup".into(),
            description: format!("promise number {n}"),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: n + 2,
                scene_order: Some(0),
                note: None,
            }),
            notes: Vec::new(),
        })
        .await
        .unwrap();
    }
    svc.create_conflict(CreateConflictInput {
        project_id: proj.project_id.clone(),
        name: "A Conflict".into(),
        conflict_type: "external".into(),
        stakes: "high".into(),
        escalation_stages: Vec::new(),
        expected_total_cycles: None,
        try_fail_cycles: Vec::new(),
        stated_consequences: vec![StatedConsequence {
            description: "cost".into(),
            stated_at: None,
            must_demonstrate_by: None,
            delivered: false,
        }],
    })
    .await
    .unwrap();
    svc.create_plot_line(CreatePlotLineInput {
        project_id: proj.project_id.clone(),
        name: "A Plot".into(),
        plot_type: "main".into(),
        summary: "s".into(),
        status: Some("developing".into()),
        convergence_points: Vec::new(),
        connected_conflict_ids: Vec::new(),
        connected_theme_ids: Vec::new(),
    })
    .await
    .unwrap();

    let repo = svc.repository();
    let branch = repo.get_active_branch(&proj.project_id).await.unwrap();
    let mk = |summary: &str| SaveSummaryInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: summary.into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    };

    repo.save_summary(&mk("first build")).await.unwrap();
    let first = repo
        .list_book_digests_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap()[0]
        .open_threads
        .clone();
    // Rebuild (idempotent re-derive over the same chapter).
    repo.save_summary(&mk("first build")).await.unwrap();
    let second = repo
        .list_book_digests_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap()[0]
        .open_threads
        .clone();
    assert_eq!(first, second, "open_threads byte-identical across rebuilds");
    assert!(!first.is_empty(), "threads present: {first:?}");
}

#[tokio::test]
async fn check_consistency_flags_pacing_drift_on_sustained_sag() {
    use spindle_core::models::{
        AnnotateSceneBeatsInput, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        CreateChapterInput, CreatePacingCurveInput, SaveSceneDraftInput,
    };
    use std::collections::BTreeMap;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "drift".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    svc.create_pacing_curve(CreatePacingCurveInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        act_breakpoints: BTreeMap::new(),
        scene_type_density: BTreeMap::new(),
        intensity_points: Vec::new(),
    })
    .await
    .unwrap();

    // Four chapters with steadily falling realized intensity.
    for (idx, intensity) in [0.9_f64, 0.7, 0.5, 0.3].iter().enumerate() {
        let chapter = idx as i32 + 1;
        svc.create_chapter(CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(chapter),
            title: None,
        })
        .await
        .unwrap();
        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: chapter,
                chapter_id: None,
                scene_order: 1,
                full_text: "x".into(),
                summary: "s".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();
        svc.annotate_scene_beats(AnnotateSceneBeatsInput {
            project_id: proj.project_id.clone(),
            scene_id: scene.scene_id.clone(),
            beats: Vec::new(),
            motif_ids: Vec::new(),
            theme_ids: Vec::new(),
            conflict_ids: Vec::new(),
            intensity: Some(*intensity),
        })
        .await
        .unwrap();
    }

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["pacing_drift".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        out.issues.iter().any(|i| i.check_type == "pacing_drift"),
        "pacing_drift should flag a sustained intensity sag: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn check_consistency_flags_premature_knowledge() {
    use spindle_core::models::{
        CharacterVoiceProfileData, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        CreateCharacterInput, RecordKnowledgeInput, SaveSceneDraftInput, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "kt".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();
    svc.record_knowledge(RecordKnowledgeInput {
        project_id: proj.project_id.clone(),
        branch_id: None,
        character_id: mara.character_id.clone(),
        fact: "knows the vault password is raven".into(),
        source_summary: "told later".into(),
        learned_at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 40,
            scene_order: Some(1),
            note: None,
        }),
        confidence: None,
        tags: Vec::new(),
        reader_visible: true,
        secret_of_fact_id: None,
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara knows the vault password is raven, somehow.".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["knowledge_timing".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        out.issues
            .iter()
            .any(|i| i.check_type == "knowledge_timing"),
        "check_consistency should flag premature knowledge: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn save_summary_recomputes_pacing_over_budget() {
    use spindle_core::models::{
        CharacterVoiceProfileData, CheckConsistencyInput, ConsistencyScopeInput,
        CreateCharacterArcInput, CreateCharacterInput, CreatePacingConfigInput, SaveSummaryInput,
        SetArcPacingConstraintsInput,
    };
    use std::collections::BTreeMap;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "pace".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();
    svc.create_pacing_config(CreatePacingConfigInput {
        project_id: proj.project_id.clone(),
        total_planned_books: 1,
        avg_chapters_per_book: 10,
        avg_scenes_per_chapter: 3,
        tension_model: "rising".into(),
    })
    .await
    .unwrap();
    let arc = svc
        .create_character_arc(CreateCharacterArcInput {
            project_id: proj.project_id.clone(),
            character_id: mara.character_id.clone(),
            arc_type: "growth".into(),
            starting_state: "naive".into(),
            ending_state: "wise".into(),
            milestones: Vec::new(),
            thematic_purpose: "coming of age".into(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    svc.set_arc_pacing_constraints(SetArcPacingConstraintsInput {
        project_id: proj.project_id.clone(),
        character_arc_id: arc.character_arc_id.clone(),
        per_book_budget: BTreeMap::from([("1".to_string(), 0.3)]),
        max_progress_per_chapter: Some(0.1),
        milestone_spacing: None,
        sprint_allowance: None,
        regression_budget: None,
    })
    .await
    .unwrap();

    // 4 chapters * 0.1 = 0.4 realized > 0.3 book budget -> over budget.
    for chapter in 1..=4 {
        svc.save_summary(SaveSummaryInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: chapter,
            entity_type: None,
            entity_id: None,
            summary: format!("chapter {chapter}"),
            key_events: Vec::new(),
            character_changes: Vec::new(),
            relationship_shifts: Vec::new(),
            arc_advances: Vec::new(),
            promise_events: Vec::new(),
        })
        .await
        .unwrap();
    }

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["pacing_budget_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        out.issues
            .iter()
            .any(|i| i.check_type == "pacing_budget_audit" && i.message.contains("over budget")),
        "pacing audit should fire once the arc outpaces its budget: {:?}",
        out.issues
    );
}

#[tokio::test]
async fn create_pacing_config_and_curve_upsert_instead_of_failing_on_recreate() {
    use spindle_core::models::{CreatePacingConfigInput, CreatePacingCurveInput};
    use std::collections::BTreeMap;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "pace-upsert".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let first = svc
        .create_pacing_config(CreatePacingConfigInput {
            project_id: proj.project_id.clone(),
            total_planned_books: 1,
            avg_chapters_per_book: 10,
            avg_scenes_per_chapter: 3,
            tension_model: "rising".into(),
        })
        .await
        .unwrap();
    // Regression: the upsert conflict path used to re-read by the freshly
    // minted id and fail with "pacing_config not found".
    let second = svc
        .create_pacing_config(CreatePacingConfigInput {
            project_id: proj.project_id.clone(),
            total_planned_books: 3,
            avg_chapters_per_book: 12,
            avg_scenes_per_chapter: 4,
            tension_model: "wave".into(),
        })
        .await
        .expect("re-creating a pacing config should upsert, not fail");
    assert_eq!(
        first.pacing_config_id, second.pacing_config_id,
        "upsert keeps the original row id"
    );

    let curve_first = svc
        .create_pacing_curve(CreatePacingCurveInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            act_breakpoints: BTreeMap::from([("act1".to_string(), 0.25)]),
            scene_type_density: BTreeMap::new(),
            intensity_points: Vec::new(),
        })
        .await
        .unwrap();
    let curve_second = svc
        .create_pacing_curve(CreatePacingCurveInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            act_breakpoints: BTreeMap::from([("act1".to_string(), 0.3)]),
            scene_type_density: BTreeMap::new(),
            intensity_points: Vec::new(),
        })
        .await
        .expect("re-creating a pacing curve should upsert, not fail");
    assert_eq!(
        curve_first.pacing_curve_id, curve_second.pacing_curve_id,
        "upsert keeps the original row id"
    );
}

/// Phase-4 scanners inside `revise_scene`: after seeding a character
/// with a `forbidden_words` voice profile, revising a scene on a
/// non-main branch to inject that forbidden phrase into the
/// character's attributed dialogue must surface a `voice_drift`
/// finding in the response.
#[tokio::test]
async fn revise_scene_emits_voice_drift_finding() {
    use spindle_core::models::{
        CharacterVoiceProfileData, ContentRating, CreateBookInput, CreateBranchInput,
        CreateChapterInput, ReviseSceneInput, SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ph4-revise".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    svc.create_book(CreateBookInput {
        project_id: proj.project_id.clone(),
        title: Some("Book One".into()),
    })
    .await
    .unwrap();
    svc.create_chapter(CreateChapterInput {
        project_id: proj.project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(1),
        title: Some("Chapter One".into()),
    })
    .await
    .unwrap();

    // Character whose voice profile forbids the phrase "as you know".
    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: proj.project_id.clone(),
        name: "Jim Dalton".into(),
        summary: "Dockmaster".into(),
        role: "supporting".into(),
        realm: None,
        voice_profile: CharacterVoiceProfileData {
            tone: Some("plainspoken".into()),
            vocabulary: Vec::new(),
            sentence_structure: Vec::new(),
            tics: Vec::new(),
            forbidden_words: vec!["as you know".into()],
            example_lines: Vec::new(),
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
            base_emotions: std::collections::BTreeMap::new(),
            suppressed: Vec::new(),
            triggers: Vec::new(),
            defense_mechanisms: Vec::new(),
            flex_range: None,
        },
        initial_state: None,
    })
    .await
    .unwrap();

    // Initial scene on main: clean prose, no forbidden phrase.
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Jim Dalton tapped the table and said, \"Keep it moving.\"".into(),
            summary: "clean".into(),
            content_rating: ContentRating::General,
            tone: Some("plainspoken".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // revise_scene requires a non-main active branch.
    let branch = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "revisions".into(),
            branch_type: "draft".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: branch.branch_id.clone(),
    })
    .await
    .unwrap();

    let out = svc
            .revise_scene(ReviseSceneInput {
                project_id: proj.project_id.clone(),
                scene_id: scene.scene_id.clone(),
                full_text:
                    "Jim Dalton tapped the table and said, \"As you know, the docks are quiet tonight.\""
                        .into(),
                summary: "drift".into(),
                content_rating: ContentRating::General,
                tone: Some("plainspoken".into()),
            })
            .await
            .unwrap();

    assert!(
        !out.voice_drift.is_empty(),
        "expected at least one voice_drift finding, got {:?}",
        out.voice_drift
    );
    let drift = &out.voice_drift[0];
    assert_eq!(
        drift.forbidden_phrase.as_deref(),
        Some("as you know"),
        "voice_drift finding should cite the forbidden phrase: {:?}",
        drift
    );
}

/// Subjects narrowing actually filters the scene set against the
/// supplied subjects (Gap 6). Pinning a character via the `subjects`
/// list must restrict the validator pass to scenes that reference
/// that character's name in their prose — scenes that don't
/// reference it are not validated.
#[tokio::test]
async fn check_consistency_subjects_narrowing_filters_scene_set() {
    use spindle_core::models::{
        CharacterStatePatch, CharacterVoiceProfileData, CheckConsistencyInput,
        ConsistencyScopeInput, ContentRating, CreateCharacterInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "chk-subj".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
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
        .await
        .unwrap();

    // Scene 1: contains "Mara" by name → should pass narrowing.
    // Scene 2: doesn't reference Mara at all → should be filtered out.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara stood at the Ash Gate, watching the dark.".into(),
        summary: "stage Mara".into(),
        content_rating: ContentRating::General,
        tone: Some("calm".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 2,
        full_text: "Aldric pondered the empty courtyard, unrelated.".into(),
        summary: "unrelated beat".into(),
        content_rating: ContentRating::General,
        tone: Some("calm".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: None,
            subjects: vec![mara.character_id.clone()],
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    // No info notice fires anymore; narrowing is real.
    assert!(
        !out.issues
            .iter()
            .any(|issue| issue.check_type == "subjects_narrowing"),
        "subjects_narrowing info notice must be gone: got {:?}",
        out.issues
    );
}

/// Phase-4 world_rule_semantic_drift wiring: with a project that has a
/// world rule whose scan_pattern matches scene prose AND surrounding
/// violation-context markers ("ignore"), `check_consistency` must
/// produce at least one `world_rule_semantic_drift` issue and a
/// matching section in `report_sections`.
#[tokio::test]
async fn check_consistency_emits_world_rule_semantic_drift_issue() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateWorldRuleInput,
        EstablishedIn, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ph4-world".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Seed a world rule whose scan_pattern matches the scene prose.
    // The surrounding window must contain a violation marker
    // ("ignore") for the scanner to promote severity to Likely (which
    // the validator maps to Warning).
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: proj.project_id.clone(),
        rule_name: "Sigils require contact".into(),
        rule_type: "magic".into(),
        description: "Magic sigils must require physical contact".into(),
        scan_pattern: Some(r"\bsigil\b".into()),
        relevance_tags: Vec::new(),
        established_in: Some(EstablishedIn {
            book_number: 1,
            chapter_number: 1,
            note: None,
        }),
    })
    .await
    .unwrap();

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Eldrin tried to ignore the sigil and cast at range anyway, breaking canon."
            .into(),
        summary: "violation beat".into(),
        content_rating: ContentRating::General,
        tone: Some("grim".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    assert!(
        out.issues.iter().any(|issue| {
            issue.check_type == "world_rule_semantic_drift" && issue.severity == "warning"
        }),
        "expected a world_rule_semantic_drift warning, got: {:?}",
        out.issues
    );
    assert!(
        out.report_sections
            .iter()
            .any(
                |section| section.validator_id == "world_rule_semantic_drift"
                    && !section.scenes.is_empty()
            ),
        "expected a world_rule_semantic_drift section in report_sections"
    );
}

/// `read_project_resource` must parse the `scene-delete-impact/...`
/// and `scene-move-impact/...` path tails into the impact-method
/// inputs and return the structured JSON payload. This is the
/// integration test for the dispatch wiring; the per-method tests
/// above already cover the analysis logic.
#[tokio::test]
async fn read_project_resource_dispatches_scene_impact_arms() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "impact-dispatch".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Delete impact: clear envelope, scene fields echo the saved scene.
    let delete_json = svc
        .read_project_resource(&proj.project_id, "scene-delete-impact/1/1/1")
        .await
        .unwrap();
    assert_eq!(delete_json["delete_readiness"], "clear");
    assert_eq!(delete_json["scene"]["scene_id"], scene.scene_id);
    assert_eq!(delete_json["scene"]["book_number"], 1);
    assert_eq!(delete_json["scene"]["chapter_number"], 1);
    assert_eq!(delete_json["scene"]["scene_order"], 1);
    assert!(delete_json["hard_blockers"].is_array());

    // Move impact (no destination scene): clear envelope, destination
    // existing_scene_id resolves to null.
    let move_json = svc
        .read_project_resource(&proj.project_id, "scene-move-impact/1/1/1/1/2/1")
        .await
        .unwrap();
    assert_eq!(move_json["move_readiness"], "clear");
    assert_eq!(move_json["scene"]["scene_id"], scene.scene_id);
    assert_eq!(move_json["destination"]["book_number"], 1);
    assert_eq!(move_json["destination"]["chapter_number"], 2);
    assert_eq!(move_json["destination"]["scene_order"], 1);
    assert!(move_json["destination"]["existing_scene_id"].is_null());

    // Malformed path tail bails with a clear error.
    let err = svc
        .read_project_resource(&proj.project_id, "scene-delete-impact/1/1")
        .await
        .expect_err("missing path segment must error");
    assert!(
        err.to_string()
            .contains("scene delete impact resource path"),
        "got: {err}"
    );
    let err = svc
        .read_project_resource(&proj.project_id, "scene-move-impact/1/1/1/1/2")
        .await
        .expect_err("missing path segment must error");
    assert!(
        err.to_string().contains("scene move impact resource path"),
        "got: {err}"
    );
}

// ── B2: secret_leak deterministic audience-direction audit ────────────
// Design §2.4 deterministic tier: an out-of-circle present character who
// speaks a secret's value in attributed dialogue leaks it → warning.
mod secret_leak_audit {
    use super::*;
    use spindle_core::models::{
        CharacterVoiceProfileData, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        CreateCharacterInput, CreateLocationInput, RecordKnowledgeInput,
        RegisterCanonicalFactInput, SaveSceneDraftInput, SecrecyScope, StoryPlacement,
        WorldStateInput,
    };

    async fn make_character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
        svc.create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project_id.to_string(),
            name: name.to_string(),
            summary: format!("{name} in play."),
            role: "supporting".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap()
        .character_id
    }

    /// Project + Mara (holder) + Bran (outsider) + a location + a secret
    /// fact whose value is the single lexeme "reincarnated". Returns
    /// (svc, project_id, mara, bran, location_id, fact_id).
    async fn fixture() -> (
        tempfile::TempDir,
        SqliteSpindleService,
        String,
        String,
        String,
        String,
        String,
    ) {
        let (tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Leak".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Secrets stay secret.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let project_id = proj.project_id;
        let mara = make_character(&svc, &project_id, "Mara").await;
        let bran = make_character(&svc, &project_id, "Bran").await;
        let location = svc
            .create_location(CreateLocationInput {
                project_id: project_id.clone(),
                name: "Ash Gate".into(),
                kind: "fortress".into(),
                realm: None,
                summary: "A wall.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: Some("tense".into()),
                    prosperity: None,
                    stability: None,
                    threat_level: None,
                    sensory_details: Vec::new(),
                },
            })
            .await
            .unwrap();

        // A seed scene to anchor the secret's declaration.
        let seed = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara kept her counsel.".into(),
                summary: "seed".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let fact = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(seed.scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(mara.clone()),
                predicate: Some("reincarnation".into()),
                value_kind: Some("string".into()),
                value_text: Some("reincarnated".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: Some(SecrecyScope {
                    holder_ids: vec![mara.clone()],
                    concealment_note: None,
                }),
            })
            .await
            .unwrap();

        (
            tmp,
            svc,
            project_id,
            mara,
            bran,
            location.location_id,
            fact.canonical_fact_id,
        )
    }

    async fn save_dialogue_scene(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
        scene_order: i32,
        text: &str,
    ) {
        // Chapter 1 exists from create_project; create later chapters.
        if chapter > 1 {
            let _ = svc
                .create_chapter(spindle_core::models::CreateChapterInput {
                    project_id: project_id.to_string(),
                    book_id: None,
                    book_number: Some(1),
                    chapter_number: Some(chapter),
                    title: None,
                })
                .await;
        }
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
            chapter_id: None,
            scene_order,
            full_text: text.to_string(),
            summary: "dialogue".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    async fn run_secret_leak(
        svc: &SqliteSpindleService,
        project_id: &str,
    ) -> Vec<spindle_core::models::ConsistencyIssue> {
        svc.check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.to_string(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["secret_leak".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap()
        .issues
        .into_iter()
        .filter(|i| i.check_type == "secret_leak")
        .collect()
    }

    #[tokio::test]
    async fn holder_first_name_suppresses_generic_written_disclosure() {
        let (_tmp, svc, project_id, _mara, _bran, _location, _fact) = fixture().await;
        let cassian = make_character(&svc, &project_id, "Cassian Reed").await;
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project_id.clone(),
            scene_id: None,
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("secret".into()),
            key: Some("Cassian:root_cut".into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(cassian.clone()),
            predicate: Some("root_cause".into()),
            value_kind: Some("text".into()),
            value_text: Some("Cassian Reed cut the orchard roots and sold the copper.".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: Some(SecrecyScope {
                holder_ids: vec![cassian],
                concealment_note: None,
            }),
        })
        .await
        .unwrap();
        save_dialogue_scene(
            &svc,
            &project_id,
            2,
            1,
            "Cassian wrote in his private letter that he cut the orchard roots and sold the copper.",
        )
        .await;
        let hits = run_secret_leak(&svc, &project_id).await;
        assert!(
            hits.is_empty(),
            "holder first-name disclosure must stay silent: {hits:?}"
        );
    }

    /// Firing: Bran (out of circle) speaks the secret's value in attributed
    /// dialogue → one warning naming the fact, the character, and the scene.
    #[tokio::test]
    async fn outsider_speaks_secret_value_warns() {
        let (_tmp, svc, project_id, _mara, bran, _loc, _fact) = fixture().await;
        save_dialogue_scene(
            &svc,
            &project_id,
            2,
            1,
            "\"You are reincarnated, I know it,\" Bran said, watching Mara.",
        )
        .await;

        let hits = run_secret_leak(&svc, &project_id).await;
        assert_eq!(hits.len(), 1, "outsider leak must warn once: {hits:?}");
        assert_eq!(hits[0].severity, "warning");
        assert!(
            hits[0].entity_ids.contains(&bran),
            "the leaking character is named: {:?}",
            hits[0].entity_ids
        );
        assert!(
            hits[0]
                .message
                .to_lowercase()
                .contains("no recorded reveal")
                && hits[0].message.contains("Bran"),
            "message names the reveal status and character: {}",
            hits[0].message
        );
    }

    /// Clean: the insider Mara speaks it → no finding (she is in the circle).
    #[tokio::test]
    async fn insider_speaks_secret_value_is_clean() {
        let (_tmp, svc, project_id, _mara, _bran, _loc, _fact) = fixture().await;
        save_dialogue_scene(
            &svc,
            &project_id,
            2,
            1,
            "\"I am reincarnated,\" Mara said, to no one.",
        )
        .await;

        let hits = run_secret_leak(&svc, &project_id).await;
        assert!(
            hits.is_empty(),
            "an insider speaking the secret is not a leak: {hits:?}"
        );
    }

    /// Reveal-timing (earlier): Bran was revealed-to in an EARLIER scene
    /// placement, so at the later leak scene he is in the circle → clean.
    #[tokio::test]
    async fn earlier_reveal_makes_speaker_in_circle_clean() {
        let (_tmp, svc, project_id, _mara, bran, _loc, fact) = fixture().await;
        // Reveal to Bran at ch 2 scene 1.
        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project_id.clone(),
            branch_id: None,
            character_id: bran.clone(),
            fact: "Mara is reincarnated.".into(),
            source_summary: "she told him".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(1),
                note: None,
            }),
            confidence: None,
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: Some(fact.clone()),
        })
        .await
        .unwrap();
        // Leak scene at ch 3 (after the reveal): Bran is now in the circle.
        save_dialogue_scene(
            &svc,
            &project_id,
            3,
            1,
            "\"So you are reincarnated,\" Bran said.",
        )
        .await;

        let hits = run_secret_leak(&svc, &project_id).await;
        assert!(
            hits.is_empty(),
            "a speaker revealed-to earlier is in the circle → no leak: {hits:?}"
        );
    }

    /// Reveal-timing (later): Bran is revealed-to only in a LATER scene, so
    /// at the earlier leak scene he is still out of circle → warning.
    #[tokio::test]
    async fn later_reveal_does_not_excuse_earlier_leak_warns() {
        let (_tmp, svc, project_id, _mara, bran, _loc, fact) = fixture().await;
        // Leak scene at ch 2 (before any reveal).
        save_dialogue_scene(
            &svc,
            &project_id,
            2,
            1,
            "\"You are reincarnated,\" Bran said.",
        )
        .await;
        // Reveal to Bran only at ch 5 — after the leak.
        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project_id.clone(),
            branch_id: None,
            character_id: bran.clone(),
            fact: "Mara is reincarnated.".into(),
            source_summary: "later reveal".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 5,
                scene_order: Some(1),
                note: None,
            }),
            confidence: None,
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: Some(fact.clone()),
        })
        .await
        .unwrap();

        let hits = run_secret_leak(&svc, &project_id).await;
        assert_eq!(
            hits.len(),
            1,
            "a later reveal cannot excuse an earlier leak: {hits:?}"
        );
        assert!(hits[0].entity_ids.contains(&bran));
    }

    /// No secret facts → the audit is silent even with matching prose.
    #[tokio::test]
    async fn no_secrets_is_silent() {
        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Public".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Nothing hidden.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let project_id = proj.project_id;
        make_character(&svc, &project_id, "Bran").await;
        save_dialogue_scene(
            &svc,
            &project_id,
            1,
            1,
            "\"You are reincarnated,\" Bran said.",
        )
        .await;

        let hits = run_secret_leak(&svc, &project_id).await;
        assert!(hits.is_empty(), "no secret facts → silent: {hits:?}");
    }
}

// ── Item 5: secret_leak DEEP tier (model-backed behavioral leak) ───────
// Design §2.4 deep tier: a model pass asks the audience question directly —
// does an out-of-circle present character reference, imply, or act upon a
// secret? Findings add to the same `secret_leak` check_type, suffixed
// "(model-detected behavioral leak)". Gated to `deep_check == true`, rating-
// aware dispatch, honest-skip on route failure, non-present discard, cap 10.
mod secret_leak_deep {
    use super::*;
    use spindle_core::models::{
        CharacterVoiceProfileData, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        CreateCharacterInput, RegisterCanonicalFactInput, SaveSceneDraftInput, SecrecyScope,
    };

    async fn make_character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
        svc.create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project_id.to_string(),
            name: name.to_string(),
            summary: format!("{name} in play."),
            role: "supporting".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap()
        .character_id
    }

    /// Project + Mara (holder) + Bran (outsider) + a secret fact "reincarnated"
    /// whose circle is {Mara}. Returns (tmp, svc, project_id, mara, bran, fact_id).
    async fn fixture() -> (
        tempfile::TempDir,
        SqliteSpindleService,
        String,
        String,
        String,
        String,
    ) {
        let (tmp, svc) = fresh_service_local().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "LeakDeep".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Secrets stay secret.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let project_id = proj.project_id;
        let mara = make_character(&svc, &project_id, "Mara").await;
        let bran = make_character(&svc, &project_id, "Bran").await;

        let seed = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara kept her counsel.".into(),
                summary: "seed".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let fact = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project_id.clone(),
                scene_id: Some(seed.scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(mara.clone()),
                predicate: Some("reincarnation".into()),
                value_kind: Some("string".into()),
                value_text: Some("reincarnated".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: Some(SecrecyScope {
                    holder_ids: vec![mara.clone()],
                    concealment_note: None,
                }),
            })
            .await
            .unwrap();

        (tmp, svc, project_id, mara, bran, fact.canonical_fact_id)
    }

    /// Save a later scene where both Mara and Bran are present (so a secret is
    /// "in play" — an out-of-circle present character exists) with a chosen
    /// content rating; return its scene id.
    async fn save_leak_scene(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
        text: &str,
        rating: ContentRating,
    ) -> String {
        if chapter > 1 {
            let _ = svc
                .create_chapter(spindle_core::models::CreateChapterInput {
                    project_id: project_id.to_string(),
                    book_id: None,
                    book_number: Some(1),
                    chapter_number: Some(chapter),
                    title: None,
                })
                .await;
        }
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
            chapter_id: None,
            scene_order: 1,
            full_text: text.to_string(),
            summary: "behavioral".into(),
            content_rating: rating,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap()
        .scene_id
    }

    async fn run(
        svc: &SqliteSpindleService,
        project_id: &str,
        deep: bool,
    ) -> Vec<spindle_core::models::ConsistencyIssue> {
        svc.check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.to_string(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["secret_leak".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(deep),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap()
        .issues
        .into_iter()
        .filter(|i| i.check_type == "secret_leak")
        .collect()
    }

    /// Firing: the scene prose carries the behavioral-leak sentinel echoing
    /// Bran's id → one warning suffixed "(model-detected behavioral leak)".
    #[tokio::test]
    async fn behavioral_leak_sentinel_warns() {
        let (_tmp, svc, project_id, _mara, bran, _fact) = fixture().await;
        // Both present; Bran acts oddly. The sentinel embeds his id so the
        // local review stub echoes it as the leaking character.
        let text = format!(
            "Mara and Bran walked the ward. Bran refused to pass the graveyard, \
                 though he had no reason to fear it. MOCK_SECRET_BEHAVIORAL_LEAK[character:{bran}]"
        );
        save_leak_scene(&svc, &project_id, 2, &text, ContentRating::General).await;

        let deep = run(&svc, &project_id, true).await;
        let behavioral: Vec<_> = deep
            .iter()
            .filter(|i| i.message.contains("(model-detected behavioral leak)"))
            .collect();
        assert_eq!(
            behavioral.len(),
            1,
            "exactly one behavioral leak finding expected: {deep:?}"
        );
        let f = behavioral[0];
        assert_eq!(f.severity, "warning");
        assert!(
            f.entity_ids.contains(&bran),
            "the leaking character id is named: {:?}",
            f.entity_ids
        );
    }

    /// Clean: no sentinel → the deep tier adds nothing, and the deterministic
    /// tier stays untouched (Bran does not speak the value here).
    #[tokio::test]
    async fn no_sentinel_no_deep_finding() {
        let (_tmp, svc, project_id, _mara, _bran, _fact) = fixture().await;
        save_leak_scene(
            &svc,
            &project_id,
            2,
            "Mara and Bran shared a quiet supper and spoke of the weather.",
            ContentRating::General,
        )
        .await;

        let deep = run(&svc, &project_id, true).await;
        assert!(
            !deep
                .iter()
                .any(|i| i.message.contains("(model-detected behavioral leak)")),
            "no sentinel → no behavioral finding: {deep:?}"
        );
    }

    /// Shallow-silent: with `deep_check == false` the deep tier never runs,
    /// even when the sentinel is present.
    #[tokio::test]
    async fn shallow_has_no_deep_finding() {
        let (_tmp, svc, project_id, _mara, bran, _fact) = fixture().await;
        let text = format!("Mara and Bran. MOCK_SECRET_BEHAVIORAL_LEAK[character:{bran}]");
        save_leak_scene(&svc, &project_id, 2, &text, ContentRating::General).await;

        let shallow = run(&svc, &project_id, false).await;
        assert!(
            !shallow
                .iter()
                .any(|i| i.message.contains("(model-detected behavioral leak)")),
            "the deep tier must be silent without deep_check: {shallow:?}"
        );
    }

    /// Non-present discard: the sentinel echoes an id that is NOT a present
    /// out-of-circle character → the model's finding is discarded.
    #[tokio::test]
    async fn non_present_character_finding_discarded() {
        let (_tmp, svc, project_id, _mara, _bran, _fact) = fixture().await;
        let text =
            "Mara and Bran walked. MOCK_SECRET_BEHAVIORAL_LEAK[character:character:ghost999]";
        save_leak_scene(&svc, &project_id, 2, text, ContentRating::General).await;

        let deep = run(&svc, &project_id, true).await;
        assert!(
            !deep
                .iter()
                .any(|i| i.message.contains("(model-detected behavioral leak)")),
            "a finding for a non-present character must be discarded: {deep:?}"
        );
    }

    /// Honest-skip: an unroutable review route yields exactly one info finding
    /// that reads as SKIPPED (the deep behavioral tier), not clean.
    #[tokio::test]
    async fn honest_skip_when_review_route_unreachable() {
        use spindle_core::models::ConfigureAgentsInput;
        let (tmp, svc, project_id, _mara, bran, _fact) = fixture().await;
        let config_path = tmp.path().join("unreachable-review.toml");
        std::fs::write(
            &config_path,
            r#"
[health_check]
enabled = false

[[agents]]
id = "review-http"
name = "Review HTTP"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:1/v1"
model = "unreachable"

[[routing]]
route = "review"
agent = "review-http"
"#,
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.display().to_string()),
        })
        .unwrap();

        let text = format!("Mara and Bran. MOCK_SECRET_BEHAVIORAL_LEAK[character:{bran}]");
        save_leak_scene(&svc, &project_id, 2, &text, ContentRating::General).await;

        let deep = run(&svc, &project_id, true).await;
        let skips: Vec<_> = deep
            .iter()
            .filter(|i| i.severity == "info" && i.message.to_lowercase().contains("skipped"))
            .collect();
        assert_eq!(
            skips.len(),
            1,
            "an unreachable review route must yield exactly one skip finding: {deep:?}"
        );
        assert!(
            skips[0].message.to_lowercase().contains("behavioral"),
            "the skip finding must name the behavioral tier: {}",
            skips[0].message
        );
    }

    /// Rating-carriage: the request-construction helper stamps the scene's
    /// content rating (lowercase) so `resolve_route`'s (route, rating)
    /// override chain applies — an explicit-rated scene resolves through the
    /// ("review","explicit") override rather than the base review agent.
    #[test]
    fn deep_request_carries_scene_rating_lowercased() {
        let prompt = super::build_secret_behavioral_leak_prompt(
            "the prose body",
            &[super::SecretInPlay {
                fact_value: "reincarnated".into(),
                known_present: vec!["character:mara".into()],
                unaware_present: vec!["character:bran".into()],
            }],
        );
        let req = super::build_secret_behavioral_leak_request("explicit", prompt);
        assert_eq!(req.route, "review");
        assert_eq!(
            req.rating.as_deref(),
            Some("explicit"),
            "the outgoing request must carry the scene's lowercase rating"
        );
        assert!(req.prompt.contains("the prose body"));
        assert!(
            req.prompt.contains("secret knowledge leak audit"),
            "the distinctive header must be present so the fake router keys on it"
        );

        // Mixed-case DB rating strings normalize to the lowercase override key.
        let mixed = super::build_secret_behavioral_leak_request("Explicit", "x".to_string());
        assert_eq!(mixed.rating.as_deref(), Some("explicit"));
    }

    /// Rating-carriage (runtime seam): an EXPLICIT-rated scene routed with a
    /// configured ("review","explicit") override still fires. The finding can
    /// only appear if the request's rating reached `resolve_route` and the
    /// explicit override served it (both the default and the override are
    /// local `review` routes, so the fake stub responds identically — the
    /// point is that the rating-carrying request routed at all).
    #[tokio::test]
    async fn explicit_scene_routes_through_rating_override() {
        use spindle_core::models::ConfigureAgentsInput;
        let (tmp, svc, project_id, _mara, bran, _fact) = fixture().await;
        let config_path = tmp.path().join("review-explicit.toml");
        std::fs::write(
            &config_path,
            r#"
[health_check]
enabled = false

[[agents]]
id = "review-default"
name = "Review Default"
provider = "local"
endpoint = "local"
model = "review-default"

[[agents]]
id = "review-explicit"
name = "Review Explicit"
provider = "local"
endpoint = "local"
model = "review-explicit"
ratings = ["explicit"]

[[routing]]
route = "review"
agent = "review-default"

[[routing]]
route = "review"
agent = "review-explicit"
rating = "explicit"
"#,
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.display().to_string()),
        })
        .unwrap();

        let text = format!("Mara and Bran. MOCK_SECRET_BEHAVIORAL_LEAK[character:{bran}]");
        save_leak_scene(&svc, &project_id, 2, &text, ContentRating::Explicit).await;

        let deep = run(&svc, &project_id, true).await;
        let behavioral: Vec<_> = deep
            .iter()
            .filter(|i| i.message.contains("(model-detected behavioral leak)"))
            .collect();
        assert_eq!(
            behavioral.len(),
            1,
            "the explicit-rated audit must route through the override and fire: {deep:?}"
        );
        assert!(behavioral[0].entity_ids.contains(&bran));
    }

    /// Rating-not-covered honest-skip (evolution §4): an EXPLICIT scene whose
    /// only `review` agent declares no explicit coverage must NOT reach that
    /// agent. The chokepoint returns RatingNotCovered, and the audit emits one
    /// skip finding naming the route and rating — never any prose.
    #[tokio::test]
    async fn honest_skip_when_review_route_not_cleared_for_rating() {
        use spindle_core::models::ConfigureAgentsInput;
        let (tmp, svc, project_id, _mara, bran, _fact) = fixture().await;
        let config_path = tmp.path().join("uncovered-review.toml");
        std::fs::write(
            &config_path,
            r#"
[health_check]
enabled = false

[[agents]]
id = "review-safe"
name = "Review Safe"
provider = "local"
endpoint = "local"
model = "review-safe"
ratings = ["general", "teen"]

[[routing]]
route = "review"
agent = "review-safe"
"#,
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.display().to_string()),
        })
        .unwrap();

        let text = format!("Mara and Bran. MOCK_SECRET_BEHAVIORAL_LEAK[character:{bran}]");
        save_leak_scene(&svc, &project_id, 2, &text, ContentRating::Explicit).await;

        let deep = run(&svc, &project_id, true).await;
        // No behavioral finding — the explicit prose never reached the agent.
        assert!(
            deep.iter()
                .all(|i| !i.message.contains("(model-detected behavioral leak)")),
            "explicit prose must not have been audited by an uncleared agent: {deep:?}"
        );
        let skips: Vec<_> = deep
            .iter()
            .filter(|i| i.severity == "info" && i.message.to_lowercase().contains("skipped"))
            .collect();
        assert_eq!(
            skips.len(),
            1,
            "exactly one rating-not-covered skip: {deep:?}"
        );
        assert!(skips[0].message.contains("`review`"));
        assert!(skips[0].message.contains("`explicit`"));
        assert!(skips[0].message.contains("not cleared for rating"));
    }

    #[test]
    fn deep_secret_leak_parse_extracts_findings() {
        let out = r#"{"findings":[{"character_id":"character:abc","severity":"warning","description":"avoids the graveyard","evidence":"Bran refused to pass"}]}"#;
        let findings = super::parse_deep_secret_leak_output(out).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].character_id, "character:abc");
    }

    #[test]
    fn deep_secret_leak_parse_handles_code_fence() {
        let out = "audit:\n```json\n{\"findings\":[{\"character_id\":\"c\"}]}\n```";
        let findings = super::parse_deep_secret_leak_output(out).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn deep_secret_leak_parse_empty() {
        let findings = super::parse_deep_secret_leak_output(r#"{"findings":[]}"#).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn deep_secret_leak_parse_rejects_non_json() {
        assert!(
            super::parse_deep_secret_leak_output(
                "Literary critic and craft technician both reviewed: ..."
            )
            .is_err()
        );
    }
}

#[test]
fn max_scene_rating_picks_strictest_across_the_batch() {
    use crate::sqlite::records::Scene;
    fn scene(rating: &str) -> Scene {
        Scene {
            id: "scene:x".to_string(),
            project_id: "project:x".to_string(),
            branch_id: "bible_branch:main".to_string(),
            book_id: "book:x".to_string(),
            chapter_id: "chapter:x".to_string(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: "prose".to_string(),
            summary: "s".to_string(),
            content_rating: rating.to_string(),
            tone: None,
            draft_origin: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            location_id: None,
        }
    }
    // Empty batch → None.
    assert_eq!(super::max_scene_rating(&[]), None);
    // A single explicit scene makes the whole batch explicit.
    assert_eq!(
        super::max_scene_rating(&[scene("general"), scene("explicit"), scene("teen")]).as_deref(),
        Some("explicit")
    );
    // Strictest of general/teen/mature is mature.
    assert_eq!(
        super::max_scene_rating(&[scene("teen"), scene("mature"), scene("general")]).as_deref(),
        Some("mature")
    );
    // Mixed case normalizes and still ranks.
    assert_eq!(
        super::max_scene_rating(&[scene("General"), scene("Explicit")]).as_deref(),
        Some("explicit")
    );
    // Unknown token ranks as general (0) and never outranks a real rating.
    assert_eq!(
        super::max_scene_rating(&[scene("weird"), scene("teen")]).as_deref(),
        Some("teen")
    );
}

#[test]
fn secret_leak_skip_issue_names_route_and_rating_on_rating_not_covered() {
    let err = anyhow::Error::new(crate::ai::RouteClearanceError::RatingNotCovered {
        route: "review".to_string(),
        rating: "mature".to_string(),
        agent_id: "reviewer".to_string(),
    });
    let issue = super::secret_leak_skip_issue(&err);
    assert_eq!(issue.check_type, "secret_leak");
    assert!(issue.message.contains("SKIPPED"));
    assert!(issue.message.contains("`review`"));
    assert!(issue.message.contains("`mature`"));
}

#[test]
fn secret_leak_skip_issue_generic_message_for_non_clearance_error() {
    let err = anyhow::anyhow!("timeout");
    let issue = super::secret_leak_skip_issue(&err);
    assert!(issue.message.contains("SKIPPED"));
    assert!(issue.message.contains("no usable review route"));
}

#[test]
fn world_rule_deep_skip_issue_names_route_and_rating_on_rating_not_covered() {
    let err = anyhow::Error::new(crate::ai::RouteClearanceError::RatingNotCovered {
        route: "review".to_string(),
        rating: "mature".to_string(),
        agent_id: "reviewer".to_string(),
    });
    let issue = super::world_rule_deep_skip_issue(&err);
    assert_eq!(issue.check_type, "world_rule_compliance");
    assert_eq!(issue.severity, "info");
    assert!(issue.message.contains("SKIPPED"));
    assert!(issue.message.contains("`review`"));
    assert!(issue.message.contains("`mature`"));
    assert!(issue.message.contains("not cleared for rating"));
    assert!(issue.message.to_lowercase().contains("world rule"));
}

#[test]
fn world_rule_deep_skip_issue_generic_message_for_non_clearance_error() {
    let err = anyhow::anyhow!("timeout");
    let issue = super::world_rule_deep_skip_issue(&err);
    // The world-rule transport branch reads as a DEGRADATION (the heuristic
    // tier still runs), not a bare skip — coverage was not lost, just the
    // model-backed tier. See `..._states_heuristic_degradation` for the full
    // contract.
    assert!(issue.message.contains("DEGRADED"));
    assert!(issue.message.contains("no usable review route"));
}

/// The generic (non-clearance / transport) branch of the world-rule helper
/// is emitted ONCE per run and must state that the deep tier DEGRADED to the
/// deterministic/heuristic tier (which keeps running), not that coverage was
/// lost entirely.
#[test]
fn world_rule_deep_skip_issue_generic_message_states_heuristic_degradation() {
    let err = anyhow::anyhow!("timeout");
    let issue = super::world_rule_deep_skip_issue(&err);
    assert_eq!(issue.check_type, "world_rule_compliance");
    assert_eq!(issue.severity, "info");
    assert!(issue.message.contains("no usable review route"));
    assert!(
        issue.message.to_lowercase().contains("deep tier"),
        "transport-degradation must frame the whole deep tier: {}",
        issue.message
    );
    assert!(
        issue.message.to_lowercase().contains("heuristic"),
        "the degradation finding must name the heuristic fallback that stays: {}",
        issue.message
    );
    assert!(
        !issue.message.contains("This scene"),
        "transport-degradation is once-per-run, not per-scene: {}",
        issue.message
    );
}

// ── P3.3: scene_purpose_fulfillment deep-check ────────────────────────
// Model-backed, deep-only advisory check (evolution design §3.6): did the
// drafted scene accomplish the plan's `purpose`? For each scoped persisted
// scene that has BOTH non-empty prose AND a chapter-plan scene entry with a
// non-empty `purpose`, one `review` call carries the scene's own rating
// (per-scene pattern, mirroring the behavioral secret-leak audit). A
// `fulfilled:false` verdict yields one `info` finding; silence is the pass.
// All tests hermetic (`fresh_service_local` + TempDir configs).
mod scene_purpose_fulfillment {
    use super::*;
    use crate::ai::DispatchRecord;
    use spindle_core::models::{
        CheckConsistencyInput, ConfigureAgentsInput, ConsistencyScopeInput, ContentRating,
        PlanChapterInput, PlanChapterSceneInput, SaveSceneDraftInput,
    };

    async fn seed_project(svc: &SqliteSpindleService, name: &str) -> String {
        svc.create_project(CreateProjectInput {
            name: name.into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap()
        .project_id
    }

    /// Plan chapter `chapter` with a single planned scene at `scene_order`
    /// carrying `purpose` (pass an empty string for the no-purpose case).
    async fn plan_scene_purpose(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
        scene_order: i32,
        purpose: &str,
    ) {
        svc.plan_chapter(PlanChapterInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
            pov_character_id: None,
            synopsis: "s".into(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![PlanChapterSceneInput {
                scene_order,
                summary: "planned".into(),
                purpose: purpose.into(),
                ..Default::default()
            }],
        })
        .await
        .unwrap();
    }

    async fn save_scene(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
        scene_order: i32,
        full_text: &str,
        rating: ContentRating,
    ) {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
            chapter_id: None,
            scene_order,
            full_text: full_text.into(),
            summary: "s".into(),
            content_rating: rating,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    fn purpose_input(project_id: &str, deep: bool) -> CheckConsistencyInput {
        CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.to_string(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["scene_purpose_fulfillment".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(deep),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        }
    }

    /// A `review` agent declaring exactly `ratings`, written to a TempDir and
    /// installed. A rated agent (not a built-in local route) makes the
    /// clearance gate actually engage.
    fn install_review_agent(tmp: &TempDir, svc: &SqliteSpindleService, ratings: &[&str]) {
        let ratings_toml = ratings
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let config_path = tmp.path().join("review-agent.toml");
        std::fs::write(
            &config_path,
            format!(
                r#"
[health_check]
enabled = false

[[agents]]
id = "review-rated"
name = "Review Rated"
provider = "local"
endpoint = "local"
model = "review-rated"
ratings = [{ratings_toml}]

[[routing]]
route = "review"
agent = "review-rated"
"#
            ),
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.display().to_string()),
        })
        .unwrap();
    }

    fn purpose_findings(
        out: &spindle_core::models::CheckConsistencyOutput,
    ) -> Vec<&spindle_core::models::ConsistencyIssue> {
        out.issues
            .iter()
            .filter(|i| i.check_type == "scene_purpose_fulfillment")
            .collect()
    }

    // ── (a) unfulfilled sentinel → one info finding ────────────────────
    /// A planned-purpose scene whose prose carries `MOCK_PURPOSE_UNFULFILLED`
    /// yields exactly one info finding naming the scene ref, the planned
    /// purpose, and the model's assessment fragment.
    #[tokio::test]
    async fn unfulfilled_scene_yields_one_info_finding() {
        let (_tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-unfulfilled").await;
        plan_scene_purpose(&svc, &project_id, 1, 1, "rupture the alliance").await;
        save_scene(
            &svc,
            &project_id,
            1,
            1,
            "They shared a warm supper and parted friends. MOCK_PURPOSE_UNFULFILLED.",
            ContentRating::General,
        )
        .await;

        let deep = svc
            .check_consistency(purpose_input(&project_id, true))
            .await
            .unwrap();
        let findings = purpose_findings(&deep);
        assert_eq!(
            findings.len(),
            1,
            "exactly one unfulfilled finding expected: {:?}",
            deep.issues
        );
        let f = findings[0];
        assert_eq!(f.severity, "info");
        assert!(
            f.message.contains("rupture the alliance"),
            "message names the planned purpose: {}",
            f.message
        );
        assert!(
            f.message
                .contains("comfort where the plan demanded rupture"),
            "message carries the model's assessment: {}",
            f.message
        );
    }

    // ── (b) fulfilled (no sentinel) → zero findings ────────────────────
    #[tokio::test]
    async fn fulfilled_scene_yields_no_finding() {
        let (_tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-fulfilled").await;
        plan_scene_purpose(&svc, &project_id, 1, 1, "rupture the alliance").await;
        save_scene(
            &svc,
            &project_id,
            1,
            1,
            "The knives came out and the pact shattered on the table.",
            ContentRating::General,
        )
        .await;

        let deep = svc
            .check_consistency(purpose_input(&project_id, true))
            .await
            .unwrap();
        assert!(
            purpose_findings(&deep).is_empty(),
            "a fulfilled scene must yield no finding (silence is the pass): {:?}",
            deep.issues
        );
    }

    // ── (c) no plan entry / empty purpose → silent skip, no dispatch ───
    /// A scene with prose but NO chapter-plan purpose entry is silently
    /// skipped: zero findings and zero model dispatches (nothing to audit).
    #[tokio::test]
    async fn scene_without_planned_purpose_is_silently_skipped() {
        let (_tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-noplan").await;
        // No plan at all for chapter 1.
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();
        save_scene(
            &svc,
            &project_id,
            1,
            1,
            "A scene with prose but no planned purpose. MOCK_PURPOSE_UNFULFILLED.",
            ContentRating::General,
        )
        .await;

        let deep = svc
            .check_consistency(purpose_input(&project_id, true))
            .await
            .unwrap();
        assert!(
            purpose_findings(&deep).is_empty(),
            "no planned purpose → no finding: {:?}",
            deep.issues
        );
        let dispatched = log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, DispatchRecord::Dispatch { .. }))
            .count();
        assert_eq!(
            dispatched,
            0,
            "no planned purpose → no model dispatch: {:?}",
            log.lock().unwrap()
        );
    }

    /// An EMPTY planned purpose is treated identically to no plan: silently
    /// skipped even with the unfulfilled sentinel present.
    #[tokio::test]
    async fn scene_with_empty_planned_purpose_is_silently_skipped() {
        let (_tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-empty").await;
        plan_scene_purpose(&svc, &project_id, 1, 1, "   ").await;
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();
        save_scene(
            &svc,
            &project_id,
            1,
            1,
            "Prose here. MOCK_PURPOSE_UNFULFILLED.",
            ContentRating::General,
        )
        .await;

        let deep = svc
            .check_consistency(purpose_input(&project_id, true))
            .await
            .unwrap();
        assert!(
            purpose_findings(&deep).is_empty(),
            "empty planned purpose → no finding: {:?}",
            deep.issues
        );
        let dispatched = log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, DispatchRecord::Dispatch { .. }))
            .count();
        assert_eq!(dispatched, 0, "empty purpose → no model dispatch");
    }

    // ── (d) shallow run → zero (deep-only) ─────────────────────────────
    #[tokio::test]
    async fn shallow_run_yields_no_finding() {
        let (_tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-shallow").await;
        plan_scene_purpose(&svc, &project_id, 1, 1, "rupture the alliance").await;
        save_scene(
            &svc,
            &project_id,
            1,
            1,
            "Warm supper. MOCK_PURPOSE_UNFULFILLED.",
            ContentRating::General,
        )
        .await;

        let shallow = svc
            .check_consistency(purpose_input(&project_id, false))
            .await
            .unwrap();
        assert!(
            purpose_findings(&shallow).is_empty(),
            "the deep-only check must be silent without deep_check: {:?}",
            shallow.issues
        );
    }

    // ── (e) explicit + general-only agent → skip finding, no prose dispatch
    /// An EXPLICIT scene routed to a `review` agent that declares only
    /// `general` must NOT dispatch the prose. The check emits one skip
    /// finding naming route + rating, and the recorder shows zero prose
    /// dispatches past the gate.
    #[tokio::test]
    async fn uncleared_explicit_skips_and_never_dispatches_prose() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-uncleared").await;
        plan_scene_purpose(&svc, &project_id, 1, 1, "rupture the alliance").await;
        install_review_agent(&tmp, &svc, &["general"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();
        save_scene(
            &svc,
            &project_id,
            1,
            1,
            "UNIQUE_PURPOSE_PROSE_SENTINEL warm supper. MOCK_PURPOSE_UNFULFILLED.",
            ContentRating::Explicit,
        )
        .await;

        let deep = svc
            .check_consistency(purpose_input(&project_id, true))
            .await
            .unwrap();
        let records = log.lock().unwrap().clone();
        for record in &records {
            if let DispatchRecord::Dispatch { prompt, .. } = record {
                assert!(
                    !prompt.contains("UNIQUE_PURPOSE_PROSE_SENTINEL"),
                    "explicit prose must never dispatch to an uncleared agent: {records:?}"
                );
            }
        }
        let skips: Vec<_> = purpose_findings(&deep)
            .into_iter()
            .filter(|i| i.severity == "info" && i.message.to_lowercase().contains("skipped"))
            .collect();
        assert_eq!(
            skips.len(),
            1,
            "an uncleared review route must yield exactly one skip finding: {:?}",
            deep.issues
        );
        assert!(
            skips[0].message.contains("`review`"),
            "{}",
            skips[0].message
        );
        assert!(
            skips[0].message.contains("`explicit`"),
            "{}",
            skips[0].message
        );
        assert!(
            skips[0].message.contains("not cleared for rating"),
            "{}",
            skips[0].message
        );
    }

    // ── (f) cleared explicit → dispatch carries rating Some("explicit") ─
    #[tokio::test]
    async fn cleared_explicit_dispatches_with_explicit_rating() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-cleared").await;
        plan_scene_purpose(&svc, &project_id, 1, 1, "rupture the alliance").await;
        install_review_agent(&tmp, &svc, &["general", "explicit"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();
        save_scene(
            &svc,
            &project_id,
            1,
            1,
            "UNIQUE_PURPOSE_PROSE_SENTINEL warm supper. MOCK_PURPOSE_UNFULFILLED.",
            ContentRating::Explicit,
        )
        .await;

        let deep = svc
            .check_consistency(purpose_input(&project_id, true))
            .await
            .unwrap();
        // No skip finding fired — the agent covers explicit.
        assert!(
            !purpose_findings(&deep)
                .iter()
                .any(|i| i.message.to_lowercase().contains("skipped")),
            "a cleared explicit route must not skip: {:?}",
            deep.issues
        );
        let records = log.lock().unwrap().clone();
        let explicit_dispatch = records.iter().any(|r| {
            matches!(
                r,
                DispatchRecord::Dispatch { rating, prompt, .. }
                    if rating.as_deref() == Some("explicit")
                        && prompt.contains("UNIQUE_PURPOSE_PROSE_SENTINEL")
            )
        });
        assert!(
            explicit_dispatch,
            "a cleared purpose audit must dispatch with rating=explicit: {records:?}"
        );
        assert!(
            !records
                .iter()
                .any(|r| matches!(r, DispatchRecord::Rejection { .. })),
            "no clearance rejection when the agent covers the rating: {records:?}"
        );
    }

    // ── (g) 12 eligible scenes → 10 scanned + summary reports 2 ─────────
    #[tokio::test]
    async fn caps_at_ten_scenes_and_reports_unscanned() {
        let (_tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "purpose-cap").await;
        // 12 eligible scenes across chapters 1..=12, each with a planned
        // purpose and the unfulfilled sentinel, so every one would flag.
        for ch in 1..=12 {
            plan_scene_purpose(&svc, &project_id, ch, 1, &format!("purpose {ch}")).await;
            save_scene(
                &svc,
                &project_id,
                ch,
                1,
                "Off-plan prose. MOCK_PURPOSE_UNFULFILLED.",
                ContentRating::General,
            )
            .await;
        }

        let deep = svc
            .check_consistency(purpose_input(&project_id, true))
            .await
            .unwrap();
        let findings = purpose_findings(&deep);
        // 10 unfulfilled findings + 1 summary = 11 total.
        let unfulfilled: Vec<_> = findings
            .iter()
            .filter(|i| !i.message.contains("not scanned"))
            .collect();
        assert_eq!(
            unfulfilled.len(),
            10,
            "at most 10 scenes scanned per run: {:?}",
            deep.issues
        );
        let summary = findings
            .iter()
            .find(|i| i.severity == "info" && i.message.contains("not scanned"))
            .expect("a summary finding must report the unscanned overflow");
        assert!(
            summary.message.contains('2'),
            "12 eligible - 10 scanned = 2 unscanned: {}",
            summary.message
        );
        let coverage = &deep.audit_coverage[0];
        assert_eq!((coverage.eligible, coverage.next_offset), (12, Some(10)));
        assert!(
            coverage.evaluated_ids.is_empty(),
            "local stubs are not model reviews"
        );
        assert_eq!(coverage.heuristic_ids.len(), 10);
        assert_eq!(coverage.not_evaluated_ids.len(), 12);
        let mut next = purpose_input(&project_id, true);
        next.deep_scan_offset = coverage.next_offset;
        let page = svc.check_consistency(next).await.unwrap();
        let next = &page.audit_coverage[0];
        assert_eq!(next.heuristic_ids.len(), 2);
        assert!(
            next.heuristic_ids
                .iter()
                .all(|id| !coverage.heuristic_ids.contains(id))
        );
        assert_eq!(next.next_offset, None);
    }

    // ── (h) parser units ───────────────────────────────────────────────
    #[test]
    fn purpose_parse_extracts_verdict() {
        let out = r#"{"fulfilled":false,"assessment":"drifted","evidence":"quote"}"#;
        let verdict = super::parse_scene_purpose_output(out).unwrap();
        assert_eq!(verdict.fulfilled, Some(false));
        assert_eq!(verdict.assessment.as_deref(), Some("drifted"));
    }

    #[test]
    fn purpose_parse_handles_code_fence_and_prose() {
        let out = "Audit:\n```json\n{\"fulfilled\":true,\"assessment\":\"landed\"}\n```";
        let verdict = super::parse_scene_purpose_output(out).unwrap();
        assert_eq!(verdict.fulfilled, Some(true));
    }

    #[test]
    fn purpose_parse_rejects_non_json() {
        assert!(
            super::parse_scene_purpose_output(
                "Literary critic and craft technician both reviewed: ..."
            )
            .is_err()
        );
    }

    #[test]
    fn purpose_parse_missing_fulfilled_is_discarded() {
        // A payload with no `fulfilled` field parses, but the caller must
        // discard it (never guess a verdict): `fulfilled` is None.
        let out = r#"{"assessment":"ambiguous","evidence":"q"}"#;
        let verdict = super::parse_scene_purpose_output(out).unwrap();
        assert_eq!(verdict.fulfilled, None);
    }

    #[test]
    fn purpose_skip_issue_names_route_and_rating_on_rating_not_covered() {
        let err = anyhow::Error::new(crate::ai::RouteClearanceError::RatingNotCovered {
            route: "review".to_string(),
            rating: "explicit".to_string(),
            agent_id: "reviewer".to_string(),
        });
        let issue = super::scene_purpose_skip_issue(&err);
        assert_eq!(issue.check_type, "scene_purpose_fulfillment");
        assert_eq!(issue.severity, "info");
        assert!(issue.message.contains("SKIPPED"));
        assert!(issue.message.contains("`review`"));
        assert!(issue.message.contains("`explicit`"));
        assert!(issue.message.contains("not cleared for rating"));
    }

    #[test]
    fn purpose_skip_issue_generic_message_for_non_clearance_error() {
        let err = anyhow::anyhow!("connection refused");
        let issue = super::scene_purpose_skip_issue(&err);
        assert!(issue.message.contains("SKIPPED"));
        assert!(issue.message.contains("no usable review route"));
    }
}

// ── Canon-delta ratification (list_canon_deltas + decide_canon_deltas) ────
//
// These exercise the apply-dispatcher (ADR 0001 D3): pre-flight-all,
// per-class apply to the SAME existing write tool, decision recording, and
// the honesty guarantees (finality, atomicity, mid-apply-failure).
#[test]
fn world_rule_deep_check_prompt_carries_scene_content_rating() {
    // Defect item 7: rating-scoped rule mandates (e.g. "explicit scenes
    // must …") were judged without knowing the scene's declared rating, so
    // a Mature scene was audited as if it were Explicit. The prompt must
    // name the scene's content rating and scope rating-conditional
    // mandates to it.
    let scene = crate::sqlite::records::Scene {
        id: "scene:s1".to_string(),
        project_id: "project:p1".to_string(),
        branch_id: "bible_branch:main".to_string(),
        book_id: "book:b1".to_string(),
        chapter_id: "chapter:c1".to_string(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        full_text: "They closed the door.".to_string(),
        summary: "An intimate scene.".to_string(),
        content_rating: "mature".to_string(),
        tone: None,
        draft_origin: None,
        created_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        updated_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        location_id: None,
    };
    let rule = crate::sqlite::records::WorldRule {
        id: "world_rule:limits".to_string(),
        project_id: "project:p1".to_string(),
        branch_id: "bible_branch:main".to_string(),
        rule_name: "Content Hard Limits".to_string(),
        rule_type: "tone".to_string(),
        description: "Explicit scenes must be rendered fully on-page.".to_string(),
        established_in: None,
        relevance_tags: None,
        scan_pattern: None,
        notes: None,
        created_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        updated_at: None,
    };
    let prompt = build_world_rule_deep_check_prompt(&scene, &[&rule]);
    assert!(
        prompt.contains("Scene content rating: mature"),
        "prompt must carry the scene's declared rating:\n{prompt}"
    );
    assert!(
        prompt.to_lowercase().contains("rating"),
        "prompt must instruct rating-scoped judgement:\n{prompt}"
    );
}

#[tokio::test]
async fn recommitting_scene_continuity_package_reverses_omitted_pairs_and_respects_mined_precedence()
 {
    use spindle_core::models::{
        CharacterVoiceProfileData, CommitContinuityGate, CommitSceneChangesInput, ContentRating,
        CreateCharacterInput, CreateProjectInput, CreateRelationshipInput, RelationshipUpdateEntry,
        SaveSceneDraftInput, UpdateRelationshipInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "recommit-omitted-pair".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "omitted relationship pairs reversed on recommit".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let mut chars = Vec::new();
    for name in ["Aria", "Bram", "Cora", "Dorian"] {
        let ch = svc
            .create_character(CreateCharacterInput {
                aliases: Vec::new(),
                project_id: project.project_id.clone(),
                name: name.to_string(),
                summary: format!("{name} character."),
                role: "supporting".to_string(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: None,
                    vocabulary: Vec::new(),
                    sentence_structure: Vec::new(),
                    tics: Vec::new(),
                    forbidden_words: Vec::new(),
                    example_lines: Vec::new(),
                    established_in_scene_id: None,
                    updated_at: None,
                },
                emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                    base_emotions: std::collections::BTreeMap::new(),
                    suppressed: Vec::new(),
                    triggers: Vec::new(),
                    defense_mechanisms: Vec::new(),
                    flex_range: None,
                },
                initial_state: None,
            })
            .await
            .unwrap();
        chars.push(ch.character_id);
    }
    let (char_a, char_b, char_c, char_d) = (
        chars[0].clone(),
        chars[1].clone(),
        chars[2].clone(),
        chars[3].clone(),
    );

    for other in [&char_b, &char_c, &char_d] {
        svc.create_relationship(CreateRelationshipInput {
            character_a_id: char_a.clone(),
            character_b_id: other.clone(),
            relationship_type: "companions".to_string(),
            initial_trust: 50,
            initial_tension: 20,
            dynamics: Vec::new(),
        })
        .await
        .unwrap();
    }

    let scene1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Aria spoke with Bram, Cora, and Dorian around the hearth.".to_string(),
            summary: "Council at the hearth.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Commit #1: continuity package updates A/B (+10, -3) and A/D (+8, -1).
    let commit1 = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene1.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: vec![
                RelationshipUpdateEntry {
                    character_a_id: char_a.clone(),
                    character_b_id: char_b.clone(),
                    trust_delta: 10,
                    tension_delta: -3,
                    reason: "hearth alliance".to_string(),
                },
                RelationshipUpdateEntry {
                    character_a_id: char_a.clone(),
                    character_b_id: char_d.clone(),
                    trust_delta: 8,
                    tension_delta: -1,
                    reason: "initial accord".to_string(),
                },
            ],
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(CommitContinuityGate::Off),
        })
        .await
        .unwrap();
    assert_eq!(commit1.relationship_updates[0].trust, Some(60));
    assert_eq!(commit1.relationship_updates[0].tension, Some(17));
    assert_eq!(commit1.relationship_updates[1].trust, Some(58));
    assert_eq!(commit1.relationship_updates[1].tension, Some(19));

    // Apply a mined relationship_shift on A/D (+12, -4) for scene1.
    let mined_ad = svc
        .update_relationship_with_source(
            UpdateRelationshipInput {
                character_a_id: char_a.clone(),
                character_b_id: char_d.clone(),
                trust_delta: 12,
                tension_delta: -4,
                reason: "mined shift on A/D".to_string(),
                scene_id: scene1.scene_id.clone(),
            },
            "mined_relationship_shift",
        )
        .await
        .unwrap();
    assert_eq!(mined_ad.trust, 62);
    assert_eq!(mined_ad.tension, 16);

    // Commit #2: revised continuity package only includes A/C (+5, -2),
    // omitting A/B (which was from continuity_package) and A/D (which has a mined shift).
    let commit2 = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene1.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: vec![RelationshipUpdateEntry {
                character_a_id: char_a.clone(),
                character_b_id: char_c.clone(),
                trust_delta: 5,
                tension_delta: -2,
                reason: "revised scene focuses on Cora".to_string(),
            }],
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(CommitContinuityGate::Off),
        })
        .await
        .unwrap();
    assert_eq!(commit2.relationship_updates[0].trust, Some(55));
    assert_eq!(commit2.relationship_updates[0].tension, Some(18));

    let active_branch = svc
        .repository
        .active_branch_id_public(&project.project_id)
        .await
        .unwrap();
    let rel_ab = svc
        .repository
        .get_relationship(&active_branch, &char_a, &char_b)
        .await
        .unwrap();
    assert_eq!(
        rel_ab.trust, 50,
        "omitted continuity_package pair A/B must have its prior +10 reversed"
    );
    assert_eq!(rel_ab.tension, 20);

    let rel_ad = svc
        .repository
        .get_relationship(&active_branch, &char_a, &char_d)
        .await
        .unwrap();
    assert_eq!(
        rel_ad.trust, 62,
        "mined_relationship_shift on A/D must not be reversed when omitted from continuity_package"
    );
    assert_eq!(rel_ad.tension, 16);

    // Commit #3: empty relationship_updates reverses A/C back to baseline (50, 20).
    svc.commit_scene_changes(CommitSceneChangesInput {
        project_id: project.project_id.clone(),
        scene_id: scene1.scene_id.clone(),
        character_states: Vec::new(),
        canonical_facts: Vec::new(),
        relationship_updates: Vec::new(),
        accept_world_rule_risks: false,
        accept_continuity_risks: false,
        continuity_gate: Some(CommitContinuityGate::Off),
    })
    .await
    .unwrap();

    let rel_ac = svc
        .repository
        .get_relationship(&active_branch, &char_a, &char_c)
        .await
        .unwrap();
    assert_eq!(rel_ac.trust, 50);
    assert_eq!(rel_ac.tension, 20);
}

#[tokio::test]
async fn is_dead_status_word_boundaries_and_retcon_severity_downgrade() {
    use spindle_core::models::{
        CharacterStatePatch, CharacterVoiceProfileData, CommitCharacterStateInput,
        CommitContinuityGate, CommitSceneChangesInput, ContentRating, CreateCharacterInput,
        CreateProjectInput, SaveSceneDraftInput,
    };

    assert!(!is_dead_status("dead tired"));
    assert!(!is_dead_status("dead wrong"));
    assert!(!is_dead_status("nearly dead"));
    assert!(!is_dead_status("left for dead"));
    assert!(!is_dead_status("not dead"));
    assert!(!is_dead_status("undead"));
    assert!(!is_dead_status("brain-dead"));
    assert!(!is_dead_status("killed the lights"));
    assert!(!is_dead_status("alive and well"));

    assert!(is_dead_status("dead"));
    assert!(is_dead_status("deceased"));
    assert!(is_dead_status("slain in battle"));
    assert!(is_dead_status("killed in battle"));
    assert!(is_dead_status("status:deceased"));
    assert!(is_explicit_typed_deceased_marker("status:deceased"));
    assert!(!is_explicit_typed_deceased_marker("deceased"));

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "dead-check".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "accurate retcon".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let char_tired = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Garrick".to_string(),
            summary: "Veteran soldier.".to_string(),
            role: "supporting".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let char_slain = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Veyra".to_string(),
            summary: "Fallen warrior.".to_string(),
            role: "supporting".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let scene1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Garrick slumped against the wall while Veyra fell in the courtyard."
                .to_string(),
            summary: "Garrick is exhausted; Veyra falls.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    svc.commit_character_state(CommitCharacterStateInput {
        character_id: char_tired.character_id.clone(),
        scene_id: Some(scene1.scene_id.clone()),
        anchor: None,
        changes: CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: Some(vec!["dead tired".to_string()]),
            notes: None,
            source_summary: Some("after battle".to_string()),
        },
    })
    .await
    .unwrap();

    svc.commit_character_state(CommitCharacterStateInput {
        character_id: char_slain.character_id.clone(),
        scene_id: Some(scene1.scene_id.clone()),
        anchor: None,
        changes: CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: Some(vec!["killed in battle".to_string()]),
            notes: None,
            source_summary: Some("fell in courtyard".to_string()),
        },
    })
    .await
    .unwrap();

    let scene2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Garrick remembered the courtyard. Veyra stepped into the hall.".to_string(),
            summary: "Garrick remembers Veyra.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Committing scene2 under BlockErrors should SUCCEED because:
    // 1. Garrick's "dead tired" does NOT trigger DeadCharacterAct at all.
    // 2. Veyra's free-form "killed in battle" triggers a "warning", not an "error".
    let commit_ok = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene2.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(CommitContinuityGate::BlockErrors),
        })
        .await
        .unwrap();

    let dead_issues: Vec<_> = commit_ok
        .blocking_continuity_findings
        .iter()
        .filter(|i| i.check_type == "retcon_reachability")
        .collect();
    assert_eq!(dead_issues.len(), 1);
    assert_eq!(dead_issues[0].severity, "warning");
    assert!(dead_issues[0].message.contains("Veyra"));
    assert!(!dead_issues[0].message.contains("Garrick"));

    // Both broad consistency and the actual verify subset surface the same
    // finding once, attributed to this scene (including a warm cache hit).
    assert_eq!(scene2.retcon_findings.len(), 1);
    for checks in [
        vec!["retcon_reachability".to_string()],
        spindle_core::models::SCENE_VERIFY_CHECKS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    ] {
        let mut scope = spindle_core::models::ConsistencyScopeInput::chapter_range(1, 1, 1, 1);
        scope.scene_order = Some(2);
        let report = svc
            .check_consistency(spindle_core::models::CheckConsistencyInput {
                project_id: project.project_id.clone(),
                scope,
                checks,
                deep_scan_offset: None,
                severity_filter: Vec::new(),
                deep_check: Some(false),
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();
        let hits: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.check_type == "retcon_reachability")
            .collect();
        assert_eq!(hits.len(), 1, "{:?}", report.issues);
        assert_eq!(hits[0].message, dead_issues[0].message);
        assert_eq!(hits[0].severity, dead_issues[0].severity);
        assert!(hits[0].entity_ids.contains(&scene2.scene_id));
    }

    // Now mark Veyra with an explicit typed deceased marker ("status:deceased") in scene 2
    // and verify that scene 3 triggers an "error" severity finding that blocks under BlockErrors.
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: char_slain.character_id.clone(),
        scene_id: Some(scene2.scene_id.clone()),
        anchor: None,
        changes: CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: Some(vec!["status:deceased".to_string()]),
            notes: None,
            source_summary: Some("explicit deceased marker".to_string()),
        },
    })
    .await
    .unwrap();

    let scene3 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 3,
            full_text: "Veyra stepped into the hall and drew her sword.".to_string(),
            summary: "Veyra speaks.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let commit_blocked = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene3.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(CommitContinuityGate::BlockErrors),
        })
        .await;
    assert!(
        commit_blocked.is_err(),
        "explicit typed marker status:deceased must produce error and block under BlockErrors"
    );

    for (order, text) in [
        (4, "Garrick remembered Veyra and whispered a prayer."),
        (5, "In a flashback, Veyra stepped into the hall."),
        (6, "On the recording, Veyra said goodbye."),
    ] {
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: order,
                full_text: text.into(),
                summary: "control".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(saved.retcon_findings.is_empty(), "{text}");
        let scene = svc.repository().get_scene(&saved.scene_id).await.unwrap();
        assert!(
            svc.scan_retcon_findings(&project.project_id, &scene)
                .await
                .unwrap()
                .is_empty()
        );
    }
    // Review regressions: objects named recording/ghost are ordinary action;
    // an explicit conditional must never become a blocking death error.
    for (text, blocked) in [
        ("Veyra opened the recording room door.", true),
        ("Veyra stepped past the ghost town.", true),
        ("If Veyra opened the door, the alarm would sound.", false),
    ] {
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 4,
                full_text: text.into(),
                summary: "modality control".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(!saved.retcon_findings.is_empty(), blocked, "{text}");
        let committed = svc
            .commit_scene_changes(CommitSceneChangesInput {
                project_id: project.project_id.clone(),
                scene_id: saved.scene_id,
                character_states: Vec::new(),
                canonical_facts: Vec::new(),
                relationship_updates: Vec::new(),
                accept_world_rule_risks: false,
                accept_continuity_risks: false,
                continuity_gate: None,
            })
            .await;
        assert_eq!(committed.is_err(), blocked, "{text}: {committed:?}");
    }
    let scene = svc.repository().get_scene(&scene3.scene_id).await.unwrap();
    svc.repository()
        .upsert_scene_clock(
            &scene.id,
            &project.project_id,
            &scene.branch_id,
            &spindle_core::models::StoryClock {
                day_index: None,
                time_of_day: None,
                duration_days: None,
                precision: None,
            },
            Some("flashback"),
            None,
        )
        .await
        .unwrap();
    assert!(
        svc.scan_retcon_findings(&project.project_id, &scene)
            .await
            .unwrap()
            .is_empty()
    );

    // A resurrection in a later scene clears the prior life state without
    // retroactively hiding the earlier contradiction.
    let restored = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 7,
            full_text: "A resurrection restored Veyra to life.".into(),
            summary: "resurrection".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: char_slain.character_id.clone(),
        scene_id: Some(restored.scene_id),
        anchor: None,
        changes: CharacterStatePatch {
            status: Some(vec!["alive".into()]),
            emotional_state: Default::default(),
            goals: None,
            notes: None,
            source_summary: None,
        },
    })
    .await
    .unwrap();
    let living = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 8,
            full_text: "Veyra stepped into the hall.".into(),
            summary: "alive".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(living.retcon_findings.is_empty());
    let main_branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let child = svc
        .create_branch(spindle_core::models::CreateBranchInput {
            project_id: project.project_id.clone(),
            parent_branch_id: None,
            name: "dead-alternative".into(),
            branch_type: "alternative".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(spindle_core::models::SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: char_slain.character_id.clone(),
        scene_id: Some(living.scene_id.clone()),
        anchor: None,
        changes: CharacterStatePatch {
            status: Some(vec!["status:deceased".into()]),
            emotional_state: Default::default(),
            goals: None,
            notes: None,
            source_summary: None,
        },
    })
    .await
    .unwrap();
    svc.switch_branch(spindle_core::models::SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: main_branch.id.clone(),
    })
    .await
    .unwrap();
    let subsequent = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 9,
            full_text: "Veyra opened the door.".into(),
            summary: "action".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // The same inherited prose sees child-only death, while main stays alive.
    for (branch, expected) in [(child.branch_id, 1), (main_branch.id, 0)] {
        svc.switch_branch(spindle_core::models::SwitchBranchInput {
            project_id: project.project_id.clone(),
            branch_id: branch,
        })
        .await
        .unwrap();
        let mut scope = spindle_core::models::ConsistencyScopeInput::chapter_range(1, 1, 1, 1);
        scope.scene_order = Some(9);
        let report = svc
            .check_consistency(spindle_core::models::CheckConsistencyInput {
                project_id: project.project_id.clone(),
                scope,
                checks: vec!["retcon_reachability".into()],
                deep_scan_offset: None,
                severity_filter: Vec::new(),
                deep_check: Some(false),
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();
        let hits: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.check_type == "retcon_reachability")
            .collect();
        assert_eq!(hits.len(), expected, "{:?}", report.issues);
        if let Some(hit) = hits.first() {
            assert!(hit.entity_ids.contains(&subsequent.scene_id));
        }
    }
    let earlier = svc.repository().get_scene(&scene3.scene_id).await.unwrap();
    svc.repository()
        .upsert_scene_clock(
            &earlier.id,
            &project.project_id,
            &earlier.branch_id,
            &spindle_core::models::StoryClock::default(),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        svc.scan_retcon_findings(&project.project_id, &earlier)
            .await
            .unwrap()
            .len(),
        1
    );
}

// ── PR F: shared claim-vs-canon deterministic lanes ─────────────────────

mod claim_validation_lanes {
    use super::*;
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateWorldRuleInput,
        RecordKnowledgeInput, RegisterCanonicalFactInput, StoryPlacement,
    };

    async fn save_scene(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
        text: &str,
    ) -> String {
        if chapter > 1 {
            let _ = svc
                .create_chapter(spindle_core::models::CreateChapterInput {
                    project_id: project_id.to_string(),
                    book_id: None,
                    book_number: Some(1),
                    chapter_number: Some(chapter),
                    title: None,
                })
                .await;
        }
        svc.save_scene_draft(spindle_core::models::SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: chapter,
            chapter_id: None,
            scene_order: 1,
            full_text: text.to_string(),
            summary: "claim validation".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap()
        .scene_id
    }

    #[tokio::test]
    async fn dead_character_reads_canonical_life_status_and_first_names() {
        use spindle_core::models::RegisterCanonicalFactInput;
        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "canonical-death".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "the dead stay dead".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let sela = svc
            .create_character(CreateCharacterInput {
                aliases: vec!["The Kestrel".into()],
                project_id: project.project_id.clone(),
                name: "Sela Voss".into(),
                summary: "Cartographer.".into(),
                role: "protagonist".into(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: None,
                    vocabulary: Vec::new(),
                    sentence_structure: Vec::new(),
                    tics: Vec::new(),
                    forbidden_words: Vec::new(),
                    example_lines: Vec::new(),
                    established_in_scene_id: None,
                    updated_at: None,
                },
                emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                    base_emotions: std::collections::BTreeMap::new(),
                    suppressed: Vec::new(),
                    triggers: Vec::new(),
                    defense_mechanisms: Vec::new(),
                    flex_range: None,
                },
                initial_state: None,
            })
            .await
            .unwrap();
        let first = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Sela Voss closed the atlas and went still.".into(),
                summary: "Sela dies.".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(first.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("life_status".into()),
            key: Some("Sela Voss:life_status".into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(sela.character_id.clone()),
            predicate: Some("life_status".into()),
            value_kind: Some("text".into()),
            value_text: Some("dead".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();
        let _ = svc
            .create_chapter(spindle_core::models::CreateChapterInput {
                project_id: project.project_id.clone(),
                book_id: None,
                book_number: Some(1),
                chapter_number: Some(2),
                title: None,
            })
            .await;
        let act = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 2,
                chapter_id: None,
                scene_order: 1,
                full_text: "I nodded at the map as if nothing had happened.".into(),
                summary: "Sela acts after death.".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();
        svc.commit_character_state(CommitCharacterStateInput {
            character_id: sela.character_id.clone(),
            scene_id: Some(act.scene_id.clone()),
            anchor: None,
            changes: CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: None,
                notes: Some(vec!["present in this scene".into()]),
                source_summary: None,
            },
        })
        .await
        .unwrap();
        let issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        let dead = issues
            .iter()
            .find(|issue| {
                issue.check_type == "retcon_reachability" && issue.message.contains("Sela Voss")
            })
            .expect("canonical life_status death must flag a first-name action");
        assert!(dead.message.contains("life_status") || dead.message.contains("dead"));

        // Unrelated quoted first-person dialogue is not POV action evidence.
        let _ = svc
            .create_chapter(spindle_core::models::CreateChapterInput {
                project_id: project.project_id.clone(),
                book_id: None,
                book_number: Some(1),
                chapter_number: Some(3),
                title: None,
            })
            .await;
        let quoted = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 3,
                chapter_id: None,
                scene_order: 1,
                full_text: "My hands shook. Tomas said, \"I met her yesterday.\"".into(),
                summary: "Quoted first person only.".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let quoted_issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        assert!(!quoted_issues.iter().any(|issue| {
            issue.check_type == "retcon_reachability"
                && issue.entity_ids.contains(&quoted.scene_id)
                && issue.message.contains("Sela Voss")
        }));
    }

    #[tokio::test]
    async fn story_start_anchor_requires_exactly_one_placement() {
        use spindle_core::models::{
            CharacterStateAnchor, CharacterStatePatch, CommitCharacterStateInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "anchor-validation".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        let changes = CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: Some(vec!["dead".into()]),
            notes: None,
            source_summary: None,
        };
        assert!(
            svc.commit_character_state(CommitCharacterStateInput {
                character_id: character_id.clone(),
                scene_id: None,
                anchor: None,
                changes: changes.clone(),
            })
            .await
            .is_err(),
            "neither placement must fail closed"
        );
        assert!(
            svc.commit_character_state(CommitCharacterStateInput {
                character_id,
                scene_id: Some("scene:missing".into()),
                anchor: Some(CharacterStateAnchor::StoryStart),
                changes,
            })
            .await
            .is_err(),
            "both placements must fail closed before any write"
        );
    }

    #[tokio::test]
    async fn story_start_patch_does_not_merge_scene_one_state_backwards() {
        use spindle_core::models::{
            CharacterStateAnchor, CharacterStatePatch, CommitCharacterStateInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "anchor-cutoff".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        let scene_one =
            save_scene(&svc, &project.project_id, 1, "Ada Ward crossed the yard.").await;
        let scene_state = svc
            .commit_character_state(CommitCharacterStateInput {
                character_id: character_id.clone(),
                scene_id: Some(scene_one),
                anchor: None,
                changes: CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: Some(vec!["scene-one goal".into()]),
                    status: Some(vec!["dead".into()]),
                    notes: Some(vec!["scene-one note".into()]),
                    source_summary: None,
                },
            })
            .await
            .unwrap();
        let scene_state = svc
            .repository()
            .get_character_state_by_id(&scene_state.state_id)
            .await
            .unwrap();
        assert!(scene_state.status.iter().any(|status| status == "dead"));

        let anchor = svc
            .commit_character_state(CommitCharacterStateInput {
                character_id,
                scene_id: None,
                anchor: Some(CharacterStateAnchor::StoryStart),
                changes: CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: None,
                    status: None,
                    notes: Some(vec!["backstory note".into()]),
                    source_summary: None,
                },
            })
            .await
            .unwrap();
        let anchor_state = svc
            .repository()
            .get_character_state_by_id(&anchor.state_id)
            .await
            .unwrap();
        assert!(
            !anchor_state.status.iter().any(|status| status == "dead"),
            "scene-1 death must not merge backwards: {anchor_state:?}"
        );
        assert!(
            anchor_state.goals.is_empty(),
            "scene-1 goals must not merge backwards: {anchor_state:?}"
        );
        assert_eq!(anchor_state.notes, vec!["backstory note".to_string()]);
    }

    #[tokio::test]
    async fn story_start_anchor_sets_dead_state_before_first_scene() {
        use spindle_core::models::{
            CharacterStateAnchor, CharacterStatePatch, CommitCharacterStateInput,
            GetCharacterSnapshotInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "story-start-death".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "the dead stay dead".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        svc.commit_character_state(CommitCharacterStateInput {
            character_id: character_id.clone(),
            scene_id: None,
            anchor: Some(CharacterStateAnchor::StoryStart),
            changes: CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: Some(vec!["dead".into()]),
                notes: None,
                source_summary: Some("died before the story opens".into()),
            },
        })
        .await
        .unwrap();

        let snapshot = svc
            .get_character_snapshot(GetCharacterSnapshotInput {
                project_id: project.project_id.clone(),
                character_id: character_id.clone(),
                branch_id: None,
            })
            .await
            .unwrap();
        assert!(
            snapshot
                .current_state
                .as_ref()
                .is_some_and(|state| state.status.iter().any(|status| status == "dead")),
            "story-start state must be visible in the first snapshot: {:?}",
            snapshot.current_state
        );

        save_scene(&svc, &project.project_id, 1, "Ada Ward opened the door.").await;
        let issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("Ada Ward") && issue.message.contains("dead")),
            "story-start death must flag an action in the first scene: {issues:?}"
        );
    }

    async fn register_life_status(
        svc: &SqliteSpindleService,
        project_id: &str,
        character_id: &str,
        key: &str,
        value: &str,
        valid_from: Option<spindle_core::models::StoryPlacement>,
    ) {
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project_id.to_string(),
            scene_id: None,
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("life_status".into()),
            key: Some(key.to_string()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(character_id.to_string()),
            predicate: Some("life_status".into()),
            value_kind: Some("text".into()),
            value_text: Some(value.to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();
    }

    async fn register_dead_life_status(
        svc: &SqliteSpindleService,
        project_id: &str,
        character_id: &str,
        valid_from: Option<spindle_core::models::StoryPlacement>,
    ) {
        register_life_status(
            svc,
            project_id,
            character_id,
            "Ada Ward:life_status",
            "dead",
            valid_from,
        )
        .await;
    }

    #[tokio::test]
    async fn explicit_story_start_life_status_fact_flags_first_scene_action() {
        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "planned-story-start-death".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "the dead stay dead".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        register_dead_life_status(
            &svc,
            &project.project_id,
            &character_id,
            Some(spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(0),
                note: Some("pre-story death".into()),
            }),
        )
        .await;
        save_scene(&svc, &project.project_id, 1, "Ada Ward opened the door.").await;
        let issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("Ada Ward") && issue.message.contains("dead")),
            "explicit (1,1,0) life_status must count from story start: {issues:?}"
        );
    }

    #[tokio::test]
    async fn ordinary_chapter_one_life_status_fact_does_not_flag_opening_scene() {
        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "ordinary-chapter-one-death".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "the dead stay dead".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        register_dead_life_status(
            &svc,
            &project.project_id,
            &character_id,
            Some(spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            }),
        )
        .await;
        save_scene(&svc, &project.project_id, 1, "Ada Ward opened the door.").await;
        let issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        assert!(
            !issues
                .iter()
                .any(|issue| issue.message.contains("Ada Ward") && issue.message.contains("dead")),
            "an ordinary chapter-1 fact must not apply before its scene: {issues:?}"
        );
    }

    #[tokio::test]
    async fn newer_dead_state_at_story_start_beats_empty_state() {
        use spindle_core::models::{
            CharacterStateAnchor, CharacterStatePatch, CommitCharacterStateInput,
            GetCharacterSnapshotInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "same-position-death".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "the dead stay dead".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        svc.commit_character_state(CommitCharacterStateInput {
            character_id: character_id.clone(),
            scene_id: None,
            anchor: Some(CharacterStateAnchor::StoryStart),
            changes: CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: Some(vec!["life_status:dead".into()]),
                notes: None,
                source_summary: Some("backstory death".into()),
            },
        })
        .await
        .unwrap();
        save_scene(&svc, &project.project_id, 1, "Ada Ward opened the door.").await;

        let snapshot = svc
            .get_character_snapshot(GetCharacterSnapshotInput {
                project_id: project.project_id.clone(),
                character_id: character_id.clone(),
                branch_id: None,
            })
            .await
            .unwrap();
        assert!(
            snapshot.current_state.as_ref().is_some_and(|state| state
                .status
                .iter()
                .any(|status| status == "life_status:dead")),
            "snapshot must select the newer dead state: {:?}",
            snapshot.current_state
        );

        let issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        let dead = issues
            .iter()
            .find(|issue| {
                issue.message.contains("Ada Ward")
                    && (issue.check_type == "dead_character_acting"
                        || issue.check_type == "retcon_reachability")
            })
            .expect("dead action must be reported");
        assert_eq!(dead.severity, "error", "explicit death marker is an error");
    }

    #[tokio::test]
    async fn latest_of_two_states_in_one_scene_wins_and_snapshot_agrees() {
        use spindle_core::models::{
            CharacterStatePatch, CommitCharacterStateInput, GetCharacterSnapshotInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "same-scene-revisions".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "the dead stay dead".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        let scene_one = save_scene(&svc, &project.project_id, 1, "Ada Ward waited.").await;
        for (status, goal, note) in [
            ("life_status:alive", "first goal", "first note"),
            ("life_status:dead", "second goal", "second note"),
        ] {
            svc.commit_character_state(CommitCharacterStateInput {
                character_id: character_id.clone(),
                scene_id: Some(scene_one.clone()),
                anchor: None,
                changes: CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: Some(vec![goal.into()]),
                    status: Some(vec![status.into()]),
                    notes: Some(vec![note.into()]),
                    source_summary: None,
                },
            })
            .await
            .unwrap();
        }
        save_scene(&svc, &project.project_id, 2, "Ada Ward opened the door.").await;

        let snapshot = svc
            .get_character_snapshot(GetCharacterSnapshotInput {
                project_id: project.project_id.clone(),
                character_id: character_id.clone(),
                branch_id: None,
            })
            .await
            .unwrap();
        let state = snapshot
            .current_state
            .as_ref()
            .expect("current state must be present");
        assert_eq!(state.goals, vec!["second goal".to_string()]);
        assert!(
            state
                .status
                .iter()
                .any(|status| status == "life_status:dead")
        );

        let issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        let dead = issues
            .iter()
            .find(|issue| {
                issue.message.contains("Ada Ward")
                    && (issue.check_type == "dead_character_acting"
                        || issue.check_type == "retcon_reachability")
            })
            .expect("the latest same-scene state must drive the detector");
        assert_eq!(dead.severity, "error");
    }

    #[tokio::test]
    async fn newer_alive_life_status_fact_clears_older_dead_fact_at_same_position() {
        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "fact-recency".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "the dead stay dead".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let character_id = create_character_named(&svc, &project.project_id, "Ada Ward").await;
        let story_start = Some(spindle_core::models::StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(0),
            note: Some("pre-story".into()),
        });
        register_life_status(
            &svc,
            &project.project_id,
            &character_id,
            "Ada Ward:first_life_status",
            "dead",
            story_start.clone(),
        )
        .await;
        register_life_status(
            &svc,
            &project.project_id,
            &character_id,
            "Ada Ward:second_life_status",
            "alive",
            story_start,
        )
        .await;
        save_scene(&svc, &project.project_id, 1, "Ada Ward opened the door.").await;
        let issues = run_checks(&svc, &project.project_id, "retcon_reachability").await;
        assert!(
            !issues.iter().any(|issue| {
                issue.message.contains("Ada Ward")
                    && (issue.check_type == "dead_character_acting"
                        || issue.check_type == "retcon_reachability")
            }),
            "the newer alive fact must clear the older dead fact: {issues:?}"
        );
    }

    async fn run_checks(
        svc: &SqliteSpindleService,
        project_id: &str,
        check: &str,
    ) -> Vec<spindle_core::models::ConsistencyIssue> {
        svc.check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.to_string(),
            scope: ConsistencyScopeInput::full(),
            checks: vec![check.to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap()
        .issues
    }

    #[tokio::test]
    async fn unkeyed_attribute_contradiction_is_detected() {
        let (_tmp, svc) = fresh_service().await;
        let (project, seed_scene_id, _) = project_with_scene(&svc, "Unkeyed Attribute").await;
        let character_id = create_character_named(&svc, &project.project_id, "Mara Vance").await;
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(seed_scene_id),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("character_attribute".into()),
            key: Some("Mara left hand prosthetic".into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(character_id),
            predicate: Some("Mara left hand prosthetic".into()),
            value_kind: Some("text".into()),
            value_text: Some("articulated iron gauntlet".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();
        save_scene(
            &svc,
            &project.project_id,
            11,
            "Mara Vance flexed the five bare, unscarred fingers of her flesh-and-blood left hand.",
        )
        .await;

        let issues = run_checks(&svc, &project.project_id, "canonical_fact_prose_drift").await;
        assert!(
            issues.iter().any(|issue| {
                issue.check_type == "canonical_fact_prose_drift"
                    && issue.message.contains("chapter 11")
                    && (issue.message.contains("prosthetic") || issue.message.contains("gauntlet"))
            }),
            "expected unkeyed attribute drift, got {issues:?}"
        );
    }

    #[tokio::test]
    async fn unkeyed_attribute_negation_and_pov_controls_are_clean() {
        let (_tmp, svc) = fresh_service().await;
        let (project, seed_scene_id, _) =
            project_with_scene(&svc, "Unkeyed Attribute Control").await;
        let character_id = create_character_named(&svc, &project.project_id, "Mara Vance").await;
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(seed_scene_id),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("character_attribute".into()),
            key: Some("Mara left hand prosthetic".into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(character_id),
            predicate: Some("Mara left hand prosthetic".into()),
            value_kind: Some("text".into()),
            value_text: Some("articulated iron gauntlet".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();
        save_scene(
            &svc,
            &project.project_id,
            11,
            "Mara Vance did not have a bare flesh-and-blood left hand. In the dream, she flexed bare fingers.",
        )
        .await;
        let issues = run_checks(&svc, &project.project_id, "canonical_fact_prose_drift").await;
        assert!(
            issues
                .iter()
                .all(|issue| !issue.message.contains("conflicts with canonical value")),
            "negation/POV control must not flag: {issues:?}"
        );
    }

    #[tokio::test]
    async fn paraphrased_knowledge_leak_is_detected_before_learned() {
        let (_tmp, svc) = fresh_service().await;
        let (project, _seed_scene_id, _) = project_with_scene(&svc, "Paraphrase Knowledge").await;
        let character_id = create_character_named(&svc, &project.project_id, "Caelen Dravos").await;
        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project.project_id.clone(),
            character_id,
            branch_id: None,
            fact: "Commander Varek poisoned the well at Salt Spire".into(),
            source_summary: "Nightshade found at the well.".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 16,
                scene_order: Some(1),
                note: None,
            }),
            confidence: Some(1.0),
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: None,
        })
        .await
        .unwrap();
        let scene_id = save_scene(
            &svc,
            &project.project_id,
            6,
            "Before retreating from the arch, Caelen Dravos warned the guards that Varek had already tainted the drinking water beneath the Salt Spire.",
        )
        .await;
        let issues = run_checks(&svc, &project.project_id, "knowledge_timing").await;
        assert!(
            issues.iter().any(|issue| {
                issue.check_type == "knowledge_timing"
                    && issue.message.contains("chapter 16")
                    && issue.message.contains("Caelen Dravos")
            }),
            "expected paraphrased knowledge leak, got {issues:?}"
        );
        let scene = svc.repository().get_scene(&scene_id).await.unwrap();
        let retcon = svc
            .scan_retcon_findings(&project.project_id, &scene)
            .await
            .unwrap();
        assert!(
            retcon.iter().any(|finding| matches!(
                finding,
                spindle_core::models::RetconFinding::PrematureKnowledge { .. }
            )),
            "write-time retcon scanner must share the paraphrase lane: {retcon:?}"
        );
    }

    #[tokio::test]
    async fn negated_communication_control_is_clean_in_retcon_scanner() {
        let (_tmp, svc) = fresh_service().await;
        let (project, _seed_scene_id, _) = project_with_scene(&svc, "Negated Knowledge").await;
        let character_id = create_character_named(&svc, &project.project_id, "Caelen Dravos").await;
        svc.record_knowledge(RecordKnowledgeInput {
            project_id: project.project_id.clone(),
            character_id,
            branch_id: None,
            fact: "Commander Varek poisoned the well at Salt Spire".into(),
            source_summary: "Nightshade found at the well.".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 16,
                scene_order: Some(1),
                note: None,
            }),
            confidence: Some(1.0),
            tags: Vec::new(),
            reader_visible: true,
            secret_of_fact_id: None,
        })
        .await
        .unwrap();
        let scene_id = save_scene(
            &svc,
            &project.project_id,
            6,
            "Caelen Dravos never warned the guards that Varek had tainted the water beneath Salt Spire.",
        )
        .await;
        let scene = svc.repository().get_scene(&scene_id).await.unwrap();
        let retcon = svc
            .scan_retcon_findings(&project.project_id, &scene)
            .await
            .unwrap();
        assert!(
            !retcon.iter().any(|finding| matches!(
                finding,
                spindle_core::models::RetconFinding::PrematureKnowledge { .. }
            )),
            "negated communication must not become premature-knowledge: {retcon:?}"
        );
    }

    #[tokio::test]
    async fn world_rule_implicit_cost_omission_is_detected() {
        let (_tmp, svc) = fresh_service().await;
        let (project, _seed_scene_id, _) = project_with_scene(&svc, "Rule Cost Omission").await;
        svc.create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "Neural Splice Cooldown".into(),
            rule_type: "magic_system".into(),
            description: "Running a deep neural splice override drains thirty percent of the rig battery and causes eye strain.".into(),
            scan_pattern: Some("channel|cast|invoke|unleash".into()),
            relevance_tags: vec!["splice".into()],
            established_in: Some(spindle_core::models::EstablishedIn {
                book_number: 1,
                chapter_number: 1,
                note: None,
            }),
        })
        .await
        .unwrap();
        save_scene(
            &svc,
            &project.project_id,
            13,
            "Tarek Nouri then invoked a deep neural splice on the relay using zero battery charge and suffering no eye strain at all.",
        )
        .await;
        let issues = run_checks(&svc, &project.project_id, "world_rule_semantic_drift").await;
        assert!(
            issues.iter().any(|issue| {
                issue.check_type == "world_rule_semantic_drift"
                    && issue.message.contains("likely violates world rule")
                    && issue.message.contains("Neural Splice Cooldown")
            }),
            "expected implicit cost omission, got {issues:?}"
        );
    }
}

mod secret_leak_narration {
    use super::*;
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateCharacterInput,
        RegisterCanonicalFactInput, SaveSceneDraftInput, SecrecyScope,
    };

    async fn make_character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
        svc.create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project_id.to_string(),
            name: name.to_string(),
            summary: format!("{name} in play."),
            role: "supporting".into(),
            realm: None,
            voice_profile: spindle_core::models::CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: spindle_core::models::CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap()
        .character_id
    }

    #[tokio::test]
    async fn out_of_circle_narration_leak_is_detected() {
        let (_tmp, svc) = fresh_service().await;
        let (project, seed_scene_id, _) = project_with_scene(&svc, "Narration Secret").await;
        let holder = make_character(&svc, &project.project_id, "Mara Vance").await;
        let outsider = make_character(&svc, &project.project_id, "Caelen Dravos").await;
        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(seed_scene_id),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("secret_identity".into()),
            key: Some("vault passphrase".into()),
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(holder.clone()),
            predicate: Some("vault passphrase".into()),
            value_kind: Some("text".into()),
            value_text: Some("obsidiancrown".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: Some(SecrecyScope {
                holder_ids: vec![holder.clone()],
                concealment_note: None,
            }),
        })
        .await
        .unwrap();
        let _ = svc
            .create_chapter(spindle_core::models::CreateChapterInput {
                project_id: project.project_id.clone(),
                book_id: None,
                book_number: Some(1),
                chapter_number: Some(17),
                title: None,
            })
            .await;
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 17,
            chapter_id: None,
            scene_order: 1,
            full_text: "While Mara Vance turned away, Caelen Dravos quietly copied the vault passphrase obsidiancrown into his personal field notebook.".into(),
            summary: "narration leak".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
        let issues = svc
            .check_consistency(CheckConsistencyInput {
                deep_scan_offset: None,
                project_id: project.project_id.clone(),
                scope: ConsistencyScopeInput::full(),
                checks: vec!["secret_leak".to_string()],
                severity_filter: Vec::new(),
                deep_check: Some(false),
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap()
            .issues;
        let narration: Vec<_> = issues
            .iter()
            .filter(|issue| {
                issue.check_type == "secret_leak"
                    && issue.message.contains("narrates secret fact")
                    && issue.message.contains("obsidiancrown")
            })
            .collect();
        assert_eq!(
            narration.len(),
            1,
            "expected one narration leak: {issues:?}"
        );
        assert!(
            narration[0].entity_ids.contains(&outsider),
            "the acting outsider is the leak entity: {:?}",
            narration[0].entity_ids
        );
    }
}

#[tokio::test]
async fn check_consistency_does_not_emit_info_for_clean_world_rule_cost_demo() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateWorldRuleInput,
        EstablishedIn, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ph4-world-neutral".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: proj.project_id.clone(),
        rule_name: "Salt ward blood price".into(),
        rule_type: "magic".into(),
        description: "A salt ward costs one drop of blood to hold.".into(),
        scan_pattern: Some("salt ward".into()),
        relevance_tags: Vec::new(),
        established_in: Some(EstablishedIn {
            book_number: 1,
            chapter_number: 1,
            note: None,
        }),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara paid the blood price and the salt ward held.".into(),
        summary: "ward check".into(),
        content_rating: ContentRating::General,
        tone: Some("calm".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        !out.issues
            .iter()
            .any(|issue| issue.check_type == "world_rule_semantic_drift"),
        "an on-page cost payment is a clean demonstration: {:?}",
        out.issues
    );
}
