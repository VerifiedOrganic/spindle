use super::*;

/// Deterministic full per-scene step-event trace (J3 test 3, revise arm
/// included). Drives `authoring_emit_step_events` for the exact NextAction
/// sequence a verify+revise+mine scene walks, then asserts the emitted kind
/// order — including `scene_verify_completed(findings)` → `scene_revised` →
/// `scene_verify_completed(clean)` — and that no step event double-emits.
#[tokio::test(flavor = "current_thread")]
async fn step_event_trace_covers_verify_revise_mine_sequence() {
    let (_tmp, repo, run_id) = repo_with_run().await;
    let scene_id = "scene:trace1";

    // draft (agent)
    authoring_emit_step_events(
        &repo,
        &run_id,
        &NextAction::DraftScene {
            chapter_number: 1,
            scene_order: 1,
        },
        &trace_state(scene_id, ScenePhase::DraftSaved, None, None, None, None, 0),
        "active",
        "active",
    )
    .await;
    // verify → findings
    authoring_emit_step_events(
        &repo,
        &run_id,
        &NextAction::VerifyScene {
            chapter_number: 1,
            scene_order: 1,
        },
        &trace_state(
            scene_id,
            ScenePhase::DraftSaved,
            Some("findings"),
            Some("2 finding(s) at or above warning"),
            None,
            None,
            0,
        ),
        "active",
        "active",
    )
    .await;
    // revise
    authoring_emit_step_events(
        &repo,
        &run_id,
        &NextAction::ReviseScene {
            chapter_number: 1,
            scene_order: 1,
            attempt: 1,
        },
        &trace_state(scene_id, ScenePhase::DraftSaved, None, None, None, None, 1),
        "active",
        "active",
    )
    .await;
    // verify → clean
    authoring_emit_step_events(
        &repo,
        &run_id,
        &NextAction::VerifyScene {
            chapter_number: 1,
            scene_order: 1,
        },
        &trace_state(
            scene_id,
            ScenePhase::DraftSaved,
            Some("clean"),
            Some("0 finding(s) at or above warning"),
            None,
            None,
            1,
        ),
        "active",
        "active",
    )
    .await;
    // commit
    authoring_emit_step_events(
        &repo,
        &run_id,
        &NextAction::CommitSceneChanges {
            chapter_number: 1,
            scene_order: 1,
            scene_id: scene_id.into(),
        },
        &trace_state(
            scene_id,
            ScenePhase::ChangesCommitted,
            Some("clean"),
            None,
            None,
            None,
            1,
        ),
        "active",
        "active",
    )
    .await;
    // mine → staged
    authoring_emit_step_events(
        &repo,
        &run_id,
        &NextAction::MineScene {
            chapter_number: 1,
            scene_order: 1,
        },
        &trace_state(
            scene_id,
            ScenePhase::ChangesCommitted,
            Some("clean"),
            None,
            Some("staged"),
            Some("staged 1 delta(s)"),
            1,
        ),
        "active",
        "active",
    )
    .await;
    // annotate
    authoring_emit_step_events(
        &repo,
        &run_id,
        &NextAction::AnnotateSceneBeats {
            chapter_number: 1,
            scene_order: 1,
            scene_id: scene_id.into(),
        },
        &trace_state(
            scene_id,
            ScenePhase::BeatsAnnotated,
            Some("clean"),
            None,
            Some("staged"),
            None,
            1,
        ),
        "active",
        "active",
    )
    .await;

    let events = repo.list_run_events(&run_id, None, None).await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "scene_drafted",
            "scene_verify_completed",
            "scene_revised",
            "scene_verify_completed",
            "scene_committed",
            "scene_mined",
            "beats_annotated",
        ],
        "full per-scene step trace, revise arm included, no duplicates"
    );

    // Verdict discipline: first verify is findings, second is clean.
    let verifies: Vec<&serde_json::Value> = events
        .iter()
        .filter(|e| e.kind == "scene_verify_completed")
        .map(|e| &e.payload)
        .collect();
    assert_eq!(verifies[0]["verdict"], serde_json::json!("findings"));
    assert_eq!(
        verifies[0]["finding_counts"]["actionable"],
        serde_json::json!(2)
    );
    assert_eq!(verifies[1]["verdict"], serde_json::json!("clean"));

    // scene_revised carries the attempt; scene_mined the staged count.
    let revised = events.iter().find(|e| e.kind == "scene_revised").unwrap();
    assert_eq!(revised.payload["attempt"], serde_json::json!(1));
    let mined = events.iter().find(|e| e.kind == "scene_mined").unwrap();
    assert_eq!(mined.payload["mine_status"], serde_json::json!("staged"));
    assert_eq!(mined.payload["staged_count"], serde_json::json!(1));

    // Dense seqs 1..=N (resume-token integrity).
    let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=events.len() as i64).collect::<Vec<_>>());
}
