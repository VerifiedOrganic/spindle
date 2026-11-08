use super::*;

#[tokio::test]
async fn append_run_event_assigns_dense_sequence_and_round_trips() {
    let (_tmp, repo, project, branch, _scene_id) = repo_with_scene().await;
    let run_id = seed_authoring_run(&repo, &project, &branch).await;

    let seq1 = repo
        .append_run_event(
            &run_id,
            "run_started",
            serde_json::json!({ "book_number": 1 }),
        )
        .await
        .unwrap();
    let seq2 = repo
        .append_run_event(
            &run_id,
            "scene_drafted",
            serde_json::json!({ "chapter": 1, "scene_order": 1, "origin": "agent" }),
        )
        .await
        .unwrap();
    let seq3 = repo
        .append_run_event(&run_id, "run_completed", serde_json::json!({}))
        .await
        .unwrap();

    assert_eq!(seq1, 1, "first event on a run is seq 1");
    assert_eq!(seq2, 2, "seq is dense and monotonic");
    assert_eq!(seq3, 3);

    let events = repo.list_run_events(&run_id, None, None).await.unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[0].kind, "run_started");
    assert_eq!(events[0].payload["book_number"], serde_json::json!(1));
    assert_eq!(events[1].kind, "scene_drafted");
    assert_eq!(events[1].payload["origin"], serde_json::json!("agent"));
    assert_eq!(events[2].kind, "run_completed");
    assert!(events[0].id.starts_with("authoring_run_event:"));
}

#[tokio::test]
async fn list_run_events_honors_after_seq_and_limit() {
    let (_tmp, repo, project, branch, _scene_id) = repo_with_scene().await;
    let run_id = seed_authoring_run(&repo, &project, &branch).await;
    for i in 0..5 {
        repo.append_run_event(&run_id, "scene_committed", serde_json::json!({ "n": i }))
            .await
            .unwrap();
    }

    // after_seq = 2 → only seq 3,4,5 (the resume-from-Last-Event-ID path).
    let after = repo.list_run_events(&run_id, Some(2), None).await.unwrap();
    assert_eq!(
        after.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![3, 4, 5]
    );

    // limit caps the window, still in ascending seq order.
    let limited = repo.list_run_events(&run_id, None, Some(2)).await.unwrap();
    assert_eq!(
        limited.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[tokio::test]
async fn run_events_survive_a_subsequent_run_state_save() {
    // Regression: the journal FK cascades on run delete. `save_authoring_run`
    // must upsert the run row in place (not DELETE+INSERT), or every state
    // persist would wipe the run's journal via ON DELETE CASCADE. This pins
    // that appended events survive re-saving the run.
    let (_tmp, repo, project, branch, _scene_id) = repo_with_scene().await;
    let run_id = seed_authoring_run(&repo, &project, &branch).await;

    repo.append_run_event(&run_id, "run_started", serde_json::json!({}))
        .await
        .unwrap();
    repo.append_run_event(&run_id, "scene_drafted", serde_json::json!({}))
        .await
        .unwrap();

    // Re-save the run (as every execute_next step does) — must not touch the
    // journal.
    let now = chrono::Utc::now();
    let run = crate::sqlite::records::AuthoringRun {
        id: run_id.clone(),
        project_id: project.id.clone(),
        active_branch_id: branch.id.clone(),
        book_number: 1,
        start_chapter: 1,
        end_chapter: 1,
        checkpoint_interval: 1,
        last_checkpoint_end_chapter: 0,
        artifacts_dir: "../artifacts".into(),
        editorial_directives: Vec::new(),
        status: "blocked".into(), // a status change
        created_at: now,
        updated_at: now,
        mining_policy: None,
        max_revise_attempts: None,
        checkpoint_policy: None,
        replan_policy: None,
    };
    repo.save_authoring_run(run, Vec::new(), Vec::new(), Vec::new())
        .await
        .unwrap();

    let events = repo.list_run_events(&run_id, None, None).await.unwrap();
    assert_eq!(
        events.len(),
        2,
        "journal must survive a run re-save (no cascade wipe)"
    );
    // The next seq continues densely from where it left off.
    let seq = repo
        .append_run_event(&run_id, "run_blocked", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(seq, 3, "seq continues densely after a run re-save");
}

#[tokio::test]
async fn run_events_sequences_are_independent_per_run() {
    let (_tmp, repo, project, branch, _scene_id) = repo_with_scene().await;
    let run_a = seed_authoring_run(&repo, &project, &branch).await;
    let run_b = seed_authoring_run(&repo, &project, &branch).await;

    let a1 = repo
        .append_run_event(&run_a, "run_started", serde_json::json!({}))
        .await
        .unwrap();
    let b1 = repo
        .append_run_event(&run_b, "run_started", serde_json::json!({}))
        .await
        .unwrap();
    let a2 = repo
        .append_run_event(&run_a, "scene_drafted", serde_json::json!({}))
        .await
        .unwrap();

    // Each run owns its own dense 1-based sequence; runs never share numbers.
    assert_eq!(a1, 1);
    assert_eq!(b1, 1);
    assert_eq!(a2, 2);

    let a_events = repo.list_run_events(&run_a, None, None).await.unwrap();
    let b_events = repo.list_run_events(&run_b, None, None).await.unwrap();
    assert_eq!(a_events.len(), 2);
    assert_eq!(b_events.len(), 1);
}
