//! Phase 5 adapter proof: rolling chapter quotas on save/verify, V0031-tied
//! suppressions, experimental flag off by default. Save stays advisory.

use spindle_adapters::ai::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateChapterInput,
    CreateProjectInput, DraftAuthorship, ReaderContract, SaveSceneDraftInput, UpdateEntityInput,
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

async fn project(svc: &SqliteSpindleService, name: &str) -> String {
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

fn contrast_one() -> &'static str {
    "It wasn't anger. It was disappointment. He set the cup down."
}

fn contrast_two() -> &'static str {
    "This wasn't a homecoming. It was a reckoning. She locked the door."
}

#[tokio::test]
async fn save_rolls_chapter_quota_and_stays_advisory() {
    let (_tmp, svc) = fresh_service_local().await;
    let project_id = project(&svc, "Chapter counters").await;

    let first = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: contrast_one().into(),
            summary: "first contrast".into(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .expect("save must not fail");
    let first_report = first.anti_slop.expect("scan attached");
    assert_eq!(
        first_report.hard_count, 0,
        "scene 1 is within the chapter quota: {first_report:?}"
    );
    assert!(first_report.structural_observations.is_empty());

    let second = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 2,
            full_text: contrast_two().into(),
            summary: "second contrast".into(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .expect("hard chapter over-limit must not fail a save");
    let second_report = second.anti_slop.expect("scan attached");
    assert!(
        second_report.hard_count >= 1,
        "scene 2 must roll the chapter contrast quota: {second_report:?}"
    );
    assert!(second_report.structural_observations.is_empty());

    svc.create_chapter(CreateChapterInput {
        project_id: project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(2),
        title: Some("Chapter two".into()),
    })
    .await
    .unwrap();
    let other_chapter = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id,
            book_number: 1,
            chapter_number: 2,
            scene_order: 1,
            full_text: contrast_two().into(),
            summary: "new chapter".into(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
    let other = other_chapter.anti_slop.expect("scan attached");
    assert_eq!(
        other.hard_count, 0,
        "a new chapter resets the rolling quota: {other:?}"
    );
}

#[tokio::test]
async fn verify_rolls_chapter_quota_hard_only() {
    let (_tmp, svc) = fresh_service_local().await;
    let project_id = project(&svc, "Chapter verify").await;

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        full_text: contrast_one().into(),
        summary: "first".into(),
        content_rating: ContentRating::General,
        ..Default::default()
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 2,
        full_text: contrast_two().into(),
        summary: "second".into(),
        content_rating: ContentRating::General,
        ..Default::default()
    })
    .await
    .unwrap();

    let result = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id,
            scope: ConsistencyScopeInput::chapter_range(1, 1, 1, 1),
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
        result
            .issues
            .iter()
            .any(|issue| issue.check_type == "anti_slop" && issue.severity == "warning"),
        "rolled chapter over-limit must fail-close on verify: {result:?}"
    );
}

#[tokio::test]
async fn style_learning_keeps_excerpt_as_suppression() {
    let (_tmp, svc) = fresh_service_local().await;
    let project_id = project(&svc, "Suppression learn").await;
    let agent = "It wasn't anger. It was disappointment.\n\
                 This wasn't a homecoming. It was a reckoning.\n\
                 She felt a mix of relief and dread when the letter came.";
    let first = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            full_text: agent.into(),
            summary: "agent cocktail".into(),
            content_rating: ContentRating::General,
            authorship: DraftAuthorship::Assistant,
            ..Default::default()
        })
        .await
        .unwrap();
    let before = first.anti_slop.expect("scan");
    assert!(
        before
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "emotion_cocktail"),
        "{before:?}"
    );

    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "project".into(),
        entity_id: project_id.clone(),
        changes: serde_json::json!({ "style_learning": 1 }),
    })
    .await
    .unwrap();

    let operator = "He set the cup down without drinking.\n\
                    She felt a mix of relief and dread when the letter came.";
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
        full_text: operator.into(),
        summary: "operator keep cocktail".into(),
        content_rating: ContentRating::General,
        authorship: DraftAuthorship::Human,
        ..Default::default()
    })
    .await
    .unwrap();

    svc.create_chapter(CreateChapterInput {
        project_id: project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(2),
        title: Some("Chapter two".into()),
    })
    .await
    .unwrap();
    let replay = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id,
            book_number: 1,
            chapter_number: 2,
            scene_order: 1,
            full_text: "She felt a mix of relief and dread when the letter came.".into(),
            summary: "later cocktail".into(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
    let report = replay.anti_slop.expect("scan");
    assert!(
        report
            .hits
            .iter()
            .all(|hit| hit.shelf_id != "emotion_cocktail"),
        "learned suppression must apply on a later scene: {report:?}"
    );
    assert_eq!(report.hard_count, 0);
}
