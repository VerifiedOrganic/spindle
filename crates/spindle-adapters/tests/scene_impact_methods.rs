//! Integration test for Gap 2: `get_scene_delete_impact` +
//! `get_scene_move_impact`. Pre-port both methods didn't exist and
//! `read_project_resource`'s impact arms bailed; this test exercises
//! both methods directly + via the resource dispatch path.

use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    ContentRating, CreateProjectInput, GetSceneDeleteImpactInput, GetSceneMoveImpactInput,
    ReaderContract, SaveSceneDraftInput,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, SqliteSpindleService::new(repo))
}

#[tokio::test]
async fn scene_delete_impact_returns_structured_payload() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ImpactDel".into(),
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
        full_text: "Mara at the gate.".into(),
        summary: "scene".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
    })
    .await
    .unwrap();

    let out = svc
        .get_scene_delete_impact(GetSceneDeleteImpactInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
        })
        .await
        .unwrap();

    // The output exposes the target scene + groups; we just need it to
    // be a real structured response, not a bail.
    assert_eq!(out.scene.book_number, 1);
    assert_eq!(out.scene.chapter_number, 1);
    assert_eq!(out.scene.scene_order, 1);
}

#[tokio::test]
async fn scene_move_impact_returns_structured_payload() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ImpactMove".into(),
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
        full_text: "Mara at the gate.".into(),
        summary: "scene".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
    })
    .await
    .unwrap();

    let out = svc
        .get_scene_move_impact(GetSceneMoveImpactInput {
            project_id: proj.project_id.clone(),
            from_book_number: 1,
            from_chapter_number: 1,
            from_scene_order: 1,
            to_book_number: 1,
            to_chapter_number: 2,
            to_scene_order: 1,
        })
        .await
        .unwrap();

    assert_eq!(out.scene.book_number, 1);
    assert_eq!(out.destination.book_number, 1);
    assert_eq!(out.destination.chapter_number, 2);
}

#[tokio::test]
async fn read_project_resource_dispatches_scene_impact_arms() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Dispatch".into(),
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
        full_text: "scene.".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
    })
    .await
    .unwrap();

    // scene-delete-impact arm: was a bail, must now return JSON.
    let del = svc
        .read_project_resource(&proj.project_id, "scene-delete-impact/1/1/1")
        .await
        .expect("scene-delete-impact arm must dispatch");
    assert!(del.is_object(), "delete impact must be a JSON object");

    // scene-move-impact arm: same.
    let mv = svc
        .read_project_resource(&proj.project_id, "scene-move-impact/1/1/1/1/2/1")
        .await
        .expect("scene-move-impact arm must dispatch");
    assert!(mv.is_object(), "move impact must be a JSON object");
}
