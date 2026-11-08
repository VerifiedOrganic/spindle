use super::*;

#[test]
fn project_local_exports_resolve_to_workspace_root() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join(".spindle");
    assert_eq!(
        export_dir_for_data_dir(&data_dir),
        tmp.path().join("exports")
    );
}

#[test]
fn global_exports_stay_under_data_dir() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("spindle-data");
    assert_eq!(export_dir_for_data_dir(&data_dir), data_dir.join("exports"));
}

#[tokio::test]
async fn preflight_book_export_flags_blocking_and_warning_issues() {
    use spindle_core::models::{
        ContentRating, ExportIssueSeverity, PreflightBookExportInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Export Drill".into(),
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

    // Save a non-empty scene in chapter 1 — no blocking issues, but
    // chapter title was never set so we expect a warning.
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Real prose.".into(),
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
        .preflight_book_export(PreflightBookExportInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            start_chapter_number: None,
            end_chapter_number: None,
        })
        .await
        .unwrap();

    assert!(
        out.issues.iter().any(|i| i.code == "chapter_missing_title"
            && matches!(i.severity, ExportIssueSeverity::Warning))
    );
    assert!(
        out.issues
            .iter()
            .all(|i| i.code != "chapter_without_scenes"),
        "chapter 1 has a scene, must not be flagged blocking"
    );

    // Half-open chapter range → error.
    let err = svc
        .preflight_book_export(PreflightBookExportInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            start_chapter_number: Some(1),
            end_chapter_number: None,
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("requires both"));
}

/// Live-run bug: scenes whose `full_text` was literally "placeholder"
/// (1 word) or a handful of words passed `preflight_book_export` with
/// `issues: []` and an EPUB was exported and read before anyone noticed.
/// Stub scenes must be flagged Blocking so the export gate stops them.
#[tokio::test]
async fn preflight_flags_placeholder_and_thin_scenes_as_blocking() {
    use spindle_core::models::{ExportIssueSeverity, PreflightBookExportInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Stub Gate".into(),
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
        .preflight_book_export(PreflightBookExportInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            start_chapter_number: None,
            end_chapter_number: None,
        })
        .await
        .unwrap();

    let stub_issues: Vec<_> = out
        .issues
        .iter()
        .filter(|issue| issue.code == "scene_stub_text")
        .collect();
    let flagged_orders: Vec<i32> = stub_issues
        .iter()
        .map(|issue| issue.scene_order.unwrap_or(-1))
        .collect();
    assert!(
        flagged_orders.contains(&1),
        "the literal \"placeholder\" scene must be flagged, issues: {:?}",
        out.issues
    );
    assert!(
        flagged_orders.contains(&2),
        "the 5-word scene must be flagged below the word floor, issues: {:?}",
        out.issues
    );
    assert!(
        !flagged_orders.contains(&3),
        "the healthy scene must not be flagged, issues: {:?}",
        out.issues
    );
    assert!(
        stub_issues
            .iter()
            .all(|issue| matches!(issue.severity, ExportIssueSeverity::Blocking)),
        "stub scenes must block export so they cannot ship to EPUB unnoticed"
    );
}

/// `compile_manuscript` must name stub scenes in a dedicated field —
/// they render into the Markdown, but the compiled document may never
/// silently present them as finished prose.
#[tokio::test]
async fn compile_manuscript_reports_stub_scenes() {
    use spindle_core::models::CompileManuscriptInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Stub Compile".into(),
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
        .compile_manuscript(CompileManuscriptInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            start_chapter: None,
            end_chapter: None,
            write_to_workspace: false,
        })
        .await
        .unwrap();

    assert_eq!(out.scene_count, 3, "all drafted scenes still render");
    assert!(
        out.missing_scenes.is_empty(),
        "stub scenes exist, so nothing is missing"
    );
    assert_eq!(
        out.stub_scenes,
        vec!["1.1".to_string(), "1.2".to_string()],
        "the placeholder and thin scenes must be reported as stubs"
    );
}

#[tokio::test]
async fn compile_manuscript_assembles_headings_prose_and_flags_undrafted_scenes() {
    use spindle_core::models::{
        CompileManuscriptInput, ContentRating, PlanChapterInput, PlanChapterSceneInput,
        SaveSceneDraftInput, UpdateEntityInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Read So Far".into(),
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

    // Chapter 1: two committed scenes, no plan.
    for (order, text) in [
        (1, "Ch1 scene1 opens the tale."),
        (2, "Ch1 scene2 raises stakes."),
    ] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: order,
            full_text: text.into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    // Give chapter 1 a title so the heading renders it.
    let ch1 = svc
        .repository
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .unwrap();
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "chapter".into(),
        entity_id: ch1.id.clone(),
        changes: serde_json::json!({ "title": "The Gate" }),
    })
    .await
    .unwrap();

    // Chapter 2: plan spine of 2 scenes; only scene 2.1 persisted.
    svc.plan_chapter(PlanChapterInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 2,
        pov_character_id: None,
        synopsis: "The reckoning".into(),
        target_theme_ids: Vec::new(),
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        scenes: vec![
            PlanChapterSceneInput {
                scene_order: 1,
                summary: "drafted".into(),
                purpose: "establishing".into(),
                ..Default::default()
            },
            PlanChapterSceneInput {
                scene_order: 2,
                summary: "undrafted".into(),
                purpose: "complication".into(),
                ..Default::default()
            },
        ],
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 2,
        chapter_id: None,
        scene_order: 1,
        full_text: "Ch2 scene1 lands the blow.".into(),
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
        .compile_manuscript(CompileManuscriptInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            start_chapter: None,
            end_chapter: None,
            write_to_workspace: false,
        })
        .await
        .unwrap();

    // Ordered headings and prose bodies for chapter 1 (title included).
    assert!(out.markdown.contains("## Chapter 1: The Gate"));
    assert!(out.markdown.contains("### Scene 1.1"));
    assert!(out.markdown.contains("Ch1 scene1 opens the tale."));
    assert!(out.markdown.contains("### Scene 1.2"));
    assert!(out.markdown.contains("Ch1 scene2 raises stakes."));
    assert!(out.markdown.contains("## Chapter 2"));
    assert!(out.markdown.contains("### Scene 2.1"));
    assert!(out.markdown.contains("Ch2 scene1 lands the blow."));
    // Undrafted 2.2 renders an explicit placeholder, never silently omitted.
    assert!(out.markdown.contains("> [scene 2.2 not yet drafted]"));

    // Chapter 1 precedes chapter 2 in the assembled document.
    let ch1 = out.markdown.find("## Chapter 1").unwrap();
    let ch2 = out.markdown.find("## Chapter 2").unwrap();
    assert!(ch1 < ch2, "chapters must be assembled in order");

    assert_eq!(out.missing_scenes, vec!["2.2".to_string()]);
    assert_eq!(out.scene_count, 3);
    assert!(out.word_count > 0);
    assert!(out.artifact_path.is_none());
}

#[tokio::test]
async fn compile_manuscript_range_filters_to_requested_chapters() {
    use spindle_core::models::{
        CompileManuscriptInput, ContentRating, CreateChapterInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Range".into(),
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

    // Chapter 1 exists from create_project; create chapter 2 explicitly.
    svc.create_chapter(CreateChapterInput {
        project_id: proj.project_id.clone(),
        book_id: None,
        book_number: Some(1),
        chapter_number: Some(2),
        title: None,
    })
    .await
    .unwrap();

    for chapter in [1, 2] {
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: chapter,
            chapter_id: None,
            scene_order: 1,
            full_text: format!("Chapter {chapter} unique body."),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    let out = svc
        .compile_manuscript(CompileManuscriptInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            start_chapter: Some(2),
            end_chapter: Some(2),
            write_to_workspace: false,
        })
        .await
        .unwrap();

    assert!(out.markdown.contains("## Chapter 2"));
    assert!(out.markdown.contains("Chapter 2 unique body."));
    assert!(!out.markdown.contains("## Chapter 1"));
    assert!(!out.markdown.contains("Chapter 1 unique body."));
    assert_eq!(out.scene_count, 1);
}

#[tokio::test]
async fn compile_manuscript_writes_deterministic_artifact_inside_workspace() {
    use spindle_core::models::{CompileManuscriptInput, ContentRating, SaveSceneDraftInput};
    use std::path::Path;

    let (tmp, svc) = fresh_service_workspace().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Artifact".into(),
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
        full_text: "Persisted prose.".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let input = || CompileManuscriptInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        start_chapter: None,
        end_chapter: None,
        write_to_workspace: true,
    };

    let first = svc.compile_manuscript(input()).await.unwrap();
    let path = first.artifact_path.clone().expect("artifact path");
    assert!(
        path.ends_with("book-1-ch1-1.md"),
        "deterministic filename: {path}"
    );

    // The written path canonicalizes to inside the canonicalized workspace
    // root, checked component-wise (never string prefix).
    let workspace_root = tmp.path().canonicalize().unwrap();
    let written = Path::new(&path).canonicalize().unwrap();
    assert!(
        written.starts_with(&workspace_root),
        "{written:?} must live inside {workspace_root:?}"
    );

    // Calling twice overwrites: same path, no duplicate files.
    let second = svc.compile_manuscript(input()).await.unwrap();
    assert_eq!(second.artifact_path.as_deref(), Some(path.as_str()));
    let artifacts_dir = written.parent().unwrap();
    let md_count = std::fs::read_dir(artifacts_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .count();
    assert_eq!(md_count, 1, "overwrite must not duplicate files");
}

#[tokio::test]
async fn compile_manuscript_empty_book_yields_structured_empty_result() {
    use spindle_core::models::CompileManuscriptInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Empty Book".into(),
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

    // Compile a book number with no chapters at all — structured empty,
    // not an error.
    let out = svc
        .compile_manuscript(CompileManuscriptInput {
            project_id: proj.project_id.clone(),
            book_number: 7,
            start_chapter: None,
            end_chapter: None,
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert_eq!(out.scene_count, 0);
    assert!(out.missing_scenes.is_empty());
    assert!(out.artifact_path.is_none());
}

#[tokio::test]
async fn compile_manuscript_is_active_branch_scoped() {
    use spindle_core::models::{
        CompileManuscriptInput, ContentRating, CreateBranchInput, SaveSceneDraftInput,
        SwitchBranchInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Branchy".into(),
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

    // Scene committed on branch A only.
    let branch_a = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "alt-a".into(),
            branch_type: "alternative".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: branch_a.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Only on branch A.".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // Compile on a different active branch B: the branch-A scene is absent.
    let branch_b = svc
        .create_branch(CreateBranchInput {
            project_id: proj.project_id.clone(),
            parent_branch_id: None,
            name: "alt-b".into(),
            branch_type: "alternative".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: proj.project_id.clone(),
        branch_id: branch_b.branch_id.clone(),
    })
    .await
    .unwrap();

    let out = svc
        .compile_manuscript(CompileManuscriptInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            start_chapter: None,
            end_chapter: None,
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert!(!out.markdown.contains("Only on branch A."));
    assert_eq!(out.scene_count, 0);
}

#[tokio::test]
async fn promise_history_tracks_actual_payoff_reopening_correction_and_unknown_dates() {
    use crate::format::story_index;
    use spindle_core::models::ExportRecapInput;
    let (_tmp, svc) = fresh_service().await;
    let project_id = recap_project(&svc, "Promise history").await;
    let placement = |chapter| StoryPlacement {
        book_number: 1,
        chapter_number: chapter,
        scene_order: Some(1),
        note: None,
    };
    let id = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project_id.clone(),
            promise_type: "mystery".into(),
            description: "The missing key".into(),
            planted_at: placement(1),
            planned_payoff: Some(placement(2)),
            notes: vec![],
        })
        .await
        .unwrap()
        .narrative_promise_id;
    svc.update_promise_status(UpdatePromiseStatusInput {
        narrative_promise_id: id.clone(),
        status: "paid_off".into(),
        at: Some(placement(40)),
        note: Some("The key was recovered".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    svc.update_promise_status(UpdatePromiseStatusInput {
        narrative_promise_id: id.clone(),
        status: "reopened".into(),
        at: Some(placement(50)),
        ..Default::default()
    })
    .await
    .unwrap();
    let promise = svc.repository.get_narrative_promise(&id).await.unwrap();
    assert_eq!(promise.status_at(story_index(1, 3, 9)), Some("planted"));
    assert_eq!(promise.status_at(story_index(1, 45, 9)), Some("paid_off"));
    assert_eq!(promise.status_at(story_index(1, 50, 9)), Some("reopened"));
    assert_eq!(
        promise.status_history[0].note.as_deref(),
        Some("The key was recovered")
    );
    let correction = UpdatePromiseStatusInput {
        narrative_promise_id: id.clone(),
        status: "paid_off".into(),
        at: Some(placement(42)),
        replaces_event_id: Some(promise.status_history[0].id.clone()),
        ..Default::default()
    };
    assert_eq!(
        svc.update_promise_status(correction.clone())
            .await
            .unwrap()
            .status,
        "reopened"
    );
    assert!(svc.update_promise_status(correction).await.is_err());
    let promise = svc.repository.get_narrative_promise(&id).await.unwrap();
    assert_eq!(promise.status_at(story_index(1, 41, 9)), Some("planted"));
    assert_eq!(promise.status_at(story_index(1, 43, 9)), Some("paid_off"));
    let recap = svc
        .export_recap(ExportRecapInput {
            project_id,
            book_number: 1,
            through_chapter: 43,
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert!(
        recap
            .markdown
            .split("## Questions still hanging")
            .next()
            .unwrap()
            .contains("The missing key")
    );

    // The generic mutation path must also journal; omitting a date never
    // converts the planned chapter into an asserted historical payoff.
    svc.repository
        .update_entity_field(
            "narrative_promise",
            &id,
            "status",
            serde_json::json!("paid_off"),
        )
        .await
        .unwrap();
    let promise = svc.repository.get_narrative_promise(&id).await.unwrap();
    assert_eq!(promise.status_history.len(), 4);
    assert_eq!(promise.status_at(story_index(1, 45, 9)), None);
    let mut legacy = promise;
    legacy.status_history.clear();
    assert_eq!(legacy.status_at(i64::MAX), None);

    let (other, scene_id, _) = project_with_scene(&svc, "Other project").await;
    assert_ne!(other.project_id, legacy.project_id);
    assert!(
        svc.update_promise_status(UpdatePromiseStatusInput {
            narrative_promise_id: id.clone(),
            status: "paid_off".into(),
            source_scene_id: Some(scene_id),
            ..Default::default()
        })
        .await
        .is_err()
    );
    assert_eq!(
        svc.repository
            .get_narrative_promise(&id)
            .await
            .unwrap()
            .status_history
            .len(),
        4
    );
}

#[tokio::test]
async fn export_recap_folds_summaries_and_promises_at_cursor() {
    use spindle_core::models::{
        CreateNarrativePromiseInput, ExportRecapInput, SaveSummaryInput, StoryPlacement,
        UpdatePromiseStatusInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = recap_project(&svc, "Recap").await;

    // Three summarized chapters; chapter 3 carries a sentinel event.
    for (ch, summary) in [
        (1, "Chapter one opens the tale."),
        (2, "Chapter two raises the stakes."),
        (3, "Chapter three burns the CH3SENTINEL bridge."),
    ] {
        svc.save_summary(SaveSummaryInput {
            project_id: proj.clone(),
            book_number: 1,
            chapter_number: ch,
            entity_type: None,
            entity_id: None,
            summary: summary.into(),
            key_events: Vec::new(),
            character_changes: Vec::new(),
            relationship_shifts: Vec::new(),
            arc_advances: Vec::new(),
            promise_events: Vec::new(),
        })
        .await
        .unwrap();
    }

    // A paid-off promise whose planned payoff is at chapter 2 (<= cursor).
    let paid = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: proj.clone(),
            promise_type: "mystery".into(),
            description: "PAIDOFFPROMISE about the harbor".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: Some(1),
                note: None,
            }),
            notes: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_promise_status(UpdatePromiseStatusInput {
        narrative_promise_id: paid.narrative_promise_id.clone(),
        status: "paid_off".into(),
        at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 2,
            scene_order: Some(1),
            note: None,
        }),
        note: None,
        ..Default::default()
    })
    .await
    .unwrap();

    // An open promise planted at chapter 1 (<= cursor).
    svc.create_narrative_promise(CreateNarrativePromiseInput {
        project_id: proj.clone(),
        promise_type: "mystery".into(),
        description: "OPENPROMISE about the stranger".into(),
        planted_at: StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        planned_payoff: None,
        notes: Vec::new(),
    })
    .await
    .unwrap();

    let out = svc
        .export_recap(ExportRecapInput {
            project_id: proj.clone(),
            book_number: 1,
            through_chapter: 2,
            write_to_workspace: false,
        })
        .await
        .unwrap();

    assert!(out.word_count > 0, "recap must have prose");
    assert_eq!(out.chapter_count, 2, "only ch1+ch2 at/under cursor");
    // Chapter-3 events are spoilers past the cursor.
    assert!(
        !out.markdown.contains("CH3SENTINEL"),
        "chapter past cursor must be withheld"
    );
    assert!(out.markdown.contains("Chapter one opens the tale."));
    assert!(out.markdown.contains("Chapter two raises the stakes."));
    // Paid-off promise (payoff <= cursor) appears; open promise appears.
    assert!(out.markdown.contains("PAIDOFFPROMISE"), "paid off section");
    assert!(out.markdown.contains("OPENPROMISE"), "still open section");
    // Reader voice, not urgency jargon.
    assert!(!out.markdown.to_lowercase().contains("overdue"));
}

#[tokio::test]
async fn export_recap_and_bible_withhold_unrevealed_secret_then_reveal() {
    use spindle_core::models::{
        CreateTermInput, ExportRecapInput, ExportSeriesBibleInput, RecordKnowledgeInput,
        SaveSummaryInput, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = recap_project(&svc, "Secret Sweep").await;

    // A holder character and an anchor scene at ch1.
    let holder = create_character_named(&svc, &proj, "Vex").await;
    let scene_id = recap_scene(&svc, &proj, 1, 1, 1, "Vex walks the docks.").await;

    // A chapter summary that names the sentinel (recap prose carrier).
    svc.save_summary(SaveSummaryInput {
        project_id: proj.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "Vex conceals SENTINELSECRET from the court.".into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();

    // A glossary entry that names the sentinel (series-bible carrier), so the
    // sweep covers a bible section too, not only the recap prose.
    svc.create_term(CreateTermInput {
        project_id: proj.clone(),
        term_text: "The Concealed Line".into(),
        pronunciation: None,
        definition: "SENTINELSECRET — the bloodline nobody may speak of.".into(),
        usage_context: None,
        origin: None,
    })
    .await
    .unwrap();

    let fact_id = recap_secret_fact(&svc, &proj, &scene_id, &holder, "SENTINELSECRET").await;

    let recap_input = || ExportRecapInput {
        project_id: proj.clone(),
        book_number: 1,
        through_chapter: 5,
        write_to_workspace: false,
    };
    let bible_input = || ExportSeriesBibleInput {
        project_id: proj.clone(),
        through: None,
        write_to_workspace: false,
    };

    // Pre-reveal: sentinel in NEITHER recap NOR bible.
    let pre_recap = svc.export_recap(recap_input()).await.unwrap().markdown;
    let pre_bible = svc
        .export_series_bible(bible_input())
        .await
        .unwrap()
        .markdown;
    assert!(!pre_recap.contains("SENTINELSECRET"), "pre-reveal recap");
    assert!(!pre_bible.contains("SENTINELSECRET"), "pre-reveal bible");

    // Add a reader-visible reveal at chapter 1 (<= cursor).
    svc.record_knowledge(RecordKnowledgeInput {
        project_id: proj.clone(),
        branch_id: None,
        character_id: holder.clone(),
        fact: "the reader now learns it".into(),
        source_summary: "reveal".into(),
        learned_at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        }),
        confidence: None,
        tags: Vec::new(),
        reader_visible: true,
        secret_of_fact_id: Some(fact_id.clone()),
    })
    .await
    .unwrap();

    // Post-reveal: sentinel now appears in BOTH.
    let post_recap = svc.export_recap(recap_input()).await.unwrap().markdown;
    let post_bible = svc
        .export_series_bible(bible_input())
        .await
        .unwrap()
        .markdown;
    assert!(post_recap.contains("SENTINELSECRET"), "post-reveal recap");
    assert!(post_bible.contains("SENTINELSECRET"), "post-reveal bible");
}

#[tokio::test]
async fn export_bible_respects_reveal_timing_at_cursor() {
    use spindle_core::models::{
        CreateTermInput, ExportSeriesBibleInput, RecordKnowledgeInput, RegisterCanonicalFactInput,
        SecrecyScope, StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = recap_project(&svc, "Timing").await;
    let holder = create_character_named(&svc, &proj, "Vex").await;
    let scene_id = recap_scene(&svc, &proj, 1, 1, 1, "Vex walks the docks.").await;

    // Glossary carrier that names the sentinel value in the bible.
    svc.create_term(CreateTermInput {
        project_id: proj.clone(),
        term_text: "Heir-mark".into(),
        pronunciation: None,
        definition: "TIMINGSENTINEL — the sign of the true line.".into(),
        usage_context: None,
        origin: None,
    })
    .await
    .unwrap();

    let fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: proj.clone(),
            scene_id: Some(scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: Some("Vex:trueheir".into()),
            value: Some("TIMINGSENTINEL".into()),
            context: None,
            subject_table: None,
            subject_id: None,
            predicate: None,
            value_kind: None,
            value_text: Some("TIMINGSENTINEL".into()),
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

    // Reveal placed at chapter 3, reader-visible.
    svc.record_knowledge(RecordKnowledgeInput {
        project_id: proj.clone(),
        branch_id: None,
        character_id: holder.clone(),
        fact: "revealed at ch3".into(),
        source_summary: "reveal".into(),
        learned_at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 3,
            scene_order: Some(1),
            note: None,
        }),
        confidence: None,
        tags: Vec::new(),
        reader_visible: true,
        secret_of_fact_id: Some(fact.canonical_fact_id.clone()),
    })
    .await
    .unwrap();

    // Cursor at ch2 → reveal is in the future → withheld.
    let at_ch2 = svc
        .export_series_bible(ExportSeriesBibleInput {
            project_id: proj.clone(),
            through: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: None,
                note: None,
            }),
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert!(
        !at_ch2.markdown.contains("TIMINGSENTINEL"),
        "reveal in future"
    );

    // Cursor at ch3 → reveal reached → included.
    let at_ch3 = svc
        .export_series_bible(ExportSeriesBibleInput {
            project_id: proj.clone(),
            through: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 3,
                scene_order: None,
                note: None,
            }),
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert!(at_ch3.markdown.contains("TIMINGSENTINEL"), "reveal reached");
}

#[tokio::test]
async fn export_series_bible_state_as_of_cursor_glossary_sorted_and_empty() {
    use spindle_core::models::{
        CharacterStatePatch, CommitCharacterStateInput, CreateTermInput, ExportSeriesBibleInput,
        StoryPlacement,
    };

    let (_tmp, svc) = fresh_service().await;

    // Empty project → structured empty markdown, no error.
    let empty_proj = recap_project(&svc, "Empty Bible").await;
    let empty = svc
        .export_series_bible(ExportSeriesBibleInput {
            project_id: empty_proj,
            through: None,
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert_eq!(empty.chapter_count, 0);
    assert!(empty.artifact_path.is_none());
    assert!(
        !empty.markdown.is_empty(),
        "empty is still structured markdown"
    );

    let proj = recap_project(&svc, "Bible").await;
    let vex = create_character_named(&svc, &proj, "Vex").await;

    // Two placement-stamped states: an earlier (ch1) and a later (ch3).
    let s1 = recap_scene(&svc, &proj, 1, 1, 1, "Vex is calm at the start.").await;
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: vex.clone(),
        scene_id: s1,
        changes: CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: Some(vec!["EARLYSTATE".into()]),
            notes: None,
            source_summary: None,
        },
    })
    .await
    .unwrap();
    let s3 = recap_scene(&svc, &proj, 1, 3, 1, "Vex is furious later.").await;
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: vex.clone(),
        scene_id: s3,
        changes: CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: None,
            status: Some(vec!["LATESTATE".into()]),
            notes: None,
            source_summary: None,
        },
    })
    .await
    .unwrap();

    // Two glossary terms out of alphabetical order.
    for term in ["Zephyr", "Aegis"] {
        svc.create_term(CreateTermInput {
            project_id: proj.clone(),
            term_text: term.into(),
            pronunciation: None,
            definition: format!("Definition of {term}."),
            usage_context: None,
            origin: None,
        })
        .await
        .unwrap();
    }

    // Cursor at ch2 → earlier state shown, later state not.
    let out = svc
        .export_series_bible(ExportSeriesBibleInput {
            project_id: proj.clone(),
            through: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 2,
                scene_order: None,
                note: None,
            }),
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert!(out.markdown.contains("EARLYSTATE"), "state as-of cursor");
    assert!(!out.markdown.contains("LATESTATE"), "later state withheld");
    // Glossary sorted: Aegis before Zephyr.
    let a = out.markdown.find("Aegis").expect("Aegis present");
    let z = out.markdown.find("Zephyr").expect("Zephyr present");
    assert!(a < z, "glossary sorted ascending");
}

#[tokio::test]
async fn export_recap_writes_deterministic_artifact_inside_workspace() {
    use spindle_core::models::{ExportRecapInput, SaveSummaryInput};
    use std::path::Path;

    let (tmp, svc) = fresh_service_workspace().await;
    let proj = recap_project(&svc, "Artifact Recap").await;
    svc.save_summary(SaveSummaryInput {
        project_id: proj.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "The tale begins.".into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();

    let input = || ExportRecapInput {
        project_id: proj.clone(),
        book_number: 1,
        through_chapter: 1,
        write_to_workspace: true,
    };

    let first = svc.export_recap(input()).await.unwrap();
    let path = first.artifact_path.clone().expect("artifact path");
    assert!(
        path.ends_with("recap-book-1-through-ch1.md"),
        "deterministic filename: {path}"
    );
    let workspace_root = tmp.path().canonicalize().unwrap();
    let written = Path::new(&path).canonicalize().unwrap();
    assert!(
        written.starts_with(&workspace_root),
        "{written:?} must live inside {workspace_root:?}"
    );

    // Overwrite on repeat: same path, one file.
    let second = svc.export_recap(input()).await.unwrap();
    assert_eq!(second.artifact_path.as_deref(), Some(path.as_str()));
    let artifacts_dir = written.parent().unwrap();
    let md_count = std::fs::read_dir(artifacts_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .count();
    assert_eq!(md_count, 1, "overwrite must not duplicate files");
}

#[tokio::test]
async fn export_epub_writes_bytes_and_passes_warnings() {
    use spindle_core::models::{
        ContentRating, ExportEpubInput, ExportIssueSeverity, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Export Test".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "epic".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    // The scene body clears the stub gate's word floor so the export
    // exercises the happy path (the gate itself is covered by
    // preflight_flags_placeholder_and_thin_scenes_as_blocking).
    svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate while the last light bled out of the sky and the wardens lit the wall torches one by one.".into(),
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
        .export_epub(ExportEpubInput {
            project_id: proj.project_id.clone(),
            author: Some("T. Author".into()),
            book_number: Some(1),
            start_chapter_number: None,
            end_chapter_number: None,
        })
        .await
        .unwrap();

    assert_eq!(out.total_chapters, 1);
    assert_eq!(out.total_scenes, 1);
    // chapter 1 has no title → preflight surfaces a chapter_missing_title warning.
    assert!(
        out.preflight_warnings
            .iter()
            .any(|w| w.code == "chapter_missing_title"
                && matches!(w.severity, ExportIssueSeverity::Warning))
    );
    // No scene_source_link rows were created in this fixture, so
    // SourceBridge divergence detection has nothing to report.
    assert!(out.divergence_warnings.is_empty());
    // Bytes hit disk.
    let bytes = std::fs::read(&out.file_path).unwrap();
    assert!(bytes.starts_with(b"PK"), "expected ZIP magic for EPUB");
    assert!(out.filename.ends_with(".epub"));
}

#[tokio::test]
async fn export_epub_emits_divergence_warnings_for_drifted_files() {
    use spindle_core::models::{
        ContentRating, DivergenceKind, ExportEpubInput, PushChapterToFileInput, SaveSceneDraftInput,
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

    // The scene body clears the stub gate's word floor so the export
    // reaches the divergence walk it is exercising.
    svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The first scene opened on the harbor at dawn, gulls wheeling over the fishing boats as the tide pulled away from the stones.".into(),
            summary: "s1".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let chapter = svc
        .repository()
        .find_chapter_by_number(&proj.project_id, 1, 1)
        .await
        .unwrap()
        .expect("chapter exists");

    let pushed = svc
        .push_chapter_to_file(PushChapterToFileInput {
            chapter_id: chapter.id.clone(),
            target_path: "ch1.txt".into(),
        })
        .await
        .unwrap();

    // Mutate file directly so the on-disk SHA disagrees with the link.
    // The replacement must stay at least as long as the tracked byte
    // range, or the divergence resolver cannot re-slice the file and
    // reports `Unknown` instead of `ContentMismatch`.
    std::fs::write(
            &pushed.target_path,
            "Externally edited body that someone rewrote overnight, changing every sentence of the scene while keeping the file roughly the same size.",
        )
        .unwrap();

    let out = svc
        .export_epub(ExportEpubInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            start_chapter_number: None,
            end_chapter_number: None,
            author: Some("Anon".into()),
        })
        .await
        .unwrap();
    assert_eq!(out.divergence_warnings.len(), 1);
    assert!(matches!(
        out.divergence_warnings[0].kind,
        DivergenceKind::ContentMismatch
    ));
}

#[tokio::test]
async fn export_bible_writes_payload_with_project_scoped_rows() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterInput,
        ExportBibleInput,
    };

    let (tmp, svc) = fresh_service().await;

    let project = svc
        .create_project(CreateProjectInput {
            name: "Export Target!".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "p".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // One character so at least one branch-scoped row is present.
    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project.project_id.clone(),
        name: "Mara".to_string(),
        summary: "Warden.".to_string(),
        role: "protagonist".to_string(),
        realm: None,
        voice_profile: CharacterVoiceProfileData {
            tone: Some("grim".to_string()),
            vocabulary: Vec::new(),
            sentence_structure: Vec::new(),
            tics: Vec::new(),
            forbidden_words: Vec::new(),
            example_lines: Vec::new(),
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: CharacterEmotionalProfileData {
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

    let out = svc
        .export_bible(ExportBibleInput {
            project_id: project.project_id.clone(),
        })
        .await
        .unwrap();

    assert_eq!(out.filename, "export-target-bible-export.json");
    assert!(out.file_path.ends_with(&out.filename));
    // Bible export should have a non-trivial table count.
    assert!(
        out.exported_tables >= 5,
        "got {} tables",
        out.exported_tables
    );
    assert!(
        out.exported_records >= 3,
        "got {} records",
        out.exported_records
    );
    // File written into data_dir/exports/.
    let path = std::path::PathBuf::from(&out.file_path);
    assert!(path.exists(), "export file must exist at {}", out.file_path);
    let bytes = std::fs::read(&path).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        payload.get("format").and_then(|v| v.as_str()),
        Some("spindle-bible-export-v1"),
    );
    assert_eq!(
        payload.get("project_name").and_then(|v| v.as_str()),
        Some("Export Target!"),
    );
    // Project + branch + character should all be present in `tables`.
    let tables = payload.get("tables").and_then(|v| v.as_object()).unwrap();
    assert!(tables.contains_key("project"));
    assert!(tables.contains_key("bible_branch"));
    let chars = tables
        .get("character")
        .and_then(|v| v.as_array())
        .expect("character table is a list");
    assert_eq!(chars.len(), 1, "exactly one character was created");
    assert_eq!(chars[0].get("name").and_then(|v| v.as_str()), Some("Mara"),);
    // Voice-profile JSON columns are stored field-by-field in the V0001
    // schema (each list is its own json_valid TEXT column). Confirm
    // both the scalar `tone` and one of the JSON columns round-trip
    // through `looks_like_json_column`.
    let voice_profiles = tables
        .get("character_voice_profile")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(voice_profiles.len(), 1);
    assert_eq!(
        voice_profiles[0].get("tone").and_then(|v| v.as_str()),
        Some("grim"),
    );
    assert!(
        voice_profiles[0]
            .get("vocabulary")
            .and_then(|v| v.as_array())
            .is_some(),
        "vocabulary JSON column should be re-parsed as an array",
    );
    drop(tmp);
}

/// Acceptance test for the typed-record export shape: seed one of each
/// major entity, run `export_bible`, then deserialize each row in the
/// exported `tables.*` arrays back into the matching `records::*` type
/// via `serde_json::from_value`. Asserts no `from_value` errors and
/// that every timestamp field renders as an RFC-3339 string (e.g.
/// `"2026-05-22T..."`) rather than a unix-microsecond integer.
#[tokio::test]
async fn export_bible_round_trips_typed_records_with_rfc3339_timestamps() {
    use crate::sqlite::records;
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateBookInput,
        CreateChapterInput, CreateCharacterInput, CreateConflictInput, CreateEconomyInput,
        CreateFactionInput, CreateLocationInput, CreateMotifInput, CreateNarrativePromiseInput,
        CreatePlotLineInput, CreateReligionInput, CreateTermInput, CreateThemeInput,
        CreateTimelineEventInput, CreateWorldRuleInput, ExportBibleInput, StoryPlacement,
        WorldStateInput,
    };

    let (tmp, svc) = fresh_service().await;

    let project = svc
        .create_project(CreateProjectInput {
            name: "Roundtrip Subject".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "p".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let project_id = project.project_id.clone();

    svc.create_book(CreateBookInput {
        project_id: project_id.clone(),
        title: Some("Book One".to_string()),
    })
    .await
    .unwrap();
    svc.create_chapter(CreateChapterInput {
        project_id: project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(1),
        title: Some("Chapter One".to_string()),
    })
    .await
    .unwrap();

    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project_id.clone(),
        name: "Mara".to_string(),
        summary: "Warden.".to_string(),
        role: "protagonist".to_string(),
        realm: None,
        voice_profile: CharacterVoiceProfileData {
            tone: Some("grim".to_string()),
            vocabulary: Vec::new(),
            sentence_structure: Vec::new(),
            tics: Vec::new(),
            forbidden_words: Vec::new(),
            example_lines: Vec::new(),
            established_in_scene_id: None,
            updated_at: None,
        },
        emotional_profile: CharacterEmotionalProfileData {
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

    svc.create_location(CreateLocationInput {
        project_id: project_id.clone(),
        name: "Riverhold".to_string(),
        kind: "city".to_string(),
        realm: None,
        summary: "Hub of trade.".to_string(),
        initial_state: WorldStateInput::default(),
    })
    .await
    .unwrap();

    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project_id.clone(),
        rule_name: "Salt warding".to_string(),
        rule_type: "magic".to_string(),
        description: "Salt repels wraiths.".to_string(),
        scan_pattern: None,
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    svc.create_faction(CreateFactionInput {
        project_id: project_id.clone(),
        name: "The Order".to_string(),
        faction_type: "knightly".to_string(),
        realm: None,
        summary: "Sword-sworn protectors.".to_string(),
        tags: Vec::new(),
    })
    .await
    .unwrap();

    svc.create_religion(CreateReligionInput {
        project_id: project_id.clone(),
        name: "The Bright Path".to_string(),
        deity_or_principle: "Sun".to_string(),
        summary: "Worship of dawn.".to_string(),
        tags: Vec::new(),
    })
    .await
    .unwrap();

    svc.create_economy(CreateEconomyInput {
        project_id: project_id.clone(),
        name: "River Mark".to_string(),
        realm: None,
        summary: "Grain-based barter.".to_string(),
        scarce_resources: vec!["iron".to_string()],
        trade_goods: vec!["grain".to_string()],
        currency: Some("marks".to_string()),
        notes: Vec::new(),
    })
    .await
    .unwrap();

    svc.create_term(CreateTermInput {
        project_id: project_id.clone(),
        term_text: "Wraith".to_string(),
        pronunciation: None,
        definition: "A vengeful spirit.".to_string(),
        usage_context: None,
        origin: None,
    })
    .await
    .unwrap();

    svc.create_plot_line(CreatePlotLineInput {
        project_id: project_id.clone(),
        name: "Mara's hunt".to_string(),
        plot_type: "main".to_string(),
        summary: "Hunt the wraith.".to_string(),
        status: None,
        convergence_points: Vec::new(),
        connected_conflict_ids: Vec::new(),
        connected_theme_ids: Vec::new(),
    })
    .await
    .unwrap();

    svc.create_conflict(CreateConflictInput {
        project_id: project_id.clone(),
        name: "Mara vs Wraith".to_string(),
        conflict_type: "person-vs-supernatural".to_string(),
        stakes: "Life of the village.".to_string(),
        escalation_stages: Vec::new(),
        expected_total_cycles: None,
        try_fail_cycles: Vec::new(),
        stated_consequences: Vec::new(),
    })
    .await
    .unwrap();

    svc.create_theme(CreateThemeInput {
        project_id: project_id.clone(),
        theme_statement: "Duty over self.".to_string(),
        thesis_antithesis: "Duty vs love.".to_string(),
        introduction_point: None,
        resolution_point: None,
    })
    .await
    .unwrap();

    svc.create_motif(CreateMotifInput {
        project_id: project_id.clone(),
        name: "Salt".to_string(),
        description: "Warding crystal.".to_string(),
        max_uses_per_chapter: None,
        connected_theme_ids: Vec::new(),
    })
    .await
    .unwrap();

    svc.create_narrative_promise(CreateNarrativePromiseInput {
        project_id: project_id.clone(),
        promise_type: "mystery".to_string(),
        description: "What killed the elder?".to_string(),
        planted_at: StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        planned_payoff: None,
        notes: Vec::new(),
    })
    .await
    .unwrap();

    svc.create_timeline_event(CreateTimelineEventInput {
        project_id: project_id.clone(),
        title: "Founding".to_string(),
        event_type: "history".to_string(),
        placement: StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        summary: "The town was founded.".to_string(),
        related_entity_ids: Vec::new(),
    })
    .await
    .unwrap();

    // Run the export.
    let out = svc
        .export_bible(ExportBibleInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap();
    let bytes = std::fs::read(&out.file_path).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let tables = payload
        .get("tables")
        .and_then(|v| v.as_object())
        .expect("tables map");

    // Project: single object, RFC-3339 timestamp.
    let project_v = tables.get("project").expect("project entry");
    let p: records::Project =
        serde_json::from_value(project_v.clone()).expect("project deserializes typed");
    assert_eq!(p.name, "Roundtrip Subject");
    let created_str = project_v
        .get("created_at")
        .and_then(|v| v.as_str())
        .expect("project.created_at is a string");
    assert!(
        created_str.starts_with("20") && created_str.contains('T'),
        "expected RFC-3339 timestamp, got {created_str}",
    );

    // Helper: deserialize every entry in `tables.<name>` as `R` and
    // assert each row's `created_at` (or first timestamp field) is an
    // RFC-3339 string. Returns the count round-tripped.
    fn round_trip<R: for<'de> serde::Deserialize<'de>>(
        tables: &serde_json::Map<String, serde_json::Value>,
        name: &str,
        ts_field: &str,
    ) -> usize {
        let arr = tables
            .get(name)
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("{name} table is an array"));
        let mut count = 0usize;
        for row in arr {
            serde_json::from_value::<R>(row.clone())
                .unwrap_or_else(|e| panic!("{name} row {row} did not deserialize: {e}"));
            let ts = row
                .get(ts_field)
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("{name}.{ts_field} not a string in row {row}"));
            assert!(
                ts.starts_with("20") && ts.contains('T'),
                "{name}.{ts_field} is not RFC-3339: {ts}",
            );
            count += 1;
        }
        count
    }

    let mut entity_total = 0usize;
    entity_total += round_trip::<records::BibleBranch>(tables, "bible_branch", "created_at");
    entity_total += round_trip::<records::Book>(tables, "book", "created_at");
    entity_total += round_trip::<records::Chapter>(tables, "chapter", "created_at");
    entity_total += round_trip::<records::Character>(tables, "character", "created_at");
    entity_total += round_trip::<records::CharacterVoiceProfile>(
        tables,
        "character_voice_profile",
        "created_at",
    );
    entity_total += round_trip::<records::CharacterEmotionalProfile>(
        tables,
        "character_emotional_profile",
        "created_at",
    );
    entity_total += round_trip::<records::Location>(tables, "location", "created_at");
    entity_total += round_trip::<records::WorldRule>(tables, "world_rule", "created_at");
    entity_total += round_trip::<records::Faction>(tables, "faction", "created_at");
    entity_total += round_trip::<records::Religion>(tables, "religion", "created_at");
    entity_total += round_trip::<records::Economy>(tables, "economy", "created_at");
    entity_total += round_trip::<records::Term>(tables, "term", "created_at");
    entity_total += round_trip::<records::PlotLine>(tables, "plot_line", "created_at");
    entity_total += round_trip::<records::Conflict>(tables, "conflict", "created_at");
    entity_total += round_trip::<records::Theme>(tables, "theme", "created_at");
    entity_total += round_trip::<records::Motif>(tables, "motif", "created_at");
    entity_total +=
        round_trip::<records::NarrativePromise>(tables, "narrative_promise", "created_at");
    entity_total += round_trip::<records::TimelineEvent>(tables, "timeline_event", "created_at");

    // 17 entity tables + project (asserted above) = 18 typed round-trips.
    assert!(
        entity_total >= 17,
        "expected at least 17 round-tripped entities, got {entity_total}",
    );

    // Cross-check timestamp shape on a few known fields: branch's
    // created_at is RFC-3339, not a number.
    let branch_arr = tables
        .get("bible_branch")
        .and_then(|v| v.as_array())
        .unwrap();
    assert!(!branch_arr.is_empty());
    assert!(
        branch_arr[0]
            .get("created_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "bible_branch.created_at should be a string, got {:?}",
        branch_arr[0].get("created_at"),
    );
    assert!(
        branch_arr[0]
            .get("created_at")
            .and_then(|v| v.as_i64())
            .is_none(),
        "bible_branch.created_at must NOT be an integer",
    );

    drop(tmp);
}
