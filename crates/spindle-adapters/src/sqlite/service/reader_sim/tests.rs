use super::*;

// ── Reader-simulation chapter pass (evolution §3.6, P3.4) ───────────────

#[tokio::test]
async fn reader_sim_chapter_reads_prose_and_carries_prior_notes() {
    // Test 1 (memory flows, service half): prior notes flow INTO the prompt.
    // The local `review` stub (reader_sim falls back to review) echoes a
    // PRIOR:<first 40 chars> marker into its notes when the prompt carries
    // MOCK_READER_NOTES_ECHO, so a landed PRIOR: proves the prior-notes block
    // reached the model.
    use spindle_core::models::ReaderSimChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Reader Echo").await;
    set_scene_prose(
        &svc,
        &scene_id,
        "The market woke slowly. MOCK_READER_NOTES_ECHO drifted through the stalls.",
    )
    .await;

    let prior = "Chapter 1 landed well; the reader trusts the narrator.";
    let out = svc
        .reader_sim_chapter(ReaderSimChapterInput {
            project_id: project.project_id.clone(),
            scene_ids: vec![scene_id.clone()],
            rating: Some("general".to_string()),
            prior_notes: prior.to_string(),
        })
        .await
        .unwrap();

    assert_eq!(out.status, "read", "parseable read: {out:?}");
    assert_eq!(out.engagement, "steady");
    assert!(
        out.notes.contains("PRIOR:"),
        "notes must echo the prior-notes marker so memory flow is provable: {}",
        out.notes
    );
    // The echoed prefix is the first 40 chars of the prior notes.
    let expected: String = prior.chars().take(40).collect();
    assert!(
        out.notes.contains(&expected),
        "echoed PRIOR must carry the first 40 chars of prior notes: {}",
        out.notes
    );
}

#[tokio::test]
async fn reader_sim_chapter_dip_surfaces_warning_concern() {
    // Test 2 (dip, service half): MOCK_READER_DIP → dipping + one warning
    // concern about the retread.
    use spindle_core::models::ReaderSimChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Reader Dip").await;
    set_scene_prose(
        &svc,
        &scene_id,
        "Back to the market. MOCK_READER_DIP and the same haggling as before.",
    )
    .await;

    let out = svc
        .reader_sim_chapter(ReaderSimChapterInput {
            project_id: project.project_id.clone(),
            scene_ids: vec![scene_id.clone()],
            rating: Some("general".to_string()),
            prior_notes: String::new(),
        })
        .await
        .unwrap();

    assert_eq!(out.status, "read");
    assert_eq!(out.engagement, "dipping");
    assert_eq!(out.concerns.len(), 1, "one warning concern: {out:?}");
    assert_eq!(out.concerns[0].severity, "warning");
    assert!(
        out.concerns[0].description.contains("market"),
        "concern names the retread: {:?}",
        out.concerns[0]
    );
}

#[tokio::test]
async fn reader_sim_chapter_skips_when_rating_uncovered() {
    // Test 3 (clearance skip, service half): the review agent covers only
    // non-explicit ratings, the chapter's rating is explicit → the pass is
    // skipped with a route+rating reason, and NO prose is dispatched.
    use spindle_core::models::{ConfigureAgentsInput, ContentRating, ReaderSimChapterInput};
    let (tmp, svc) = fresh_service_local().await;
    let config_path = tmp.path().join("reader-routes.toml");
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
ratings = ["general", "teen", "mature"]

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

    let (project, scene_id, _) = project_with_scene(&svc, "Reader Explicit").await;
    let scene = svc.repository().get_scene(&scene_id).await.unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: scene.book_number,
        chapter_number: scene.chapter_number,
        chapter_id: None,
        scene_order: scene.scene_order,
        full_text: "MOCK_READER_DIP explicit prose here.".to_string(),
        summary: "explicit".to_string(),
        content_rating: ContentRating::Explicit,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let dispatch_log = svc
        .repository()
        .model_router()
        .install_dispatch_recorder()
        .unwrap();
    let out = svc
        .reader_sim_chapter(ReaderSimChapterInput {
            project_id: project.project_id.clone(),
            scene_ids: vec![scene_id.clone()],
            rating: Some("general".to_string()),
            prior_notes: String::new(),
        })
        .await
        .unwrap();

    assert_eq!(out.status, "skipped");
    assert!(out.concerns.is_empty());
    let reason = out.skip_reason.expect("skip reason present");
    assert!(
        reason.contains("explicit"),
        "reason names the uncovered rating: {reason}"
    );
    // No dispatch carried the prose (the chokepoint rejected before any call).
    let records = dispatch_log.lock().unwrap();
    assert!(
        !records
            .iter()
            .any(|r| matches!(r, crate::ai::DispatchRecord::Dispatch { .. })),
        "no prose may be dispatched on a rating-skip: {records:?}"
    );
}

#[test]
fn reader_sim_parse_rejects_malformed_and_normalizes_valid() {
    // Test 5 (malformed, parse half): a non-JSON stub (the default review
    // block) parses to None so the caller keeps prior notes; valid JSON
    // normalizes engagement + concern severities to the vocabulary.
    // Malformed / non-JSON → None (caller preserves prior notes).
    assert!(super::parse_reader_sim_output("STRENGTHS:\n- ok\nCONCERNS:\n- none").is_none());
    assert!(super::parse_reader_sim_output("").is_none());
    assert!(super::parse_reader_sim_output("{not json").is_none());

    // Valid JSON normalizes.
    let parsed = super::parse_reader_sim_output(
            r#"{"engagement":"DIPPING","notes":"n","concerns":[{"severity":"WARNING","description":"d"},{"severity":"chatty","description":"e"}]}"#,
        )
        .expect("valid reader-sim JSON parses");
    assert_eq!(parsed.engagement, "dipping");
    assert_eq!(parsed.notes, "n");
    assert_eq!(parsed.concerns.len(), 2);
    assert_eq!(parsed.concerns[0].severity, "warning");
    // Out-of-vocabulary severity coerces to the safe (info) end.
    assert_eq!(parsed.concerns[1].severity, "info");

    // Unknown engagement coerces to the signal-bearing (dipping) end.
    let odd = super::parse_reader_sim_output(r#"{"engagement":"meh","notes":""}"#).expect("parses");
    assert_eq!(odd.engagement, "dipping");
}

#[tokio::test]
async fn reader_sim_chapter_transport_error_skips_honestly() {
    // Test 5 companion: a transport failure (dead HTTP review agent) is an
    // honest skip, never a fake clean read; prior notes are not fabricated.
    use spindle_core::models::{ConfigureAgentsInput, ReaderSimChapterInput};
    let (tmp, svc) = fresh_service_local().await;
    let config_path = tmp.path().join("reader-dead.toml");
    std::fs::write(
        &config_path,
        r#"
[health_check]
enabled = false

[[agents]]
id = "review-dead"
name = "Dead Review"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:1/v1"
model = "model"
ratings = ["general", "teen", "mature", "explicit"]

[[routing]]
route = "review"
agent = "review-dead"
"#,
    )
    .unwrap();
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.display().to_string()),
    })
    .unwrap();
    let (project, scene_id, _) = project_with_scene(&svc, "Reader Transport").await;
    set_scene_prose(&svc, &scene_id, "A plain chapter, nothing special.").await;

    let out = svc
        .reader_sim_chapter(ReaderSimChapterInput {
            project_id: project.project_id.clone(),
            scene_ids: vec![scene_id.clone()],
            rating: Some("general".to_string()),
            prior_notes: String::new(),
        })
        .await
        .unwrap();
    assert_eq!(out.status, "skipped", "transport error must skip: {out:?}");
    assert!(out.skip_reason.is_some());
}

#[tokio::test]
async fn reader_sim_chapter_skips_empty_prose_without_model_call() {
    // No prose across the chapter's scenes → honest skip, no model call.
    use spindle_core::models::ReaderSimChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _scene_id, _) = project_with_scene(&svc, "Reader Empty").await;

    let dispatch_log = svc
        .repository()
        .model_router()
        .install_dispatch_recorder()
        .unwrap();
    let out = svc
        .reader_sim_chapter(ReaderSimChapterInput {
            project_id: project.project_id.clone(),
            scene_ids: Vec::new(),
            rating: Some("general".to_string()),
            prior_notes: String::new(),
        })
        .await
        .unwrap();
    assert_eq!(out.status, "skipped");
    assert!(
        dispatch_log.lock().unwrap().is_empty(),
        "no dispatch on empty scenes"
    );
}
