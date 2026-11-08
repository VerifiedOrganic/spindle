use super::*;

#[tokio::test]
async fn archive_entity_supports_research_tables_and_sets_archived_at() {
    // Live-run bug 5: `archive_entity` failed on research rows with
    // "entity table 'research_claim' has no archived_at column", leaving
    // junk research entities unremovable. After V0033 the three research
    // tables carry an `archived_at` column and are in the archival
    // allowlist, so archival succeeds and stamps the column.
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

    let source = repo
        .create_research_source(
            &project.id,
            Some(&branch.id),
            "Junk Source",
            "web",
            None,
            None,
            None,
            None,
            None,
            chrono::Utc::now(),
            "uncertain",
            &[],
            None,
        )
        .await
        .unwrap();
    let note = repo
        .create_research_note(
            &project.id,
            Some(&source.id),
            Some(&branch.id),
            "Junk note",
            None,
            None,
            &[],
        )
        .await
        .unwrap();
    let claim = repo
        .create_research_claim(
            &project.id,
            Some(&source.id),
            Some(&note.id),
            Some(&branch.id),
            "Junk claim",
            None,
            None,
            None,
            "uncertain",
            &[],
        )
        .await
        .unwrap();

    for (table, id) in [
        ("research_source", &source.id),
        ("research_note", &note.id),
        ("research_claim", &claim.id),
    ] {
        repo.archive_entity(table, id)
            .await
            .unwrap_or_else(|e| panic!("archiving {table} must succeed, got: {e}"));
    }

    assert!(
        repo.get_research_source(&source.id)
            .await
            .unwrap()
            .unwrap()
            .archived_at
            .is_some(),
        "research_source archived_at must be set"
    );
    assert!(
        repo.get_research_note(&note.id)
            .await
            .unwrap()
            .unwrap()
            .archived_at
            .is_some(),
        "research_note archived_at must be set"
    );
    assert!(
        repo.get_research_claim(&claim.id)
            .await
            .unwrap()
            .unwrap()
            .archived_at
            .is_some(),
        "research_claim archived_at must be set"
    );
}
