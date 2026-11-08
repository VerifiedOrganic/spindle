use super::*;

#[tokio::test]
async fn create_project_round_trip() {
    let (_tmp, repo) = fresh_repo().await;
    let input = CreateProjectInput {
        name: "Spindle".into(),
        project_type: "novel".into(),
        genre: "fantasy".into(),
        reader_contract: ReaderContract {
            promise: "epic".into(),
            style_notes: vec!["sparse".into()],
            boundaries: vec!["no second-person".into()],
        },
    };
    let (project, branch, book, chapter) = repo.create_project(&input).await.unwrap();
    assert!(project.id.starts_with("project:"));
    assert_eq!(project.name, "Spindle");
    assert_eq!(
        project.active_branch_id.as_deref(),
        Some(branch.id.as_str())
    );
    assert_eq!(branch.name, "main");
    assert_eq!(branch.status, "active");
    assert_eq!(book.book_number, 1);
    assert_eq!(chapter.chapter_number, 1);

    // get_project round-trips
    let again = repo.get_project(&project.id).await.unwrap();
    assert_eq!(again.id, project.id);
    assert_eq!(again.reader_contract.promise, "epic");

    // get_active_branch resolves through active_branch_id
    let active = repo.get_active_branch(&project.id).await.unwrap();
    assert_eq!(active.id, branch.id);

    // list_branches_by_project returns the main branch only
    let branches = repo.list_branches_by_project(&project.id).await.unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].id, branch.id);
}

#[tokio::test]
async fn missing_project_returns_error() {
    let (_tmp, repo) = fresh_repo().await;
    let err = repo
        .get_project("project:does-not-exist")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("project not found"), "{err}");
}
