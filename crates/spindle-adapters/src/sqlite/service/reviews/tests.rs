use super::*;

#[tokio::test]
async fn run_dual_persona_review_round_trips_local_adapter() {
    use spindle_core::models::{ContentRating, RunDualPersonaReviewInput, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Review".into(),
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
            full_text: "Brief opening prose with felt and seemed for the filter check.".into(),
            summary: "a thin sketch".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Without GEMINI_API_KEY, the model router resolves to the local
    // adapter, which exercises the derive_*_concerns / derive_review_actions
    // heuristic fallbacks deterministically. That's exactly what the
    // unit test wants — no live network calls, but the full code path
    // through the dual-persona orchestration runs.
    let out = svc
        .run_dual_persona_review(RunDualPersonaReviewInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            scene_id: saved.scene_id.clone(),
            rounds: Some(1),
        })
        .await
        .unwrap();

    assert_eq!(out.scene_id, saved.scene_id);
    assert_eq!(out.rounds_completed, 1);
    assert_eq!(out.status, "current");
    assert!(!out.review_id.is_empty());
    assert_eq!(out.review_rounds.len(), 1);
    // tone is None → derive_literary_concerns must surface the missing-tone
    // concern; derive_craft_concerns must catch the filter words.
    let round = &out.review_rounds[0];
    assert!(
        round
            .literary_critic
            .concerns
            .iter()
            .any(|c| c.contains("tone metadata is missing"))
    );
    assert!(
        round
            .craft_technician
            .concerns
            .iter()
            .any(|c| c.contains("filter words"))
    );
    // derive_review_actions also adds the tone-setter action.
    assert!(
        round
            .priority_actions
            .iter()
            .any(|a| a.contains("tone metadata"))
    );
}

#[tokio::test]
async fn dual_persona_review_rounds_top_up_across_calls_on_same_prose() {
    // Defect item 8: the checkpoint gate needs status=current AND
    // rounds_completed >= 2, but a rounds=1 review followed by another
    // rounds=1 review of the SAME prose previously REPLACED the row
    // (rounds stuck at 1), forcing a full 2-round re-run. Rounds must
    // accumulate across reviews of the same prose version, and reset only
    // when the prose actually changes.
    use spindle_core::models::{ContentRating, RunDualPersonaReviewInput, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Review Topup".into(),
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
            full_text: "First version of the prose.".into(),
            summary: "sketch".into(),
            content_rating: ContentRating::General,
            tone: Some("wry".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let review_input = RunDualPersonaReviewInput {
        project_id: proj.project_id.clone(),
        branch_id: None,
        scene_id: saved.scene_id.clone(),
        rounds: Some(1),
    };
    let first = svc
        .run_dual_persona_review(review_input.clone())
        .await
        .unwrap();
    assert_eq!(first.rounds_completed, 1);

    // Second single-round review of the SAME prose tops up to 2 rounds.
    let second = svc
        .run_dual_persona_review(review_input.clone())
        .await
        .unwrap();
    assert_eq!(
        second.rounds_completed, 2,
        "a rounds=1 review of unchanged prose must top up, not replace"
    );
    assert_eq!(second.status, "current");
    assert_eq!(second.review_rounds.len(), 2);
    assert_eq!(second.review_rounds[0].round, 1);
    assert_eq!(second.review_rounds[1].round, 2);

    // The persisted row satisfies the checkpoint gate shape.
    let branch = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();
    let row = svc
        .repository()
        .get_dual_persona_review(&branch.id, &saved.scene_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.rounds_completed, 2);
    assert_eq!(row.status, "current");

    // Changing the prose invalidates currency: the next review starts over.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Second version of the prose, revised.".into(),
        summary: "sketch".into(),
        content_rating: ContentRating::General,
        tone: Some("wry".into()),
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    let third = svc.run_dual_persona_review(review_input).await.unwrap();
    assert_eq!(
        third.rounds_completed, 1,
        "changed prose must reset the cumulative round count"
    );
}

#[test]
fn review_prompt_frame_includes_book_and_explicit_safety_context() {
    assert!(FICTION_REVIEW_PROMPT_FRAME.contains("fictional book project"));
    assert!(FICTION_REVIEW_PROMPT_FRAME.contains("manuscript prose"));
    assert!(FICTION_REVIEW_PROMPT_FRAME.contains("real-world instructions"));
    assert!(FICTION_REVIEW_PROMPT_FRAME.contains("explicit-rated scenes"));
    assert!(FICTION_REVIEW_PROMPT_FRAME.contains("consenting adults"));
    assert!(FICTION_REVIEW_PROMPT_FRAME.contains("exploitation"));
}

#[tokio::test]
async fn review_routes_can_expose_explicit_rating_binding() {
    use spindle_core::models::ConfigureAgentsInput;

    let (tmp, svc) = fresh_service_local().await;
    let config_path = tmp.path().join("review-routes.toml");
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

    let routes = svc.model_routes().unwrap();
    assert!(routes.iter().any(|route| {
        route.route_name == "review"
            && route.rating.as_deref() == Some("explicit")
            && route.model_name == "review-explicit"
    }));
}

/// Honest-skip: when the `review` route errors (unroutable HTTP endpoint,
/// no real egress), the check emits exactly one info finding that reads as
/// SKIPPED rather than clean.
#[tokio::test]
async fn deep_check_honest_skips_when_review_route_unreachable() {
    use spindle_core::models::ConfigureAgentsInput;
    let (tmp, svc) = fresh_service_local().await;
    // Point the `review` route at an unroutable localhost port (reserved
    // port 1). No real network egress; the connection fails fast.
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

    let (project_id, _promise_id) = seed_promise_payoff_project(
        &svc,
        "promise-payoff-skip",
        "At last the reclaimed crown settled on her brow. MOCK_PROMISE_PAYOFF.",
    )
    .await;

    let deep = svc
        .check_consistency(promise_payoff_input(&project_id, true))
        .await
        .unwrap();

    let findings: Vec<_> = deep
        .issues
        .iter()
        .filter(|i| i.check_type == "promise_payoff_detection")
        .collect();
    assert_eq!(
        findings.len(),
        1,
        "an unreachable review route must yield exactly one skip finding: {:?}",
        deep.issues
    );
    assert_eq!(findings[0].severity, "info");
    assert!(
        findings[0].message.to_lowercase().contains("skipped"),
        "the finding must read as skipped, not clean: {}",
        findings[0].message
    );
}

/// Rating-not-covered honest-skip (evolution §4): the payoff prompt
/// concatenates the scoped scenes' prose, so an EXPLICIT scene in the batch
/// makes the request explicit. A `review` agent that declares no explicit
/// coverage must NOT receive it — the chokepoint returns RatingNotCovered and
/// the check emits one skip finding naming route+rating, never any prose.
#[tokio::test]
async fn deep_check_honest_skips_when_review_route_not_cleared_for_rating() {
    use spindle_core::models::{ConfigureAgentsInput, ContentRating, SaveSceneDraftInput};
    let (tmp, svc) = fresh_service_local().await;
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

    let (project_id, _promise_id) = seed_promise_payoff_project(
        &svc,
        "promise-payoff-uncleared",
        "At last the reclaimed crown settled on her brow. MOCK_PROMISE_PAYOFF.",
    )
    .await;
    // Overwrite scene 1.1 with an EXPLICIT rating so the batch max is explicit.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "At last the reclaimed crown settled on her brow. MOCK_PROMISE_PAYOFF.".into(),
        summary: "s".into(),
        content_rating: ContentRating::Explicit,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let deep = svc
        .check_consistency(promise_payoff_input(&project_id, true))
        .await
        .unwrap();

    let findings: Vec<_> = deep
        .issues
        .iter()
        .filter(|i| i.check_type == "promise_payoff_detection")
        .collect();
    assert_eq!(
        findings.len(),
        1,
        "an uncleared review route must yield exactly one skip finding: {:?}",
        deep.issues
    );
    assert_eq!(findings[0].severity, "info");
    assert!(findings[0].message.to_lowercase().contains("skipped"));
    assert!(findings[0].message.contains("`review`"));
    assert!(findings[0].message.contains("`explicit`"));
    assert!(findings[0].message.contains("not cleared for rating"));
}

#[tokio::test]
async fn poisoned_review_job_registry_returns_err_naming_the_lock() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _text) = project_with_scene(&svc, "poison-review").await;
    let scene = svc.repository.get_scene(&scene_id).await.unwrap();
    poison_mutex(svc.review_jobs.clone());
    let err = match svc.lookup_review_job("job-key") {
        Ok(_) => panic!("lookup must fail on poisoned lock"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("review job registry lock"),
        "lookup must name the poisoned lock, got: {err:?}"
    );
    let err = svc.clear_review_job("job-key", "job-id").unwrap_err();
    assert!(
        err.to_string().contains("review job registry lock"),
        "clear must name the poisoned lock, got: {err:?}"
    );
    let input = spindle_core::models::RunDualPersonaReviewInput {
        project_id: project.project_id.clone(),
        branch_id: None,
        scene_id: scene_id.clone(),
        rounds: Some(1),
    };
    let err = match svc.spawn_review_job("job-key", &input, &scene, 1, vec![]) {
        Ok(_) => panic!("spawn must fail on poisoned lock"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("review job registry lock"),
        "spawn must name the poisoned lock, got: {err:?}"
    );
}

/// Regression for defect item 8: after a scene is revised, the next review
/// call must not report the PREVIOUS review's `stale` status and id while the
/// new job is still running. It reports `in_progress` with the new job id (and
/// the new review id once a round has banked), and a second call joins that
/// same job instead of starting another.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revised_scene_reports_in_progress_with_new_job_not_stale_review() {
    use spindle_core::models::{ContentRating, RunDualPersonaReviewInput, SaveSceneDraftInput};

    // Each round outlives REVIEW_JOB_JOIN_WAIT (150ms under cfg!(test)), so the
    // post-revision call returns while its job is genuinely still running.
    let (_tmp, svc, project_id, scene_id) =
        slow_review_fixture(std::time::Duration::from_millis(800)).await;
    let branch_id = svc
        .repository
        .get_active_branch(&project_id)
        .await
        .unwrap()
        .id;
    let request = || RunDualPersonaReviewInput {
        project_id: project_id.clone(),
        branch_id: None,
        scene_id: scene_id.clone(),
        rounds: Some(1),
    };

    // Finish a first review of the original text.
    let first = svc.run_dual_persona_review(request()).await.unwrap();
    assert_eq!(first.status, "in_progress");
    let first_job_id = first.job_id.clone().expect("running review has a job id");
    await_review_rounds(&svc, &branch_id, &scene_id, 1).await;
    let old_review_id = svc
        .repository
        .get_dual_persona_review(&branch_id, &scene_id)
        .await
        .unwrap()
        .expect("first review row")
        .id;

    // Revise the prose: the old row becomes `stale`.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "The wall held, and then the horns sounded once more at dawn.".into(),
        summary: "Quiet watch, broken.".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // The reported defect: this used to return `status: "stale"`, the OLD
    // review id, and zero rounds while the new job was running.
    let second = svc.run_dual_persona_review(request()).await.unwrap();
    assert_eq!(
        second.status, "in_progress",
        "a running review of the revised text must not report stale: {second:?}"
    );
    assert_eq!(
        second.review_id, "",
        "no review exists for the revised text yet, so no id may be returned"
    );
    assert_ne!(second.review_id, old_review_id);
    let second_job_id = second
        .job_id
        .clone()
        .expect("the new running review must expose its job id");
    assert_ne!(
        second_job_id, first_job_id,
        "a revised scene starts a new job"
    );
    assert_eq!(second.rounds_completed, 0);
    assert!(second.review_rounds.is_empty());
    assert!(
        second
            .next_action
            .as_deref()
            .is_some_and(|action| action.contains("still running")),
        "an in-progress result must say how to collect it: {second:?}"
    );

    // A retry joins the same job rather than spawning another.
    let joined = svc.run_dual_persona_review(request()).await.unwrap();
    assert_eq!(joined.status, "in_progress");
    assert_eq!(joined.job_id.as_deref(), Some(second_job_id.as_str()));
    assert_eq!(joined.review_id, "");
    assert_ne!(joined.review_id, old_review_id);

    // Completion produces a NEW review row for the revised text.
    await_review_rounds(&svc, &branch_id, &scene_id, 1).await;
    let revised_row = svc
        .repository
        .get_dual_persona_review(&branch_id, &scene_id)
        .await
        .unwrap()
        .expect("revised review row");
    assert_eq!(revised_row.status, "current");
    let new_review_id = revised_row.id;
    assert_ne!(new_review_id, old_review_id);

    // Collect through the tool deterministically: the job's zero-delay reaper
    // may already have removed its handle, so stand in the finished handle for
    // the row the real job just persisted.
    let scene = svc.repository.get_scene(&scene_id).await.unwrap();
    let fingerprint = crate::sqlite::repository::Repository::scene_revision_fingerprint(&scene);
    let (_tx, rx) = tokio::sync::watch::channel(ReviewJobState::Done);
    svc.review_jobs.lock().unwrap().insert(
        format!("{branch_id}|{scene_id}|{fingerprint}"),
        ReviewJob {
            job_id: "review-job-collect".to_string(),
            target_rounds: 1,
            state: rx,
        },
    );
    let collected = svc.run_dual_persona_review(request()).await.unwrap();
    assert_eq!(collected.status, "current");
    assert_eq!(collected.review_id, new_review_id);
    assert_ne!(collected.review_id, old_review_id);
    assert_eq!(collected.rounds_completed, 1);
    assert_eq!(collected.review_rounds.len(), 1);
}

/// `stale` is only honest when the review belongs to superseded prose AND no
/// job is running for the current text. A finished job that persisted nothing
/// for the revised text is exactly that state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_is_reported_only_when_no_job_runs_for_the_current_text() {
    use spindle_core::models::{ContentRating, RunDualPersonaReviewInput, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Review Stale".into(),
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
            full_text: "First version of the prose.".into(),
            summary: "sketch".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let request = RunDualPersonaReviewInput {
        project_id: proj.project_id.clone(),
        branch_id: None,
        scene_id: saved.scene_id.clone(),
        rounds: Some(1),
    };
    let first = svc.run_dual_persona_review(request.clone()).await.unwrap();
    assert_eq!(first.status, "current");
    let old_review_id = first.review_id;

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Second version of the prose, revised.".into(),
        summary: "sketch".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let branch_id = svc
        .repository
        .get_active_branch(&proj.project_id)
        .await
        .unwrap()
        .id;
    let scene = svc.repository.get_scene(&saved.scene_id).await.unwrap();
    let fingerprint = crate::sqlite::repository::Repository::scene_revision_fingerprint(&scene);
    let job_key = format!("{branch_id}|{}|{fingerprint}", saved.scene_id);
    // A finished job for the current text that left no matching row: no job is
    // running, so `stale` is the honest status.
    let (_tx, rx) = tokio::sync::watch::channel(ReviewJobState::Done);
    svc.review_jobs.lock().unwrap().insert(
        job_key,
        ReviewJob {
            job_id: "review-job-finished".to_string(),
            target_rounds: 1,
            state: rx,
        },
    );

    let stale = svc.run_dual_persona_review(request).await.unwrap();
    assert_eq!(stale.status, "stale");
    assert_eq!(stale.review_id, old_review_id);
    assert_eq!(stale.rounds_completed, 0);
    assert!(stale.review_rounds.is_empty());
    assert!(
        stale
            .next_action
            .as_deref()
            .is_some_and(|action| action.contains("changed")),
        "a stale result must explain the next step: {stale:?}"
    );
    assert!(
        svc.lookup_review_job(&format!("{branch_id}|{}|{fingerprint}", saved.scene_id))
            .unwrap()
            .is_none(),
        "the finished job must be cleared"
    );
}

/// Cleanup removes a registry entry only when the caller's job id still owns
/// it, so an old job's delayed reaper cannot evict its replacement.
#[tokio::test]
async fn review_job_cleanup_is_identity_safe() {
    let (_tmp, svc) = fresh_service_local().await;
    let (_tx_a, rx_a) = tokio::sync::watch::channel(ReviewJobState::Running);
    svc.review_jobs.lock().unwrap().insert(
        "shared-key".to_string(),
        ReviewJob {
            job_id: "review-job-a".to_string(),
            target_rounds: 1,
            state: rx_a,
        },
    );

    // A different job's cleanup must not remove job A.
    assert!(!svc.clear_review_job("shared-key", "review-job-b").unwrap());
    assert_eq!(
        svc.lookup_review_job("shared-key")
            .unwrap()
            .expect("job A survives")
            .job_id,
        "review-job-a"
    );

    // A replacement reuses the key; job A's late reaper must not evict it.
    let (_tx_b, rx_b) = tokio::sync::watch::channel(ReviewJobState::Running);
    svc.review_jobs.lock().unwrap().insert(
        "shared-key".to_string(),
        ReviewJob {
            job_id: "review-job-b".to_string(),
            target_rounds: 1,
            state: rx_b,
        },
    );
    assert!(!svc.clear_review_job("shared-key", "review-job-a").unwrap());
    assert_eq!(
        svc.lookup_review_job("shared-key")
            .unwrap()
            .expect("replacement survives")
            .job_id,
        "review-job-b"
    );

    // The owning job removes itself.
    assert!(svc.clear_review_job("shared-key", "review-job-b").unwrap());
    assert!(svc.lookup_review_job("shared-key").unwrap().is_none());
}
