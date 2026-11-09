use super::*;
use serde_json::json;
use spindle_core::models::{CommitSceneOutput, PrepareSceneOutput};

async fn invoke_json(router: &ToolRouter, name: &str, args: Value) -> Value {
    let map = args.as_object().cloned().unwrap_or_default();
    let result = router
        .call_tool(name, Some(&map))
        .await
        .unwrap_or_else(|err| panic!("{name} failed: {err:#}"));
    if result.is_error == Some(true) {
        panic!(
            "{name} returned tool error: {}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    }
    structured_json(result)
}

/// The P4 scene-loop contract: after one session project selection, an ordinary
/// scene with complete supplied continuity is prepared and committed with
/// exactly two calls. Setup happens before selection.
#[tokio::test(flavor = "current_thread")]
async fn scene_loop_contract_uses_prepare_and_commit_only() {
    let router = router().await;
    let project = invoke_json(
        &router,
        "create_project",
        json!({
            "name": "Scene Loop Novel",
            "project_type": "novel",
            "genre": "fantasy",
            "reader_contract": {
                "promise": "Brisk intrigue with sharp banter",
                "style_notes": ["Keep dialogue brisk"],
                "boundaries": ["No modern slang"]
            }
        }),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap().to_string();
    let character = invoke_json(
        &router,
        "create_character",
        json!({
            "project_id": project_id,
            "name": "Mara",
            "role": "protagonist",
            "summary": "A sharp-eyed courier."
        }),
    )
    .await;
    let character_id = character["character_id"].as_str().unwrap().to_string();
    let location = invoke_json(
        &router,
        "create_location",
        json!({
            "project_id": project_id,
            "name": "Glass Wharf",
            "kind": "harbor",
            "summary": "Lantern-lit canal docks."
        }),
    )
    .await;
    let location_id = location["location_id"].as_str().unwrap().to_string();

    // One session project selection, then the ordinary two-call scene loop.
    invoke_json(
        &router,
        "set_active_project",
        json!({ "project_id": project_id }),
    )
    .await;

    let prepared_val = invoke_json(
        &router,
        "prepare_scene",
        json!({
            "project_id": project_id,
            "intent": "draft",
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "character_ids": [character_id],
            "location_id": location_id,
            "budget_tokens": 8000
        }),
    )
    .await;
    let prepared: PrepareSceneOutput = serde_json::from_value(prepared_val).unwrap();
    assert!(prepared.preparation_id.starts_with("prep:"));

    let prose = "Mara counted the lockhouse lamps twice before she knocked.\n\n\
                 \"You're late,\" the keeper said, and slid the cipher coin across the counter.";
    let committed_val = invoke_json(
        &router,
        "commit_scene",
        json!({
            "project_id": project_id,
            "preparation_id": prepared.preparation_id,
            "context_revision_token": prepared.context_revision_token,
            "scene_id": prepared.scene_id,
            "expected_scene_version": prepared.scene_version,
            "book_number": prepared.placement.book_number,
            "chapter_number": prepared.placement.chapter_number,
            "scene_order": prepared.placement.scene_order,
            "full_text": prose,
            "summary": "Mara trades the cipher coin at the lockhouse.",
            "content_rating": "general",
            "location_id": location_id,
            "tone": "tense",
            "authorship": "assistant",
            "continuity": {
                "character_states": [{
                    "character_id": character_id,
                    "changes": {
                        "status": ["focused"],
                        "notes": ["Keeps the cipher coin hidden."],
                        "source_summary": "the lockhouse exchange"
                    }
                }],
                "canonical_facts": [{
                    "subject_table": "character",
                    "subject_id": character_id,
                    "predicate": "held_item",
                    "value_text": "cipher coin",
                    "value_kind": "string",
                    "scope": "evolving"
                }]
            },
            "beat_annotations": [
                { "beat_type": "approach", "summary": "Mara counts the lamps." },
                { "beat_type": "revelation", "summary": "The keeper recognizes the coin." }
            ],
            "chapter_summary": {
                "summary": "Mara trades the cipher coin and learns the keeper is lying.",
                "key_events": ["the lockhouse bargain"],
                "character_changes": ["Mara commits to the handoff"],
                "relationship_shifts": [],
                "arc_advances": [],
                "promise_events": ["the keeper's lie"]
            }
        }),
    )
    .await;
    let committed: CommitSceneOutput = serde_json::from_value(committed_val).unwrap();
    assert!(!committed.dry_run);
    assert_eq!(committed.blocking_count, 0);
    assert_eq!(committed.applied.character_state_ids.len(), 1);
    assert_eq!(committed.applied.canonical_fact_ids.len(), 1);
    assert_eq!(committed.beat_count, 2);
    assert!(committed.chapter_summary_saved);

    let branch_id = router
        .service
        .repository()
        .active_branch_id_public(&project_id)
        .await
        .unwrap();
    let scene = router
        .service
        .repository()
        .get_scene(&committed.scene_id)
        .await
        .unwrap();
    assert_eq!(scene.full_text, prose);
    assert_eq!(
        scene.summary,
        "Mara trades the cipher coin at the lockhouse."
    );
    let cursor = router
        .service
        .repository()
        .get_writer_position(&project_id, &branch_id)
        .await
        .unwrap()
        .expect("the ordinary scene loop moves the cursor");
    assert_eq!(
        cursor.scene_id.as_deref(),
        Some(committed.scene_id.as_str())
    );
    assert!(
        router
            .service
            .repository()
            .get_scene_beat_annotation(&branch_id, &committed.scene_id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        router
            .service
            .repository()
            .get_chapter_summary(&project_id, &branch_id, 1, 1)
            .await
            .unwrap()
            .is_some()
    );
}

fn authoring_scene(
    run_id: &str,
    phase: &str,
) -> (
    spindle_adapters::sqlite::records::AuthoringRunChapter,
    spindle_adapters::sqlite::records::AuthoringRunScene,
) {
    use spindle_adapters::sqlite::records::{AuthoringRunChapter, AuthoringRunScene};
    (
        AuthoringRunChapter {
            authoring_run_id: run_id.to_string(),
            chapter_number: 1,
            planned: true,
            synopsis: "Mara at the lockhouse.".to_string(),
            pov_character_id: None,
            status: "in_progress".to_string(),
            summary_saved: false,
            summary_artifact_path: None,
            replan_status: None,
            replan_detail: None,
        },
        AuthoringRunScene {
            authoring_run_id: run_id.to_string(),
            chapter_number: 1,
            scene_order: 1,
            character_ids: Vec::new(),
            location_id: "location:x".to_string(),
            content_rating: "general".to_string(),
            tone: None,
            source_path: None,
            phase: phase.to_string(),
            scene_id: None,
            scene_artifact_path: None,
            draft_diagnostics: None,
            blocked_reason: None,
            research_required: None,
            research_tags: Vec::new(),
            explicit_query: None,
            mine_status: None,
            mine_detail: None,
            verify_status: None,
            verify_detail: None,
            revise_attempts: 0,
            last_finding_fingerprint: None,
        },
    )
}

async fn install_run(router: &ToolRouter, project_id: &str, phase: &str) -> String {
    use spindle_adapters::sqlite::records::AuthoringRun;
    let run_id = format!(
        "authoring_run:{}",
        ulid::Ulid::new().to_string().to_lowercase()
    );
    let now = chrono::Utc::now();
    let branch_id = router
        .service
        .repository()
        .active_branch_id_public(project_id)
        .await
        .unwrap();
    let run = AuthoringRun {
        id: run_id.clone(),
        project_id: project_id.to_string(),
        active_branch_id: branch_id,
        book_number: 1,
        start_chapter: 1,
        end_chapter: 1,
        checkpoint_interval: 1,
        last_checkpoint_end_chapter: 0,
        artifacts_dir: "../artifacts".to_string(),
        editorial_directives: Vec::new(),
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        mining_policy: None,
        max_revise_attempts: None,
        checkpoint_policy: None,
        replan_policy: None,
    };
    let (chapter, scene) = authoring_scene(&run_id, phase);
    router
        .service
        .repository()
        .save_authoring_run(run, vec![chapter], vec![scene], Vec::new())
        .await
        .unwrap();
    run_id
}

#[tokio::test(flavor = "current_thread")]
async fn commit_scene_advances_the_authoring_run_phase_only_on_success() {
    let router = router().await;
    let project = invoke_json(
        &router,
        "create_project",
        json!({
            "name": "Run Phase Novel",
            "project_type": "novel",
            "genre": "fantasy",
            "reader_contract": {
                "promise": "Brisk intrigue",
                "style_notes": [],
                "boundaries": []
            }
        }),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap().to_string();
    let run_id = install_run(&router, &project_id, "pending").await;

    let committed = invoke_json(
        &router,
        "commit_scene",
        json!({
            "project_id": project_id,
            "run_id": run_id,
            "expected_phase": "pending",
            "idempotency_key": "run-phase-1",
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": "The lockhouse door opened before Mara knocked.",
            "summary": "Mara enters the lockhouse.",
            "content_rating": "general",
            "continuity": {
                "canonical_facts": [{
                    "subject_table": "project",
                    "predicate": "lockhouse_door",
                    "value_text": "unlatched",
                    "value_kind": "string"
                }]
            }
        }),
    )
    .await;
    let commit: CommitSceneOutput = serde_json::from_value(committed).unwrap();
    assert_eq!(commit.blocking_count, 0);
    assert!(commit.follow_ups.iter().any(|follow_up| {
        serde_json::to_value(follow_up.kind).unwrap().as_str() == Some("authoring_run")
    }));

    let (_, _, scenes, _) = router
        .service
        .repository()
        .get_authoring_run(&run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(scenes[0].phase, "beats_annotated");
    assert_eq!(
        scenes[0].scene_id.as_deref(),
        Some(commit.scene_id.as_str())
    );

    // A lost-response retry with the same key/payload must return the stored
    // receipt even though the run phase has already advanced.
    let retried_val = invoke_json(
        &router,
        "commit_scene",
        json!({
            "project_id": project_id,
            "run_id": run_id,
            "expected_phase": "pending",
            "idempotency_key": "run-phase-1",
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": "The lockhouse door opened before Mara knocked.",
            "summary": "Mara enters the lockhouse.",
            "content_rating": "general",
            "continuity": {
                "canonical_facts": [{
                    "subject_table": "project",
                    "predicate": "lockhouse_door",
                    "value_text": "unlatched",
                    "value_kind": "string"
                }]
            }
        }),
    )
    .await;
    let retried: CommitSceneOutput = serde_json::from_value(retried_val).unwrap();
    assert!(retried.idempotent_replay);
    assert_eq!(retried.operation_id, commit.operation_id);
    let (_, _, scenes, _) = router
        .service
        .repository()
        .get_authoring_run(&run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(scenes[0].phase, "beats_annotated");

    // A mismatched expected phase is rejected before any prose is written.
    let other_run = install_run(&router, &project_id, "pending").await;
    let result = router
        .call_tool(
            "commit_scene",
            json!({
                "project_id": project_id,
                "run_id": other_run,
                "expected_phase": "beats_annotated",
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "full_text": "A second draft that must not land.",
                "summary": "rejected",
                "content_rating": "general"
            })
            .as_object(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.is_error,
        Some(true),
        "stale phase must be a tool error"
    );
    assert!(
        serde_json::to_string(&result)
            .unwrap()
            .contains("expected_phase"),
        "phase mismatch message must name expected_phase"
    );
    let branch_id = router
        .service
        .repository()
        .active_branch_id_public(&project_id)
        .await
        .unwrap();
    let unchanged = router
        .service
        .repository()
        .find_scene_by_natural_key(&project_id, &branch_id, 1, 1, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unchanged.full_text,
        "The lockhouse door opened before Mara knocked."
    );

    // Run target ownership: project, branch, and book must all match the run.
    let other_project = invoke_json(
        &router,
        "create_project",
        json!({
            "name": "Other Project",
            "project_type": "novel",
            "genre": "fantasy",
            "reader_contract": { "promise": "p", "style_notes": [], "boundaries": [] }
        }),
    )
    .await;
    let other_project_id = other_project["project_id"].as_str().unwrap().to_string();
    let wrong_project = router
        .call_tool(
            "commit_scene",
            json!({
                "project_id": other_project_id,
                "run_id": run_id,
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "full_text": "This must not write anywhere.",
                "summary": "wrong project",
                "content_rating": "general"
            })
            .as_object(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_project.is_error, Some(true));
    assert!(
        serde_json::to_string(&wrong_project)
            .unwrap()
            .contains("requires project"),
        "project mismatch must name the run project"
    );

    let branch = invoke_json(
        &router,
        "create_branch",
        json!({
            "project_id": project_id,
            "name": "run-child",
            "branch_type": "alternate",
            "parent_branch_id": branch_id
        }),
    )
    .await;
    let child_branch_id = branch["branch_id"].as_str().unwrap().to_string();
    let wrong_branch = router
        .call_tool(
            "commit_scene",
            json!({
                "project_id": project_id,
                "branch_id": child_branch_id,
                "run_id": run_id,
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "full_text": "This must not write to the child branch.",
                "summary": "wrong branch",
                "content_rating": "general"
            })
            .as_object(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_branch.is_error, Some(true));
    assert!(
        serde_json::to_string(&wrong_branch)
            .unwrap()
            .contains("requires branch"),
        "branch mismatch must name the run branch"
    );
    assert!(
        router
            .service
            .repository()
            .find_scene_by_natural_key(&project_id, &child_branch_id, 1, 1, 1)
            .await
            .unwrap()
            .is_none(),
        "rejected branch target must not create a scene"
    );

    // A scene_id target must also belong to the run's project and branch.
    let child_scene = invoke_json(
        &router,
        "commit_scene",
        json!({
            "project_id": project_id,
            "branch_id": child_branch_id,
            "book_number": 1,
            "chapter_number": 1,
            "scene_order": 1,
            "full_text": "A child-branch scene for target validation.",
            "summary": "child scene",
            "content_rating": "general"
        }),
    )
    .await;
    let child_scene_id = child_scene["scene_id"].as_str().unwrap().to_string();
    let wrong_scene_branch = router
        .call_tool(
            "commit_scene",
            json!({
                "project_id": project_id,
                "run_id": run_id,
                "scene_id": child_scene_id,
                "full_text": "This must not overwrite the child-branch scene.",
                "summary": "wrong scene branch",
                "content_rating": "general"
            })
            .as_object(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_scene_branch.is_error, Some(true));
    assert!(
        serde_json::to_string(&wrong_scene_branch)
            .unwrap()
            .contains("does not belong to authoring run"),
        "scene_id branch mismatch must be rejected"
    );

    let wrong_book = router
        .call_tool(
            "commit_scene",
            json!({
                "project_id": project_id,
                "run_id": run_id,
                "book_number": 2,
                "chapter_number": 1,
                "scene_order": 1,
                "full_text": "This must not write into book two.",
                "summary": "wrong book",
                "content_rating": "general"
            })
            .as_object(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_book.is_error, Some(true));
    assert!(
        serde_json::to_string(&wrong_book)
            .unwrap()
            .contains("requires book"),
        "book mismatch must name the run book"
    );
}
