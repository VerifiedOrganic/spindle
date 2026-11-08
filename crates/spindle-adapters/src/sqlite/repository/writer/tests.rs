use super::*;

#[tokio::test]
async fn writer_position_upsert_replaces_in_place() {
    let (_tmp, repo) = fresh_repo().await;
    let (project, branch, book, chapter) = repo
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

    let p1 = repo
        .upsert_writer_position(UpsertWriterPositionParams {
            project_id: project.id.clone(),
            branch_id: branch.id.clone(),
            book_id: Some(book.id.clone()),
            chapter_id: Some(chapter.id.clone()),
            scene_id: None,
            intent: "draft".into(),
            next_focus: Some("intro".into()),
            updated_by: "test".into(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    assert_eq!(p1.intent, "draft");

    // Second upsert with different intent must update in place (no new row).
    let p2 = repo
        .upsert_writer_position(UpsertWriterPositionParams {
            project_id: project.id.clone(),
            branch_id: branch.id.clone(),
            book_id: Some(book.id.clone()),
            chapter_id: Some(chapter.id.clone()),
            scene_id: None,
            intent: "revise".into(),
            next_focus: None,
            updated_by: "test".into(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    assert_eq!(p2.id, p1.id, "upsert must reuse the row id");
    assert_eq!(p2.intent, "revise");
    assert!(p2.next_focus.is_none());

    repo.delete_writer_position(&project.id, &branch.id)
        .await
        .unwrap();
    assert!(
        repo.get_writer_position(&project.id, &branch.id)
            .await
            .unwrap()
            .is_none()
    );
}
