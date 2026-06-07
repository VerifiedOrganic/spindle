use rusqlite::Connection;
use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    ContentRating, CreateProjectInput, ReaderContract, SaveSceneDraftInput,
};
use spindle_core::style::{
    ApplyStyleProfileInput, CreateStyleProfileFromMarkdownInput, GetStyleProfileInput,
    ListStyleProfilesInput, StyleProfileApplyMode,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    // Use local-only model router for deterministic test fallback
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    (tmp, SqliteSpindleService::new(repo))
}

#[tokio::test]
async fn test_create_and_apply_style_profile_lifecycle() {
    let (tmp, svc) = fresh_service().await;

    // 1. Create a project
    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Test Project".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "Epic adventure".to_string(),
                style_notes: vec!["Existing note".to_string()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let project_id = project_out.project_id;

    // 2. Prepare test data paths.
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/style");
    let src1 = base.join("fast-serial-chapter-1.md");
    let src2 = base.join("dialogue-heavy-scene.md");

    let dest_dir = tmp.path();
    let dest1 = dest_dir.join("fast-serial-chapter-1.md");
    let dest2 = dest_dir.join("dialogue-heavy-scene.md");

    std::fs::copy(&src1, &dest1).unwrap();
    std::fs::copy(&src2, &dest2).unwrap();

    let fixture1 = dest1.to_string_lossy().to_string();
    let fixture2 = dest2.to_string_lossy().to_string();

    // 3. Create style profile from Markdown
    let create_input = CreateStyleProfileFromMarkdownInput {
        project_id: project_id.clone(),
        profile_name: "Fast Action Profile".to_string(),
        source_paths: vec![fixture1, fixture2],
        recursive: Some(false),
        include_globs: None,
        exclude_globs: None,
        max_files: None,
        max_bytes_per_file: None,
        max_total_words: None,
        apply: Some(false),
        application_mode: None,
        source_sample_word_budget: None,
        metrics_only: None,
    };

    let create_out = svc
        .create_style_profile_from_markdown(create_input)
        .await
        .unwrap();
    let profile = create_out.profile;

    assert_eq!(profile.name, "Fast Action Profile");
    assert_eq!(profile.project_id, project_id);
    assert_eq!(
        profile.status,
        spindle_core::style::StyleProfileStatus::Ready
    );
    assert!(profile.corpus.source_count >= 2);
    assert_eq!(profile.corpus.analyzed_source_count, 2);
    assert!(profile.corpus.total_words > 0);
    assert!(profile.metrics.average_sentence_words > 0.0);

    // Verify source refs are correctly populated and no source text is persisted
    assert_eq!(profile.corpus.source_refs.len(), 2);
    for r in &profile.corpus.source_refs {
        assert!(r.included);
        assert!(!r.sha256.is_empty());
        assert!(r.word_count > 0);
    }
    let conn = Connection::open(tmp.path().join("svc.db")).unwrap();
    let persisted_source_text_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM style_profile \
             WHERE card_json LIKE '%Jake stared at the status screen%' \
                OR guidance_json LIKE '%Jake stared at the status screen%' \
                OR metrics_json LIKE '%Jake stared at the status screen%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        persisted_source_text_count, 0,
        "style profile persistence must not store source prose"
    );

    // 4. Test listing style profiles
    let list_out = svc
        .list_style_profiles(ListStyleProfilesInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(list_out.profiles.len(), 1);
    assert_eq!(list_out.profiles[0].profile_id, profile.profile_id);

    // 5. Test getting a single style profile
    let get_out = svc
        .get_style_profile(GetStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile.profile_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(get_out.profile.profile_id, profile.profile_id);

    // 6. Test applying the style profile
    let apply_out = svc
        .apply_style_profile(ApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile.profile_id.clone(),
            mode: StyleProfileApplyMode::Merge,
        })
        .await
        .unwrap();

    assert_eq!(apply_out.project_id, project_id);
    assert_eq!(apply_out.profile_id, profile.profile_id);

    // Verify NarratorVoice got set
    assert_eq!(
        apply_out.narrator_voice.emotional_register.as_deref(),
        Some("brooding-and-reflective")
    );
    assert_eq!(
        apply_out.narrator_voice.pacing_feel.as_deref(),
        Some("contemplative")
    );

    // Verify ReaderContract style notes got updated (merged)
    assert!(
        apply_out
            .reader_contract_style_notes
            .contains(&"Existing note".to_string())
    );
    assert!(
        apply_out
            .reader_contract_style_notes
            .iter()
            .any(|n| n.contains("Mock style profile guidance"))
    );

    // Verify style world rule was created
    assert!(apply_out.style_rule_id.is_some());

    // 7. Verify cache invalidation count is reported
    let _ = apply_out.invalidated_validator_findings;
}

#[tokio::test]
async fn test_create_style_profile_thin_corpus_warnings() {
    let (_tmp, svc) = fresh_service().await;

    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Test Project".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "Epic adventure".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let project_id = project_out.project_id;
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/style");
    let src = base.join("thin-corpus.md");
    let dest = _tmp.path().join("thin-corpus.md");
    std::fs::copy(&src, &dest).unwrap();
    let thin_fixture = dest.to_string_lossy().to_string();

    let create_out = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id,
            profile_name: "Thin Profile".to_string(),
            source_paths: vec![thin_fixture],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(false),
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: None,
        })
        .await
        .unwrap();

    assert!(
        create_out
            .profile
            .corpus
            .warnings
            .iter()
            .any(|w| w.contains("thin"))
    );
}

#[tokio::test]
async fn test_style_profile_audit_rollback_and_drift_lifecycle() {
    let (tmp, svc) = fresh_service().await;

    // 1. Create project
    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Style Lifecycle Project".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "An adventurous tale".to_string(),
                style_notes: vec![
                    "User-authored style note".to_string(),
                    "Another user note".to_string(),
                ],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let project_id = project_out.project_id;
    let branch_id = project_out.branch_id.clone();

    // 2. Write Markdown files for profile creation
    let dest_dir = tmp.path();
    let dest1 = dest_dir.join("ch1.md");
    // Write a short style text: 10 words, short sentence
    std::fs::write(
        &dest1,
        "Jake ran. Jake jumped. Jake succeeded. Jake was quick. Jake won.",
    )
    .unwrap();

    let create_out = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Short Sentence Profile".to_string(),
            source_paths: vec![dest1.to_string_lossy().to_string()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(false),
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: None,
        })
        .await
        .unwrap();
    let profile = create_out.profile;

    // 3. Test PreviewApplyStyleProfile
    let preview_out = svc
        .preview_apply_style_profile(spindle_core::style::PreviewApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile.profile_id.clone(),
            mode: StyleProfileApplyMode::ReplaceGeneratedStyleNotes,
        })
        .await
        .unwrap();

    // Verify preview returns expected changes
    assert!(
        preview_out
            .added_style_notes
            .iter()
            .any(|n| n.contains("Mock style profile guidance"))
    );
    assert!(preview_out.removed_style_notes.is_empty());

    // Verify state was NOT mutated
    let project_after_preview = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(project_after_preview.reader_contract.style_notes.len(), 2);

    // 4. Test ApplyStyleProfile (writes audit row, handles ReplaceGeneratedStyleNotes)
    let apply_out = svc
        .apply_style_profile(ApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile.profile_id.clone(),
            mode: StyleProfileApplyMode::ReplaceGeneratedStyleNotes,
        })
        .await
        .unwrap();

    // Verify user notes are preserved and generated notes added
    assert!(
        apply_out
            .reader_contract_style_notes
            .contains(&"User-authored style note".to_string())
    );
    assert!(
        apply_out
            .reader_contract_style_notes
            .iter()
            .any(|n| n.contains("Mock style profile guidance"))
    );
    assert!(apply_out.style_rule_id.is_some());

    // Verify audit record was written
    let audit_list = svc
        .list_style_profile_applications(spindle_core::style::ListStyleProfileApplicationsInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(audit_list.applications.len(), 1);
    let app = &audit_list.applications[0];
    assert_eq!(app.profile_id, profile.profile_id);
    assert_eq!(app.rollback_status, "not_rolled_back");
    assert_eq!(app.style_rule_action, "created");
    assert_eq!(app.before_style_notes.len(), 2);

    // Verify no source prose persisted in audit table
    let conn = rusqlite::Connection::open(tmp.path().join("svc.db")).unwrap();
    let audit_source_text_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM style_profile_application \
             WHERE before_narrator_voice_json LIKE '%Jake ran%' \
                OR after_narrator_voice_json LIKE '%Jake ran%' \
                OR added_style_notes_json LIKE '%Jake ran%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_source_text_count, 0);

    // 5. Test style drift checking (deterministic divergence)
    // Draft prose with very long sentences to trigger drift against the short-sentence profile
    let draft_prose = "This is a remarkably long sentence designed specifically to mismatch with the style profile and produce a deterministic warning about sentence length which should be easily detected by the style drift checking service method. And this is another extremely long and winding sentence that just goes on and on to ensure the average sentence length exceeds the threshold significantly.";
    let drift_out = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: project_id.clone(),
            profile_id: Some(profile.profile_id.clone()),
            scene_id: None,
            raw_text: Some(draft_prose.to_string()),
        })
        .await
        .unwrap();

    assert!(!drift_out.findings.is_empty());
    assert!(
        drift_out
            .findings
            .iter()
            .any(|f| f.category == "sentence_length")
    );

    // 6. Test Rollback
    let rollback_out = svc
        .rollback_style_profile_application(
            spindle_core::style::RollbackStyleProfileApplicationInput {
                project_id: project_id.clone(),
                application_id: app.id.clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(rollback_out.style_rule_action, "deleted");
    assert_eq!(rollback_out.reader_contract_style_notes.len(), 2);
    assert!(
        rollback_out
            .reader_contract_style_notes
            .contains(&"User-authored style note".to_string())
    );

    // Verify active branch no longer has the created style world rule
    let rules = svc
        .repository()
        .list_world_rules_by_project_and_branch(&project_id, &branch_id)
        .await
        .unwrap();
    let expected_name = format!("Style Profile: {}", profile.name);
    let rule_exists = rules
        .iter()
        .any(|r| r.rule_name == profile.name || r.rule_name == expected_name);
    assert!(!rule_exists);

    // Verify audit record is updated to rolled_back
    let audit_list_after = svc
        .list_style_profile_applications(spindle_core::style::ListStyleProfileApplicationsInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        audit_list_after.applications[0].rollback_status,
        "rolled_back"
    );
    assert!(audit_list_after.applications[0].rolled_back_at.is_some());
}

#[tokio::test]
async fn test_style_drift_rejects_cross_project_scene() {
    let (tmp, svc) = fresh_service().await;

    let source_project = svc
        .create_project(CreateProjectInput {
            name: "Source Project".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "An adventurous tale".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let other_project = svc
        .create_project(CreateProjectInput {
            name: "Other Project".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "A different tale".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let corpus_path = tmp.path().join("corpus.md");
    std::fs::write(
        &corpus_path,
        "Jake ran. Jake jumped. Jake succeeded. Jake was quick. Jake won.",
    )
    .unwrap();
    let profile = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: source_project.project_id.clone(),
            profile_name: "Source Profile".to_string(),
            source_paths: vec![corpus_path.to_string_lossy().to_string()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(false),
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: Some(true),
        })
        .await
        .unwrap()
        .profile;

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: other_project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "This scene belongs to a different project.".to_string(),
            summary: "Other scene".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let err = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: source_project.project_id,
            profile_id: Some(profile.profile_id),
            scene_id: Some(scene.scene_id),
            raw_text: None,
        })
        .await
        .unwrap_err();

    assert!(err.to_string().contains("scene does not belong to project"));
}
