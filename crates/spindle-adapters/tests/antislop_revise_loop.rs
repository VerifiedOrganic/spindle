//! Phase 3 critic / revise loop: save stays advisory, hard verify fail-closes,
//! dual-persona injects the report, Craft Technician cites shelf IDs.

use spindle_adapters::ai::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateProjectInput,
    ReaderContract, RunDualPersonaReviewInput, SaveSceneDraftInput,
};
use tempfile::TempDir;

async fn fresh_service_local() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    (tmp, SqliteSpindleService::new(repo))
}

fn hard_prose() -> &'static str {
    "It wasn't anger. It was disappointment.\n\
     This wasn't a homecoming. It was a reckoning.\n\
     She felt a mix of relief and dread when the letter came."
}

fn soft_prose() -> &'static str {
    "She spent the afternoon thinking about what he'd said.\n\n\
     Hours passed.\n\n\
     Later, at the meeting, she told him everything."
}

#[tokio::test]
async fn save_scene_draft_attaches_anti_slop_but_never_fails() {
    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "AntiSlop Save".into(),
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

    let hard = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: hard_prose().into(),
            summary: "hard shelves fire".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .expect("hard anti-slop must not fail a save");
    assert!(hard.status == "saved" || hard.status == "updated");
    let report = hard.anti_slop.expect("save attaches the scan report");
    assert!(
        report.hard_count >= 1,
        "hard over-limit must be counted: {report:?}"
    );
    assert!(
        report
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "emotion_cocktail" || hit.shelf_id == "contrast_not_x_but_y")
    );

    let soft = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: soft_prose().into(),
            summary: "soft solitary fade".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .expect("soft anti-slop must not fail a save");
    let soft_report = soft.anti_slop.expect("save attaches the scan report");
    assert_eq!(soft_report.hard_count, 0);
    assert!(
        soft_report
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "solitary_fade"
                && matches!(hit.severity, spindle_core::style::antislop::Severity::Soft))
    );
}

#[tokio::test]
async fn check_consistency_anti_slop_fail_closes_hard_only() {
    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "AntiSlop Verify".into(),
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
        full_text: hard_prose().into(),
        summary: "hard".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
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
        full_text: soft_prose().into(),
        summary: "soft".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let hard = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput {
                scene_order: Some(1),
                ..ConsistencyScopeInput::chapter_range(1, 1, 1, 1)
            },
            checks: vec!["anti_slop".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        hard.issues
            .iter()
            .any(|issue| issue.check_type == "anti_slop" && issue.severity == "warning"),
        "hard over-limit must join verify as warning: {:?}",
        hard.issues
    );
    assert!(
        hard.issues.iter().all(|issue| {
            !issue.message.contains("solitary_fade") || issue.severity != "warning"
        })
    );
    assert!(
        hard.issues.iter().any(
            |issue| issue.suggested_action.as_deref().is_some_and(|action| {
                action.contains("beat") && action.to_lowercase().contains("paraphrase")
            })
        ),
        "hard finding must carry rewrite-from-beats, not paraphrase: {:?}",
        hard.issues
    );

    let soft = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput {
                scene_order: Some(2),
                ..ConsistencyScopeInput::chapter_range(1, 1, 1, 1)
            },
            checks: vec!["anti_slop".into()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        soft.issues
            .iter()
            .all(|issue| issue.check_type != "anti_slop"),
        "soft shelves must not fail-close verify: {:?}",
        soft.issues
    );
}

#[tokio::test]
async fn dual_persona_local_adapter_cites_shelves_and_structure() {
    let (_tmp, svc) = fresh_service_local().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "AntiSlop Review".into(),
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
            full_text: hard_prose().into(),
            summary: "a thin sketch of the hard shelves".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let out = svc
        .run_dual_persona_review(RunDualPersonaReviewInput {
            project_id: proj.project_id.clone(),
            branch_id: None,
            scene_id: saved.scene_id.clone(),
            rounds: Some(1),
        })
        .await
        .unwrap();
    let round = &out.review_rounds[0];
    assert!(
        round
            .craft_technician
            .concerns
            .iter()
            .any(|c| c.contains('`')
                && (c.contains("emotion_cocktail") || c.contains("contrast_not_x_but_y"))),
        "Craft Technician must cite a shelf ID: {:?}",
        round.craft_technician.concerns
    );
    assert!(
        round
            .literary_critic
            .concerns
            .iter()
            .any(|c| c.contains("structure") && c.to_lowercase().contains("bluf")),
        "Literary Critic must include the structure block: {:?}",
        round.literary_critic.concerns
    );
}
