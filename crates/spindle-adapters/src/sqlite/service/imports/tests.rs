use super::*;

#[tokio::test]
async fn import_manuscript_persists_session_and_status_round_trips() {
    use spindle_core::models::{ImportManuscriptInput, ImportSessionStatus, ImportStatusInput};

    // SPINDLE_DATA_DIR pins the import data root inside the test temp.
    let env_guard = ScopedDataDir::new();

    let (tmp, svc) = fresh_service().await;

    // Create the target project up front so we don't need to create one
    // on-the-fly inside the import call (which uses default_import_data_dir).
    let project = svc
        .create_project(CreateProjectInput {
            name: "ImportTarget".to_string(),
            project_type: "novel".to_string(),
            genre: "fiction".to_string(),
            reader_contract: ReaderContract {
                promise: "p".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Stage a minimal manuscript file the slicer can chew on.
    let source_path = tmp.path().join("manuscript.txt");
    std::fs::write(
        &source_path,
        "Chapter 1\n\nMara opened the gate. Kade waited beyond.\n\n* * *\n\nKade nodded once.\n",
    )
    .unwrap();

    let import_output = svc
        .import_manuscript(ImportManuscriptInput {
            source_paths: vec![source_path.display().to_string()],
            target_project_id: Some(project.project_id.clone()),
            target_branch_id: None,
            create_project_name: None,
            source_format_hint: None,
            duplicate_strategy: None,
        })
        .await
        .unwrap();

    assert_eq!(import_output.structure.chapters.len(), 1);
    assert!(matches!(
        import_output.session.status,
        ImportSessionStatus::ReadyToHydrate | ImportSessionStatus::ReviewNeeded
    ));

    let status = svc
        .import_status(ImportStatusInput {
            project_id: project.project_id.clone(),
            session_id: import_output.session.session_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        status.session.session_id, import_output.session.session_id,
        "import_status round-trips the same session id"
    );
    assert!(
        status.structure.is_some(),
        "structural summary is persisted"
    );

    let _ = env_guard;
}
