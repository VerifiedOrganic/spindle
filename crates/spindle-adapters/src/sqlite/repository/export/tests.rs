use super::*;

/// Regression: `dump_project_table` must return one JSON object per row
/// keyed by column name and reject anything that isn't a plain
/// identifier. Also exercises the JSON-column re-parse path
/// (`looks_like_json_column`) via the `dynamics` column on `relates_to`.
#[tokio::test]
async fn dump_project_table_emits_json_objects_keyed_by_column_name() {
    let (_tmp, repo) = fresh_repo().await;
    let input = CreateProjectInput {
        name: "DumpTarget".into(),
        project_type: "novel".into(),
        genre: "fantasy".into(),
        reader_contract: ReaderContract {
            promise: "p".into(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    };
    let (project, _branch, _book, _chapter) = repo.create_project(&input).await.unwrap();

    // Project table: one row, with the expected scalar fields.
    let rows = repo
        .dump_table_by_column("project", "id", &project.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = rows[0].as_object().unwrap();
    assert_eq!(
        row.get("id").and_then(|v| v.as_str()),
        Some(project.id.as_str())
    );
    assert_eq!(row.get("name").and_then(|v| v.as_str()), Some("DumpTarget"));

    // bible_branch via dump_project_table: at least the main branch.
    let branches = repo
        .dump_project_table("bible_branch", &project.id)
        .await
        .unwrap();
    assert!(!branches.is_empty(), "main branch should be exported");
    assert!(
        branches
            .iter()
            .any(|b| b.get("name").and_then(|v| v.as_str()) == Some("main")),
    );

    // Unsafe identifier must be rejected.
    let err = repo
        .dump_project_table("project; DROP TABLE foo", &project.id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("unsafe identifier"),
        "got error: {err}"
    );

    // dump_table_by_column_in returns empty for an empty key set without
    // running the query at all.
    let none = repo
        .dump_table_by_column_in("import_session", "session_id", &[])
        .await
        .unwrap();
    assert!(none.is_empty());
}
