use super::*;

#[tokio::test]
async fn archive_entity_sets_archived_at_and_rejects_non_archivable() {
    use spindle_core::models::CreateFactionInput;
    let (_tmp, repo) = fresh_repo().await;
    let (project, _branch, _book, _chapter) = repo
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

    let faction = repo
        .create_faction(&CreateFactionInput {
            project_id: project.id.clone(),
            name: "Wardens".into(),
            faction_type: "military".into(),
            realm: None,
            summary: "Oathbound.".into(),
            tags: Vec::new(),
        })
        .await
        .unwrap();
    assert!(faction.archived_at.is_none());

    repo.archive_entity("faction", &faction.id).await.unwrap();
    let after = repo.get_faction(&faction.id).await.unwrap();
    assert!(after.archived_at.is_some(), "archived_at must be set");

    // Project doesn't have an archived_at column — must reject.
    let err = repo
        .archive_entity("project", &project.id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("no archived_at column"),
        "got: {err}"
    );
}
