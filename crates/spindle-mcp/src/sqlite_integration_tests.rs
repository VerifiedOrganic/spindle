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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authoring_supervisor_integration_flow() {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_core::models::ConfigureAgentsInput;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    // 1. Create a temp directory for the database and repository data
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_supervisor.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // 2. Set up CLI mock script
    let script_path = tmp.path().join("mock_agent.sh");
    let script_content = r#"#!/bin/bash
ROUTE=$1
PROMPT=$2

if [ "$ROUTE" = "draft" ]; then
  cat <<EOF
{
  "full_text": "Mara stood watch at the Ash Gate, clutching her salt charm.",
  "summary": "Mara watch",
  "tone": "grim",
  "character_states": [],
  "canonical_facts": [],
  "relationship_updates": [],
  "beats": [],
  "continuity_notes": []
}
EOF
elif [ "$ROUTE" = "review" ]; then
  cat <<EOF
STRENGTHS:
- Strong description of the salt charm.
- Natural pacing.

CONCERNS:
- None.
EOF
else
  cat <<EOF
{
  "summary": "Mara held the gate.",
  "key_events": [],
  "character_changes": [],
  "relationship_shifts": [],
  "arc_advances": [],
  "promise_events": []
}
EOF
fi
"#;
    std::fs::write(&script_path, script_content).unwrap();

    let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).unwrap();

    // 3. Write config.toml
    let config_path = tmp.path().join("config.toml");
    let config_content = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{}"
model = "default"
ratings = ["general", "explicit"]

[[agents]]
id = "cli-agent-review"
name = "CLI Agent Review"
provider = "cli"
endpoint = "{}"
model = "default"
ratings = ["general"]

[[routing]]
route = "draft"
agent = "cli-agent-draft"

[[routing]]
route = "review"
agent = "cli-agent-review"
"#,
        script_path.display(),
        script_path.display()
    );
    std::fs::write(&config_path, config_content).unwrap();

    // Set CLI COMMAND environment variable
    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    // 4. Initialize service and configure agents
    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();

    // 5. Create project and entities
    let project = svc
        .create_project(CreateProjectInput {
            name: "Supervised Project".into(),
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

    let loc = svc
        .create_location(CreateLocationInput {
            project_id: project.project_id.clone(),
            name: "Ash Gate".into(),
            kind: "fortress".into(),
            realm: None,
            summary: "Blackened wall.".into(),
            initial_state: WorldStateInput::default(),
        })
        .await
        .unwrap();

    // 6. Test authoring_prepare_run reports missing requirements clearly
    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    let prep_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1
    });
    let prep_res = router
        .call_tool(
            "authoring_prepare_run",
            Some(prep_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(prep_res.is_error, Some(false));
    let prep_val = prep_res.structured_content.unwrap();
    assert_eq!(prep_val["ready_to_draft"].as_bool(), Some(false));
    assert!(
        prep_val["missing_requirements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|val| val.as_str().unwrap().contains("missing chapter plan"))
    );

    // 7. Add the plan chapters and scenes to satisfy requirements
    svc.plan_chapter(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(mara.character_id.clone()),
        synopsis: "First watch.".into(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![
            PlanChapterSceneInput {
                scene_order: 1,
                summary: "Mara takes the watch".into(),
                beat_structure: Vec::new(),
                character_ids: vec![mara.character_id.clone()],
                location_id: Some(loc.location_id.clone()),
                content_rating: Some(ContentRating::General),
                purpose: "establishing".into(),
                ..Default::default()
            },
            PlanChapterSceneInput {
                scene_order: 2,
                summary: "Mara encounters the beast".into(),
                beat_structure: Vec::new(),
                character_ids: vec![mara.character_id.clone()],
                location_id: Some(loc.location_id.clone()),
                content_rating: Some(ContentRating::Explicit),
                purpose: "climax".into(),
                ..Default::default()
            },
        ],
    })
    .await
    .unwrap();

    // Now call prepare again, it should say "ready"
    let prep_res2 = router
        .call_tool(
            "authoring_prepare_run",
            Some(prep_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(prep_res2.is_error, Some(false));
    let prep_val2 = prep_res2.structured_content.unwrap();
    assert_eq!(
        prep_val2["ready_to_draft"].as_bool(),
        Some(true),
        "prepare failed: {:?}",
        prep_val2
    );

    // Invalid run parameters should block before creating a broken run.
    let invalid_start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 0
    });
    let invalid_start_res = router
        .call_tool(
            "authoring_start_run",
            Some(invalid_start_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(invalid_start_res.is_error, Some(false));
    let invalid_start_val = invalid_start_res.structured_content.unwrap();
    assert_eq!(invalid_start_val["status"].as_str(), Some("blocked"));
    assert_eq!(invalid_start_val["run_id"].as_str(), Some(""));

    // 8. Start the run
    let start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1
    });
    let start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    assert_eq!(start_res.is_error, Some(false));
    let start_val = start_res.structured_content.unwrap();
    let run_id = start_val["run_id"].as_str().unwrap().to_string();
    assert!(run_id.starts_with("authoring_run:"));

    // A paused run must not continue advancing when execute_next is called.
    let paused_start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    let paused_run_id = paused_start_res.structured_content.unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string();
    let cancel_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": paused_run_id
    });
    let cancel_res = router
        .call_tool(
            "authoring_cancel_run",
            Some(cancel_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(cancel_res.is_error, Some(false));
    let paused_exec_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": paused_run_id
    });
    let paused_exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(paused_exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(paused_exec_res.is_error, Some(false));
    let paused_exec_val = paused_exec_res.structured_content.unwrap();
    assert_eq!(paused_exec_val["status"].as_str(), Some("paused"));
    assert_eq!(paused_exec_val["executed_action"].as_str(), Some("none"));

    // 9. Start background Axum MCP HTTP server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    crate::write_addr_file(&data_dir, addr).unwrap();

    let svc_clone = svc.clone();
    let ct = CancellationToken::new();
    let ct_clone1 = ct.clone();
    let ct_clone2 = ct.clone();
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, crate::http::mcp_router(svc_clone, ct_clone1))
            .with_graceful_shutdown(async move { ct_clone2.cancelled_owned().await })
            .await
            .unwrap();
    });

    // 10. Execute next step (Draft Scene 1 - General)
    println!("TEST: Step 10 - Draft Scene 1.1");
    let exec_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id
    });
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(
        exec_res.is_error,
        Some(false),
        "authoring_execute_next failed: {:?}",
        exec_res
    );
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("draft book scene 1.1")
    );

    // Check status
    println!("TEST: Checking status after draft scene 1.1");
    let status_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id
    });
    let status_res = router
        .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
        .await
        .unwrap();
    let status_val = status_res.structured_content.unwrap();
    assert_eq!(status_val["blocked_reason"].as_str(), None);

    // 11. Execute next step (Commit Scene 1)
    println!("TEST: Step 11 - Commit Scene 1.1");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes")
    );

    // 11b. Execute next step (Annotate Beats Scene 1)
    println!("TEST: Step 11b - Annotate Beats Scene 1.1");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("annotate beats")
    );

    // 12. Execute next step (Draft Scene 2 - Explicit)
    println!("TEST: Step 12 - Draft Scene 1.2");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("draft book scene 1.2")
    );

    // Let's verify that the receipt was registered and it carries rating: explicit and explicit_capable = true
    println!("TEST: Verifying model receipts");
    let receipts = svc.get_all_generation_receipts();
    assert!(
        receipts.iter().any(|(rating, explicit_capable)| {
            rating.as_deref() == Some("explicit") && *explicit_capable
        }),
        "No explicit-capable explicit receipt found: {:?}",
        receipts
    );

    // 13. Execute next step (Commit Scene 2)
    println!("TEST: Step 13 - Commit Scene 1.2");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes")
    );

    // 13b. Execute next step (Annotate Beats Scene 2)
    println!("TEST: Step 13b - Annotate Beats Scene 1.2");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("annotate beats")
    );

    // 14. Execute next step (Save Chapter Summary)
    println!("TEST: Step 14 - Save Chapter Summary");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("save summary for chapter 1")
    );

    // 15. Execute next step (Run Checkpoint)
    println!("TEST: Step 15 - Run Checkpoint");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("run checkpoint")
    );

    // Verify it is blocked at checkpoint review
    println!("TEST: Verifying blocked at checkpoint review");
    let status_res = router
        .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
        .await
        .unwrap();
    let status_val = status_res.structured_content.unwrap();
    assert_eq!(
        status_val["blocked_reason"].as_str().unwrap(),
        "await_checkpoint_review"
    );
    assert!(
        status_val["next_action"]
            .as_str()
            .unwrap()
            .contains("await checkpoint review")
    );

    // Execute next should fail/be blocked
    println!("TEST: Executing next while blocked");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert_eq!(exec_val["status"].as_str().unwrap(), "blocked");

    // 16. Shut down background server to simulate service restart
    println!("TEST: Shutting down first background server");
    ct.cancel();
    server_handle.await.unwrap();
    crate::remove_addr_file(&data_dir);

    // 17. Restart service (fresh service & new background server using same DB and data dir)
    println!("TEST: Restarting service and spawning second background server");
    let pool2 = SqlitePool::open(&db_path).await.unwrap();
    let repo2 =
        Repository::with_model_router(pool2.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc2 = SqliteSpindleService::new(repo2);
    svc2.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();

    let router2 = ToolRouter::with_tool_profile_and_serialization(
        svc2.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    crate::write_addr_file(&data_dir, addr).unwrap();

    let svc2_clone = svc2.clone();
    let ct2 = CancellationToken::new();
    let ct2_clone1 = ct2.clone();
    let ct2_clone2 = ct2.clone();
    let server_handle2 = tokio::spawn(async move {
        axum::serve(listener, crate::http::mcp_router(svc2_clone, ct2_clone1))
            .with_graceful_shutdown(async move { ct2_clone2.cancelled_owned().await })
            .await
            .unwrap();
    });

    // Check status after restart, should still say "await_checkpoint_review"
    println!("TEST: Checking status after restart");
    let status_res = router2
        .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
        .await
        .unwrap();
    let status_val = status_res.structured_content.unwrap();
    assert_eq!(
        status_val["blocked_reason"].as_str().unwrap(),
        "await_checkpoint_review"
    );

    // 18. Review checkpoint and resume
    println!("TEST: Step 18 - Review Checkpoint");
    let review_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "start_chapter": 1,
        "end_chapter": 1,
        "directives": ["Keep the prose dark."]
    });
    let review_res = router2
        .call_tool(
            "authoring_review_checkpoint",
            Some(review_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(review_res.is_error, Some(false));

    // Execute next should now complete the run
    let exec_res = router2
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let exec_val = exec_res.structured_content.unwrap();
    assert_eq!(exec_val["next_action"].as_str().unwrap(), "complete");

    // Shut down second server
    ct2.cancel();
    server_handle2.await.unwrap();
    crate::remove_addr_file(&data_dir);
}

#[tokio::test]
async fn test_two_independent_book_workspaces() {
    use spindle_adapters::sqlite::{Repository, SqliteSpindleService};
    use spindle_adapters::workspace::resolve_workspace;

    // Create temp book dir A and initialize it.
    let temp_a = TempDir::new().unwrap();
    let book_a = temp_a.path().join("book-a");
    std::fs::create_dir_all(&book_a).unwrap();
    // Simulate spindle init / workspace initialization:
    std::fs::create_dir_all(book_a.join(".spindle")).unwrap();

    // Create temp book dir B and initialize it.
    let temp_b = TempDir::new().unwrap();
    let book_b = temp_b.path().join("book-b");
    std::fs::create_dir_all(&book_b).unwrap();
    // Simulate spindle init / workspace initialization:
    std::fs::create_dir_all(book_b.join(".spindle")).unwrap();

    // Resolve workspace for A
    let ws_a = resolve_workspace(&book_a, None, None);
    assert_eq!(ws_a.db_path, book_a.join(".spindle").join("spindle.db"));

    // Resolve workspace for B
    let ws_b = resolve_workspace(&book_b, None, None);
    assert_eq!(ws_b.db_path, book_b.join(".spindle").join("spindle.db"));

    // Initialize databases
    let pool_a = SqlitePool::open(&ws_a.db_path).await.unwrap();
    let repo_a = Repository::new(pool_a, ws_a.data_dir);
    let svc_a = SqliteSpindleService::new(repo_a);

    let pool_b = SqlitePool::open(&ws_b.db_path).await.unwrap();
    let repo_b = Repository::new(pool_b, ws_b.data_dir);
    let svc_b = SqliteSpindleService::new(repo_b);

    // Create a project in book A
    let project_a = svc_a
        .create_project(CreateProjectInput {
            name: "Book A Project".into(),
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

    // Create a different project in book B
    let project_b = svc_b
        .create_project(CreateProjectInput {
            name: "Book B Project".into(),
            project_type: "novel".into(),
            genre: "sci-fi".into(),
            reader_contract: ReaderContract {
                promise: "Spaceships fly.".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Prove A cannot see B's data
    let search_a = svc_a
        .search_bible(SearchBibleInput {
            project_id: project_a.project_id.clone(),
            query: "Book".into(),
            limit: Some(10),
            mode: Some(SearchBibleMode::Exact),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    // B's project shouldn't be in A's database
    assert!(!search_a.results.iter().any(|r| r.title == "Book B Project"));

    // Prove B cannot see A's data
    let search_b = svc_b
        .search_bible(SearchBibleInput {
            project_id: project_b.project_id.clone(),
            query: "Book".into(),
            limit: Some(10),
            mode: Some(SearchBibleMode::Exact),
            field: None,
            subject_table: None,
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    // A's project shouldn't be in B's database
    assert!(!search_b.results.iter().any(|r| r.title == "Book A Project"));
}
