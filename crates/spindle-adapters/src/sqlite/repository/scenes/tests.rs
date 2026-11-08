use super::*;

#[tokio::test]
async fn supersede_scene_deltas_only_flips_staged_rows() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    // Three deltas: one applied, one rejected, one staged.
    let applied = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "beat_annotation",
            "a",
        ))
        .await
        .unwrap();
    let rejected = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "quantity_change",
            "b",
        ))
        .await
        .unwrap();
    let staged = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "character_state",
            "c",
        ))
        .await
        .unwrap();
    repo.decide_canon_delta(&applied.id, CanonDeltaDecision::Applied, "op", None)
        .await
        .unwrap();
    repo.decide_canon_delta(&rejected.id, CanonDeltaDecision::Rejected, "op", None)
        .await
        .unwrap();

    let flipped = repo.supersede_scene_deltas(&scene_id).await.unwrap();
    assert_eq!(flipped, 1, "only the staged row is superseded");

    let by_id = |deltas: &[crate::sqlite::records::StoredCanonDelta], id: &str| {
        deltas.iter().find(|d| d.id == id).unwrap().status.clone()
    };
    let all = repo
        .list_canon_deltas(&project.id, &branch.id, None, None)
        .await
        .unwrap();
    assert_eq!(by_id(&all, &applied.id), "applied");
    assert_eq!(by_id(&all, &rejected.id), "rejected");
    assert_eq!(by_id(&all, &staged.id), "superseded");
}

#[tokio::test]
async fn save_scene_draft_creates_then_versions_then_cascades() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};
    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
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

    // First save: INSERT path. Created flag must be true.
    let (scene, created) = repo
        .save_scene_draft(
            &project.id,
            &branch.id,
            &SaveSceneDraftInput {
                project_id: project.id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "First draft of the gate.".into(),
                summary: "Mara stands watch.".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(created);
    assert!(scene.id.starts_with("scene:"));
    assert_eq!(scene.full_text, "First draft of the gate.");
    let scene_id = scene.id.clone();

    // Second save with changed prose: UPDATE path, scene_changed = true.
    // Expects a scene_version snapshot of the previous prose.
    let (updated, created2) = repo
        .save_scene_draft(
            &project.id,
            &branch.id,
            &SaveSceneDraftInput {
                project_id: project.id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Revised draft. The gate burns.".into(),
                summary: "Mara holds the gate.".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!created2, "second save must update in place");
    assert_eq!(updated.id, scene_id);
    assert_eq!(updated.full_text, "Revised draft. The gate burns.");

    // scene_version row should exist holding the prior prose.
    let versions: i64 = repo
        .inner
        .pool
        .read({
            let id = scene_id.clone();
            move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM scene_version WHERE scene_id = ?1",
                    [&id],
                    |r| r.get(0),
                )
            }
        })
        .await
        .unwrap();
    assert_eq!(versions, 1, "one snapshot of the prior prose");

    // Third save with same prose: UPDATE path, scene_changed = false.
    // No new scene_version, no cascade.
    let (_, created3) = repo
        .save_scene_draft(
            &project.id,
            &branch.id,
            &SaveSceneDraftInput {
                project_id: project.id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Revised draft. The gate burns.".into(),
                summary: "Mara holds the gate.".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!created3);
    let versions_again: i64 = repo
        .inner
        .pool
        .read({
            let id = scene_id.clone();
            move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM scene_version WHERE scene_id = ?1",
                    [&id],
                    |r| r.get(0),
                )
            }
        })
        .await
        .unwrap();
    assert_eq!(versions_again, 1, "unchanged prose must not snapshot");
}
