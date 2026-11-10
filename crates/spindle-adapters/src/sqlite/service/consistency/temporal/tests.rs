use super::*;

/// Happy path for `commit_scene_changes`: character state, canonical fact,
/// and first-contact relationship updates succeed; the response carries
/// the per-entry result rows, no errors, and a zero-finding summary
/// (Phase-4 fan-out is gated, so `findings_summary` is documented as
/// empty).
#[tokio::test]
async fn story_clock_round_trip_persists_calendar_and_scene_clock() {
    use spindle_core::models::{
        CalendarDef, CalendarMonth, ContentRating, LeapYearRule, SaveSceneDraftInput, StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "clock".into(),
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
            full_text: "The bells rang at dawn.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let repo = svc.repository();
    let branch_id = repo.get_scene(&scene.scene_id).await.unwrap().branch_id;

    // A non-24h invented calendar round-trips intact.
    let calendar = CalendarDef {
        days_per_week: 5,
        hours_per_day: 20,
        week_day_names: vec![
            "Ash".into(),
            "Ember".into(),
            "Frost".into(),
            "Gale".into(),
            "Tide".into(),
        ],
        months: vec![CalendarMonth {
            name: "Longdark".into(),
            days: 300,
        }],
        days_per_year: 300,
        epoch_label: Some("Founding".into()),
        leap_rule: LeapYearRule::Custom {
            month: "Longdark".into(),
            every_years: 3,
            extra_days: 2,
            anchor_year: 0,
        },
    };
    repo.upsert_project_calendar(&proj.project_id, &calendar)
        .await
        .unwrap();
    let loaded = repo
        .get_project_calendar(&proj.project_id)
        .await
        .unwrap()
        .expect("calendar persisted");
    assert_eq!(loaded.calendar, calendar);
    assert!(loaded.calendar.is_leap_year(3));
    assert_eq!(loaded.calendar.days_in_year(3), 302);

    let clock = StoryClock {
        day_index: Some(412),
        time_of_day: Some(540),
        duration_days: Some(1.5),
        precision: Some("day".into()),
    };
    repo.upsert_scene_clock(
        &scene.scene_id,
        &proj.project_id,
        &branch_id,
        &clock,
        Some("flashback"),
        Some("main"),
    )
    .await
    .unwrap();
    let stored = repo
        .get_scene_clock(&scene.scene_id)
        .await
        .unwrap()
        .expect("scene clock persisted");
    assert_eq!(stored.clock, clock);
    assert_eq!(stored.temporal_mode.as_deref(), Some("flashback"));
    assert_eq!(stored.thread_key.as_deref(), Some("main"));
    // The clock folds into a total-order index under the (non-24h) calendar.
    assert_eq!(
        stored.clock.total_index(&calendar),
        Some(412 * 20 * 60 + 540)
    );
}

#[tokio::test]
async fn chronology_check_flags_unmarked_backward_time_jump() {
    use spindle_core::models::{
        CalendarDef, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        SaveSceneDraftInput, StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "chrono".into(),
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

    let scene1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Day ten.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let scene2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Earlier that month.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let repo = svc.repository();
    let branch_id = repo.get_scene(&scene1.scene_id).await.unwrap().branch_id;
    let calendar = CalendarDef {
        days_per_week: 7,
        hours_per_day: 24,
        week_day_names: Vec::new(),
        months: Vec::new(),
        days_per_year: 365,
        epoch_label: None,
        leap_rule: Default::default(),
    };
    repo.upsert_project_calendar(&proj.project_id, &calendar)
        .await
        .unwrap();
    repo.upsert_scene_clock(
        &scene1.scene_id,
        &proj.project_id,
        &branch_id,
        &StoryClock {
            day_index: Some(10),
            ..Default::default()
        },
        None,
        Some("main"),
    )
    .await
    .unwrap();
    // scene2 is set earlier (day 5) without a flashback marker.
    repo.upsert_scene_clock(
        &scene2.scene_id,
        &proj.project_id,
        &branch_id,
        &StoryClock {
            day_index: Some(5),
            ..Default::default()
        },
        None,
        Some("main"),
    )
    .await
    .unwrap();

    let input = || CheckConsistencyInput {
        deep_scan_offset: None,
        project_id: proj.project_id.clone(),
        scope: ConsistencyScopeInput::full(),
        checks: vec!["chronology".to_string()],
        severity_filter: Vec::new(),
        deep_check: Some(false),
        subjects: Vec::new(),
        format: None,
        budget_tokens: None,
    };

    let flagged = svc.check_consistency(input()).await.unwrap();
    assert!(
        flagged.issues.iter().any(|i| i.check_type == "chronology"),
        "unmarked backward time jump should be flagged: {:?}",
        flagged.issues
    );

    // Marking scene2 as a flashback clears the finding.
    repo.upsert_scene_clock(
        &scene2.scene_id,
        &proj.project_id,
        &branch_id,
        &StoryClock {
            day_index: Some(5),
            ..Default::default()
        },
        Some("flashback"),
        Some("main"),
    )
    .await
    .unwrap();
    let cleared = svc.check_consistency(input()).await.unwrap();
    assert!(
        !cleared.issues.iter().any(|i| i.check_type == "chronology"),
        "flashback-marked scene must not be flagged: {:?}",
        cleared.issues
    );
}

/// An unmarked morning→night jump in one scene's prose must surface a
/// `temporal_coherence` warning — the intra-scene analog of the chronology
/// check. Works with no calendar and no clock (prose-only).
#[tokio::test]
async fn check_consistency_flags_temporal_teleport() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "temporal".into(),
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
        full_text: "He woke in the grey morning and ate his bread at the long table. \
                        The midnight harbour lay black and still around him as he stood on \
                        the quay."
            .into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let input = |checks: Vec<String>| CheckConsistencyInput {
        deep_scan_offset: None,
        project_id: proj.project_id.clone(),
        scope: ConsistencyScopeInput::full(),
        checks,
        severity_filter: Vec::new(),
        deep_check: Some(false),
        subjects: Vec::new(),
        format: None,
        budget_tokens: None,
    };

    let flagged = svc.check_consistency(input(Vec::new())).await.unwrap();
    assert!(
        flagged
            .issues
            .iter()
            .any(|i| i.check_type == "temporal_coherence" && i.severity == "warning"),
        "an unmarked morning->night jump must surface a temporal_coherence warning: {:?}",
        flagged.issues
    );

    // Narrowing to another check must not produce temporal_coherence issues.
    let filtered = svc
        .check_consistency(input(vec!["chronology".to_string()]))
        .await
        .unwrap();
    assert!(
        !filtered
            .issues
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "temporal_coherence must respect the check filter: {:?}",
        filtered.issues
    );
}

/// A scene explicitly marked as a flashback may carry a hard time jump; the
/// temporal_coherence check must not flag it.
#[tokio::test]
async fn check_consistency_flashback_scene_not_flagged_for_temporal_coherence() {
    use spindle_core::models::{
        CalendarDef, CheckConsistencyInput, ConsistencyScopeInput, ContentRating,
        SaveSceneDraftInput, StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "temporal-fb".into(),
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
            full_text: "He woke in the grey morning and ate his bread. The midnight \
                            harbour lay black and still around him."
                .into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let repo = svc.repository();
    let branch_id = repo.get_scene(&scene.scene_id).await.unwrap().branch_id;
    let calendar = CalendarDef {
        days_per_week: 7,
        hours_per_day: 24,
        week_day_names: Vec::new(),
        months: Vec::new(),
        days_per_year: 365,
        epoch_label: None,
        leap_rule: Default::default(),
    };
    repo.upsert_project_calendar(&proj.project_id, &calendar)
        .await
        .unwrap();
    repo.upsert_scene_clock(
        &scene.scene_id,
        &proj.project_id,
        &branch_id,
        &StoryClock {
            day_index: Some(5),
            ..Default::default()
        },
        Some("flashback"),
        Some("main"),
    )
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "a flashback-marked scene must not be flagged for temporal_coherence: {:?}",
        out.issues
    );
}

/// Revise-time: revising clean prose INTO an unmarked jump recomputes the
/// temporal findings on the revised text.
#[tokio::test]
async fn revise_scene_recomputes_temporal_findings() {
    use spindle_core::models::{
        ContentRating, CreateBranchInput, ReviseSceneInput, SaveSceneDraftInput, SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "revise-temporal".into(),
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
            full_text: "He sat at the long table in the grey morning and ate his bread.".into(),
            summary: "clean".into(),
            content_rating: ContentRating::General,
            tone: None,
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
            full_text: TELEPORT_PROSE.into(),
            summary: "now jumps".into(),
            content_rating: ContentRating::General,
            tone: None,
        })
        .await
        .unwrap();
    assert!(
        out.temporal_findings
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "revise_scene must recompute temporal findings on the revised prose: {:?}",
        out.temporal_findings
    );
}

/// A scene's `location_id` round-trips through `save_scene_draft` →
/// `get_scene` (migration V0021 + record plumbing).
#[tokio::test]
async fn scene_location_persists_through_save() {
    use spindle_core::models::{ContentRating, CreateLocationInput, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "loc-persist".into(),
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
    let loc = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Harbour Quay".into(),
            kind: "city".into(),
            realm: None,
            summary: "A quay.".into(),
            initial_state: Default::default(),
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
            full_text: "Prose.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            location_id: Some(loc.location_id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let stored = svc.repository().get_scene(&scene.scene_id).await.unwrap();
    assert_eq!(
        stored.location_id.as_deref(),
        Some(loc.location_id.as_str()),
        "scene location_id must round-trip through save"
    );
}

/// The pre-draft anchor names the previous scene's LOCATION (where the last
/// scene ended) alongside its clock, when the prior scene was saved with one.
#[tokio::test]
async fn temporal_anchor_names_prior_scene_location() {
    use spindle_core::models::{
        CalendarDef, ContentRating, ContextFormat, CreateLocationInput, GetSceneContextInput,
        SaveSceneDraftInput, StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "loc-anchor".into(),
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
    let quay = svc
        .create_location(CreateLocationInput {
            project_id: proj.project_id.clone(),
            name: "Harbour Quay".into(),
            kind: "city".into(),
            realm: None,
            summary: "A quay.".into(),
            initial_state: Default::default(),
        })
        .await
        .unwrap();
    let scene1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Prose one.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            location_id: Some(quay.location_id.clone()),
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
        full_text: "Prose two.".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let repo = svc.repository();
    let branch_id = repo.get_scene(&scene1.scene_id).await.unwrap().branch_id;
    let calendar = CalendarDef {
        days_per_week: 7,
        hours_per_day: 24,
        week_day_names: Vec::new(),
        months: Vec::new(),
        days_per_year: 365,
        epoch_label: None,
        leap_rule: Default::default(),
    };
    repo.upsert_project_calendar(&proj.project_id, &calendar)
        .await
        .unwrap();
    repo.upsert_scene_clock(
        &scene1.scene_id,
        &proj.project_id,
        &branch_id,
        &StoryClock {
            day_index: Some(5),
            time_of_day: Some(18 * 60),
            ..Default::default()
        },
        None,
        Some("main"),
    )
    .await
    .unwrap();

    let ctx = svc
        .get_scene_context(GetSceneContextInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            character_ids: Vec::new(),
            max_character_count: None,
            location_id: quay.location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: None,
            token_budget: None,
            sections: None,
        })
        .await
        .unwrap();
    let anchor = ctx
        .hard_constraints
        .iter()
        .find(|c| c.id.contains("IN-WORLD TIME"))
        .expect("temporal anchor must be present");
    assert!(
        anchor.statement.contains("Harbour Quay"),
        "anchor must name the previous scene's location: {}",
        anchor.statement
    );
}

/// Tier 2 (model-backed semantic recall): `deep_check` surfaces an
/// intra-scene time jump that the deterministic lexicon scan misses, and is
/// silent without `deep_check`. The faked `review` route returns a finding
/// only when the prose carries the test sentinel.
#[tokio::test]
async fn deep_check_surfaces_model_backed_temporal_finding() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, SaveSceneDraftInput,
    };

    // fresh_service_local: this test drives `model_router.complete` and must
    // never resolve the `review` route through a dev machine's agent config.
    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "deep-temporal".into(),
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
    // Prose with NO deterministic lexicon trigger (no part-of-day word, no
    // clock time) so any temporal_coherence issue can only come from Tier 2.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "He sat at the long table and turned the matter over and over in his \
                        mind, the figures refusing to balance. MOCK_TEMPORAL_JUMP. The lamps were \
                        lit and the room had gone cold around him, the cup at his elbow holding \
                        nothing but a skin of dregs."
            .into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let input = |deep: bool| CheckConsistencyInput {
        deep_scan_offset: None,
        project_id: proj.project_id.clone(),
        scope: ConsistencyScopeInput::full(),
        checks: vec!["temporal_coherence".to_string()],
        severity_filter: Vec::new(),
        deep_check: Some(deep),
        subjects: Vec::new(),
        format: None,
        budget_tokens: None,
    };

    // Tier 1 alone (no deep_check) finds nothing in this prose.
    let shallow = svc.check_consistency(input(false)).await.unwrap();
    assert!(
        !shallow
            .issues
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "the deterministic scan must not fire on this lexicon-free prose: {:?}",
        shallow.issues
    );

    // Tier 2 (deep_check) surfaces the model-backed finding.
    let deep = svc.check_consistency(input(true)).await.unwrap();
    assert!(
        deep.issues
            .iter()
            .any(|i| i.check_type == "temporal_coherence" && i.severity == "warning"),
        "deep_check must surface the model-backed temporal finding: {:?}",
        deep.issues
    );
}

/// Tier 2 precision: the model pass must not fabricate findings on coherent
/// prose (the faked route returns an empty findings array).
///
/// Hermetic via `fresh_service_local`: the temporal deep tier now stamps the
/// scene rating on its request, so it resolves through the rating-clearance
/// gate. `fresh_service` would read a dev machine's real `review` agent config
/// (which may not declare `general`) and honest-skip instead of running clean.
#[tokio::test]
async fn deep_check_clean_scene_has_no_temporal_finding() {
    use spindle_core::models::{
        CheckConsistencyInput, ConsistencyScopeInput, ContentRating, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "deep-clean".into(),
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
        full_text: "He sat at the long table and balanced the figures at last, and was \
                        glad of it before he rose to go."
            .into(),
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
            checks: vec!["temporal_coherence".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(true),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        !out.issues
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "the deep pass must not fabricate findings on coherent prose: {:?}",
        out.issues
    );
}

#[test]
fn temporal_deep_parse_extracts_findings() {
    let out = r#"{"findings":[{"severity":"warning","message":"jumps to night","evidence":"e"}]}"#;
    let findings = super::parse_deep_temporal_check_output(out).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].message, "jumps to night");
    assert_eq!(findings[0].severity.as_deref(), Some("warning"));
}

#[test]
fn temporal_deep_parse_handles_code_fence_and_prose() {
    // Models routinely wrap JSON in a ```json fence and add a preamble.
    let out =
        "Here is the audit:\n```json\n{\"findings\":[{\"message\":\"drifts to morning\"}]}\n```";
    let findings = super::parse_deep_temporal_check_output(out).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].message, "drifts to morning");
}

#[test]
fn temporal_deep_parse_empty_findings() {
    let findings = super::parse_deep_temporal_check_output(r#"{"findings":[]}"#).unwrap();
    assert!(findings.is_empty());
}

#[test]
fn temporal_deep_parse_rejects_non_json() {
    // The default local `review` stub returns prose, not JSON; the caller
    // turns this Err into an empty findings list (Tier 2 no-op).
    assert!(
        super::parse_deep_temporal_check_output(
            "Literary critic and craft technician both reviewed: ..."
        )
        .is_err()
    );
}

/// Rating carriage of the two pre-existing prose-bearing deep tiers — the
/// intra-scene temporal check (`deep_temporal_coherence_issues`) and the
/// semantic world-rule check (`deep_world_rule_compliance_issues`). Both
/// dispatched with `rating: None`, bypassing the clearance chokepoint, so an
/// explicit scene's prose could reach a `review` agent that never declared
/// explicit coverage. These tests pin the fix: each per-scene dispatch now
/// carries the scene's `content_rating` lowercased (the secret_leak pattern),
/// and a `RatingNotCovered` rejection emits one honest-skip info finding.
///
/// All hermetic: `fresh_service_local` + a TempDir `configure_agents` config,
/// and the dispatch recorder (`install_dispatch_recorder`) proves no prose
/// left for an uncleared agent.
mod deep_tier_rating_carriage {
    use super::*;
    use crate::ai::DispatchRecord;
    use spindle_core::models::{
        CheckConsistencyInput, ConfigureAgentsInput, ConsistencyScopeInput, ContentRating,
        CreateWorldRuleInput, SaveSceneDraftInput,
    };

    /// A `review` agent declaring exactly `ratings` (write a rated agent so
    /// clearance actually engages — a built-in local route serves every
    /// rating and is never gated). Written to a TempDir and installed.
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

    /// A `review` agent that resolves cleanly (rating clearance passes for
    /// every listed rating) but whose HTTP endpoint is a dead port (9,
    /// discard) — every model call fails with a TRANSPORT error, never a
    /// clearance rejection. This is the "dead route" fixture: the agent is
    /// configured, so `resolve_cleared_route` succeeds and a Dispatch is
    /// recorded, but `run_http` then fails to connect. Used to prove the
    /// legacy silent/degraded behavior now emits one honest transport-skip
    /// finding and stops calling the model for remaining scenes.
    fn install_unreachable_review_agent(
        tmp: &TempDir,
        svc: &SqliteSpindleService,
        ratings: &[&str],
    ) {
        let ratings_toml = ratings
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let config_path = tmp.path().join("dead-review-agent.toml");
        std::fs::write(
            &config_path,
            format!(
                r#"
[health_check]
enabled = false

[[agents]]
id = "review-dead"
name = "Review Dead"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:9/v1"
model = "review-dead"
ratings = [{ratings_toml}]

[[routing]]
route = "review"
agent = "review-dead"
"#
            ),
        )
        .unwrap();
        svc.configure_agents(ConfigureAgentsInput {
            config_path: Some(config_path.display().to_string()),
        })
        .unwrap();
    }

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

    async fn save_rated_scene(
        svc: &SqliteSpindleService,
        project_id: &str,
        full_text: &str,
        rating: ContentRating,
    ) {
        save_rated_scene_ordered(svc, project_id, full_text, rating, 1).await;
    }

    async fn save_rated_scene_ordered(
        svc: &SqliteSpindleService,
        project_id: &str,
        full_text: &str,
        rating: ContentRating,
        scene_order: i32,
    ) {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: 1,
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

    fn temporal_input(project_id: &str) -> CheckConsistencyInput {
        CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.to_string(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["temporal_coherence".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(true),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        }
    }

    fn world_rule_input(project_id: &str) -> CheckConsistencyInput {
        CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.to_string(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["world_rule_compliance".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(true),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        }
    }

    // A project-wide world rule (no `established_in` placement) applies to
    // every scene, so the deep world-rule tier dispatches for the scene.
    async fn seed_world_rule(svc: &SqliteSpindleService, project_id: &str) {
        svc.create_world_rule(CreateWorldRuleInput {
            project_id: project_id.to_string(),
            rule_name: "no remote magic".into(),
            rule_type: "magic".into(),
            description: "Every spell requires physical contact with the target.".into(),
            scan_pattern: None,
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();
    }

    const EXPLICIT_PROSE: &str = "He sat at the long table. MOCK_TEMPORAL_JUMP. UNIQUE_EXPLICIT_PROSE_SENTINEL. \
             The lamps were lit and the room had gone cold around him.";

    fn dispatched_prompts(records: &[DispatchRecord]) -> Vec<&str> {
        records
            .iter()
            .filter_map(|r| match r {
                DispatchRecord::Dispatch { prompt, .. } => Some(prompt.as_str()),
                DispatchRecord::Rejection { .. } => None,
            })
            .collect()
    }

    fn skip_findings(
        issues: &[spindle_core::models::ConsistencyIssue],
        check_type: &str,
    ) -> Vec<spindle_core::models::ConsistencyIssue> {
        issues
            .iter()
            .filter(|i| {
                i.check_type == check_type
                    && i.severity == "info"
                    && i.message.to_lowercase().contains("skipped")
            })
            .cloned()
            .collect()
    }

    // ── (a) temporal deep tier: RatingNotCovered honest-skip ───────────
    /// An EXPLICIT scene routed to a `review` agent that declares only
    /// `general` must NOT dispatch the explicit prose, and the temporal deep
    /// tier must emit one skip finding naming route + rating.
    #[tokio::test]
    async fn temporal_deep_honest_skips_and_never_dispatches_uncleared_explicit_prose() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "temporal-uncleared").await;
        install_review_agent(&tmp, &svc, &["general"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();

        save_rated_scene(&svc, &project_id, EXPLICIT_PROSE, ContentRating::Explicit).await;

        let out = svc
            .check_consistency(temporal_input(&project_id))
            .await
            .unwrap();

        // No Dispatch carried the explicit prose past the gate.
        let records = log.lock().unwrap().clone();
        for prompt in dispatched_prompts(&records) {
            assert!(
                !prompt.contains("UNIQUE_EXPLICIT_PROSE_SENTINEL"),
                "explicit prose must never dispatch to an uncleared agent: {records:?}"
            );
        }

        let skips = skip_findings(&out.issues, "temporal_coherence");
        assert_eq!(
            skips.len(),
            1,
            "exactly one temporal skip finding: {:?}",
            out.issues
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
            skips[0].message.to_lowercase().contains("temporal"),
            "the skip finding must name the temporal tier: {}",
            skips[0].message
        );
    }

    // ── (b) world-rule deep tier: RatingNotCovered honest-skip ─────────
    #[tokio::test]
    async fn world_rule_deep_honest_skips_and_never_dispatches_uncleared_explicit_prose() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "world-rule-uncleared").await;
        seed_world_rule(&svc, &project_id).await;
        install_review_agent(&tmp, &svc, &["general"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();

        save_rated_scene(&svc, &project_id, EXPLICIT_PROSE, ContentRating::Explicit).await;

        let out = svc
            .check_consistency(world_rule_input(&project_id))
            .await
            .unwrap();

        let records = log.lock().unwrap().clone();
        for prompt in dispatched_prompts(&records) {
            assert!(
                !prompt.contains("UNIQUE_EXPLICIT_PROSE_SENTINEL"),
                "explicit prose must never dispatch to an uncleared agent: {records:?}"
            );
        }

        let skips = skip_findings(&out.issues, "world_rule_compliance");
        assert_eq!(
            skips.len(),
            1,
            "exactly one world-rule skip finding: {:?}",
            out.issues
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
            skips[0].message.to_lowercase().contains("world rule"),
            "the skip finding must name the world-rule tier: {}",
            skips[0].message
        );
    }

    // ── (c) cleared config: deep tiers dispatch WITH the explicit rating ─
    /// When the `review` agent declares `explicit`, the deep tiers clear the
    /// gate and dispatch the explicit prose carrying `rating: Some("explicit")`.
    #[tokio::test]
    async fn deep_tiers_dispatch_with_explicit_rating_when_cleared() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "temporal-cleared").await;
        seed_world_rule(&svc, &project_id).await;
        install_review_agent(&tmp, &svc, &["general", "explicit"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();

        save_rated_scene(&svc, &project_id, EXPLICIT_PROSE, ContentRating::Explicit).await;

        let temporal = svc
            .check_consistency(temporal_input(&project_id))
            .await
            .unwrap();
        let world = svc
            .check_consistency(world_rule_input(&project_id))
            .await
            .unwrap();

        // No honest-skip fired for either tier — both cleared.
        assert!(
            skip_findings(&temporal.issues, "temporal_coherence").is_empty(),
            "cleared temporal tier must not skip: {:?}",
            temporal.issues
        );
        assert!(
            skip_findings(&world.issues, "world_rule_compliance").is_empty(),
            "cleared world-rule tier must not skip: {:?}",
            world.issues
        );

        // At least one Dispatch carried the explicit prose AND the explicit rating.
        let records = log.lock().unwrap().clone();
        let explicit_dispatches: Vec<_> = records
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    DispatchRecord::Dispatch { rating, prompt, .. }
                        if rating.as_deref() == Some("explicit")
                            && prompt.contains("UNIQUE_EXPLICIT_PROSE_SENTINEL")
                )
            })
            .collect();
        assert!(
            !explicit_dispatches.is_empty(),
            "a cleared deep tier must dispatch with rating=explicit: {records:?}"
        );
        // No rejection was recorded — the cleared agent covers the rating.
        assert!(
            !records
                .iter()
                .any(|r| matches!(r, DispatchRecord::Rejection { .. })),
            "no clearance rejection when the agent covers the rating: {records:?}"
        );
    }

    // ── (d) common case: general scene + general-only agent still works ─
    /// A GENERAL scene routed to a `general`-only agent must NOT skip: the
    /// deep tiers dispatch normally (rating=general is covered).
    #[tokio::test]
    async fn general_scene_general_agent_no_regression() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "temporal-general").await;
        seed_world_rule(&svc, &project_id).await;
        install_review_agent(&tmp, &svc, &["general"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();

        save_rated_scene(
            &svc,
            &project_id,
            "He sat at the long table. MOCK_TEMPORAL_JUMP. The lamps were lit.",
            ContentRating::General,
        )
        .await;

        let temporal = svc
            .check_consistency(temporal_input(&project_id))
            .await
            .unwrap();
        let world = svc
            .check_consistency(world_rule_input(&project_id))
            .await
            .unwrap();

        assert!(
            skip_findings(&temporal.issues, "temporal_coherence").is_empty(),
            "general/general must not skip temporal: {:?}",
            temporal.issues
        );
        assert!(
            skip_findings(&world.issues, "world_rule_compliance").is_empty(),
            "general/general must not skip world-rule: {:?}",
            world.issues
        );
        // The general-rated dispatches carried rating=general and cleared.
        let records = log.lock().unwrap().clone();
        assert!(
            records.iter().any(|r| matches!(
                r,
                DispatchRecord::Dispatch { rating, .. } if rating.as_deref() == Some("general")
            )),
            "a general dispatch must carry rating=general: {records:?}"
        );
        assert!(
            !records
                .iter()
                .any(|r| matches!(r, DispatchRecord::Rejection { .. })),
            "no rejection for a covered general rating: {records:?}"
        );
        // The model-backed temporal finding still surfaces (Tier 2 works).
        assert!(
            temporal
                .issues
                .iter()
                .any(|i| i.check_type == "temporal_coherence" && i.severity == "warning"),
            "the general deep temporal tier must still surface findings: {:?}",
            temporal.issues
        );
    }

    // Two general scenes, so a general-only agent CLEARS the rating gate —
    // every failure below is a TRANSPORT error (dead port 9), never a
    // clearance rejection. Both scenes carry MOCK_TEMPORAL_JUMP so, on a
    // live route, both would yield a deep temporal finding; here the route
    // is dead. Distinct scene_order keeps the two rows separate. Magic +
    // remote-action prose so the world-rule HEURISTIC fallback still fires.
    const DEAD_ROUTE_SCENE_A: &str = "He sat at the long table. MOCK_TEMPORAL_JUMP. \
             She cast the spell from across the room without touching him. \
             The lamps were lit and the room had gone cold.";
    const DEAD_ROUTE_SCENE_B: &str = "The next morning. MOCK_TEMPORAL_JUMP. \
             He worked the sigil at a distance, without contact. \
             The fire had burned down to ash.";

    fn dispatch_count(records: &[DispatchRecord]) -> usize {
        records
            .iter()
            .filter(|r| matches!(r, DispatchRecord::Dispatch { .. }))
            .count()
    }

    // ── (e) temporal deep tier: TRANSPORT error emits ONE skip, stops ──
    /// A dead `review` route over TWO cleared scenes: the temporal deep tier
    /// must emit exactly ONE info skip finding (not one per scene) and must
    /// NOT dispatch the model for the second scene once the first transport
    /// failure lands — a dead route won't heal mid-run. Deterministic Tier-1
    /// temporal findings are unaffected.
    #[tokio::test]
    async fn temporal_deep_transport_error_emits_one_skip_and_stops() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "temporal-dead-route").await;
        install_unreachable_review_agent(&tmp, &svc, &["general"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();

        save_rated_scene_ordered(
            &svc,
            &project_id,
            DEAD_ROUTE_SCENE_A,
            ContentRating::General,
            1,
        )
        .await;
        save_rated_scene_ordered(
            &svc,
            &project_id,
            DEAD_ROUTE_SCENE_B,
            ContentRating::General,
            2,
        )
        .await;

        let out = svc
            .check_consistency(temporal_input(&project_id))
            .await
            .unwrap();

        // Exactly ONE dispatch attempt: the first scene fails on transport,
        // the run stops calling the model for the second scene.
        let records = log.lock().unwrap().clone();
        assert_eq!(
            dispatch_count(&records),
            1,
            "emit-once-and-stop: only the first scene's model call is attempted: {records:?}"
        );

        // Exactly ONE info skip finding for the whole tier, not per-scene.
        let skips = skip_findings(&out.issues, "temporal_coherence");
        assert_eq!(
            skips.len(),
            1,
            "exactly one temporal transport-skip finding: {:?}",
            out.issues
        );
        assert!(
            skips[0].message.to_lowercase().contains("temporal"),
            "the skip finding must name the temporal tier: {}",
            skips[0].message
        );
        assert!(
            skips[0].message.contains("no usable review route"),
            "the skip finding must read as a dead-route/transport skip: {}",
            skips[0].message
        );
        // Deterministic Tier-1 temporal findings are unaffected: the scan
        // still runs. (There may be zero, but the tier must not error out.)
        assert!(
            out.issues
                .iter()
                .all(|i| i.check_type == "temporal_coherence"),
            "only temporal findings for a temporal-only check: {:?}",
            out.issues
        );
    }

    // ── (f) world-rule deep tier: TRANSPORT error emits ONE degradation ─
    /// A dead `review` route over TWO cleared scenes: the world-rule deep
    /// tier must emit exactly ONE info degradation finding, KEEP the
    /// deterministic heuristic coverage (real findings still produced), and
    /// stop attempting model calls after the first transport failure.
    #[tokio::test]
    async fn world_rule_deep_transport_error_emits_one_degradation_and_keeps_heuristic() {
        let (tmp, svc) = fresh_service_local().await;
        let project_id = seed_project(&svc, "world-rule-dead-route").await;
        seed_world_rule(&svc, &project_id).await;
        install_unreachable_review_agent(&tmp, &svc, &["general"]);
        let log = svc
            .repository
            .model_router()
            .install_dispatch_recorder()
            .unwrap();

        save_rated_scene_ordered(
            &svc,
            &project_id,
            DEAD_ROUTE_SCENE_A,
            ContentRating::General,
            1,
        )
        .await;
        save_rated_scene_ordered(
            &svc,
            &project_id,
            DEAD_ROUTE_SCENE_B,
            ContentRating::General,
            2,
        )
        .await;

        let out = svc
            .check_consistency(world_rule_input(&project_id))
            .await
            .unwrap();

        // Only the first scene's model call is attempted before the run
        // stops calling the model (heuristic still runs for both scenes).
        let records = log.lock().unwrap().clone();
        assert_eq!(
            dispatch_count(&records),
            1,
            "emit-once-and-stop: only the first scene's model call is attempted: {records:?}"
        );

        // Exactly ONE info DEGRADATION finding for the whole tier. It reads
        // as a degradation (not a skip) because the heuristic tier still ran.
        let degradations: Vec<_> = out
            .issues
            .iter()
            .filter(|i| {
                i.check_type == "world_rule_compliance"
                    && i.severity == "info"
                    && i.message.to_lowercase().contains("degraded")
            })
            .collect();
        assert_eq!(
            degradations.len(),
            1,
            "exactly one world-rule degradation finding: {:?}",
            out.issues
        );
        assert!(
            degradations[0]
                .message
                .to_lowercase()
                .contains("world rule"),
            "the finding must name the world-rule tier: {}",
            degradations[0].message
        );
        assert!(
            degradations[0].message.contains("no usable review route"),
            "the finding must read as a dead-route/transport degradation: {}",
            degradations[0].message
        );
        assert!(
            degradations[0].message.to_lowercase().contains("heuristic"),
            "the finding must name the heuristic fallback that stays: {}",
            degradations[0].message
        );

        // The heuristic fallback STAYS: both scenes describe remote magic
        // against a contact-required rule, so real heuristic warnings are
        // still produced despite the dead model route.
        let heuristic_hits = out
            .issues
            .iter()
            .filter(|i| {
                i.check_type == "world_rule_compliance"
                    && i.severity == "warning"
                    && i.message.contains("may violate world rule")
            })
            .count();
        assert!(
            heuristic_hits >= 1,
            "heuristic world-rule coverage must survive the dead model route: {:?}",
            out.issues
        );
    }
}

#[tokio::test]
async fn set_clock_tools_validate_and_persist() {
    use spindle_core::models::{
        CalendarDef, CalendarMonth, ContentRating, SaveSceneDraftInput, SetProjectCalendarInput,
        SetSceneClockInput, StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "set-clock".into(),
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

    // Invalid calendar (months do not sum to the year) is rejected.
    assert!(
        svc.set_project_calendar(SetProjectCalendarInput {
            project_id: proj.project_id.clone(),
            calendar: CalendarDef {
                days_per_week: 7,
                hours_per_day: 24,
                week_day_names: Vec::new(),
                months: vec![CalendarMonth {
                    name: "M".into(),
                    days: 10,
                }],
                days_per_year: 365,
                epoch_label: None,
                leap_rule: Default::default(),
            },
        })
        .await
        .is_err()
    );

    // Valid calendar persists.
    svc.set_project_calendar(SetProjectCalendarInput {
        project_id: proj.project_id.clone(),
        calendar: CalendarDef {
            days_per_week: 7,
            hours_per_day: 24,
            week_day_names: Vec::new(),
            months: Vec::new(),
            days_per_year: 365,
            epoch_label: None,
            leap_rule: Default::default(),
        },
    })
    .await
    .unwrap();

    // Out-of-range time_of_day is rejected.
    assert!(
        svc.set_scene_clock(SetSceneClockInput {
            project_id: proj.project_id.clone(),
            scene_id: scene.scene_id.clone(),
            clock: StoryClock {
                day_index: Some(1),
                time_of_day: Some(2000),
                ..Default::default()
            },
            temporal_mode: None,
            thread_key: None,
        })
        .await
        .is_err()
    );

    // A valid clock persists and round-trips.
    svc.set_scene_clock(SetSceneClockInput {
        project_id: proj.project_id.clone(),
        scene_id: scene.scene_id.clone(),
        clock: StoryClock {
            day_index: Some(7),
            time_of_day: Some(600),
            duration_days: Some(1.0),
            precision: Some("day".into()),
        },
        temporal_mode: Some("flashback".into()),
        thread_key: Some("b".into()),
    })
    .await
    .unwrap();
    let stored = svc
        .repository()
        .get_scene_clock(&scene.scene_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.clock.day_index, Some(7));
    assert_eq!(stored.temporal_mode.as_deref(), Some("flashback"));
}

#[test]
fn temporal_deep_skip_issue_names_route_and_rating_on_rating_not_covered() {
    let err = anyhow::Error::new(crate::ai::RouteClearanceError::RatingNotCovered {
        route: "review".to_string(),
        rating: "explicit".to_string(),
        agent_id: "reviewer".to_string(),
    });
    let issue = super::temporal_deep_skip_issue(&err);
    assert_eq!(issue.check_type, "temporal_coherence");
    assert_eq!(issue.severity, "info");
    assert!(issue.message.contains("SKIPPED"));
    assert!(issue.message.contains("`review`"));
    assert!(issue.message.contains("`explicit`"));
    assert!(issue.message.contains("not cleared for rating"));
    assert!(issue.message.to_lowercase().contains("temporal"));
}

#[test]
fn temporal_deep_skip_issue_generic_message_for_non_clearance_error() {
    let err = anyhow::anyhow!("connection refused");
    let issue = super::temporal_deep_skip_issue(&err);
    assert!(issue.message.contains("SKIPPED"));
    assert!(issue.message.contains("no usable review route"));
}

/// The generic (non-clearance / transport) branch of the temporal helper is
/// emitted ONCE for the whole deep run, so its message must frame the deep
/// TIER (not one scene) as skipped.
#[test]
fn temporal_deep_skip_issue_generic_message_frames_the_deep_tier() {
    let err = anyhow::anyhow!("connection refused");
    let issue = super::temporal_deep_skip_issue(&err);
    assert_eq!(issue.check_type, "temporal_coherence");
    assert_eq!(issue.severity, "info");
    assert!(issue.message.contains("no usable review route"));
    assert!(
        issue.message.to_lowercase().contains("deep tier"),
        "transport-skip must frame the whole deep tier: {}",
        issue.message
    );
    // Never scoped to a single scene — one finding covers the tier.
    assert!(
        !issue.message.contains("This scene"),
        "transport-skip is once-per-run, not per-scene: {}",
        issue.message
    );
}

#[tokio::test]
async fn set_project_calendar_persists_leap_rule_and_clock_uses_leap_day() {
    use spindle_core::models::{
        CalendarDate, CalendarDef, ContentRating, LeapYearRule, SaveSceneDraftInput,
        SetProjectCalendarInput, SetSceneClockInput, StoryClock,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "leap-calendar".into(),
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

    let calendar = CalendarDef::gregorian();
    svc.set_project_calendar(SetProjectCalendarInput {
        project_id: proj.project_id.clone(),
        calendar: calendar.clone(),
    })
    .await
    .expect("Gregorian calendar must validate");

    let stored = svc
        .repository()
        .get_project_calendar(&proj.project_id)
        .await
        .unwrap()
        .expect("calendar stored");
    assert!(matches!(stored.calendar.leap_rule, LeapYearRule::Gregorian));
    assert_eq!(stored.calendar.days_in_month(2000, 2), Some(29));

    // A custom rule naming a missing month is rejected.
    let mut bad = calendar.clone();
    bad.leap_rule = LeapYearRule::Custom {
        month: "Missing".into(),
        every_years: 4,
        extra_days: 1,
        anchor_year: 0,
    };
    assert!(
        svc.set_project_calendar(SetProjectCalendarInput {
            project_id: proj.project_id.clone(),
            calendar: bad,
        })
        .await
        .is_err()
    );

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The thaw came late that year.".into(),
            summary: "leap day".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let leap_day = calendar
        .day_index_for_date(&CalendarDate {
            year: 2000,
            month: 2,
            day: 29,
        })
        .unwrap();
    svc.set_scene_clock(SetSceneClockInput {
        project_id: proj.project_id.clone(),
        scene_id: scene.scene_id.clone(),
        clock: StoryClock {
            day_index: Some(leap_day),
            time_of_day: Some(720),
            duration_days: Some(0.5),
            precision: Some("day".into()),
        },
        temporal_mode: Some("linear".into()),
        thread_key: Some("main".into()),
    })
    .await
    .unwrap();
    let loaded = svc
        .repository()
        .get_scene_clock(&scene.scene_id)
        .await
        .unwrap()
        .expect("scene clock stored");
    assert_eq!(loaded.clock.day_index, Some(leap_day));
    assert_eq!(
        calendar
            .date_for_day_index(loaded.clock.day_index.unwrap())
            .unwrap(),
        CalendarDate {
            year: 2000,
            month: 2,
            day: 29,
        }
    );
}
