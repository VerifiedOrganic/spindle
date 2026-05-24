//! Phase 6 integration tests against the SQLite stack.
//!
//! Mirrors the structure of `integration_tests.rs` but exercises the new
//! `SqliteSpindleService` directly (without the MCP JSON-RPC layer). This
//! file lives in `spindle-mcp` to assert that the SQLite migration produces
//! a backend that satisfies the integration-test contract from the MCP crate's
//! perspective — the methods MCP would dispatch to, the input/output shapes
//! the MCP tools expose, and the cross-tool flows the original
//! integration_tests.rs walks through.
//!
//! Once Phase 6's MCP layer swap completes, these tests' content folds back
//! into the main integration_tests.rs and this module is deleted. Until then
//! they serve as the integration-level proof that the SQLite stack is sound.

#![cfg(test)]

use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CanonicalFactScope, CharacterEmotionalProfileData, CharacterStatePatch,
    CharacterVoiceProfileData, CommitCharacterStateInput, ContentRating, CreateBranchInput,
    CreateCharacterInput, CreateLocationInput, CreateProjectInput, CreateRelationshipInput,
    PlanChapterInput, PlanChapterSceneInput, ReaderContract, RecordKnowledgeInput,
    RegisterCanonicalFactInput, SaveSceneDraftInput, SaveSummaryInput, SearchBibleInput,
    SearchBibleMode, StoryPlacement, SwitchBranchInput, UpdateRelationshipInput, WorldStateInput,
};
use std::collections::BTreeMap;
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("test.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::default());
    (tmp, SqliteSpindleService::new(repo))
}

#[tokio::test]
async fn mcp_priority_flow_create_project_through_save_scene_draft() {
    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "Integration Test".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Mara holds the gate.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    assert!(project.project_id.starts_with("project:"));

    let mara = svc
        .create_character(CreateCharacterInput {
            project_id: project.project_id.clone(),
            name: "Mara".into(),
            summary: "Oathbound warden.".into(),
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
            emotional_profile: CharacterEmotionalProfileData {
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
    assert!(mara.character_id.starts_with("character:"));

    let saved = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood watch.".into(),
            summary: "Mara's first watch".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
    assert!(saved.scene_id.starts_with("scene:"));
    assert_eq!(saved.status, "saved");
}

#[tokio::test]
async fn mcp_priority_flow_full_chapter_with_branching_and_search() {
    let (_tmp, svc) = fresh_service().await;

    let project = svc
        .create_project(CreateProjectInput {
            name: "Branching Integration".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Oathbound wardens fail.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            project_id: project.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".into()),
                vocabulary: vec!["oath".into()],
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
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

    let aldric = svc
        .create_character(CreateCharacterInput {
            project_id: project.project_id.clone(),
            name: "Aldric".into(),
            summary: "Scribe.".into(),
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
            emotional_profile: CharacterEmotionalProfileData {
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

    svc.create_location(CreateLocationInput {
        project_id: project.project_id.clone(),
        name: "Ash Gate".into(),
        kind: "fortress".into(),
        realm: None,
        summary: "Blackened wall.".into(),
        initial_state: WorldStateInput::default(),
    })
    .await
    .unwrap();

    svc.create_relationship(CreateRelationshipInput {
        character_a_id: mara.character_id.clone(),
        character_b_id: aldric.character_id.clone(),
        relationship_type: "ally".into(),
        initial_trust: 60,
        initial_tension: 20,
        dynamics: vec!["wary respect".into()],
    })
    .await
    .unwrap();

    svc.plan_chapter(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(mara.character_id.clone()),
        synopsis: "First watch.".into(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: Vec::new(),
            character_ids: vec![mara.character_id.clone()],
            purpose: "establishing".into(),
        }],
    })
    .await
    .unwrap();

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "First watch".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

    svc.commit_character_state(CommitCharacterStateInput {
        character_id: mara.character_id.clone(),
        scene_id: scene.scene_id.clone(),
        changes: CharacterStatePatch {
            emotional_state: BTreeMap::new(),
            goals: Some(vec!["hold the gate".into()]),
            status: Some(vec!["determined".into()]),
            notes: None,
            source_summary: Some("first watch".into()),
        },
    })
    .await
    .unwrap();

    svc.record_knowledge(RecordKnowledgeInput {
        project_id: project.project_id.clone(),
        branch_id: None,
        character_id: mara.character_id.clone(),
        fact: "The dark advances from the north.".into(),
        source_summary: "scout report".into(),
        learned_at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        }),
        confidence: Some(0.8),
        tags: Vec::new(),
        reader_visible: true,
    })
    .await
    .unwrap();

    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: project.project_id.clone(),
        scene_id: scene.scene_id.clone(),
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
        aliases: Vec::new(),
        scope: Some(CanonicalFactScope::Invariant),
        valid_from: None,
        valid_until: None,
        legacy_untyped: None,
        supersedes_fact_id: None,
    })
    .await
    .unwrap();

    let lexical = svc
        .search_bible(SearchBibleInput {
            project_id: project.project_id.clone(),
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
        "FTS5 lexical search should find the character via summary"
    );

    let semantic = svc
        .search_bible(SearchBibleInput {
            project_id: project.project_id.clone(),
            query: "Mara stood at the gate.".into(),
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
        "Semantic search must return at least one ranked hit"
    );

    let updated = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: mara.character_id.clone(),
            character_b_id: aldric.character_id.clone(),
            trust_delta: 10,
            tension_delta: -5,
            reason: "Aldric helped".into(),
            scene_id: scene.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(updated.trust, 70);
    assert_eq!(updated.tension, 15);

    svc.save_summary(SaveSummaryInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "Mara held the gate.".into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();

    let feature = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "alt-ending".into(),
            branch_type: "feature".into(),
            description: None,
            parent_branch_id: None,
        })
        .await
        .unwrap();
    let switched = svc
        .switch_branch(SwitchBranchInput {
            project_id: project.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(switched.branch_id, feature.branch_id);
}
