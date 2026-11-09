use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    ContentRating, CreateBranchInput, CreateProjectInput, CreateSavePointInput, ListChangesInput,
    ListSceneVersionsInput, ReaderContract, ReviseSceneInput, SaveSceneDraftInput,
    SwitchBranchInput, UndoChangeInput, UndoSupport,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    (tmp, svc)
}

async fn project(svc: &SqliteSpindleService, name: &str) -> String {
    svc.create_project(CreateProjectInput {
        name: name.to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "A test promise.".to_string(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    })
    .await
    .unwrap()
    .project_id
}

async fn three_versions(svc: &SqliteSpindleService, project_id: &str) -> String {
    let main = svc
        .repository()
        .active_branch_id_public(project_id)
        .await
        .unwrap();
    let child = svc
        .create_branch(CreateBranchInput {
            project_id: project_id.to_string(),
            name: "history".to_string(),
            branch_type: "alternate".to_string(),
            description: None,
            parent_branch_id: Some(main),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project_id.to_string(),
        branch_id: child.branch_id,
    })
    .await
    .unwrap();
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.to_string(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Version one.".to_string(),
            summary: "One.".to_string(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
    for text in ["Version two.", "Version three."] {
        svc.revise_scene(ReviseSceneInput {
            project_id: project_id.to_string(),
            scene_id: scene.scene_id.clone(),
            full_text: text.to_string(),
            summary: text.to_string(),
            content_rating: ContentRating::General,
            tone: None,
        })
        .await
        .unwrap();
    }
    scene.scene_id
}

#[tokio::test]
async fn list_changes_paginates_and_marks_checkpoint_scope() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "History List").await;
    let scene_id = three_versions(&svc, &project_id).await;
    let save_point = svc
        .create_save_point(CreateSavePointInput {
            project_id: project_id.clone(),
            name: "before revision".to_string(),
            description: None,
        })
        .await
        .unwrap();

    let page = svc
        .list_changes(ListChangesInput {
            project_id: project_id.clone(),
            branch_id: None,
            scene_id: None,
            kind: None,
            limit: Some(2),
            offset: None,
        })
        .await
        .unwrap();
    assert_eq!(page.changes.len(), 2);
    assert!(page.next_offset.is_some());

    let checkpoint = svc
        .list_changes(ListChangesInput {
            project_id: project_id.clone(),
            branch_id: None,
            scene_id: None,
            kind: Some("branch_checkpoint".to_string()),
            limit: None,
            offset: None,
        })
        .await
        .unwrap();
    assert_eq!(checkpoint.changes.len(), 1);
    assert_eq!(
        checkpoint.changes[0].operation_id,
        format!("save_point:{}", save_point.save_point_id)
    );
    assert_eq!(
        checkpoint.changes[0].undo_support,
        UndoSupport::CheckpointOnly
    );

    let scene_changes = svc
        .list_changes(ListChangesInput {
            project_id: project_id.clone(),
            branch_id: None,
            scene_id: Some(scene_id.clone()),
            kind: Some("scene_save".to_string()),
            limit: Some(100),
            offset: None,
        })
        .await
        .unwrap();
    assert_eq!(scene_changes.changes.len(), 2);
    // The current version is explicitly marked as not directly undoable.
    assert_eq!(
        scene_changes.changes[0].undo_support,
        UndoSupport::Irreversible
    );
}

#[tokio::test]
async fn undo_scene_version_checks_head_and_creates_audited_compensation() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "History Undo").await;
    let scene_id = three_versions(&svc, &project_id).await;

    let versions = svc
        .list_scene_versions(ListSceneVersionsInput {
            project_id: project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap()
        .versions;
    let current = &versions[0];
    let target = versions.last().unwrap();
    let expected_head = current.scene_version_id.clone();

    let dry = svc
        .undo_change(UndoChangeInput {
            project_id: project_id.clone(),
            operation_id: format!("scene_version:{}", target.scene_version_id),
            expected_head: expected_head.clone(),
            dry_run: true,
        })
        .await
        .unwrap();
    assert!(dry.dry_run);
    assert_eq!(
        dry.affected.iter().filter(|a| a.table == "scene").count(),
        1
    );
    assert_eq!(
        svc.list_scene_versions(ListSceneVersionsInput {
            project_id: project_id.clone(),
            scene_id: scene_id.clone()
        })
        .await
        .unwrap()
        .versions
        .len(),
        2
    );

    let undone = svc
        .undo_change(UndoChangeInput {
            project_id: project_id.clone(),
            operation_id: format!("scene_version:{}", target.scene_version_id),
            expected_head: expected_head.clone(),
            dry_run: false,
        })
        .await
        .unwrap();
    assert!(!undone.dry_run);
    assert!(undone.compensating_operation_id.is_some());
    let after = svc
        .list_scene_versions(ListSceneVersionsInput {
            project_id: project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap()
        .versions;
    assert_eq!(after.len(), 3, "restore must append a compensating version");

    let audit = svc
        .list_changes(ListChangesInput {
            project_id: project_id.clone(),
            branch_id: None,
            scene_id: None,
            kind: Some("undo".to_string()),
            limit: None,
            offset: None,
        })
        .await
        .unwrap();
    assert_eq!(audit.changes.len(), 1);
    assert_eq!(audit.changes[0].undo_support, UndoSupport::Irreversible);

    // Replaying with the old head is a conflict, not a silent overwrite.
    let stale = svc
        .undo_change(UndoChangeInput {
            project_id,
            operation_id: format!("scene_version:{}", target.scene_version_id),
            expected_head,
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{stale:#}").contains("stale expected_head"),
        "{stale:#}"
    );
}

#[tokio::test]
async fn save_points_are_checkpoint_only_through_unified_undo() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "History Checkpoint").await;
    let save_point = svc
        .create_save_point(CreateSavePointInput {
            project_id: project_id.clone(),
            name: "snapshot".to_string(),
            description: None,
        })
        .await
        .unwrap();
    let error = svc
        .undo_change(UndoChangeInput {
            project_id,
            operation_id: format!("save_point:{}", save_point.save_point_id),
            expected_head: save_point.save_point_id,
            dry_run: false,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("checkpoint_only"),
        "save points must be explicitly checkpoint-scoped: {error:#}"
    );
}
