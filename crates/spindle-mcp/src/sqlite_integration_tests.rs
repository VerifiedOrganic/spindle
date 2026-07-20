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
async fn run_journal_emit_never_fails_the_step_on_journal_error() {
    // ADR 0002 D3.3: a journal write error logs at warn and returns unit — it
    // never fails the run step. Injected failure: emit against a run_id that
    // does not exist, so the FK (authoring_run_event.authoring_run_id REFERENCES
    // authoring_run(id)) rejects the insert. `emit` must swallow it.
    use crate::run_journal::{RunJournal, run_started_payload};

    let (_tmp, svc) = fresh_service().await;
    let journal = RunJournal::new(svc.repository());

    // No panic, no error propagation — emit returns unit even though the
    // underlying append fails on the missing FK target.
    journal
        .emit(
            "authoring_run:does-not-exist",
            "run_started",
            run_started_payload(1, 1, 1, None, None, None),
        )
        .await;

    // Sanity: the append itself genuinely fails for this run_id (proving the
    // emitter swallowed a real error, not a silently-succeeding write).
    let direct = svc
        .repository()
        .append_run_event(
            "authoring_run:does-not-exist",
            "run_started",
            serde_json::json!({}),
        )
        .await;
    assert!(
        direct.is_err(),
        "append against a missing run must error (FK); emit must swallow it"
    );
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
        secret_of_fact_id: None,
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
        secrecy: None,
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
# Must cover every rating the run's sampled scenes carry: the rating-gated
# dispatch chokepoint (resolve_cleared_route) refuses to send explicit prose
# to a review agent that does not declare it.
ratings = ["general", "explicit"]

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
                research_required: Some(false),
                explicit_query: Some("nonblocking authoring metadata regression marker".into()),
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
    let (_, _, persisted_scenes, _) = svc
        .repository()
        .get_authoring_run(&run_id)
        .await
        .unwrap()
        .unwrap();
    let persisted_scene = persisted_scenes
        .iter()
        .find(|scene| scene.chapter_number == 1 && scene.scene_order == 1)
        .unwrap();
    assert_eq!(persisted_scene.research_required, Some(false));
    assert_eq!(
        persisted_scene.explicit_query.as_deref(),
        Some("nonblocking authoring metadata regression marker")
    );

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

    // 10. Default interactive mode should hand non-explicit drafting back to
    // the host assistant instead of routing it through the configured draft
    // backend.
    println!("TEST: Step 10 - Hybrid mode pauses for host draft Scene 1.1");
    let host_exec_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id
    });
    let host_exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(host_exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(
        host_exec_res.is_error,
        Some(false),
        "authoring_execute_next failed: {:?}",
        host_exec_res
    );
    let host_exec_val = host_exec_res.structured_content.unwrap();
    assert_eq!(host_exec_val["executed_action"].as_str(), Some("none"));
    assert_eq!(host_exec_val["status"].as_str(), Some("active"));
    assert!(
        host_exec_val["message"]
            .as_str()
            .unwrap()
            .contains("Host draft required for non-explicit scene 1.1"),
        "expected host draft instruction, got {host_exec_val:?}"
    );

    println!("TEST: Step 10b - Host saves Scene 1.1 with structured continuity package");
    let host_save_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "book_number": 1,
        "chapter_number": 1,
        "scene_order": 1,
        "full_text": "Mara stood watch at the Ash Gate, clutching her salt charm.",
        "summary": "Mara watch",
        "content_rating": "general",
        "tone": "grim",
        "canonical_facts": [{
            "fact_type": "scene_event",
            "key": "mara_watch_ash_gate",
            "value": "Mara stood watch at the Ash Gate with her salt charm.",
            "context": "Chapter 1 scene 1 host draft"
        }],
        "beats": [{
            "beat_type": "setup",
            "summary": "Mara keeps watch at the Ash Gate."
        }],
        "continuity_notes": [
            "Mara has her salt charm during the Ash Gate watch."
        ]
    });
    let host_saved_scene = router
        .call_tool(
            "authoring_save_scene_draft",
            Some(host_save_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(host_saved_scene.is_error, Some(false));
    let host_saved_val = host_saved_scene.structured_content.unwrap();
    assert_eq!(host_saved_val["status"].as_str(), Some("saved"));
    assert_eq!(host_saved_val["structured_update_count"].as_u64(), Some(3));
    assert!(
        host_saved_val["scene_artifact_path"]
            .as_str()
            .is_some_and(|path| path.contains("scene-001.json"))
    );

    let exec_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id
    });

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
    assert_eq!(
        exec_res.is_error,
        Some(false),
        "authoring_execute_next commit failed: {:?}",
        exec_res
    );
    let exec_val = exec_res.structured_content.unwrap();
    assert!(
        exec_val["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes")
    );
    let scene_1_id = host_saved_val["scene_id"].as_str().unwrap().to_string();
    let scene_1_text = svc
        .repository()
        .get_scene(&scene_1_id)
        .await
        .unwrap()
        .full_text;
    assert_eq!(
        scene_1_text,
        "Mara stood watch at the Ash Gate, clutching her salt charm."
    );
    let scene_1_artifact_rel = host_saved_val["scene_artifact_path"]
        .as_str()
        .unwrap()
        .to_string();
    let scene_1_artifact_path = data_dir.join("artifacts").join(&scene_1_artifact_rel);
    let committed_artifact: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&scene_1_artifact_path).unwrap()).unwrap();
    assert!(
        !committed_artifact["commit_output"].is_null(),
        "first commit should cache commit_output"
    );

    println!("TEST: Step 11a - Re-save Scene 1.1 with keep-existing prose");
    let keep_existing_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "book_number": 1,
        "chapter_number": 1,
        "scene_order": 1,
        "full_text": "_keep_existing_",
        "summary": "Mara watch revised package",
        "content_rating": "general",
        "tone": "grim",
        "beats": [{
            "beat_type": "setup",
            "summary": "Mara keeps watch at the Ash Gate after package revision."
        }],
        "continuity_notes": [
            "Package-only revision keeps the existing prose intact."
        ]
    });
    let keep_existing_res = router
        .call_tool(
            "authoring_save_scene_draft",
            Some(keep_existing_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(keep_existing_res.is_error, Some(false));
    let keep_existing_text = svc
        .repository()
        .get_scene(&scene_1_id)
        .await
        .unwrap()
        .full_text;
    assert_eq!(
        keep_existing_text, scene_1_text,
        "_keep_existing_ must not overwrite scene prose"
    );
    let resaved_artifact: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&scene_1_artifact_path).unwrap()).unwrap();
    assert_eq!(
        resaved_artifact["package"]["full_text"].as_str(),
        Some(scene_1_text.as_str())
    );
    assert!(
        resaved_artifact["commit_output"].is_null(),
        "re-saving package must invalidate stale commit output"
    );
    assert!(
        resaved_artifact["beat_annotation_output"].is_null(),
        "re-saving package must invalidate stale beat annotations"
    );

    println!("TEST: Step 11a.1 - Re-commit Scene 1.1 after package-only save");
    let exec_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(exec_res.is_error, Some(false), "{exec_res:?}");
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

    // 18. Checkpoint review is fail-closed until the model-heavy gates have
    // been run as separate resumable calls and recorded on the checkpoint:
    // deep consistency first, then sampled dual-persona reviews.
    println!("TEST: Step 18 - Checkpoint Review Requires Deep Consistency Audit");
    let review_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "start_chapter": 1,
        "end_chapter": 1,
        "directives": ["Keep the prose dark."]
    });
    let missing_review_res = router2
        .call_tool(
            "authoring_review_checkpoint",
            Some(review_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(missing_review_res.is_error, Some(true));
    let missing_deep_text = missing_review_res
        .content
        .first()
        .map(|content| format!("{content:?}"))
        .unwrap_or_default();
    assert!(
        missing_deep_text.contains("deep consistency is recorded"),
        "expected missing deep consistency error, got {missing_deep_text:?}"
    );

    let report_rel = status_val["checkpoint_reports"][0]["report_artifact_path"]
        .as_str()
        .unwrap();
    let report_path = data_dir.join("artifacts").join(report_rel);
    let report_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    let sampled_scene_ids = report_json["sampled_scene_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(!sampled_scene_ids.is_empty());

    let deep_consistency_args = serde_json::json!({
        "project_id": project.project_id,
        "scope": {
            "scope_type": "chapter_range",
            "book_number": null,
            "start_book_number": 1,
            "start_chapter_number": 1,
            "end_book_number": 1,
            "end_chapter_number": 1
        },
        "checks": [],
        "severity_filter": [],
        "deep_check": true,
        "subjects": []
    });
    let deep_consistency_res = router2
        .call_tool(
            "check_consistency",
            Some(deep_consistency_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(deep_consistency_res.is_error, Some(false));
    let deep_consistency = deep_consistency_res.structured_content.unwrap();
    let record_audit_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "start_chapter": 1,
        "end_chapter": 1,
        "deep_consistency": deep_consistency
    });
    let record_audit_res = router2
        .call_tool(
            "authoring_record_checkpoint_audit",
            Some(record_audit_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(record_audit_res.is_error, Some(false));

    println!("TEST: Step 18b - Checkpoint Review Requires Sampled Scene Reviews");
    let missing_sampled_review_res = router2
        .call_tool(
            "authoring_review_checkpoint",
            Some(review_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(missing_sampled_review_res.is_error, Some(true));
    let missing_sampled_review_text = missing_sampled_review_res
        .content
        .first()
        .map(|content| format!("{content:?}"))
        .unwrap_or_default();
    assert!(
        missing_sampled_review_text.contains("sampled dual-persona reviews are current"),
        "expected missing sampled review error, got {missing_sampled_review_text:?}"
    );

    for scene_id in &sampled_scene_ids {
        let sampled_review_args = serde_json::json!({
            "project_id": project.project_id,
            "scene_id": scene_id,
            "rounds": 2
        });
        let sampled_review_res = router2
            .call_tool(
                "run_dual_persona_review",
                Some(sampled_review_args.as_object().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(sampled_review_res.is_error, Some(false));
    }

    println!("TEST: Step 18c - Review Checkpoint Rejects Unresolved Directives");
    let unresolved_review_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "start_chapter": 1,
        "end_chapter": 1,
        "operator_override_unresolved_findings": true,
        "directives": ["ACKNOWLEDGED: continuity finding needs fixing in polish pass."]
    });
    let unresolved_review_res = router2
        .call_tool(
            "authoring_review_checkpoint",
            Some(unresolved_review_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(unresolved_review_res.is_error, Some(true));
    let unresolved_review_text = unresolved_review_res
        .content
        .first()
        .map(|content| format!("{content:?}"))
        .unwrap_or_default();
    assert!(
        unresolved_review_text.contains("fixable findings unresolved"),
        "expected unresolved directive rejection, got {unresolved_review_text:?}"
    );

    println!("TEST: Step 18c - Review Checkpoint");
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

/// End-to-end run-event journal (ADR 0002) trace + payload-discipline pin (J3
/// test 3 + J4). Drives a full run (host draft + mining + in-run verify) to
/// completion, then:
///  - asserts the expected per-scene kind order is present with no duplicate
///    step events, `chapter_summarized` + `checkpoint_created`/`_reviewed`
///    present, and `run_completed` last;
///  - asserts a distinctive sentinel embedded in scene prose AND canonical-fact
///    values appears in ZERO journal payloads (D3.1 no-prose contract);
///  - asserts every emitted kind is in the ADR D2 vocabulary and seqs are dense
///    1..=N (D3.4 resume-token integrity).
///
/// Env-var free: the one scene is drafted via the host path
/// (`authoring_save_scene_draft`) so this test does not touch the
/// process-global `SPINDLE_MODEL_CLI_COMMAND` (which parallel CLI-agent tests
/// mutate). Mining and the checkpoint dual-persona review run through the
/// built-in local `review` route (deterministic mock miner), no agent config.
/// The agent-draft emission and the revise arm (findings -> scene_revised ->
/// clean) are covered deterministically by
/// `tools::tests::step_event_trace_covers_verify_revise_mine_sequence`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_journal_full_trace_and_payload_discipline_pin() {
    use crate::run_journal::is_run_event_kind;
    use crate::tools::{ToolRouter, ToolSerializationState};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    // Distinctive sentinel present in prose AND a canonical-fact value; it must
    // never surface in any journal payload (ADR D3.1). `MOCK_CANON_MINE` in the
    // prose triggers the built-in local mock miner to stage a canonical_fact.
    const SENTINEL: &str = "ZZ_PROSE_SENTINEL_QWX";

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("journal.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);

    let project = svc
        .create_project(CreateProjectInput {
            name: "Journal Trace".into(),
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

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    // One general scene.
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
            location_id: Some(loc.location_id.clone()),
            content_rating: Some(ContentRating::General),
            purpose: "establishing".into(),
            research_required: Some(false),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    // Start the run with mining AND in-run verify enabled.
    let start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
        "mining_policy": "propose_all",
        "max_revise_attempts": 1
    });
    let start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    let run_id = start_res.structured_content.unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(run_id.starts_with("authoring_run:"));

    // Background HTTP MCP server for the harness executor (verify/mine ride the
    // running server's tools).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    crate::write_addr_file(&data_dir, addr).unwrap();
    let svc_clone = svc.clone();
    let ct = CancellationToken::new();
    let ct1 = ct.clone();
    let ct2 = ct.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::http::mcp_router(svc_clone, ct1))
            .with_graceful_shutdown(async move { ct2.cancelled_owned().await })
            .await
            .unwrap();
    });

    let exec_args = serde_json::json!({ "project_id": project.project_id, "run_id": run_id });

    // Host-draft scene 1.1 (prose + a canonical fact carry the sentinel;
    // MOCK_CANON_MINE triggers the local mock miner).
    let host_exec = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(host_exec.is_error, Some(false));
    let host_exec_val = host_exec.structured_content.unwrap();
    assert!(
        host_exec_val["message"]
            .as_str()
            .unwrap()
            .contains("Host draft required"),
        "expected host-draft handoff, got {host_exec_val:?}"
    );
    let save_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "book_number": 1,
        "chapter_number": 1,
        "scene_order": 1,
        "full_text": "Mara stood watch. MOCK_CANON_MINE ZZ_PROSE_SENTINEL_QWX marked the gate.",
        "summary": "Mara watch",
        "content_rating": "general",
        "tone": "grim",
        "canonical_facts": [{
            "fact_type": "scene_event",
            "key": "mara_watch",
            "value": "Mara watched. ZZ_PROSE_SENTINEL_QWX",
            "context": "c1s1"
        }],
        "beats": [{ "beat_type": "setup", "summary": "Mara keeps watch." }],
        "continuity_notes": ["No durable canon changes beyond the watch."]
    });
    let saved = router
        .call_tool(
            "authoring_save_scene_draft",
            Some(save_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(saved.is_error, Some(false));

    // Drive execute_next until the run blocks (checkpoint) or completes.
    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 40, "run did not reach checkpoint/complete in time");
        let res = router
            .call_tool(
                "authoring_execute_next",
                Some(exec_args.as_object().unwrap()),
            )
            .await
            .unwrap();
        let val = res.structured_content.unwrap();
        let status = val["status"].as_str().unwrap();
        if status == "blocked" || status == "completed" {
            break;
        }
    }

    // Clear the checkpoint gates, then review.
    let status_args = serde_json::json!({ "project_id": project.project_id, "run_id": run_id });
    let status_val = router
        .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let report_rel = status_val["checkpoint_reports"][0]["report_artifact_path"]
        .as_str()
        .unwrap();
    let report_path = data_dir.join("artifacts").join(report_rel);
    let report_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    let sampled_scene_ids: Vec<String> = report_json["sampled_scene_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();

    let deep_args = serde_json::json!({
        "project_id": project.project_id,
        "scope": {
            "scope_type": "chapter_range",
            "start_book_number": 1, "start_chapter_number": 1,
            "end_book_number": 1, "end_chapter_number": 1
        },
        "checks": [], "severity_filter": [], "deep_check": true, "subjects": []
    });
    let deep = router
        .call_tool("check_consistency", Some(deep_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let audit_args = serde_json::json!({
        "project_id": project.project_id, "run_id": run_id,
        "start_chapter": 1, "end_chapter": 1, "deep_consistency": deep
    });
    router
        .call_tool(
            "authoring_record_checkpoint_audit",
            Some(audit_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    for scene_id in &sampled_scene_ids {
        let review_args = serde_json::json!({ "project_id": project.project_id, "scene_id": scene_id, "rounds": 2 });
        router
            .call_tool(
                "run_dual_persona_review",
                Some(review_args.as_object().unwrap()),
            )
            .await
            .unwrap();
    }
    let review_args = serde_json::json!({
        "project_id": project.project_id, "run_id": run_id,
        "start_chapter": 1, "end_chapter": 1, "directives": ["Keep it dark."]
    });
    router
        .call_tool(
            "authoring_review_checkpoint",
            Some(review_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    // Final execute_next confirms completion.
    let final_res = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(
        final_res.structured_content.unwrap()["next_action"].as_str(),
        Some("complete")
    );

    ct.cancel();
    server.await.unwrap();
    crate::remove_addr_file(&data_dir);

    // -- Assert on the journal --------------------------------------------------
    let events = svc
        .repository()
        .list_run_events(&run_id, None, None)
        .await
        .unwrap();
    assert!(!events.is_empty(), "run must have journalled events");
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();

    // Vocabulary pin (ADR D2): every emitted kind is a known kind.
    for kind in &kinds {
        assert!(
            is_run_event_kind(kind),
            "unknown journal kind emitted: {kind}"
        );
    }
    // Dense seqs 1..=N (ADR D3.4 resume-token integrity).
    let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=events.len() as i64).collect::<Vec<_>>());

    // Expected kinds present, in order, no duplicate step events for the scene.
    let idx = |k: &str| kinds.iter().position(|x| *x == k);
    assert_eq!(kinds.first(), Some(&"run_started"));
    assert_eq!(
        kinds.last(),
        Some(&"run_completed"),
        "run_completed is last"
    );
    for k in [
        "scene_drafted",
        "scene_verify_completed",
        "scene_committed",
        "scene_mined",
        "beats_annotated",
        "chapter_summarized",
        "checkpoint_created",
        "checkpoint_reviewed",
    ] {
        assert!(idx(k).is_some(), "missing expected kind {k}: {kinds:?}");
    }
    // Ordering within the scene lifecycle.
    assert!(idx("scene_drafted") < idx("scene_verify_completed"));
    assert!(idx("scene_verify_completed") < idx("scene_committed"));
    assert!(idx("scene_committed") < idx("scene_mined"));
    assert!(idx("scene_mined") < idx("beats_annotated"));
    assert!(idx("beats_annotated") < idx("chapter_summarized"));
    assert!(idx("chapter_summarized") < idx("checkpoint_created"));
    // No duplicate one-per-scene step events.
    for k in [
        "scene_drafted",
        "scene_committed",
        "scene_mined",
        "beats_annotated",
    ] {
        assert_eq!(
            kinds.iter().filter(|x| **x == k).count(),
            1,
            "step event {k} must emit exactly once for the single scene: {kinds:?}"
        );
    }

    // Payload-discipline pin (ADR D3.1): the sentinel appears in ZERO payloads.
    for event in &events {
        let payload = serde_json::to_string(&event.payload).unwrap();
        assert!(
            !payload.contains(SENTINEL),
            "sentinel leaked into {} payload: {payload}",
            event.kind
        );
        assert!(
            !payload.contains("MOCK_CANON_MINE"),
            "mine evidence leaked into {} payload: {payload}",
            event.kind
        );
    }

    // The mined delta genuinely staged (proving mining ran and the sentinel/
    // evidence it carries was excluded from the journal, not merely never mined).
    let mined = events.iter().find(|e| e.kind == "scene_mined").unwrap();
    assert_eq!(mined.payload["mine_status"], serde_json::json!("staged"));
    assert_eq!(mined.payload["staged_count"], serde_json::json!(1));
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

// ---- T-108: authoring_prepare_run draft-route preflight ----

/// Build a service whose model router is configured from `config_toml`, seed a
/// project with a single explicit-rated planned scene in book 1 / chapter 1,
/// and return the ToolRouter plus project id ready for `authoring_prepare_run`.
async fn preflight_fixture(config_toml: &str) -> (TempDir, crate::tools::ToolRouter, String) {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_core::models::ConfigureAgentsInput;
    use std::sync::Arc;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_preflight.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let config_path = tmp.path().join("spindle.toml");
    std::fs::write(&config_path, config_toml).unwrap();

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);

    let configured = svc
        .configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.to_string_lossy().to_string()),
        })
        .unwrap();
    // Hermeticity: the router loaded the injected fixture, not a home-dir file.
    assert_eq!(
        configured.source_path.as_deref(),
        Some(config_path.to_string_lossy().as_ref())
    );

    let project = svc
        .create_project(CreateProjectInput {
            name: "Preflight Project".into(),
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
            summary: "Mara meets the beast".into(),
            beat_structure: Vec::new(),
            character_ids: vec![mara.character_id.clone()],
            location_id: Some(loc.location_id.clone()),
            content_rating: Some(ContentRating::Explicit),
            purpose: "climax".into(),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );
    let project_id = project.project_id.clone();
    // Keep svc alive for the duration by leaking through the router clone; drop
    // the standalone handle now that router owns its own clone.
    drop(svc);
    (tmp, router, project_id)
}

async fn call_prepare(router: &crate::tools::ToolRouter, project_id: &str) -> serde_json::Value {
    let prep_args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1
    });
    let res = router
        .call_tool(
            "authoring_prepare_run",
            Some(prep_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(res.is_error, Some(false));
    res.structured_content.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_run_blocks_when_draft_route_lacks_explicit_coverage() {
    // Draft agent only covers general/teen; the planned scene is explicit.
    let config = r#"
[health_check]
enabled = false

[[agents]]
id = "tame-draft"
name = "Tame Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general", "teen"]

[[routing]]
route = "draft"
agent = "tame-draft"
"#;
    let (_tmp, router, project_id) = preflight_fixture(config).await;
    let val = call_prepare(&router, &project_id).await;

    assert_eq!(
        val["ready_to_draft"].as_bool(),
        Some(false),
        "explicit scene with no explicit coverage must block: {val:?}"
    );
    let reqs = val["missing_requirements"].as_array().unwrap();
    assert!(
        reqs.iter().any(|r| {
            let s = r.as_str().unwrap();
            s.contains("draft") && s.contains("explicit")
        }),
        "expected a missing_requirements entry naming route draft + rating explicit, got {reqs:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_run_blocks_when_draft_agent_missing_api_key() {
    let env_var = "SPINDLE_T108_PREPARE_MISSING_KEY";
    assert!(
        std::env::var(env_var).is_err(),
        "test precondition: {env_var} must be unset"
    );
    let config = format!(
        r#"
[health_check]
enabled = false

[[agents]]
id = "keyed-draft"
name = "Keyed Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
api_key_env = "{env_var}"
ratings = ["explicit"]

[[routing]]
route = "draft"
agent = "keyed-draft"
"#
    );
    let (_tmp, router, project_id) = preflight_fixture(&config).await;
    let val = call_prepare(&router, &project_id).await;

    assert_eq!(val["ready_to_draft"].as_bool(), Some(false));
    let reqs = val["missing_requirements"].as_array().unwrap();
    let entry = reqs
        .iter()
        .map(|r| r.as_str().unwrap())
        .find(|s| s.contains("keyed-draft"))
        .unwrap_or_else(|| panic!("expected entry naming agent keyed-draft, got {reqs:?}"));
    assert!(
        entry.contains(env_var),
        "entry must name the env var: {entry}"
    );
    // Message must carry only the env var NAME — never any value/key material.
    assert!(
        !entry.contains("secret") && !entry.contains('='),
        "entry must not leak any key material or value: {entry}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_run_passes_when_draft_agent_covers_explicit() {
    // Positive control: explicit-covering, keyless agent — prepare must pass.
    let config = r#"
[health_check]
enabled = false

[[agents]]
id = "bold-draft"
name = "Bold Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "bold-draft"
"#;
    let (_tmp, router, project_id) = preflight_fixture(config).await;
    let val = call_prepare(&router, &project_id).await;
    assert_eq!(
        val["ready_to_draft"].as_bool(),
        Some(true),
        "explicit-covering keyless agent should pass: {val:?}"
    );
}

/// Like [`call_prepare`] but threads a `mining_policy` so the mine-route
/// fallback preflight (evolution §3.1/§4.4) runs.
async fn call_prepare_with_mining(
    router: &crate::tools::ToolRouter,
    project_id: &str,
    mining_policy: &str,
) -> serde_json::Value {
    let prep_args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "mining_policy": mining_policy,
    });
    let res = router
        .call_tool(
            "authoring_prepare_run",
            Some(prep_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(res.is_error, Some(false));
    res.structured_content.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_run_with_propose_all_blocks_when_mine_route_lacks_explicit_coverage() {
    // Test 6: the draft agent covers explicit (draft preflight passes), but the
    // `mine` AND `review` routes are both overridden by external agents that
    // cover only general — so the mine fallback ladder (mine → review) is
    // exhausted for the planned explicit scene. Overriding review is required
    // because the built-in local review route otherwise serves every rating and
    // would clear the ladder. With mining_policy=propose_all prepare must push a
    // canon-mining missing_requirements entry (I8: fail at prepare, not at 2am).
    // The default prepare call (no policy) must still pass.
    let config = r#"
[health_check]
enabled = false

[[agents]]
id = "bold-draft"
name = "Bold Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general", "explicit"]

[[agents]]
id = "tame-mine"
name = "Tame Mine"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general"]

[[routing]]
route = "draft"
agent = "bold-draft"

[[routing]]
route = "mine"
agent = "tame-mine"

[[routing]]
route = "review"
agent = "tame-mine"
"#;
    let (_tmp, router, project_id) = preflight_fixture(config).await;

    // Without a policy, prepare skips the mine check entirely and passes.
    let baseline = call_prepare(&router, &project_id).await;
    assert_eq!(
        baseline["ready_to_draft"].as_bool(),
        Some(true),
        "policy-less prepare must not run the mine preflight: {baseline:?}"
    );

    // With propose_all, the explicit scene has no cleared mine/review route.
    let val = call_prepare_with_mining(&router, &project_id, "propose_all").await;
    assert_eq!(
        val["ready_to_draft"].as_bool(),
        Some(false),
        "propose_all with uncovered explicit mining must block: {val:?}"
    );
    let reqs = val["missing_requirements"].as_array().unwrap();
    assert!(
        reqs.iter().any(|r| {
            let s = r.as_str().unwrap();
            s.contains("Canon mining") && s.contains("explicit")
        }),
        "expected a canon-mining missing_requirements entry naming rating explicit, got {reqs:?}"
    );
}

/// Like [`call_prepare`] but threads a `checkpoint_policy` so the review-route
/// preflight (evolution §3.3 K2) runs.
async fn call_prepare_with_checkpoint_policy(
    router: &crate::tools::ToolRouter,
    project_id: &str,
    checkpoint_policy: &str,
) -> serde_json::Value {
    let prep_args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_policy": checkpoint_policy,
    });
    let res = router
        .call_tool(
            "authoring_prepare_run",
            Some(prep_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(res.is_error, Some(false));
    res.structured_content.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_run_with_auto_advisory_blocks_when_review_route_lacks_explicit_coverage() {
    // K2 test 2: the draft agent covers explicit (draft preflight passes), but
    // the `review` route is overridden by an external agent that covers only
    // general — so an auto_advisory run's checkpoint automation (which runs its
    // sampled dual-persona reviews AND deep pass through `review`) cannot serve
    // the planned explicit scene. prepare must push a missing_requirements entry
    // naming policy + route review + rating explicit (I8: fail at prepare, not
    // at 2am). The policy-less prepare must still pass; auto_strict blocks too.
    let config = r#"
[health_check]
enabled = false

[[agents]]
id = "bold-draft"
name = "Bold Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general", "explicit"]

[[agents]]
id = "tame-review"
name = "Tame Review"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general"]

[[routing]]
route = "draft"
agent = "bold-draft"

[[routing]]
route = "review"
agent = "tame-review"
"#;
    let (_tmp, router, project_id) = preflight_fixture(config).await;

    // Policy-less prepare skips the review preflight and passes.
    let baseline = call_prepare(&router, &project_id).await;
    assert_eq!(
        baseline["ready_to_draft"].as_bool(),
        Some(true),
        "policy-less prepare must not run the review preflight: {baseline:?}"
    );

    // auto_advisory: the explicit scene has no cleared review route → block.
    let val = call_prepare_with_checkpoint_policy(&router, &project_id, "auto_advisory").await;
    assert_eq!(
        val["ready_to_draft"].as_bool(),
        Some(false),
        "auto_advisory with uncovered explicit review must block: {val:?}"
    );
    let reqs = val["missing_requirements"].as_array().unwrap();
    assert!(
        reqs.iter().any(|r| {
            let s = r.as_str().unwrap();
            s.contains("auto_advisory") && s.contains("review") && s.contains("explicit")
        }),
        "expected a missing_requirements entry naming policy auto_advisory + route review + rating explicit, got {reqs:?}"
    );

    // auto_strict is gated identically.
    let strict = call_prepare_with_checkpoint_policy(&router, &project_id, "auto_strict").await;
    assert_eq!(strict["ready_to_draft"].as_bool(), Some(false));
    assert!(
        strict["missing_requirements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().contains("auto_strict")),
        "auto_strict must block with a policy-named entry too: {strict:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepare_run_with_auto_advisory_passes_when_review_covers_explicit() {
    // K2 positive control: a `review` agent covering explicit clears the auto
    // policy precondition. (No `review` override at all also passes, since the
    // built-in local review route serves every rating — asserted implicitly by
    // the blocking test's need to override review to fail.)
    let config = r#"
[health_check]
enabled = false

[[agents]]
id = "bold-draft"
name = "Bold Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general", "explicit"]

[[agents]]
id = "bold-review"
name = "Bold Review"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "model"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "bold-draft"

[[routing]]
route = "review"
agent = "bold-review"
"#;
    let (_tmp, router, project_id) = preflight_fixture(config).await;
    let val = call_prepare_with_checkpoint_policy(&router, &project_id, "auto_advisory").await;
    assert_eq!(
        val["ready_to_draft"].as_bool(),
        Some(true),
        "explicit-covering review agent must clear the auto_advisory precondition: {val:?}"
    );
}

// =============================================================================
// Canon-mining run integration (evolution §3.1 — P1 run integration)
// =============================================================================

/// The single mock CLI draft/review agent script shared by every CLI-agent
/// integration fixture (mining, verify/revise). `SPINDLE_MODEL_CLI_COMMAND` is
/// a PROCESS-global env var, so parallel tests may invoke each other's script;
/// keeping ONE byte-identical body makes that harmless. Argv is `[route, prompt]`.
///
/// Draft branch: emits a valid `GeneratedScenePackage`. The prose carries the
/// `MOCK_CANON_MINE` sentinel (mining tests recognize it; inert everywhere else)
/// and the tone drives `tone_consistency`:
/// - tone starts `"grim"` (outside a declared `tone: solemn` boundary → warning);
/// - on a revision (prompt carries the appended `## Revision directives` block)
///   it switches to `"solemn"` and resolves the finding — UNLESS the prompt
///   names the `STUBBORN_SCENE` synopsis marker, in which case it stays `"grim"`
///   so the convergence guard can park the scene.
///
/// Non-draft (review) branch: a plain strengths/concerns block.
const UNIVERSAL_MOCK_AGENT_SCRIPT: &str = r#"#!/bin/bash
ROUTE=$1
PROMPT=$2
if [ "$ROUTE" = "draft" ]; then
  TONE="grim"
  if echo "$PROMPT" | grep -q "Revision directives"; then
    if ! echo "$PROMPT" | grep -q "STUBBORN_SCENE"; then
      TONE="solemn"
    fi
  fi
  # Copy any reader-sim sentinel the chapter synopsis carries into the prose, so
  # a reader-sim fixture steers each chapter's committed prose purely via data in
  # the DB (this shared script is the single process-stable CLI command).
  READER=$(echo "$PROMPT" | grep -oE 'MOCK_READER_[A-Z_]+' | head -1)
  cat <<EOF
{
  "full_text": "The grey-eyed stranger crossed the hall. MOCK_CANON_MINE $READER as the door swung wide.",
  "summary": "Stranger crosses",
  "tone": "$TONE",
  "character_states": [],
  "canonical_facts": [],
  "relationship_updates": [],
  "beats": [],
  "continuity_notes": []
}
EOF
elif echo "$PROMPT" | grep -q "cumulative reader simulation"; then
  # The reader-sim pass rides the review route (reader_sim falls back to review).
  # MOCK_READER_DIP -> dipping + one warning concern (retread).
  # MOCK_READER_NOTES_ECHO -> steady + notes echoing the first 40 chars of the
  # prior-notes block with a PRIOR: marker, proving memory flow.
  # Otherwise -> high/steady, no concerns.
  if echo "$PROMPT" | grep -q "MOCK_READER_MALFORMED"; then
    # A non-JSON reply the strict parser rejects -> the pass records "unparsed"
    # and preserves prior notes (test-only sentinel, not part of R4's contract).
    echo 'the reader shrugs; no structured verdict today'
  elif echo "$PROMPT" | grep -q "MOCK_READER_DIP"; then
    echo '{"engagement":"dipping","notes":"The reader felt the second market scene retread the first.","concerns":[{"severity":"warning","description":"the second market scene retreads the first"}]}'
  elif echo "$PROMPT" | grep -q "MOCK_READER_NOTES_ECHO"; then
    # Extract the prior-notes block: everything after "YOUR PRIOR NOTES:" up to
    # the blank line, then the first 40 chars.
    ECHO=$(echo "$PROMPT" | sed -n '/YOUR PRIOR NOTES:/,/^$/p' | sed '1d;/^$/d' | head -c 40)
    echo "{\"engagement\":\"steady\",\"notes\":\"The reader stays with the story. PRIOR:${ECHO}\",\"concerns\":[]}"
  else
    echo '{"engagement":"high","notes":"The reader is fully engaged.","concerns":[]}'
  fi
else
  cat <<EOF
STRENGTHS:
- ok
CONCERNS:
- none
EOF
fi
"#;

/// Write the universal mock agent script ONCE to a process-stable path (under
/// the OS temp dir, not any per-test `TempDir`) and return it. Because
/// `SPINDLE_MODEL_CLI_COMMAND` is process-global and per-test `TempDir`s are
/// deleted on drop, a fixture that pointed the env var at a script inside its
/// own `TempDir` would leave a dangling command for any concurrently-running
/// test whose draft fires after that dir is cleaned. A single stable path that
/// lives for the whole test process removes that lifetime race.
fn universal_mock_agent_path() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;
    static PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let path = std::env::temp_dir().join("spindle_universal_mock_agent.sh");
        std::fs::write(&path, UNIVERSAL_MOCK_AGENT_SCRIPT).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    })
    .clone()
}

/// Shared setup for the mining run integration tests. Mirrors the C1 flow's
/// structure: a mock CLI draft/review agent, an HTTP MCP server the harness
/// connects to, a project with one general scene planned, and a started run
/// carrying the given `mining_policy`. Returns everything a test needs to drive
/// `authoring_execute_next` / `authoring_save_scene_draft` / `authoring_status`
/// and read canon deltas, plus the run id and project id.
struct MiningRunFixture {
    _tmp: TempDir,
    svc: SqliteSpindleService,
    router: crate::tools::ToolRouter,
    project_id: String,
    run_id: String,
    ct: tokio_util::sync::CancellationToken,
    server_handle: tokio::task::JoinHandle<()>,
    data_dir: std::path::PathBuf,
}

/// Build a mining-run fixture. `config_extra_routing` is appended to the base
/// config (which wires a general+explicit-covering draft agent and leaves the
/// review/mine routes to the built-in local adapters), letting a test override
/// the `mine` route to force a skip. `mining_policy` is the run's opt-in string
/// (e.g. "propose_all" or "disabled").
async fn mining_run_fixture(
    config_extra_routing: &str,
    mining_policy: Option<&str>,
) -> MiningRunFixture {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_core::models::ConfigureAgentsInput;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_mining.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Mock CLI agent: draft returns prose carrying the MOCK_CANON_MINE sentinel
    // so the committed scene mines into a canonical_fact; review returns a plain
    // strengths/concerns block (unused by mining, present for completeness).
    // Uses the process-stable universal script so the shared global env var is
    // race-safe across parallel mining/verify tests (see helper doc).
    let script_path = universal_mock_agent_path();

    let config_path = tmp.path().join("config.toml");
    // NOTE: the base config configures ONLY the draft route. It deliberately
    // leaves `review` unconfigured so the canon miner's mine→review fallback
    // lands on the built-in local review adapter, which recognizes the
    // MOCK_CANON_MINE sentinel and stages a deterministic delta. A test that
    // wants to force a mining skip overrides the `mine` route via `extra`.
    let config_content = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "cli-agent-draft"
{extra}
"#,
        script = script_path.display(),
        extra = config_extra_routing,
    );
    std::fs::write(&config_path, config_content).unwrap();

    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();

    let project = svc
        .create_project(CreateProjectInput {
            name: "Mining Run".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Mined canon.".into(),
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
            location_id: Some(loc.location_id.clone()),
            content_rating: Some(ContentRating::General),
            purpose: "establishing".into(),
            research_required: Some(false),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    // Start the run with the mining policy.
    let mut start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
    });
    if let Some(policy) = mining_policy {
        start_args["mining_policy"] = serde_json::Value::String(policy.to_string());
    }
    let start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    let start_val = start_res.structured_content.unwrap();
    assert_eq!(
        start_val["status"].as_str(),
        Some("active"),
        "run should start active: {start_val:?}"
    );
    let run_id = start_val["run_id"].as_str().unwrap().to_string();

    // Background HTTP MCP server the harness connects to for the mining step.
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

    MiningRunFixture {
        _tmp: tmp,
        svc,
        router,
        project_id: project.project_id,
        run_id,
        ct,
        server_handle,
        data_dir: data_dir.clone(),
    }
}

/// Host-draft scene 1.1 with `full_text` and advance through its host-draft
/// pause + save, returning the exec/save router. Shared by the mining tests.
async fn host_draft_and_save_scene_1(fx: &MiningRunFixture, full_text: &str) {
    let exec_args = serde_json::json!({ "project_id": fx.project_id, "run_id": fx.run_id });
    // First execute_next pauses for host draft.
    let host_exec = fx
        .router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    let host_exec_val = host_exec.structured_content.unwrap();
    assert!(
        host_exec_val["message"]
            .as_str()
            .unwrap()
            .contains("Host draft required"),
        "expected host-draft pause, got {host_exec_val:?}"
    );

    let save_args = serde_json::json!({
        "project_id": fx.project_id,
        "run_id": fx.run_id,
        "book_number": 1,
        "chapter_number": 1,
        "scene_order": 1,
        "full_text": full_text,
        "summary": "s",
        "content_rating": "general",
        "tone": "grim",
        "continuity_notes": ["No durable canon changes beyond the mined fact."]
    });
    let saved = fx
        .router
        .call_tool(
            "authoring_save_scene_draft",
            Some(save_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(
        saved.structured_content.unwrap()["status"].as_str(),
        Some("saved")
    );
}

async fn execute_next(fx: &MiningRunFixture) -> serde_json::Value {
    let exec_args = serde_json::json!({ "project_id": fx.project_id, "run_id": fx.run_id });
    fx.router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap()
}

async fn status(fx: &MiningRunFixture) -> serde_json::Value {
    let status_args = serde_json::json!({ "project_id": fx.project_id, "run_id": fx.run_id });
    fx.router
        .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap()
}

async fn shutdown(fx: MiningRunFixture) {
    fx.ct.cancel();
    fx.server_handle.await.unwrap();
    crate::remove_addr_file(&fx.data_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mining_run_stages_deltas_between_commit_and_beats() {
    // Test 2: a propose_all run mines the committed scene between commit and
    // beats. execute_next after commit performs mining; status shows
    // mine_status "staged" with a count; deltas exist via list_canon_deltas;
    // then beats proceed.
    let fx = mining_run_fixture("", Some("propose_all")).await;

    // Host-draft the scene carrying the mining sentinel, then commit it.
    host_draft_and_save_scene_1(
        &fx,
        "The grey-eyed stranger crossed the hall. MOCK_CANON_MINE as the door swung wide.",
    )
    .await;
    let commit = execute_next(&fx).await;
    assert!(
        commit["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes"),
        "expected commit, got {commit:?}"
    );

    // Next action must be MineScene (not annotate beats) — the propose_all gate.
    let mine = execute_next(&fx).await;
    assert!(
        mine["executed_action"]
            .as_str()
            .unwrap()
            .contains("mine canon"),
        "expected mine step between commit and beats, got {mine:?}"
    );

    // Status surfaces the honest mine outcome on the scene entry.
    let st = status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert_eq!(
        scene["mine_status"].as_str(),
        Some("staged"),
        "expected mine_status staged, got {st:?}"
    );
    assert!(
        scene["mine_detail"].as_str().unwrap().contains("staged 1"),
        "expected staged count detail, got {scene:?}"
    );

    // Deltas are really staged on the branch.
    let list_args = serde_json::json!({
        "project_id": fx.project_id,
        "status": "staged",
    });
    let list = fx
        .router
        .call_tool("list_canon_deltas", Some(list_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let deltas = list["deltas"].as_array().unwrap();
    assert_eq!(deltas.len(), 1, "one staged delta expected: {list:?}");
    assert_eq!(deltas[0]["delta_class"].as_str(), Some("canonical_fact"));

    // Beats proceed next (mining fired once, mine_status is now Some).
    let beats = execute_next(&fx).await;
    assert!(
        beats["executed_action"]
            .as_str()
            .unwrap()
            .contains("annotate beats"),
        "expected annotate beats after mining, got {beats:?}"
    );

    shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mining_run_skips_honestly_when_route_uncleared_and_advances() {
    // Test 3: the `mine` route is overridden by an agent that does NOT cover
    // the scene's `general` rating, so mining skips honestly (mine_status
    // "skipped", detail naming the rating). The run still advances to beats —
    // mining never blocks (I8).
    let extra = r#"
[[agents]]
id = "tame-mine"
name = "Tame Mine"
provider = "cli"
endpoint = "/bin/true"
model = "default"
ratings = ["teen"]

[[routing]]
route = "mine"
agent = "tame-mine"
"#;
    let fx = mining_run_fixture(extra, Some("propose_all")).await;

    host_draft_and_save_scene_1(&fx, "A quiet watch. No sentinel here, just prose.").await;
    let commit = execute_next(&fx).await;
    assert!(
        commit["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes")
    );

    // Mining runs and skips (uncleared mine route for general), advancing anyway.
    let mine = execute_next(&fx).await;
    assert!(
        mine["executed_action"]
            .as_str()
            .unwrap()
            .contains("mine canon"),
        "expected mine step, got {mine:?}"
    );
    assert_ne!(
        mine["status"].as_str(),
        Some("blocked"),
        "mining must never block the run: {mine:?}"
    );

    let st = status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert_eq!(scene["mine_status"].as_str(), Some("skipped"), "got {st:?}");
    assert!(
        scene["mine_detail"].as_str().unwrap().contains("general"),
        "skip detail must name the uncleared rating, got {scene:?}"
    );

    // No deltas were staged.
    let list_args = serde_json::json!({ "project_id": fx.project_id, "status": "staged" });
    let list = fx
        .router
        .call_tool("list_canon_deltas", Some(list_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        list["deltas"].as_array().unwrap().is_empty(),
        "no deltas: {list:?}"
    );

    // Run advances to beats despite the skip.
    let beats = execute_next(&fx).await;
    assert!(
        beats["executed_action"]
            .as_str()
            .unwrap()
            .contains("annotate beats"),
        "run must advance to beats after a skip, got {beats:?}"
    );

    shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn policy_less_run_never_yields_mine_scene() {
    // Test 4 (plan-level companion): a run started with NO mining_policy goes
    // straight from commit to beats — MineScene is never scheduled.
    let fx = mining_run_fixture("", None).await;

    host_draft_and_save_scene_1(&fx, "A watch with MOCK_CANON_MINE present but no policy.").await;
    let commit = execute_next(&fx).await;
    assert!(
        commit["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes")
    );

    // With no policy the very next action is annotate beats — never mine.
    let next = execute_next(&fx).await;
    assert!(
        next["executed_action"]
            .as_str()
            .unwrap()
            .contains("annotate beats"),
        "policy-less run must skip mining entirely, got {next:?}"
    );
    assert!(
        !next["executed_action"]
            .as_str()
            .unwrap()
            .contains("mine canon"),
        "policy-less run must never mine, got {next:?}"
    );

    // The scene's mine_status stays absent (mining not attempted).
    let st = status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert!(scene.get("mine_status").is_none() || scene["mine_status"].is_null());

    shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_upgrade_run_row_opens_and_resumes_with_null_policy_disabled() {
    // Test 5: a run row written WITHOUT a mining_policy (NULL = pre-upgrade /
    // V0024-era) opens fine and resumes. NULL policy = disabled: after commit
    // the loop goes straight to beats, no MineScene. This proves the additive
    // columns default cleanly for existing rows.
    let fx = mining_run_fixture("", None).await;

    // Confirm the persisted run row carries a NULL mining_policy (disabled).
    let (run, _, _, _) = fx
        .svc
        .repository()
        .get_authoring_run(&fx.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run.mining_policy, None,
        "pre-upgrade/default run must persist NULL policy"
    );
    // P2.2: the V0026 verify/revise columns also default cleanly for a run that
    // predates them — NULL max_revise_attempts = disabled (the same additive
    // guarantee C9 pins). A V0025-era row resumes with the revise loop off.
    assert_eq!(
        run.max_revise_attempts, None,
        "pre-upgrade/default run must persist NULL max_revise_attempts (revise disabled)"
    );

    // Resume: host-draft, commit, then the next action is beats (disabled).
    host_draft_and_save_scene_1(&fx, "A resumed watch, no mining.").await;
    let commit = execute_next(&fx).await;
    assert!(
        commit["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes")
    );
    let next = execute_next(&fx).await;
    assert!(
        next["executed_action"]
            .as_str()
            .unwrap()
            .contains("annotate beats"),
        "NULL-policy (disabled) run must go commit -> beats, got {next:?}"
    );

    shutdown(fx).await;
}

// =============================================================================
// Living-outline replan integration (ADR 0003, evolution §3.5 — P4)
// =============================================================================

/// Shared setup for the replan run integration tests. Mirrors the mining
/// fixture, but plans TWO chapters (1 = the run's range, 2 = a not-yet-drafted
/// FUTURE target the replan differ audits) and runs the range 1..=1 so chapter 2
/// stays undrafted. Chapter 2's synopsis carries the `MOCK_REPLAN_SYNOPSIS[2]`
/// sentinel so the built-in local review adapter (the replan→review fallback
/// lands there — the differ is non-prose-bearing) stages a synopsis_update
/// amendment against it. `review_config_extra` is appended to the base config so
/// a test can override the `review` route to force a skip. `replan_policy` is the
/// run's opt-in string.
struct ReplanRunFixture {
    _tmp: TempDir,
    svc: SqliteSpindleService,
    router: crate::tools::ToolRouter,
    project_id: String,
    run_id: String,
    ct: tokio_util::sync::CancellationToken,
    server_handle: tokio::task::JoinHandle<()>,
    data_dir: std::path::PathBuf,
}

async fn replan_run_fixture(
    review_config_extra: &str,
    replan_policy: Option<&str>,
) -> ReplanRunFixture {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_core::models::ConfigureAgentsInput;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_replan.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let script_path = universal_mock_agent_path();
    let config_path = tmp.path().join("config.toml");
    // Base config wires ONLY the draft route (general + explicit). The replan
    // differ has no `replan` route, so it falls to `review`, which is left to the
    // built-in local adapter that recognizes MOCK_REPLAN_SYNOPSIS. A test that
    // wants to force a replan skip overrides the `review` route via `extra`.
    let config_content = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "cli-agent-draft"
{extra}
"#,
        script = script_path.display(),
        extra = review_config_extra,
    );
    std::fs::write(&config_path, config_content).unwrap();
    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();

    let project = svc
        .create_project(CreateProjectInput {
            name: "Replan Run".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Living outline.".into(),
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

    // Chapter 1 — the run's range (will be drafted).
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
            location_id: Some(loc.location_id.clone()),
            content_rating: Some(ContentRating::General),
            purpose: "establishing".into(),
            research_required: Some(false),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    // Chapter 2 — the FUTURE, never-drafted target the replan differ audits. Its
    // synopsis carries the sentinel so the local review adapter stages an
    // amendment against it.
    svc.plan_chapter(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 2,
        pov_character_id: Some(mara.character_id.clone()),
        synopsis: "MOCK_REPLAN_SYNOPSIS[2] second chapter as planned".into(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "the planned future opening".into(),
            beat_structure: Vec::new(),
            character_ids: vec![mara.character_id.clone()],
            location_id: Some(loc.location_id.clone()),
            content_rating: Some(ContentRating::General),
            purpose: "future".into(),
            research_required: Some(false),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    // Start the run over chapter 1 ONLY (checkpoint_interval 1), with the policy.
    let mut start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
    });
    if let Some(policy) = replan_policy {
        start_args["replan_policy"] = serde_json::Value::String(policy.to_string());
    }
    let start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    let start_val = start_res.structured_content.unwrap();
    assert_eq!(
        start_val["status"].as_str(),
        Some("active"),
        "run should start active: {start_val:?}"
    );
    let run_id = start_val["run_id"].as_str().unwrap().to_string();

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

    ReplanRunFixture {
        _tmp: tmp,
        svc,
        router,
        project_id: project.project_id,
        run_id,
        ct,
        server_handle,
        data_dir: data_dir.clone(),
    }
}

impl ReplanRunFixture {
    async fn execute_next(&self) -> serde_json::Value {
        let exec_args = serde_json::json!({ "project_id": self.project_id, "run_id": self.run_id });
        self.router
            .call_tool(
                "authoring_execute_next",
                Some(exec_args.as_object().unwrap()),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap()
    }

    async fn status(&self) -> serde_json::Value {
        let status_args =
            serde_json::json!({ "project_id": self.project_id, "run_id": self.run_id });
        self.router
            .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
            .await
            .unwrap()
            .structured_content
            .unwrap()
    }

    /// Host-draft chapter 1's single scene and advance through its host-draft
    /// pause + save.
    async fn host_draft_and_save_scene_1(&self) {
        let exec_args = serde_json::json!({ "project_id": self.project_id, "run_id": self.run_id });
        let host_exec = self
            .router
            .call_tool(
                "authoring_execute_next",
                Some(exec_args.as_object().unwrap()),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert!(
            host_exec["message"]
                .as_str()
                .unwrap()
                .contains("Host draft required"),
            "expected host-draft pause, got {host_exec:?}"
        );
        let save_args = serde_json::json!({
            "project_id": self.project_id,
            "run_id": self.run_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": "The watch held through the night. Mara kept the gate.",
            "summary": "s",
            "content_rating": "general",
            "tone": "grim",
            "continuity_notes": ["No durable canon changes."]
        });
        let saved = self
            .router
            .call_tool(
                "authoring_save_scene_draft",
                Some(save_args.as_object().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(
            saved.structured_content.unwrap()["status"].as_str(),
            Some("saved")
        );
    }

    /// Drive execute_next until the given predicate matches the executed action,
    /// returning that response. Caps iterations so a stuck loop fails the test.
    async fn drive_until(&self, needle: &str) -> serde_json::Value {
        for _ in 0..12 {
            let out = self.execute_next().await;
            let action = out["executed_action"].as_str().unwrap_or("");
            if action.contains(needle) {
                return out;
            }
        }
        panic!("never reached an executed action containing {needle:?}");
    }

    async fn run_events(&self) -> Vec<spindle_adapters::sqlite::records::StoredRunEvent> {
        self.svc
            .repository()
            .list_run_events(&self.run_id, None, None)
            .await
            .unwrap()
    }

    async fn shutdown(self) {
        self.ct.cancel();
        self.server_handle.await.unwrap();
        crate::remove_addr_file(&self.data_dir);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replan_run_stages_amendments_after_summary_before_checkpoint() {
    // A propose_all run: after chapter 1's summary is saved, ReplanChapter fires
    // BEFORE the checkpoint, stages an amendment against the future chapter 2,
    // surfaces replan_status=staged with a count on the chapter, emits a
    // replan_proposed journal event, and the checkpoint is still reached.
    let fx = replan_run_fixture("", Some("propose_all")).await;

    fx.host_draft_and_save_scene_1().await;
    // Drive commit → beats → summary. (No mining policy, so no mine step.)
    fx.drive_until("commit scene changes").await;
    fx.drive_until("annotate beats").await;
    fx.drive_until("save summary").await;

    // The very next executed action must be the replan pass (before checkpoint).
    let replan = fx.execute_next().await;
    assert!(
        replan["executed_action"]
            .as_str()
            .unwrap()
            .contains("replan future plans"),
        "expected replan step right after summary, got {replan:?}"
    );

    // Status surfaces the honest replan outcome on the chapter entry.
    let st = fx.status().await;
    let ch1 = &st["chapters"][0];
    assert_eq!(
        ch1["replan_status"].as_str(),
        Some("staged"),
        "expected replan_status staged, got {st:?}"
    );
    // The MOCK_REPLAN_SYNOPSIS[2] sentinel stages a synopsis_update +
    // thread_retire against chapter 2 = 2 amendments.
    assert!(
        ch1["replan_detail"].as_str().unwrap().contains("staged 2"),
        "expected staged count detail, got {ch1:?}"
    );

    // Amendments are really staged on the branch against chapter 2.
    let list_args = serde_json::json!({ "project_id": fx.project_id, "status": "staged" });
    let list = fx
        .router
        .call_tool("list_plan_amendments", Some(list_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let amendments = list["amendments"].as_array().unwrap();
    assert_eq!(
        amendments.len(),
        2,
        "two staged amendments expected: {list:?}"
    );
    assert!(
        amendments
            .iter()
            .all(|a| a["target_chapter"].as_i64() == Some(2)),
        "all amendments target the future chapter 2: {list:?}"
    );

    // The replan_proposed journal event is present (chapter + count).
    let events = fx.run_events().await;
    let proposed = events
        .iter()
        .find(|e| e.kind == "replan_proposed")
        .expect("replan_proposed emitted");
    assert_eq!(proposed.payload["chapter"], serde_json::json!(1));
    assert_eq!(proposed.payload["amendment_count"], serde_json::json!(2));

    // The checkpoint is still reached (replan ran once, does not re-fire).
    let checkpoint = fx.drive_until("run checkpoint").await;
    assert!(
        checkpoint["executed_action"]
            .as_str()
            .unwrap()
            .contains("run checkpoint"),
        "checkpoint must still be reached after replan, got {checkpoint:?}"
    );

    fx.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replan_run_skips_honestly_when_review_route_dead_and_advances() {
    // The `review` route (the replan→review fallback) points at a dead endpoint,
    // so the replan pass skips honestly: replan_status skipped, a pass_skipped
    // journal event fires, and the run still advances to the checkpoint (replan
    // never blocks — evolution I8).
    let extra = r#"
[[agents]]
id = "review-dead"
name = "Review Dead"
provider = "openai_compatible"
endpoint = "http://127.0.0.1:1/unreachable"
model = "dead"
ratings = ["general", "teen", "mature", "explicit"]

[[routing]]
route = "review"
agent = "review-dead"
"#;
    let fx = replan_run_fixture(extra, Some("propose_all")).await;

    fx.host_draft_and_save_scene_1().await;
    fx.drive_until("commit scene changes").await;
    fx.drive_until("annotate beats").await;
    fx.drive_until("save summary").await;

    let replan = fx.execute_next().await;
    assert!(
        replan["executed_action"]
            .as_str()
            .unwrap()
            .contains("replan future plans"),
        "expected replan step, got {replan:?}"
    );
    assert_ne!(
        replan["status"].as_str(),
        Some("blocked"),
        "replan must never block the run: {replan:?}"
    );

    let st = fx.status().await;
    let ch1 = &st["chapters"][0];
    assert_eq!(
        ch1["replan_status"].as_str(),
        Some("skipped"),
        "dead review route → skipped, got {st:?}"
    );

    // No amendments were staged.
    let list_args = serde_json::json!({ "project_id": fx.project_id, "status": "staged" });
    let list = fx
        .router
        .call_tool("list_plan_amendments", Some(list_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        list["amendments"].as_array().unwrap().is_empty(),
        "no amendments: {list:?}"
    );

    // A pass_skipped(replan) journal event fired; no replan_proposed.
    let events = fx.run_events().await;
    assert!(
        events
            .iter()
            .any(|e| e.kind == "pass_skipped" && e.payload["pass"] == serde_json::json!("replan")),
        "pass_skipped(replan) expected: {:?}",
        events.iter().map(|e| e.kind.clone()).collect::<Vec<_>>()
    );
    assert!(
        !events.iter().any(|e| e.kind == "replan_proposed"),
        "no replan_proposed on a skip"
    );

    // The run still advances to the checkpoint.
    let checkpoint = fx.drive_until("run checkpoint").await;
    assert!(
        checkpoint["executed_action"]
            .as_str()
            .unwrap()
            .contains("run checkpoint"),
        "run must advance to checkpoint after a replan skip, got {checkpoint:?}"
    );

    fx.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn policy_less_run_never_yields_replan_chapter() {
    // A run started with NO replan_policy goes straight from summary to the
    // checkpoint — ReplanChapter is never scheduled, replan_status stays absent.
    let fx = replan_run_fixture("", None).await;

    // The persisted run row carries a NULL replan_policy (disabled).
    let (run, _, _, _) = fx
        .svc
        .repository()
        .get_authoring_run(&fx.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run.replan_policy, None,
        "policy-less run must persist NULL replan_policy (disabled)"
    );

    fx.host_draft_and_save_scene_1().await;
    fx.drive_until("commit scene changes").await;
    fx.drive_until("annotate beats").await;
    fx.drive_until("save summary").await;

    // With no replan policy the very next action is the checkpoint — never replan.
    let next = fx.execute_next().await;
    assert!(
        next["executed_action"]
            .as_str()
            .unwrap()
            .contains("run checkpoint"),
        "policy-less run must go summary -> checkpoint, got {next:?}"
    );
    assert!(
        !next["executed_action"].as_str().unwrap().contains("replan"),
        "policy-less run must never replan, got {next:?}"
    );

    // The chapter's replan_status stays absent (replan not attempted).
    let st = fx.status().await;
    let ch1 = &st["chapters"][0];
    assert!(ch1.get("replan_status").is_none() || ch1["replan_status"].is_null());
    // No replan_proposed journal event.
    let events = fx.run_events().await;
    assert!(!events.iter().any(|e| e.kind == "replan_proposed"));

    fx.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_upgrade_run_row_resumes_with_null_replan_policy_disabled() {
    // A run row written WITHOUT a replan_policy (NULL = pre-V0030) opens and
    // resumes fine. NULL = disabled: after the summary the loop goes straight to
    // the checkpoint, no ReplanChapter. Proves the additive column defaults
    // cleanly for existing rows.
    let fx = replan_run_fixture("", None).await;
    let (run, _, _, _) = fx
        .svc
        .repository()
        .get_authoring_run(&fx.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.replan_policy, None);

    fx.host_draft_and_save_scene_1().await;
    fx.drive_until("commit scene changes").await;
    fx.drive_until("annotate beats").await;
    fx.drive_until("save summary").await;
    let next = fx.execute_next().await;
    assert!(
        next["executed_action"]
            .as_str()
            .unwrap()
            .contains("run checkpoint"),
        "NULL-policy (disabled) run must go summary -> checkpoint, got {next:?}"
    );

    fx.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replan_run_rejects_bad_policy_string() {
    // A bad replan_policy string is an input error at authoring_start_run — the
    // run is blocked, not started (mirrors mining_policy validation).
    use crate::tools::{ToolRouter, ToolSerializationState};
    use std::sync::Arc;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_replan_bad.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    let project = svc
        .create_project(CreateProjectInput {
            name: "Replan Bad".into(),
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
    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );
    let start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
        "replan_policy": "auto_accept",
    });
    let res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(res["status"].as_str(), Some("blocked"), "{res:?}");
    assert!(
        res["message"].as_str().unwrap().contains("replan_policy"),
        "message names the bad policy: {res:?}"
    );
}

// =============================================================================
// In-run verify/revise integration (evolution §3.2 — P2.2)
// =============================================================================

/// Shared setup for the verify/revise run integration tests. Mirrors the mining
/// fixture: a CLI draft agent, an HTTP MCP server the harness connects to, a
/// project whose reader contract declares a `tone: solemn` boundary, and one
/// general scene planned. The run is started in AGENT mode with the given
/// `max_revise_attempts`, so drafting/verify/revise are all executed by the
/// harness through `authoring_execute_next`.
///
/// The deterministic violation is `tone_consistency`: the draft agent's scene
/// carries a tone outside the declared `tone: solemn` boundary, so a
/// scene-scoped `check_consistency` returns a `warning` attributed to the scene
/// with zero model calls. When `revision_fixes` is true the agent switches its
/// tone to `solemn` the moment the prompt carries a `## Revision directives`
/// block (detectable in argv), so the revision resolves the finding; when false
/// it never changes, so the convergence guard parks the scene.
struct VerifyRunFixture {
    _tmp: TempDir,
    router: crate::tools::ToolRouter,
    project_id: String,
    run_id: String,
    ct: tokio_util::sync::CancellationToken,
    server_handle: tokio::task::JoinHandle<()>,
    data_dir: std::path::PathBuf,
}

async fn verify_run_fixture(
    revision_fixes: bool,
    max_revise_attempts: Option<i32>,
) -> VerifyRunFixture {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_core::models::ConfigureAgentsInput;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_verify.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Universal mock draft agent at a process-stable path (see helper doc). The
    // synopsis (STUBBORN_SCENE marker) is the per-test signal it keys on, folded
    // into the draft prompt.
    let script_path = universal_mock_agent_path();

    let config_path = tmp.path().join("config.toml");
    let config_content = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "cli-agent-draft"
"#,
        script = script_path.display(),
    );
    std::fs::write(&config_path, config_content).unwrap();

    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();

    let project = svc
        .create_project(CreateProjectInput {
            name: "Verify Run".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Verified prose.".into(),
                style_notes: Vec::new(),
                // Declares the allowed tone so a "grim" scene trips
                // tone_consistency (the boundary contains "tone:" but not
                // the scene's tone value).
                boundaries: vec!["tone: solemn".into()],
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
                tone: Some("solemn".into()),
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

    // The synopsis is the per-test signal the universal draft script keys on
    // (it is folded into the draft prompt). When the revision must NOT resolve
    // the finding, the STUBBORN_SCENE marker keeps the re-drafted tone "grim".
    let synopsis = if revision_fixes {
        "First watch."
    } else {
        "First watch. STUBBORN_SCENE"
    };
    svc.plan_chapter(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(mara.character_id.clone()),
        synopsis: synopsis.into(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: Vec::new(),
            character_ids: vec![mara.character_id.clone()],
            location_id: Some(loc.location_id.clone()),
            content_rating: Some(ContentRating::General),
            purpose: "establishing".into(),
            research_required: Some(false),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    let mut start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
    });
    if let Some(budget) = max_revise_attempts {
        start_args["max_revise_attempts"] = serde_json::Value::from(budget);
    }
    let start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    let start_val = start_res.structured_content.unwrap();
    assert_eq!(
        start_val["status"].as_str(),
        Some("active"),
        "run should start active: {start_val:?}"
    );
    let run_id = start_val["run_id"].as_str().unwrap().to_string();

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

    VerifyRunFixture {
        _tmp: tmp,
        router,
        project_id: project.project_id,
        run_id,
        ct,
        server_handle,
        data_dir: data_dir.clone(),
    }
}

/// Drive `authoring_execute_next` in AGENT mode for a verify fixture.
async fn verify_execute_next(fx: &VerifyRunFixture) -> serde_json::Value {
    let exec_args = serde_json::json!({
        "project_id": fx.project_id,
        "run_id": fx.run_id,
        "mode": "agent",
    });
    fx.router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap()
}

async fn verify_status(fx: &VerifyRunFixture) -> serde_json::Value {
    let status_args = serde_json::json!({ "project_id": fx.project_id, "run_id": fx.run_id });
    fx.router
        .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap()
}

async fn verify_shutdown(fx: VerifyRunFixture) {
    fx.ct.cancel();
    fx.server_handle.await.unwrap();
    crate::remove_addr_file(&fx.data_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_run_revises_findings_then_commits_clean() {
    // Test 2: agent-mode run with max_revise_attempts=1. The first draft trips a
    // tone_consistency warning; the run verifies (findings), revises (the agent
    // fixes the tone), re-verifies (clean), then commits. revise_attempts == 1.
    let fx = verify_run_fixture(true, Some(1)).await;

    // Draft.
    let draft = verify_execute_next(&fx).await;
    assert!(
        draft["executed_action"]
            .as_str()
            .unwrap()
            .contains("draft book scene"),
        "expected draft, got {draft:?}"
    );

    // Verify — first pass finds the tone warning.
    let verify1 = verify_execute_next(&fx).await;
    assert!(
        verify1["executed_action"]
            .as_str()
            .unwrap()
            .contains("verify scene"),
        "expected verify after draft, got {verify1:?}"
    );
    let st = verify_status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert_eq!(
        scene["verify_status"].as_str(),
        Some("findings"),
        "first verify must find the tone warning, got {scene:?}"
    );

    // Revise — the agent re-drafts with the fixed tone.
    let revise = verify_execute_next(&fx).await;
    assert!(
        revise["executed_action"]
            .as_str()
            .unwrap()
            .contains("revise scene"),
        "expected revise after findings, got {revise:?}"
    );

    // Verify again — now clean.
    let verify2 = verify_execute_next(&fx).await;
    assert!(
        verify2["executed_action"]
            .as_str()
            .unwrap()
            .contains("verify scene"),
        "expected a second verify after revise, got {verify2:?}"
    );
    let st = verify_status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert_eq!(
        scene["verify_status"].as_str(),
        Some("clean"),
        "re-verify after a fixing revision must be clean, got {scene:?}"
    );
    assert_eq!(
        scene["revise_attempts"].as_i64(),
        Some(1),
        "exactly one revision consumed, got {scene:?}"
    );

    // Commit proceeds.
    let commit = verify_execute_next(&fx).await;
    assert!(
        commit["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes"),
        "expected commit after a clean re-verify, got {commit:?}"
    );

    verify_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_run_convergence_guard_parks_unchanged_findings() {
    // Test 3: the revision does NOT fix the violation (tone stays "grim"). The
    // first verify finds it; a revision runs; the second verify computes the
    // SAME fingerprint → parked_findings "unchanged after revision". The run
    // proceeds to commit; revise_attempts == 1 (never re-revised for the same
    // findings), even though the budget was 2.
    let fx = verify_run_fixture(false, Some(2)).await;

    verify_execute_next(&fx).await; // draft
    let verify1 = verify_execute_next(&fx).await;
    assert!(
        verify1["executed_action"]
            .as_str()
            .unwrap()
            .contains("verify scene")
    );
    let st = verify_status(&fx).await;
    assert_eq!(
        st["chapters"][0]["scenes"][0]["verify_status"].as_str(),
        Some("findings")
    );

    let revise = verify_execute_next(&fx).await;
    assert!(
        revise["executed_action"]
            .as_str()
            .unwrap()
            .contains("revise scene"),
        "expected a revise pass, got {revise:?}"
    );

    // Second verify: fingerprint unchanged → parked, not another finding.
    let verify2 = verify_execute_next(&fx).await;
    assert!(
        verify2["executed_action"]
            .as_str()
            .unwrap()
            .contains("verify scene")
    );
    let st = verify_status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert_eq!(
        scene["verify_status"].as_str(),
        Some("parked_findings"),
        "unchanged findings must park, got {scene:?}"
    );
    assert!(
        scene["verify_detail"]
            .as_str()
            .unwrap()
            .contains("unchanged after revision"),
        "parked detail must name the convergence guard, got {scene:?}"
    );
    assert_eq!(
        scene["revise_attempts"].as_i64(),
        Some(1),
        "the same findings are never revised twice, got {scene:?}"
    );

    // The run proceeds to commit despite the parked findings.
    let commit = verify_execute_next(&fx).await;
    assert!(
        commit["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes"),
        "parked scene must proceed to commit, got {commit:?}"
    );

    verify_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_run_disabled_goes_draft_to_commit() {
    // Test 4 (integration companion to plan-level opt-out): a run started with
    // NO max_revise_attempts goes straight from draft to commit — VerifyScene is
    // never scheduled even though the draft trips the tone violation.
    let fx = verify_run_fixture(false, None).await;

    let draft = verify_execute_next(&fx).await;
    assert!(
        draft["executed_action"]
            .as_str()
            .unwrap()
            .contains("draft book scene")
    );
    let next = verify_execute_next(&fx).await;
    assert!(
        next["executed_action"]
            .as_str()
            .unwrap()
            .contains("commit scene changes"),
        "disabled run must go draft -> commit, got {next:?}"
    );
    assert!(
        !next["executed_action"]
            .as_str()
            .unwrap()
            .contains("verify scene"),
        "disabled run must never verify, got {next:?}"
    );
    let st = verify_status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert!(scene.get("verify_status").is_none() || scene["verify_status"].is_null());

    verify_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_run_rejects_out_of_bounds_max_revise_attempts() {
    // Test 7: max_revise_attempts=3 is above the 0..=2 bound → input error at
    // start (blocked, no run created).
    let fx = verify_run_fixture(false, None).await;
    let start_args = serde_json::json!({
        "project_id": fx.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
        "max_revise_attempts": 3,
    });
    let res = fx
        .router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        res["status"].as_str(),
        Some("blocked"),
        "out-of-bounds budget must block, got {res:?}"
    );
    assert!(
        res["message"]
            .as_str()
            .unwrap()
            .contains("max_revise_attempts"),
        "message must name the offending input, got {res:?}"
    );
    assert!(res["run_id"].as_str().unwrap_or("").is_empty());

    verify_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_run_rejects_unknown_checkpoint_policy() {
    // K1 test 1: an unknown checkpoint_policy string is an input error at start
    // (blocked, no run created); the message names the offending input.
    let fx = verify_run_fixture(false, None).await;
    let start_args = serde_json::json!({
        "project_id": fx.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
        "checkpoint_policy": "auto",
    });
    let res = fx
        .router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        res["status"].as_str(),
        Some("blocked"),
        "an unknown checkpoint_policy must block, got {res:?}"
    );
    assert!(
        res["message"]
            .as_str()
            .unwrap()
            .contains("checkpoint_policy"),
        "message must name the offending input, got {res:?}"
    );
    assert!(res["run_id"].as_str().unwrap_or("").is_empty());

    verify_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_run_manual_checkpoint_policy_persists_null_byte_identical() {
    // K1 test 1 (continued): an explicit "manual" checkpoint_policy canonicalizes
    // to NULL so the persisted run row is byte-identical to a pre-policy run
    // (manual/NULL is the default — I1). The run starts active and its persisted
    // checkpoint_policy is None.
    let fx = verify_run_fixture(false, None).await;
    for policy in ["manual", "  Manual "] {
        let start_args = serde_json::json!({
            "project_id": fx.project_id,
            "book_number": 1,
            "start_chapter": 1,
            "end_chapter": 1,
            "checkpoint_interval": 1,
            "checkpoint_policy": policy,
        });
        let res = fx
            .router
            .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(
            res["status"].as_str(),
            Some("active"),
            "manual policy must start active: {res:?}"
        );
        let run_id = res["run_id"].as_str().unwrap().to_string();
        let (run, _, _, _) = fx
            .router
            .service()
            .repository()
            .get_authoring_run(&run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            run.checkpoint_policy, None,
            "manual/{policy:?} must persist a NULL checkpoint_policy (byte-identical to today)"
        );
    }

    verify_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hybrid_run_surfaces_findings_and_host_resave_resets_verify_state() {
    // Test 6: an enabled run in HYBRID mode. The host drafts a general scene
    // whose tone trips the tone violation. execute_next verifies (findings),
    // then the next execute_next is intercepted as a host-revision hand-off
    // whose message lists the findings. A host re-save resets verify_status and
    // increments revise_attempts; a re-save with a fixed tone re-verifies clean.
    let fx = verify_run_fixture(true, Some(1)).await;
    let exec_args = serde_json::json!({ "project_id": fx.project_id, "run_id": fx.run_id });

    // Hybrid mode (default): first execute_next asks the host to draft.
    let host_exec = fx
        .router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        host_exec["message"]
            .as_str()
            .unwrap()
            .contains("Host draft required"),
        "expected host-draft pause, got {host_exec:?}"
    );

    // Host saves a draft with the violating "grim" tone.
    let save_grim = serde_json::json!({
        "project_id": fx.project_id,
        "run_id": fx.run_id,
        "book_number": 1,
        "chapter_number": 1,
        "scene_order": 1,
        "full_text": "The warden kept a grim vigil.",
        "summary": "s",
        "content_rating": "general",
        "tone": "grim",
        "continuity_notes": ["No durable canon."]
    });
    fx.router
        .call_tool(
            "authoring_save_scene_draft",
            Some(save_grim.as_object().unwrap()),
        )
        .await
        .unwrap();

    // Verify runs host-independently and finds the tone warning.
    let verify1 = fx
        .router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        verify1["executed_action"]
            .as_str()
            .unwrap()
            .contains("verify scene"),
        "hybrid verify must still run, got {verify1:?}"
    );
    let st = verify_status(&fx).await;
    assert_eq!(
        st["chapters"][0]["scenes"][0]["verify_status"].as_str(),
        Some("findings")
    );

    // Next execute_next is the host-revision hand-off listing the findings.
    let handoff = fx
        .router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        handoff["message"]
            .as_str()
            .unwrap()
            .contains("Host revision required"),
        "expected host-revision hand-off, got {handoff:?}"
    );
    assert!(
        handoff["message"]
            .as_str()
            .unwrap()
            .contains("tone_consistency"),
        "hand-off must list the findings, got {handoff:?}"
    );

    // Host re-saves with a fixed "solemn" tone: resets verify_status + counts.
    let save_fixed = serde_json::json!({
        "project_id": fx.project_id,
        "run_id": fx.run_id,
        "book_number": 1,
        "chapter_number": 1,
        "scene_order": 1,
        "full_text": "The warden kept a solemn vigil.",
        "summary": "s",
        "content_rating": "general",
        "tone": "solemn",
        "continuity_notes": ["No durable canon."]
    });
    fx.router
        .call_tool(
            "authoring_save_scene_draft",
            Some(save_fixed.as_object().unwrap()),
        )
        .await
        .unwrap();

    let st = verify_status(&fx).await;
    let scene = &st["chapters"][0]["scenes"][0];
    assert!(
        scene.get("verify_status").is_none() || scene["verify_status"].is_null(),
        "re-save must reset verify_status to None, got {scene:?}"
    );
    assert_eq!(
        scene["revise_attempts"].as_i64(),
        Some(1),
        "host re-save after findings must count the attempt, got {scene:?}"
    );

    // Re-verify: the fixed tone is clean.
    let verify2 = fx
        .router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert!(
        verify2["executed_action"]
            .as_str()
            .unwrap()
            .contains("verify scene")
    );
    let st = verify_status(&fx).await;
    assert_eq!(
        st["chapters"][0]["scenes"][0]["verify_status"].as_str(),
        Some("clean"),
        "re-verify of the fixed draft must be clean, got {st:?}"
    );

    verify_shutdown(fx).await;
}

// =============================================================================
// Auto-checkpoint policy integration (evolution §3.3 — P3.1/P3.2)
// =============================================================================

/// Shared setup for the auto-checkpoint policy tests. Mirrors the verify
/// fixture (CLI draft agent, `tone: solemn` boundary so a `grim` draft trips a
/// deterministic `tone_consistency` warning, an HTTP MCP server the harness
/// connects to, one general chapter/scene, agent mode). Adds a
/// `checkpoint_policy` and lets a test append extra routing (e.g. override the
/// `review` route). Also installs a dispatch recorder so a test can prove which
/// prose was (and was NOT) dispatched. `revision_fixes`/`max_revise_attempts`
/// steer whether the committed scene ends `solemn` (clean range) or `grim`
/// (a parked ≥ warning finding).
struct AutoCheckpointFixture {
    _tmp: TempDir,
    svc: SqliteSpindleService,
    router: crate::tools::ToolRouter,
    project_id: String,
    run_id: String,
    ct: tokio_util::sync::CancellationToken,
    server_handle: tokio::task::JoinHandle<()>,
    data_dir: std::path::PathBuf,
    dispatch_log: std::sync::Arc<std::sync::Mutex<Vec<spindle_adapters::ai::DispatchRecord>>>,
}

#[allow(clippy::too_many_arguments)]
async fn auto_checkpoint_fixture(
    checkpoint_policy: &str,
    revision_fixes: bool,
    max_revise_attempts: Option<i32>,
    review_agent_ratings: &[&str],
    // When true, the review agent is an `openai-compatible` (HTTP) agent pointing
    // at a dead port, so its config-level preflight passes (the run starts) but
    // the actual review dispatch fails with a connection-refused transport error
    // at checkpoint time. When false, review uses the working mock CLI agent.
    review_http_dead: bool,
    expect_start_ok: bool,
) -> Option<AutoCheckpointFixture> {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_core::models::ConfigureAgentsInput;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_auto_checkpoint.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let script_path = universal_mock_agent_path();
    let config_path = tmp.path().join("config.toml");
    // Base config: draft agent (general+explicit) plus a `review` agent whose
    // declared ratings the test controls (so a test can make review NOT cover
    // the scene's rating and force the explicit-manual-fallback). The review
    // route resolves to it. Extra routing may override further.
    let review_ratings_toml = review_agent_ratings
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let review_agent_block = if review_http_dead {
        // Config-level preflight passes (an http agent covering the ratings),
        // but the dispatch fails at a dead port (connection refused).
        format!(
            r#"
[[agents]]
id = "cli-agent-review"
name = "Dead Review"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:1/v1"
model = "model"
ratings = [{review_ratings}]
"#,
            review_ratings = review_ratings_toml,
        )
    } else {
        format!(
            r#"
[[agents]]
id = "cli-agent-review"
name = "CLI Agent Review"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = [{review_ratings}]
"#,
            script = script_path.display(),
            review_ratings = review_ratings_toml,
        )
    };
    let config_content = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]
{review_agent_block}
[[routing]]
route = "draft"
agent = "cli-agent-draft"

[[routing]]
route = "review"
agent = "cli-agent-review"
"#,
        script = script_path.display(),
        review_agent_block = review_agent_block,
    );
    std::fs::write(&config_path, config_content).unwrap();

    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();
    let dispatch_log = svc.repository().model_router().install_dispatch_recorder();

    let project = svc
        .create_project(CreateProjectInput {
            name: "Auto Checkpoint".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Self-clearing checkpoints.".into(),
                style_notes: Vec::new(),
                boundaries: vec!["tone: solemn".into()],
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
                tone: Some("solemn".into()),
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

    let synopsis = if revision_fixes {
        "First watch."
    } else {
        "First watch. STUBBORN_SCENE"
    };
    svc.plan_chapter(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(mara.character_id.clone()),
        synopsis: synopsis.into(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![PlanChapterSceneInput {
            scene_order: 1,
            summary: "Mara takes the watch".into(),
            beat_structure: Vec::new(),
            character_ids: vec![mara.character_id.clone()],
            location_id: Some(loc.location_id.clone()),
            content_rating: Some(ContentRating::General),
            purpose: "establishing".into(),
            research_required: Some(false),
            ..Default::default()
        }],
    })
    .await
    .unwrap();

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    let mut start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 1,
        "checkpoint_policy": checkpoint_policy,
    });
    if let Some(budget) = max_revise_attempts {
        start_args["max_revise_attempts"] = serde_json::Value::from(budget);
    }
    let start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    let start_val = start_res.structured_content.unwrap();
    if !expect_start_ok {
        // Caller expects a start block (K2 precondition failure). Return None so
        // the test asserts on the start result it already has; no server spun up.
        assert_eq!(
            start_val["status"].as_str(),
            Some("blocked"),
            "expected start to block: {start_val:?}"
        );
        return None;
    }
    assert_eq!(
        start_val["status"].as_str(),
        Some("active"),
        "run should start active: {start_val:?}"
    );
    let run_id = start_val["run_id"].as_str().unwrap().to_string();

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

    Some(AutoCheckpointFixture {
        _tmp: tmp,
        svc,
        router,
        project_id: project.project_id,
        run_id,
        ct,
        server_handle,
        data_dir: data_dir.clone(),
        dispatch_log,
    })
}

/// Drive `authoring_execute_next` in AGENT mode for an auto-checkpoint fixture.
async fn auto_execute_next(fx: &AutoCheckpointFixture) -> serde_json::Value {
    let exec_args = serde_json::json!({
        "project_id": fx.project_id,
        "run_id": fx.run_id,
        "mode": "agent",
    });
    fx.router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap()
}

async fn auto_status(fx: &AutoCheckpointFixture) -> serde_json::Value {
    let status_args = serde_json::json!({ "project_id": fx.project_id, "run_id": fx.run_id });
    fx.router
        .call_tool("authoring_status", Some(status_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap()
}

/// Drive execute_next in agent mode until the run reaches a STABLE terminal
/// state (completed, or blocked with the automation having already had its
/// chance to run). Returns the final execute_next response.
///
/// The auto-checkpoint automation fires at the TOP of execute_next when the run
/// is already at a pending checkpoint — i.e. on the call AFTER `RunCheckpoint`
/// creates it (mirroring the manual flow: one call creates, the next surfaces
/// the await). So a single "blocked" is not terminal: the driver calls once
/// more to let the automation run, and only stops when a "blocked"/"completed"
/// REPEATS (a genuine fixed point) or the run completes.
async fn auto_drive_to_block_or_complete(fx: &AutoCheckpointFixture) -> serde_json::Value {
    let mut last = serde_json::Value::Null;
    let mut prev_key: Option<(String, String)> = None;
    for _ in 0..40 {
        last = auto_execute_next(fx).await;
        let status = last["status"].as_str().unwrap_or("").to_string();
        let next = last["next_action"].as_str().unwrap_or("").to_string();
        if status == "completed" {
            break;
        }
        if status == "blocked" {
            // Stop only when the same blocked (next_action) repeats — the
            // automation has run and the block is stable.
            if prev_key.as_ref() == Some(&(status.clone(), next.clone())) {
                break;
            }
        }
        prev_key = Some((status, next));
    }
    last
}

async fn auto_run_events(fx: &AutoCheckpointFixture) -> Vec<String> {
    fx.svc
        .repository()
        .list_run_events(&fx.run_id, None, None)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

async fn auto_shutdown(fx: AutoCheckpointFixture) {
    fx.ct.cancel();
    fx.server_handle.await.unwrap();
    crate::remove_addr_file(&fx.data_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_advisory_clean_range_self_clears_checkpoint() {
    // K3 test 3: an auto_advisory run over a clean checkpoint range. The scene
    // starts "grim" (would trip tone_consistency) but a revise budget lets the
    // agent fix the tone to "solemn", so the committed scene — and therefore the
    // checkpoint's deep consistency — is genuinely clean. The checkpoint
    // auto-approves WITHOUT any manual review_checkpoint / record_audit /
    // run_dual_persona_review tool calls; status shows approved-under-policy; the
    // journal has checkpoint_created then checkpoint_auto_approved; the run
    // proceeds past the checkpoint to completion.
    let fx = auto_checkpoint_fixture("auto_advisory", true, Some(1), &["general"], false, true)
        .await
        .expect("run should start");

    let final_res = auto_drive_to_block_or_complete(&fx).await;
    assert_eq!(
        final_res["status"].as_str(),
        Some("completed"),
        "clean auto_advisory run must complete without manual intervention: {final_res:?}"
    );

    let st = auto_status(&fx).await;
    let cp = &st["checkpoint_reports"][0];
    assert_eq!(
        cp["auto_outcome"].as_str(),
        Some("approved"),
        "checkpoint must show approved-under-policy, got {cp:?}"
    );
    assert_eq!(cp["checkpoint_policy"].as_str(), Some("auto_advisory"));
    assert_eq!(cp["status"].as_str(), Some("reviewed"));

    let kinds = auto_run_events(&fx).await;
    let idx = |k: &str| kinds.iter().position(|x| x == k);
    assert!(
        idx("checkpoint_created").is_some(),
        "expected checkpoint_created: {kinds:?}"
    );
    assert!(
        idx("checkpoint_auto_approved").is_some(),
        "expected checkpoint_auto_approved: {kinds:?}"
    );
    assert!(
        idx("checkpoint_created") < idx("checkpoint_auto_approved"),
        "created must precede auto_approved: {kinds:?}"
    );
    // No manual review event was needed.
    assert!(
        idx("checkpoint_reviewed").is_none(),
        "auto approval must not emit checkpoint_reviewed: {kinds:?}"
    );

    auto_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_advisory_blocking_finding_holds_checkpoint_then_manual_clears() {
    // K3 test 4: an auto_advisory run whose scene keeps a deterministic
    // tone_consistency warning (no revise budget → the "grim" tone is committed
    // and parked). The checkpoint deep consistency then carries a ≥ warning
    // finding, so the automation BLOCKS: the checkpoint stays pending_review,
    // the journal has checkpoint_blocked, and the manual escape hatch
    // (authoring_review_checkpoint) still clears it afterward.
    let fx = auto_checkpoint_fixture("auto_advisory", false, None, &["general"], false, true)
        .await
        .expect("run should start");

    let blocked = auto_drive_to_block_or_complete(&fx).await;
    assert_eq!(
        blocked["status"].as_str(),
        Some("blocked"),
        "a ≥ warning finding must block the auto_advisory checkpoint: {blocked:?}"
    );

    let st = auto_status(&fx).await;
    let cp = &st["checkpoint_reports"][0];
    assert_eq!(cp["status"].as_str(), Some("pending_review"));
    assert_eq!(
        cp["auto_outcome"].as_str(),
        Some("blocked"),
        "checkpoint must record the auto-block outcome, got {cp:?}"
    );
    assert_eq!(
        st["blocked_reason"].as_str(),
        Some("await_checkpoint_review"),
        "run blocked_reason names the pending checkpoint: {st:?}"
    );

    let kinds = auto_run_events(&fx).await;
    assert!(
        kinds.iter().any(|k| k == "checkpoint_blocked"),
        "expected checkpoint_blocked: {kinds:?}"
    );
    assert!(
        !kinds.iter().any(|k| k == "checkpoint_auto_approved"),
        "a blocked checkpoint must not emit auto_approved: {kinds:?}"
    );

    // The manual escape hatch works under an auto policy: clear the deep audit +
    // sampled reviews by hand, then review_checkpoint.
    let report_rel = cp["report_artifact_path"].as_str().unwrap();
    let report_path = fx.data_dir.join("artifacts").join(report_rel);
    let report_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    let sampled_scene_ids: Vec<String> = report_json["sampled_scene_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    let deep_args = serde_json::json!({
        "project_id": fx.project_id,
        "scope": {
            "scope_type": "chapter_range",
            "start_book_number": 1, "start_chapter_number": 1,
            "end_book_number": 1, "end_chapter_number": 1
        },
        "checks": [], "severity_filter": [], "deep_check": true, "subjects": []
    });
    let deep = fx
        .router
        .call_tool("check_consistency", Some(deep_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let audit_args = serde_json::json!({
        "project_id": fx.project_id, "run_id": fx.run_id,
        "start_chapter": 1, "end_chapter": 1, "deep_consistency": deep
    });
    fx.router
        .call_tool(
            "authoring_record_checkpoint_audit",
            Some(audit_args.as_object().unwrap()),
        )
        .await
        .unwrap();
    for scene_id in &sampled_scene_ids {
        let review_args =
            serde_json::json!({ "project_id": fx.project_id, "scene_id": scene_id, "rounds": 2 });
        fx.router
            .call_tool(
                "run_dual_persona_review",
                Some(review_args.as_object().unwrap()),
            )
            .await
            .unwrap();
    }
    let review_args = serde_json::json!({
        "project_id": fx.project_id, "run_id": fx.run_id,
        "start_chapter": 1, "end_chapter": 1, "directives": ["Operator override: tone acceptable."]
    });
    let reviewed = fx
        .router
        .call_tool(
            "authoring_review_checkpoint",
            Some(review_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_ne!(
        reviewed["status"].as_str(),
        Some("blocked"),
        "manual review must clear the auto-blocked checkpoint: {reviewed:?}"
    );

    auto_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_strict_blocks_where_advisory_approves_on_info_only_findings() {
    // K3 test 5: an info-only finding set. auto_advisory approves (info is
    // allowed); auto_strict blocks (zero findings of ANY severity required). The
    // clean range (revision fixes tone → no ≥ warning finding) may still carry
    // info-severity findings from the deep pass; the two policies must diverge on
    // exactly that set. Assert both directions.
    let advisory =
        auto_checkpoint_fixture("auto_advisory", true, Some(1), &["general"], false, true)
            .await
            .expect("advisory run starts");
    let advisory_final = auto_drive_to_block_or_complete(&advisory).await;
    let advisory_cp = auto_status(&advisory).await["checkpoint_reports"][0].clone();
    let advisory_approved = advisory_cp["auto_outcome"].as_str() == Some("approved");
    let advisory_info = advisory_cp["report_artifact_path"]
        .as_str()
        .map(|rel| {
            let path = advisory.data_dir.join("artifacts").join(rel);
            let report: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            report["deep_consistency"]["summary"]["info_count"]
                .as_i64()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    assert!(
        advisory_approved,
        "auto_advisory must approve the clean-of-warnings range: {advisory_final:?} / {advisory_cp:?}"
    );
    // The clean range (tone fixed to solemn → 0 warnings/errors) still carries
    // info-severity findings from the deep pass; the local stubs deterministically
    // produce them, so the advisory-vs-strict divergence is load-bearing, not
    // vacuous. Pin that the finding set is info-only.
    assert!(
        advisory_info > 0,
        "the clean range must carry info-only findings so the divergence is real, got {advisory_info}"
    );
    auto_shutdown(advisory).await;

    // Same range under auto_strict: info findings are NOT tolerated → block.
    let strict = auto_checkpoint_fixture("auto_strict", true, Some(1), &["general"], false, true)
        .await
        .expect("strict run starts");
    let strict_final = auto_drive_to_block_or_complete(&strict).await;
    let strict_cp = auto_status(&strict).await["checkpoint_reports"][0].clone();
    assert_eq!(
        strict_final["status"].as_str(),
        Some("blocked"),
        "auto_strict must block on the info-only finding set advisory approved: {strict_final:?}"
    );
    assert_eq!(
        strict_cp["auto_outcome"].as_str(),
        Some("blocked"),
        "auto_strict records an auto-block on info-only findings: {strict_cp:?}"
    );
    auto_shutdown(strict).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_advisory_explicit_scene_falls_back_to_manual_no_dispatch() {
    // K3 test 6: explicit-manual-fallback (I3). Review covers explicit at START
    // (so the run starts), but configure_agents hot-reloads to a review config
    // that does NOT cover explicit between draft and checkpoint. The explicit
    // sampled scene's review dispatch hits RatingNotCovered at the chokepoint →
    // pending-manual; the checkpoint blocks listing that scene; and the dispatch
    // recorder shows ZERO review dispatches carrying the explicit scene's prose.
    //
    // Two scenes in one chapter (explicit = 1, general = 2) so both the explicit
    // (first) and general (last) scenes are sampled — proving the general scene's
    // review still runs while the explicit one falls back.
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test_fallback.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let script_path = universal_mock_agent_path();

    // START config: review agent covers explicit (so start_run preflight passes).
    let config_path = tmp.path().join("config.toml");
    let start_config = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]

[[agents]]
id = "cli-agent-review"
name = "CLI Agent Review"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]

[[routing]]
route = "draft"
agent = "cli-agent-draft"

[[routing]]
route = "review"
agent = "cli-agent-review"
"#,
        script = script_path.display(),
    );
    std::fs::write(&config_path, &start_config).unwrap();
    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(spindle_core::models::ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();
    let dispatch_log = svc.repository().model_router().install_dispatch_recorder();

    let project = svc
        .create_project(CreateProjectInput {
            name: "Fallback".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Explicit prose never leaves uncleared.".into(),
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
            summary: "Wall.".into(),
            initial_state: WorldStateInput::default(),
        })
        .await
        .unwrap();
    // Two chapters so BOTH chapters' scenes are sampled at the single
    // end-of-range checkpoint: `sample_checkpoint_scene_ids` takes the FIRST
    // scene of the start chapter and the LAST scene of the end chapter. Chapter
    // 1 = general (its review completes); chapter 2 = explicit (its review falls
    // back to manual once review no longer covers explicit).
    for (chapter_number, rating, label) in [
        (1, ContentRating::General, "General watch"),
        (2, ContentRating::Explicit, "Explicit watch"),
    ] {
        svc.plan_chapter(PlanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number,
            pov_character_id: Some(mara.character_id.clone()),
            synopsis: format!("Chapter {chapter_number}."),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![PlanChapterSceneInput {
                scene_order: 1,
                summary: label.into(),
                beat_structure: Vec::new(),
                character_ids: vec![mara.character_id.clone()],
                location_id: Some(loc.location_id.clone()),
                content_rating: Some(rating),
                purpose: "establishing".into(),
                research_required: Some(false),
                ..Default::default()
            }],
        })
        .await
        .unwrap();
    }

    let router = crate::tools::ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        std::sync::Arc::new(crate::tools::ToolSerializationState::default()),
    );

    let start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 2,
        "checkpoint_interval": 2,
        "checkpoint_policy": "auto_advisory",
    });
    let start_val = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        start_val["status"].as_str(),
        Some("active"),
        "start must pass with explicit-covering review: {start_val:?}"
    );
    let run_id = start_val["run_id"].as_str().unwrap().to_string();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    crate::write_addr_file(&data_dir, addr).unwrap();
    let svc_clone = svc.clone();
    let ct = tokio_util::sync::CancellationToken::new();
    let ct1 = ct.clone();
    let ct2 = ct.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::http::mcp_router(svc_clone, ct1))
            .with_graceful_shutdown(async move { ct2.cancelled_owned().await })
            .await
            .unwrap();
    });

    let exec_args =
        serde_json::json!({ "project_id": project.project_id, "run_id": run_id, "mode": "agent" });

    // Drive drafting for BOTH scenes, but hot-swap the review config to NOT
    // cover explicit before the checkpoint fires. Draft/commit/beats/summary for
    // both scenes first; then swap; then the checkpoint runs.
    let mut swapped = false;
    for _ in 0..40 {
        // Once both scenes are committed and we are about to hit the checkpoint,
        // swap the review config so explicit is uncovered.
        if !swapped {
            let st = router
                .call_tool(
                    "authoring_status",
                    Some(
                        serde_json::json!({ "project_id": project.project_id, "run_id": run_id })
                            .as_object()
                            .unwrap(),
                    ),
                )
                .await
                .unwrap()
                .structured_content
                .unwrap();
            let next = st["next_action"].as_str().unwrap_or("");
            if next.contains("checkpoint") {
                // Hot-reload (configure_agents) to a review config that no longer
                // covers explicit — the mid-run config change the fallback rule
                // is defense-in-depth against (evolution §3.3 I3).
                let swapped_config = format!(
                    r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]

[[agents]]
id = "cli-agent-review"
name = "CLI Agent Review"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general"]

[[routing]]
route = "draft"
agent = "cli-agent-draft"

[[routing]]
route = "review"
agent = "cli-agent-review"
"#,
                    script = script_path.display(),
                );
                std::fs::write(&config_path, swapped_config).unwrap();
                svc.configure_agents(spindle_core::models::ConfigureAgentsInput {
                    config_path: Some(config_path.to_string_lossy().to_string()),
                })
                .unwrap();
                swapped = true;
            }
        }
        let res = router
            .call_tool(
                "authoring_execute_next",
                Some(exec_args.as_object().unwrap()),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let status = res["status"].as_str().unwrap_or("");
        let executed = res["executed_action"].as_str().unwrap_or("");
        // Stop only once the automation has run (an auto-checkpoint executed
        // action) or the run completes. A plain "blocked" right after
        // RunCheckpoint is NOT terminal — the automation fires on the next call
        // (the top-of-execute_next interception), so keep going until the
        // executed_action names the auto-checkpoint or the run completes.
        if status == "completed" || executed.contains("auto-checkpoint") {
            break;
        }
    }

    // The checkpoint blocked on pending-manual for the explicit scene.
    let st = router
        .call_tool(
            "authoring_status",
            Some(
                serde_json::json!({ "project_id": project.project_id, "run_id": run_id })
                    .as_object()
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let cp = &st["checkpoint_reports"][0];
    assert_eq!(
        cp["status"].as_str(),
        Some("pending_review"),
        "pending-manual must leave the checkpoint pending_review: {cp:?}"
    );
    assert_eq!(
        cp["auto_outcome"].as_str(),
        Some("manual"),
        "checkpoint must record the manual-fallback outcome: {cp:?}"
    );
    let pending = cp["pending_manual_scene_ids"].as_array().unwrap();
    assert_eq!(
        pending.len(),
        1,
        "exactly the explicit scene falls back to manual: {cp:?}"
    );
    // Explicit scene is chapter 2 scene 1; general scene is chapter 1 scene 1.
    let explicit_scene_id = st["chapters"][1]["scenes"][0]["scene_id"]
        .as_str()
        .unwrap()
        .to_string();
    let general_scene_id = st["chapters"][0]["scenes"][0]["scene_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        pending[0].as_str(),
        Some(explicit_scene_id.as_str()),
        "the pending-manual scene must be the explicit one: {cp:?}"
    );
    assert_ne!(
        pending[0].as_str(),
        Some(general_scene_id.as_str()),
        "the general scene's review must have COMPLETED, not fallen back: {cp:?}"
    );

    ct.cancel();
    server.await.unwrap();
    crate::remove_addr_file(&data_dir);

    // Fetch the explicit scene's prose to sweep the dispatch log for leakage.
    let explicit_scene = svc
        .repository()
        .get_scene(&explicit_scene_id)
        .await
        .unwrap();
    let explicit_prose = explicit_scene.full_text.clone();
    let dispatches = dispatch_log.lock().expect("dispatch log lock").clone();
    // Zero REVIEW dispatches carry the explicit scene's prose (the fallback
    // never routed it anywhere).
    for record in &dispatches {
        if let spindle_adapters::ai::DispatchRecord::Dispatch { route, prompt, .. } = record
            && route == "review"
        {
            assert!(
                !prompt.contains(explicit_prose.trim()) || explicit_prose.trim().is_empty(),
                "explicit prose must never be dispatched to a review route: {record:?}"
            );
        }
    }

    drop(router);
    drop(svc);
    drop(tmp);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_advisory_transport_failure_blocks_without_crashing() {
    // K3 test 7: an unroutable review endpoint at checkpoint time. The review
    // agent is an HTTP agent covering the ratings (so config-level preflight
    // passes and the run starts) but pointing at a dead port, so the actual
    // review dispatch fails with a NON-clearance connection-refused transport
    // error. The automation blocks naming the failed step — the run is not
    // crashed and stays resumable.
    let fx = auto_checkpoint_fixture(
        "auto_advisory",
        true,
        Some(1),
        &["general", "explicit"],
        /* review_http_dead */ true,
        true,
    )
    .await
    .expect("run should start (config-level preflight passes for the dead HTTP agent)");

    let blocked = auto_drive_to_block_or_complete(&fx).await;
    assert_eq!(
        blocked["status"].as_str(),
        Some("blocked"),
        "a transport failure at checkpoint must block, not crash: {blocked:?}"
    );
    assert_eq!(
        blocked["run_id"].as_str(),
        Some(fx.run_id.as_str()),
        "the run must survive the failure and remain addressable: {blocked:?}"
    );

    let st = auto_status(&fx).await;
    let cp = &st["checkpoint_reports"][0];
    assert_eq!(cp["status"].as_str(), Some("pending_review"));
    assert_eq!(
        cp["auto_outcome"].as_str(),
        Some("blocked"),
        "transport failure records an auto-block outcome: {cp:?}"
    );

    let kinds = auto_run_events(&fx).await;
    assert!(
        kinds.iter().any(|k| k == "checkpoint_blocked"),
        "transport-failed checkpoint must emit checkpoint_blocked: {kinds:?}"
    );

    // The dispatch log is inspected only for absence of a leak (there is none:
    // the general scene's review may dispatch, but nothing crashed the run).
    let _ = &fx.dispatch_log;

    auto_shutdown(fx).await;
}

// =============================================================================
// Cumulative reader-simulation pass (evolution §3.6, P3.4)
// =============================================================================

/// Build a reader-sim auto-checkpoint fixture: `chapters` chapters (one general
/// scene each unless `explicit` is set, then the single chapter's scene is
/// explicit), an `auto_advisory`/`manual` policy, and per-chapter draft
/// sentinels the reader-sim pass then keys on. `review_ratings` controls the
/// review agent's declared coverage (so a test can strand an explicit chapter).
/// Returns an `AutoCheckpointFixture`; drive it with the shared `auto_*` helpers.
#[allow(clippy::too_many_arguments)]
async fn reader_sim_fixture(
    checkpoint_policy: &str,
    chapters: i32,
    ch1_sentinel: &str,
    ch2_sentinel: &str,
    explicit: bool,
    review_ratings: &[&str],
    // When Some, the review agent is hot-reloaded to these ratings right after
    // the run starts (start preflight still saw `review_ratings`). Lets a test
    // start an explicit auto policy that passes preflight, then strand the
    // explicit chapter at checkpoint time.
    swap_review_to: Option<&[&str]>,
) -> AutoCheckpointFixture {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_core::models::ConfigureAgentsInput;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("reader_sim.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Reuse the single process-stable universal mock (SPINDLE_MODEL_CLI_COMMAND
    // is process-global; a per-fixture script would race parallel tests). The
    // universal draft branch copies any MOCK_READER_* sentinel from the synopsis
    // into the prose, and its review branch handles the reader-sim JSON.
    let script_path = universal_mock_agent_path();
    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    let review_ratings_toml = review_ratings
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let draft_ratings = if explicit {
        r#""general", "explicit""#
    } else {
        r#""general""#
    };
    let config_content = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = [{draft_ratings}]

[[agents]]
id = "cli-agent-review"
name = "CLI Agent Review"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = [{review_ratings_toml}]

[[routing]]
route = "draft"
agent = "cli-agent-draft"

[[routing]]
route = "review"
agent = "cli-agent-review"
"#,
        script = script_path.display(),
    );
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, config_content).unwrap();

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();
    let dispatch_log = svc.repository().model_router().install_dispatch_recorder();

    let project = svc
        .create_project(CreateProjectInput {
            name: "Reader Sim".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "A reader who reads in order and remembers.".into(),
                style_notes: vec!["taut, grounded".into()],
                boundaries: vec!["tone: solemn".into()],
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
                tone: Some("solemn".into()),
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

    for chapter_number in 1..=chapters {
        let rating = if explicit {
            ContentRating::Explicit
        } else {
            ContentRating::General
        };
        // The per-chapter reader sentinel rides the synopsis into the draft
        // prompt; the mock copies it into the committed prose (hermetic, no env).
        let sentinel = match chapter_number {
            1 => ch1_sentinel,
            2 => ch2_sentinel,
            _ => "",
        };
        let synopsis = format!("First watch. {sentinel}");
        svc.plan_chapter(PlanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number,
            pov_character_id: Some(mara.character_id.clone()),
            synopsis: synopsis.clone(),
            target_theme_ids: Vec::new(),
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes: vec![PlanChapterSceneInput {
                scene_order: 1,
                summary: "Mara takes the watch".into(),
                beat_structure: Vec::new(),
                character_ids: vec![mara.character_id.clone()],
                location_id: Some(loc.location_id.clone()),
                content_rating: Some(rating),
                purpose: "establishing".into(),
                research_required: Some(false),
                ..Default::default()
            }],
        })
        .await
        .unwrap();
    }

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    let start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": chapters,
        // One checkpoint spanning the whole range, so a multi-chapter reader-sim
        // pass runs inside a single auto-checkpoint (memory accumulates c→c).
        "checkpoint_interval": chapters,
        "checkpoint_policy": checkpoint_policy,
        "max_revise_attempts": 1,
    });
    let start_res = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap();
    let start_val = start_res.structured_content.unwrap();
    assert_eq!(
        start_val["status"].as_str(),
        Some("active"),
        "reader-sim run should start active: {start_val:?}"
    );
    let run_id = start_val["run_id"].as_str().unwrap().to_string();

    // Optional mid-run hot-reload: strand a rating at checkpoint time (start
    // preflight already passed with the wider coverage).
    if let Some(new_ratings) = swap_review_to {
        let new_toml = new_ratings
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let swapped = format!(
            r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = [{draft_ratings}]

[[agents]]
id = "cli-agent-review"
name = "CLI Agent Review"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = [{new_toml}]

[[routing]]
route = "draft"
agent = "cli-agent-draft"

[[routing]]
route = "review"
agent = "cli-agent-review"
"#,
            script = script_path.display(),
        );
        std::fs::write(&config_path, swapped).unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.to_string_lossy().to_string()),
        })
        .unwrap();
    }

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

    AutoCheckpointFixture {
        _tmp: tmp,
        svc,
        router,
        project_id: project.project_id,
        run_id,
        ct,
        server_handle,
        data_dir,
        dispatch_log,
    }
}

/// Read the checkpoint report artifact's `reader_sim` section for a fixture's
/// single checkpoint (chapters 1..=`end`).
fn reader_sim_report_section(fx: &AutoCheckpointFixture, report_rel: &str) -> serde_json::Value {
    let report_path = fx.data_dir.join("artifacts").join(report_rel);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    report["reader_sim"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_sim_memory_flows_across_two_chapters() {
    // Test 1: a two-chapter checkpoint range. Chapter 1's read produces notes;
    // chapter 2's prose carries MOCK_READER_NOTES_ECHO, so the model echoes the
    // first 40 chars of chapter-1's notes as PRIOR:<...> into chapter 2's notes —
    // proving the prior notes flowed into chapter 2's prompt. The rolling notes
    // artifact ends at updated_through_chapter == 2.
    let fx = reader_sim_fixture(
        "auto_advisory",
        2,
        "MOCK_READER_NOTES_ECHO",
        "MOCK_READER_NOTES_ECHO",
        false,
        &["general"],
        None,
    )
    .await;

    let final_res = auto_drive_to_block_or_complete(&fx).await;
    assert_eq!(
        final_res["status"].as_str(),
        Some("completed"),
        "clean two-chapter auto_advisory run completes: {final_res:?}"
    );

    let st = auto_status(&fx).await;
    let cp = &st["checkpoint_reports"][0];
    let report_rel = cp["report_artifact_path"].as_str().unwrap();
    let section = reader_sim_report_section(&fx, report_rel);
    let chs = section["chapters"].as_array().expect("reader_sim chapters");
    assert_eq!(chs.len(), 2, "one entry per chapter in range: {section:?}");
    assert_eq!(chs[0]["chapter"].as_i64(), Some(1));
    assert_eq!(chs[1]["chapter"].as_i64(), Some(2));

    // The rolling notes artifact reached chapter 2 and carries chapter 2's notes,
    // which echo chapter 1's notes (PRIOR: proves memory flow).
    let notes_rel = section["notes_artifact_path"].as_str().unwrap();
    let notes_path = fx.data_dir.join("artifacts").join(notes_rel);
    let notes: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&notes_path).unwrap()).unwrap();
    assert_eq!(
        notes["updated_through_chapter"].as_i64(),
        Some(2),
        "memory advanced through chapter 2: {notes:?}"
    );
    assert!(
        notes["notes"].as_str().unwrap().contains("PRIOR:"),
        "chapter 2's notes echo the prior notes → memory flowed: {notes:?}"
    );
    // history has both ranges.
    let history = notes["history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1]["range"].as_str(), Some("2..2"));

    // authoring_status surfaces per-chapter engagement additively.
    let engagement = cp["reader_sim_engagement"].as_array().expect("engagement");
    assert_eq!(engagement.len(), 2);
    assert_eq!(engagement[0]["chapter"].as_i64(), Some(1));
    assert_eq!(engagement[0]["engagement"].as_str(), Some("steady"));

    auto_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_sim_dip_surfaces_in_report_without_blocking() {
    // Test 2: MOCK_READER_DIP in the chapter prose → the report section carries
    // dipping + the warning concern. Per the STUDIED verdict semantics
    // (deep-consistency-only; sampled-review outcomes are report-only), the
    // reader-sim concern is REPORT-ONLY and does NOT block: the clean range still
    // auto-approves and completes.
    let fx = reader_sim_fixture(
        "auto_advisory",
        1,
        "MOCK_READER_DIP",
        "",
        false,
        &["general"],
        None,
    )
    .await;

    let final_res = auto_drive_to_block_or_complete(&fx).await;
    assert_eq!(
        final_res["status"].as_str(),
        Some("completed"),
        "a reader-sim warning concern is report-only and must NOT block: {final_res:?}"
    );

    let st = auto_status(&fx).await;
    let cp = &st["checkpoint_reports"][0];
    assert_eq!(
        cp["auto_outcome"].as_str(),
        Some("approved"),
        "the range auto-approves despite the reader-sim dip: {cp:?}"
    );
    let report_rel = cp["report_artifact_path"].as_str().unwrap();
    let section = reader_sim_report_section(&fx, report_rel);
    let ch = &section["chapters"][0];
    assert_eq!(ch["engagement"].as_str(), Some("dipping"));
    let concerns = ch["concerns"].as_array().expect("concerns");
    assert_eq!(concerns.len(), 1, "the warning concern is recorded: {ch:?}");
    assert_eq!(concerns[0]["severity"].as_str(), Some("warning"));
    assert!(
        concerns[0]["description"]
            .as_str()
            .unwrap()
            .contains("market")
    );

    auto_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_sim_rating_uncovered_chapter_skips_without_dispatching_prose() {
    // Test 3: an explicit chapter whose review agent covers only non-explicit
    // ratings. The reader-sim pass for that chapter is SKIPPED with an honest
    // report entry naming the rating; zero reader-sim dispatches carry the prose
    // (the chokepoint rejects before any call); the checkpoint still reaches a
    // verdict (the deep audit + sampled reviews are the verdict inputs, and a
    // reader-sim skip never marks scenes pending-manual).
    //
    // The sampled dual-persona review of the explicit scene ALSO falls back to
    // manual (same uncovered rating), so the checkpoint blocks pending-manual —
    // but that is the sampled-review fallback, NOT the reader-sim skip. What we
    // assert here is the reader-sim skip entry and the absence of any prose
    // dispatch, plus that the automation reached a decision (didn't crash).
    let fx = reader_sim_fixture(
        "auto_advisory",
        1,
        "MOCK_READER_DIP",
        "",
        true,                     // explicit scene
        &["general", "explicit"], // review covers explicit AT START (preflight passes)
        // …then hot-reloaded to drop explicit before the checkpoint fires, so
        // the explicit chapter's reader-sim (and sampled review) hit
        // RatingNotCovered at the chokepoint (evolution §3.3/§3.6 I3).
        Some(&["general", "teen", "mature"]),
    )
    .await;

    // Clear the dispatch log up to the checkpoint so we inspect only the
    // checkpoint-phase dispatches (draft dispatched the explicit prose to the
    // explicit-cleared DRAFT agent legitimately; that is not a leak).
    let drained_before = fx.dispatch_log.lock().unwrap().len();

    let final_res = auto_drive_to_block_or_complete(&fx).await;
    // The explicit sampled review falls back to manual → the checkpoint blocks
    // pending-manual. The run did not crash and remains addressable.
    assert_eq!(
        final_res["run_id"].as_str(),
        Some(fx.run_id.as_str()),
        "run survives the uncovered-rating checkpoint: {final_res:?}"
    );

    let st = auto_status(&fx).await;
    let cp = &st["checkpoint_reports"][0];
    let report_rel = cp["report_artifact_path"].as_str().unwrap();
    let section = reader_sim_report_section(&fx, report_rel);
    assert!(!section.is_null(), "reader-sim section present: {st:?}");
    let ch = &section["chapters"][0];
    assert_eq!(
        ch["engagement"].as_str(),
        Some("skipped"),
        "the uncovered chapter's reader-sim is skipped: {ch:?}"
    );
    let reason = ch["skipped_reason"].as_str().expect("skip reason");
    assert!(
        reason.contains("explicit"),
        "skip reason names the uncovered rating (no prose): {reason}"
    );

    // No reader-sim dispatch carried the explicit prose past the chokepoint. The
    // reader-sim prompt is the only one keyed on "cumulative reader simulation",
    // so we assert no such prompt was ever DISPATCHED (only rejected). Scope the
    // guard so it never crosses the await in auto_shutdown.
    let leaked = {
        let records = fx.dispatch_log.lock().unwrap();
        records.iter().skip(drained_before).any(|r| match r {
            spindle_adapters::ai::DispatchRecord::Dispatch { prompt, .. } => {
                prompt.contains("cumulative reader simulation")
            }
            _ => false,
        })
    };
    assert!(
        !leaked,
        "no reader-sim prose may be dispatched on a rating skip"
    );

    auto_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_sim_never_runs_under_manual_policy() {
    // Test 4: a manual-policy run — the auto-checkpoint automation never fires,
    // so reader-sim never runs: no reader-sim dispatch, no notes artifact, and
    // the report carries no reader_sim section.
    let fx = reader_sim_fixture(
        "manual",
        1,
        "MOCK_READER_DIP",
        "",
        false,
        &["general"],
        None,
    )
    .await;

    // Drive to the checkpoint block (manual policy surfaces await_checkpoint_review).
    let _ = auto_drive_to_block_or_complete(&fx).await;

    // No reader-sim prompt was ever dispatched.
    {
        let records = fx.dispatch_log.lock().unwrap();
        assert!(
            !records.iter().any(|r| matches!(
                r,
                spindle_adapters::ai::DispatchRecord::Dispatch { prompt, .. }
                    if prompt.contains("cumulative reader simulation")
            )),
            "manual policy must never dispatch a reader-sim prompt: {records:?}"
        );
    }

    // No rolling notes artifact was written.
    let notes_path = fx.data_dir.join("artifacts").join("reader-sim-notes.json");
    assert!(
        !notes_path.exists(),
        "manual policy must not write a reader-sim notes artifact"
    );

    // The report (if created) carries no reader_sim section.
    let st = auto_status(&fx).await;
    if let Some(cp) = st["checkpoint_reports"].as_array().and_then(|a| a.first()) {
        if let Some(report_rel) = cp["report_artifact_path"].as_str() {
            let section = reader_sim_report_section(&fx, report_rel);
            assert!(
                section.is_null(),
                "manual report has no reader_sim section: {section:?}"
            );
        }
        assert!(
            cp["reader_sim_engagement"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true),
            "manual status surfaces no reader-sim engagement: {cp:?}"
        );
    }

    auto_shutdown(fx).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_sim_malformed_chapter_preserves_notes_and_next_chapter_runs() {
    // Test 5: chapter 1's reader-sim output is malformed (MOCK_READER_MALFORMED)
    // → prior notes preserved (empty here), "unparsed" recorded in history, and
    // chapter 2 still runs (its NOTES_ECHO landing proves the loop continued).
    let fx = reader_sim_fixture(
        "auto_advisory",
        2,
        "MOCK_READER_MALFORMED",
        "MOCK_READER_NOTES_ECHO",
        false,
        &["general"],
        None,
    )
    .await;

    let final_res = auto_drive_to_block_or_complete(&fx).await;
    assert_eq!(
        final_res["status"].as_str(),
        Some("completed"),
        "a malformed reader-sim read never blocks: {final_res:?}"
    );

    let st = auto_status(&fx).await;
    let cp = &st["checkpoint_reports"][0];
    let report_rel = cp["report_artifact_path"].as_str().unwrap();
    let section = reader_sim_report_section(&fx, report_rel);
    let chs = section["chapters"].as_array().unwrap();
    assert_eq!(chs.len(), 2);
    assert_eq!(
        chs[0]["engagement"].as_str(),
        Some("unparsed"),
        "the malformed chapter records unparsed: {section:?}"
    );
    // Chapter 2 still ran (the loop continued past the unparsed chapter).
    assert_eq!(chs[1]["chapter"].as_i64(), Some(2));
    assert!(
        chs[1]["engagement"].as_str() == Some("steady")
            || chs[1]["engagement"].as_str() == Some("high"),
        "chapter 2 read after the unparsed chapter: {section:?}"
    );

    // The rolling notes artifact recorded the unparsed history entry and only
    // advanced its watermark on the chapter-2 read (chapter 1 kept prior notes).
    let notes_path = fx.data_dir.join("artifacts").join("reader-sim-notes.json");
    let notes: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&notes_path).unwrap()).unwrap();
    let history = notes["history"].as_array().unwrap();
    assert_eq!(history[0]["engagement"].as_str(), Some("unparsed"));
    assert_eq!(
        notes["updated_through_chapter"].as_i64(),
        Some(2),
        "watermark advances only on the chapter-2 read: {notes:?}"
    );

    auto_shutdown(fx).await;
}

// =============================================================================
// Explicit-offload contract test (evolution §4 pinned P2 exit; §5 row C2)
// =============================================================================

/// The §4 pinned offload contract: a run over one `general` + one `explicit`
/// scene where the `mine` route resolves ONLY to a non-explicit-cleared agent
/// must prove no request containing the explicit scene's prose/brief was ever
/// DISPATCHED to that uncleared agent. Enforced against the router's post-gate
/// recording seam (`ModelRouter::install_dispatch_recorder`, evolution §4 rule
/// 2), not merely status strings — a leak is a recorded dispatch, so the seam
/// makes the invariant falsifiable (see the falsification note in the module's
/// dev record).
///
/// Fixture (evolution §4 / §2.3):
///   - draft route → the mock CLI agent, `ratings = ["general","explicit"]`
///     (explicit-cleared, so the explicit scene drafts through the origin agent);
///   - mine route → a separate agent `ratings = ["general"]` (NOT explicit);
///   - review route left unconfigured → built-in local (serves every rating), so
///     the propose_all preflight passes via the mine→review fallback ladder and
///     the run actually STARTS (start_run runs prepare internally). The explicit
///     mine still SKIPS at runtime because the `mine` route itself resolves to
///     the uncleared agent and `RatingNotCovered` does not trigger the NoRoute
///     review fallback — exactly the honest-skip path the ladder specifies.
///
/// One chapter, two scenes (explicit = scene_order 1, general = scene_order 2)
/// keeps the single end-of-run checkpoint's sample on the LAST scene (the
/// general one, `sample_checkpoint_scene_ids`), so clearing the checkpoint never
/// sends the explicit scene to any review route.
///
/// The chapter synopsis carries a distinctive `EXPLICIT_BRIEF_SENTINEL`. It
/// reaches the draft mega-prompt (the CLI adapter receives the whole prompt), so
/// it is recorded on the draft dispatch to the cleared agent; the mine and
/// review prompts are built from scene prose + digest only (no chapter synopsis)
/// so a non-cleared mine/review dispatch could never carry it. The sweep asserts
/// the sentinel appears ONLY in draft-route dispatches to the cleared agent, and
/// in zero mine/review dispatches and zero journal payloads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_offload_contract_no_explicit_prose_to_uncleared_agent() {
    use crate::tools::{ToolRouter, ToolSerializationState};
    use spindle_adapters::ai::DispatchRecord;
    use spindle_core::models::ConfigureAgentsInput;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    // A distinctive brief sentinel that must never reach an uncleared agent nor
    // any journal payload. Lives in the explicit chapter's synopsis (→ draft
    // mega-prompt), never in mine/review prompts.
    const EXPLICIT_BRIEF_SENTINEL: &str = "ZZ_EXPLICIT_BRIEF_QVX";

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("offload_contract.db");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let script_path = universal_mock_agent_path();
    let config_path = tmp.path().join("config.toml");
    // draft → explicit-cleared mock CLI; mine → a general-only agent (the
    // uncleared analyst for explicit); review left to the built-in local route.
    let config_content = format!(
        r#"
[[agents]]
id = "cli-agent-draft"
name = "CLI Agent Draft"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general", "explicit"]

[[agents]]
id = "tame-analyst"
name = "Tame Analyst"
provider = "cli"
endpoint = "{script}"
model = "default"
ratings = ["general"]

[[routing]]
route = "draft"
agent = "cli-agent-draft"

[[routing]]
route = "mine"
agent = "tame-analyst"
"#,
        script = script_path.display(),
    );
    std::fs::write(&config_path, config_content).unwrap();

    unsafe {
        std::env::set_var(
            "SPINDLE_MODEL_CLI_COMMAND",
            script_path.to_string_lossy().to_string(),
        );
    }

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let repo =
        Repository::with_model_router(pool.clone(), data_dir.clone(), ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.to_string_lossy().to_string()),
    })
    .unwrap();

    // Install the post-gate dispatch recorder on the SAME router the harness,
    // mining, verify, and review all dispatch through (there is exactly one:
    // `configure_agents` mutates the router in place, so the recorder survives).
    let dispatch_log = svc.repository().model_router().install_dispatch_recorder();

    let project = svc
        .create_project(CreateProjectInput {
            name: "Offload Contract".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Explicit prose never leaves for an uncleared model.".into(),
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

    // One chapter, two scenes: scene 1 EXPLICIT, scene 2 GENERAL. The single
    // end-of-run checkpoint samples the LAST scene (general), so the explicit
    // scene is never routed to review. The synopsis carries the brief sentinel.
    svc.plan_chapter(PlanChapterInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        pov_character_id: Some(mara.character_id.clone()),
        synopsis: format!("First watch. {EXPLICIT_BRIEF_SENTINEL}"),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![
            PlanChapterSceneInput {
                scene_order: 1,
                summary: "Mara and the stranger, explicit".into(),
                beat_structure: Vec::new(),
                character_ids: vec![mara.character_id.clone()],
                location_id: Some(loc.location_id.clone()),
                content_rating: Some(ContentRating::Explicit),
                purpose: "intimacy".into(),
                research_required: Some(false),
                ..Default::default()
            },
            PlanChapterSceneInput {
                scene_order: 2,
                summary: "Mara takes the watch".into(),
                beat_structure: Vec::new(),
                character_ids: vec![mara.character_id.clone()],
                location_id: Some(loc.location_id.clone()),
                content_rating: Some(ContentRating::General),
                purpose: "establishing".into(),
                research_required: Some(false),
                ..Default::default()
            },
        ],
    })
    .await
    .unwrap();

    let router = ToolRouter::with_tool_profile_and_serialization(
        svc.clone(),
        Some("write".to_string()),
        Arc::new(ToolSerializationState::default()),
    );

    // Start the run: propose_all mining + one revise attempt. checkpoint_interval
    // 2 > the single chapter, so only the final end-of-run checkpoint fires. The
    // start must SUCCEED (the mine→review fallback ladder clears explicit at
    // preflight via the built-in local review route).
    let start_args = serde_json::json!({
        "project_id": project.project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1,
        "checkpoint_interval": 2,
        "mining_policy": "propose_all",
        "max_revise_attempts": 1,
    });
    let start_val = router
        .call_tool("authoring_start_run", Some(start_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        start_val["status"].as_str(),
        Some("active"),
        "run must start (mine→review fallback clears explicit at preflight): {start_val:?}"
    );
    let run_id = start_val["run_id"].as_str().unwrap().to_string();

    // Background HTTP MCP server the harness executor connects to.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    crate::write_addr_file(&data_dir, addr).unwrap();
    let svc_clone = svc.clone();
    let ct = CancellationToken::new();
    let ct1 = ct.clone();
    let ct2 = ct.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::http::mcp_router(svc_clone, ct1))
            .with_graceful_shutdown(async move { ct2.cancelled_owned().await })
            .await
            .unwrap();
    });

    // Drive the run in AGENT mode until it blocks at the checkpoint (or, defensively,
    // completes). The mock CLI drafts both scenes through the draft route.
    let exec_args = serde_json::json!({
        "project_id": project.project_id,
        "run_id": run_id,
        "mode": "agent",
    });
    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 60, "run did not reach checkpoint/complete in time");
        let val = router
            .call_tool(
                "authoring_execute_next",
                Some(exec_args.as_object().unwrap()),
            )
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let status = val["status"].as_str().unwrap();
        if status == "blocked" || status == "completed" {
            break;
        }
    }

    // --- Per-scene status assertions (evolution §4 rule 3, I8) ----------------
    let status_val = router
        .call_tool(
            "authoring_status",
            Some(
                serde_json::json!({ "project_id": project.project_id, "run_id": run_id })
                    .as_object()
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let scenes = status_val["chapters"][0]["scenes"].as_array().unwrap();
    let explicit_scene = scenes
        .iter()
        .find(|s| s["scene_order"].as_i64() == Some(1))
        .expect("explicit scene present");
    let general_scene = scenes
        .iter()
        .find(|s| s["scene_order"].as_i64() == Some(2))
        .expect("general scene present");

    // (1) The explicit scene drafted successfully through the cleared draft route.
    assert!(
        explicit_scene["scene_id"].as_str().is_some(),
        "explicit scene must have drafted+committed a scene_id: {explicit_scene:?}"
    );
    // (2) The explicit scene's mining SKIPPED, naming the rating (honest skip).
    assert_eq!(
        explicit_scene["mine_status"].as_str(),
        Some("skipped"),
        "explicit mine must skip at the gate: {explicit_scene:?}"
    );
    assert!(
        explicit_scene["mine_detail"]
            .as_str()
            .unwrap_or_default()
            .contains("explicit"),
        "explicit mine skip detail must name the uncleared rating: {explicit_scene:?}"
    );
    // (3) The general scene's mining COMPLETED (dispatched to the general-cleared
    // agent; its non-JSON output → model_output_rejected). Crucially NOT skipped:
    // that is the contrast that proves the general mine reached the agent while
    // the explicit mine was gated away.
    assert_eq!(
        general_scene["mine_status"].as_str(),
        Some("model_output_rejected"),
        "general mine must dispatch and complete (not skip): {general_scene:?}"
    );
    assert_ne!(
        general_scene["mine_status"].as_str(),
        Some("skipped"),
        "general mine must not be a gate skip: {general_scene:?}"
    );

    // --- Journal assertions (evolution §3.4, I8) ------------------------------
    let events = svc
        .repository()
        .list_run_events(&run_id, None, None)
        .await
        .unwrap();
    assert!(!events.is_empty(), "run must have journalled events");

    // (2 cont.) A `pass_skipped` event exists for the explicit scene's mine pass
    // with a prose-free reason naming the rating.
    let explicit_skip = events.iter().find(|e| {
        e.kind == "pass_skipped"
            && e.payload["pass"] == serde_json::json!("mine")
            && e.payload["scene_order"] == serde_json::json!(1)
    });
    let explicit_skip = explicit_skip.expect("a pass_skipped(mine, scene 1) event must exist");
    let skip_reason = explicit_skip.payload["reason"].as_str().unwrap_or_default();
    assert!(
        skip_reason.contains("explicit"),
        "pass_skipped reason must name the rating (prose-free): {skip_reason}"
    );

    // --- (4) THE CORE PIN: post-gate dispatch sweep ---------------------------
    let dispatches = dispatch_log.lock().expect("dispatch log lock").clone();
    assert!(
        !dispatches.is_empty(),
        "the recorder must have observed dispatches"
    );

    // The uncleared analyst agent's id — no explicit-carrying prompt may ever be
    // dispatched to it, and no mine/review dispatch may carry the brief sentinel.
    const UNCLEARED_AGENT: &str = "tame-analyst";
    const CLEARED_DRAFT_AGENT: &str = "cli-agent-draft";

    let mut explicit_mine_rejected = false;
    for record in &dispatches {
        match record {
            DispatchRecord::Dispatch {
                route,
                agent,
                rating,
                prompt,
            } => {
                // No dispatch to the uncleared agent may carry the explicit rating.
                if agent == UNCLEARED_AGENT {
                    assert_ne!(
                        rating.as_deref(),
                        Some("explicit"),
                        "explicit-rated request dispatched to the uncleared agent: {record:?}"
                    );
                }
                // The brief sentinel may appear ONLY in draft-route dispatches to
                // the cleared draft agent — never in mine/review dispatches.
                if prompt.contains(EXPLICIT_BRIEF_SENTINEL) {
                    assert_eq!(
                        route, "draft",
                        "explicit brief sentinel leaked into a non-draft dispatch: {record:?}"
                    );
                    assert_eq!(
                        agent, CLEARED_DRAFT_AGENT,
                        "explicit brief sentinel reached a non-cleared agent: {record:?}"
                    );
                }
                // Specifically: zero mine/review dispatches carry the sentinel.
                assert!(
                    !(matches!(route.as_str(), "mine" | "review")
                        && prompt.contains(EXPLICIT_BRIEF_SENTINEL)),
                    "a mine/review dispatch carried the explicit brief sentinel: {record:?}"
                );
            }
            DispatchRecord::Rejection {
                route,
                rating,
                error,
            } => {
                if route == "mine" && rating.as_deref() == Some("explicit") {
                    explicit_mine_rejected = true;
                    assert!(
                        error.contains("not cleared") || error.contains("explicit"),
                        "explicit mine rejection must be a clearance error: {error}"
                    );
                }
            }
        }
    }
    // The explicit scene's mine dispatch was REFUSED at the gate (recorded as a
    // Rejection, never a Dispatch): the prose-bearing mine prompt never left.
    assert!(
        explicit_mine_rejected,
        "the explicit mine must have been rejected at the clearance gate: {dispatches:?}"
    );
    // And there is genuinely at least one draft dispatch carrying the sentinel to
    // the cleared agent (the positive half of the pin is not vacuous).
    assert!(
        dispatches.iter().any(|r| matches!(
            r,
            DispatchRecord::Dispatch { route, agent, prompt, .. }
                if route == "draft" && agent == CLEARED_DRAFT_AGENT
                    && prompt.contains(EXPLICIT_BRIEF_SENTINEL)
        )),
        "the explicit brief must have been dispatched to the cleared draft agent: {dispatches:?}"
    );

    // --- (5) Journal payload sweep: the sentinel is in ZERO payloads ----------
    for event in &events {
        let payload = serde_json::to_string(&event.payload).unwrap();
        assert!(
            !payload.contains(EXPLICIT_BRIEF_SENTINEL),
            "brief sentinel leaked into a {} journal payload: {payload}",
            event.kind
        );
    }

    // --- (6) The run completes (skips never block) ----------------------------
    // Clear the single end-of-run checkpoint. The sampled scene is the general
    // (last) scene, so no explicit prose is ever sent to a review route.
    let report_rel = status_val["checkpoint_reports"][0]["report_artifact_path"]
        .as_str()
        .expect("a checkpoint report must exist");
    let report_path = data_dir.join("artifacts").join(report_rel);
    let report_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    let sampled_scene_ids: Vec<String> = report_json["sampled_scene_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    // The explicit scene must NOT be among the sampled scenes (offload-safe
    // checkpoint): reviewing it would route explicit prose. The fixture's
    // single-chapter/last-scene sampling guarantees this.
    let explicit_scene_id = explicit_scene["scene_id"].as_str().unwrap();
    assert!(
        !sampled_scene_ids.iter().any(|id| id == explicit_scene_id),
        "the explicit scene must not be sampled for review (offload-safe checkpoint)"
    );

    let deep_args = serde_json::json!({
        "project_id": project.project_id,
        "scope": {
            "scope_type": "chapter_range",
            "start_book_number": 1, "start_chapter_number": 1,
            "end_book_number": 1, "end_chapter_number": 1
        },
        "checks": [], "severity_filter": [], "deep_check": true, "subjects": []
    });
    let deep = router
        .call_tool("check_consistency", Some(deep_args.as_object().unwrap()))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    router
        .call_tool(
            "authoring_record_checkpoint_audit",
            Some(
                serde_json::json!({
                    "project_id": project.project_id, "run_id": run_id,
                    "start_chapter": 1, "end_chapter": 1, "deep_consistency": deep
                })
                .as_object()
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    for scene_id in &sampled_scene_ids {
        router
            .call_tool(
                "run_dual_persona_review",
                Some(
                    serde_json::json!({ "project_id": project.project_id, "scene_id": scene_id, "rounds": 2 })
                        .as_object()
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
    }
    router
        .call_tool(
            "authoring_review_checkpoint",
            Some(
                serde_json::json!({
                    "project_id": project.project_id, "run_id": run_id,
                    "start_chapter": 1, "end_chapter": 1, "directives": ["Keep it dark."]
                })
                .as_object()
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let final_val = router
        .call_tool(
            "authoring_execute_next",
            Some(exec_args.as_object().unwrap()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(
        final_val["next_action"].as_str(),
        Some("complete"),
        "the run must complete (skips never block): {final_val:?}"
    );

    // Re-sweep post-checkpoint: the deep audit + general-scene review dispatched
    // more requests; re-confirm none carried the explicit brief sentinel to a
    // mine/review route and none reached the uncleared agent at explicit rating.
    let dispatches = dispatch_log.lock().expect("dispatch log lock").clone();
    for record in &dispatches {
        if let DispatchRecord::Dispatch {
            route,
            agent,
            rating,
            prompt,
        } = record
        {
            assert!(
                !(matches!(route.as_str(), "mine" | "review")
                    && prompt.contains(EXPLICIT_BRIEF_SENTINEL)),
                "post-checkpoint: mine/review dispatch carried the explicit brief: {record:?}"
            );
            if agent == UNCLEARED_AGENT {
                assert_ne!(
                    rating.as_deref(),
                    Some("explicit"),
                    "post-checkpoint: explicit request dispatched to uncleared agent: {record:?}"
                );
            }
        }
    }

    ct.cancel();
    server.await.unwrap();
    crate::remove_addr_file(&data_dir);
}
