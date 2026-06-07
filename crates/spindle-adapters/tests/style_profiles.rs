use rusqlite::Connection;
use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    ContentRating, CreateProjectInput, ReaderContract, SaveSceneDraftInput,
};
use spindle_core::style::{
    ApplyStyleProfileInput, ApplyStyleRevisionPatchInput, ArchiveStyleProfileInput,
    CompareStyleProfilesInput, CreateStyleProfileFromMarkdownInput, GetStyleProfileInput,
    ListStyleProfilesInput, PreviewStyleRevisionPatchInput, StyleDriftSummaryScore,
    StyleProfileApplyMode,
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
        force_apply: None,
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
            force_apply: None,
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
            force_apply: None,
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
            chapter_id: None,
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
            force_apply: None,
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
            chapter_id: None,
        })
        .await
        .unwrap_err();

    assert!(err.to_string().contains("scene does not belong to project"));
}

#[tokio::test]
async fn test_style_drift_rejects_cross_project_chapter_and_ambiguous_targets() {
    let (tmp, svc) = fresh_service().await;

    let source_project = svc
        .create_project(CreateProjectInput {
            name: "Source Chapter Project".to_string(),
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
            name: "Other Chapter Project".to_string(),
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
            profile_name: "Source Chapter Profile".to_string(),
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
            force_apply: None,
        })
        .await
        .unwrap()
        .profile;

    let cross_project_err = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: source_project.project_id.clone(),
            profile_id: Some(profile.profile_id.clone()),
            chapter_id: Some(other_project.chapter_id),
            scene_id: None,
            raw_text: None,
        })
        .await
        .unwrap_err();
    assert!(
        cross_project_err
            .to_string()
            .contains("chapter does not belong to project")
    );

    let ambiguous_err = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: source_project.project_id,
            profile_id: Some(profile.profile_id),
            chapter_id: Some(source_project.chapter_id),
            scene_id: None,
            raw_text: Some("Mixed target input should be rejected.".to_string()),
        })
        .await
        .unwrap_err();
    assert!(
        ambiguous_err
            .to_string()
            .contains("provide only one of chapter_id, scene_id, or raw_text")
    );
}

#[tokio::test]
async fn test_active_style_profile_lifecycle() {
    let (tmp, svc) = fresh_service().await;

    // 1. Create a project
    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Test Project Active".to_string(),
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

    // Verify initially no active profile is set
    let project = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(project.active_style_profile_id, None);

    // 2. Prepare test data paths.
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/style");
    let src1 = base.join("fast-serial-chapter-1.md");
    let dest1 = tmp.path().join("fast-serial-chapter-1.md");
    std::fs::copy(&src1, &dest1).unwrap();
    let fixture1 = dest1.to_string_lossy().to_string();

    // 3. Create style profile from Markdown
    let create_out = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Profile A".to_string(),
            source_paths: vec![fixture1.clone()],
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
            force_apply: None,
        })
        .await
        .unwrap();
    let profile_a = create_out.profile;

    let create_out_b = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Profile B".to_string(),
            source_paths: vec![fixture1],
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
            force_apply: None,
        })
        .await
        .unwrap();
    let profile_b = create_out_b.profile;

    // Check drift before active profile: it should fail when no profile_id is specified
    let drift_err = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: project_id.clone(),
            profile_id: None,
            scene_id: None,
            raw_text: Some("Jake ran. Jake winced.".to_string()),
            chapter_id: None,
        })
        .await;
    assert!(drift_err.is_err());
    assert!(
        drift_err
            .unwrap_err()
            .to_string()
            .contains("No profile_id specified and no active style profile is set")
    );

    // Apply Profile A
    let apply_a = svc
        .apply_style_profile(ApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile_a.profile_id.clone(),
            mode: StyleProfileApplyMode::Merge,
        })
        .await
        .unwrap();
    assert_eq!(apply_a.profile_id, profile_a.profile_id);

    // Verify active_style_profile_id is set to Profile A
    let project = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(
        project.active_style_profile_id,
        Some(profile_a.profile_id.clone())
    );

    // Verify drift checking defaults to Profile A
    let drift_default_a = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: project_id.clone(),
            profile_id: None,
            scene_id: None,
            raw_text: Some("Jake ran. Jake winced.".to_string()),
            chapter_id: None,
        })
        .await
        .unwrap();
    assert_eq!(drift_default_a.profile_id, profile_a.profile_id);

    // Apply Profile B
    let apply_b = svc
        .apply_style_profile(ApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile_b.profile_id.clone(),
            mode: StyleProfileApplyMode::Merge,
        })
        .await
        .unwrap();
    assert_eq!(apply_b.profile_id, profile_b.profile_id);

    // Verify active_style_profile_id is set to Profile B
    let project = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(
        project.active_style_profile_id,
        Some(profile_b.profile_id.clone())
    );

    // Verify drift checking defaults to Profile B
    let drift_default_b = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: project_id.clone(),
            profile_id: None,
            scene_id: None,
            raw_text: Some("Jake ran. Jake winced.".to_string()),
            chapter_id: None,
        })
        .await
        .unwrap();
    assert_eq!(drift_default_b.profile_id, profile_b.profile_id);

    // Get list of applications to retrieve audit application IDs
    let apps = svc
        .list_style_profile_applications(spindle_core::style::ListStyleProfileApplicationsInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(apps.applications.len(), 2);
    let app_b = &apps.applications[0]; // most recent is first
    let app_a = &apps.applications[1];

    // Rollback Profile B
    svc.rollback_style_profile_application(
        spindle_core::style::RollbackStyleProfileApplicationInput {
            project_id: project_id.clone(),
            application_id: app_b.id.clone(),
        },
    )
    .await
    .unwrap();

    // Verify active_style_profile_id restored to Profile A
    let project = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(
        project.active_style_profile_id,
        Some(profile_a.profile_id.clone())
    );

    // Rollback Profile A
    svc.rollback_style_profile_application(
        spindle_core::style::RollbackStyleProfileApplicationInput {
            project_id: project_id.clone(),
            application_id: app_a.id.clone(),
        },
    )
    .await
    .unwrap();

    // Verify active_style_profile_id is now None
    let project = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(project.active_style_profile_id, None);
}

#[tokio::test]
async fn test_profile_quality_report() {
    let (tmp, svc) = fresh_service().await;

    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Quality Project".to_string(),
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

    // 1. Thin corpus: under 3000 words
    let thin_file = tmp.path().join("thin.md");
    std::fs::write(
        &thin_file,
        "This is a thin corpus. It has very few words. Just fifteen words in total.",
    )
    .unwrap();
    let thin_path = thin_file.to_string_lossy().to_string();

    let thin_res = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Thin Profile".to_string(),
            source_paths: vec![thin_path.clone()],
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
            force_apply: None,
        })
        .await
        .unwrap();

    assert_eq!(
        thin_res.profile.quality.classification,
        spindle_core::style::StyleProfileQualityClassification::Thin
    );
    assert!(
        thin_res
            .profile
            .quality
            .warnings
            .iter()
            .any(|w| w.contains("thin"))
    );

    // Auto-apply should fail for thin profile
    let auto_apply_err = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Thin Apply Fail".to_string(),
            source_paths: vec![thin_path.clone()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(true),
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: None,
            force_apply: None,
        })
        .await;
    assert!(auto_apply_err.is_err());
    assert!(
        auto_apply_err
            .unwrap_err()
            .to_string()
            .contains("quality is too low to auto-apply")
    );
    let profiles_after_failed_apply = svc
        .list_style_profiles(ListStyleProfilesInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        profiles_after_failed_apply
            .profiles
            .iter()
            .all(|profile| profile.name != "Thin Apply Fail")
    );

    // Auto-apply should succeed if force_apply is true
    let auto_apply_forced = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Thin Apply Forced".to_string(),
            source_paths: vec![thin_path],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(true),
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: None,
            force_apply: Some(true),
        })
        .await;
    assert!(auto_apply_forced.is_ok());

    // 2. Inconsistent corpus: dialogue ratios are extremely different between chunks
    // We want total words >= 3000 to avoid Thin classification taking priority.
    let inconsistent_file_1 = tmp.path().join("inconsistent_1.md");
    let inconsistent_file_2 = tmp.path().join("inconsistent_2.md");

    // File 1: 1600 words, no dialogue, short sentences
    let mut f1_content = String::new();
    for _ in 0..400 {
        f1_content.push_str("Jake ran fast. He was quick. The wind blew. He jumped high. ");
    }
    std::fs::write(&inconsistent_file_1, f1_content).unwrap();

    // File 2: 1600 words, all dialogue, long sentences
    let mut f2_content = String::new();
    for _ in 0..60 {
        f2_content.push_str("\"This is an extremely long dialogue sentence that we repeat over and over to make sure the average sentence length is high,\" he said to the other person who was standing right next to him. ");
    }
    std::fs::write(&inconsistent_file_2, f2_content).unwrap();

    let inconsistent_res = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Inconsistent Profile".to_string(),
            source_paths: vec![
                inconsistent_file_1.to_string_lossy().to_string(),
                inconsistent_file_2.to_string_lossy().to_string(),
            ],
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
            force_apply: None,
        })
        .await
        .unwrap();

    assert_eq!(
        inconsistent_res.profile.quality.classification,
        spindle_core::style::StyleProfileQualityClassification::Inconsistent
    );
    assert!(
        inconsistent_res
            .profile
            .quality
            .warnings
            .iter()
            .any(|w| w.contains("inconsistent"))
    );
}

#[tokio::test]
async fn test_drift_chapter_level_and_summary_score() {
    let (tmp, svc) = fresh_service().await;

    // 1. Create project
    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Drift Chapter Project".to_string(),
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
    let chapter_id = project_out.chapter_id;

    // 2. Create profile with short sentences
    let corpus_file = tmp.path().join("corpus.md");
    std::fs::write(
        &corpus_file,
        "Jake ran. Jake jumped. Jake succeeded. Jake was quick. Jake won.",
    )
    .unwrap();

    let _profile = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Short Sentences".to_string(),
            source_paths: vec![corpus_file.to_string_lossy().to_string()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(true), // apply it so it is the active profile
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: Some(true),
            force_apply: Some(true),
        })
        .await
        .unwrap()
        .profile;

    // 3. Add scenes to Chapter 1 with extremely long sentences (strong drift)
    let long_sentence_prose = "This is a remarkably long sentence designed specifically to mismatch with the style profile and produce a deterministic warning about sentence length which should be easily detected by the style drift checking service method. And this is another extremely long and winding sentence that just goes on and on to ensure the average sentence length exceeds the threshold significantly.";

    let scene_1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: Some(chapter_id.clone()),
            scene_order: 1,
            full_text: long_sentence_prose.to_string(),
            summary: "Scene 1".to_string(),
            content_rating: ContentRating::Teen,
            ..Default::default()
        })
        .await
        .unwrap();

    let scene_2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: Some(chapter_id.clone()),
            scene_order: 2,
            full_text: long_sentence_prose.to_string(),
            summary: "Scene 2".to_string(),
            content_rating: ContentRating::Teen,
            ..Default::default()
        })
        .await
        .unwrap();

    // 4. Run drift check for the full chapter
    let drift_res = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: project_id.clone(),
            profile_id: None, // default to active profile
            chapter_id: Some(chapter_id.clone()),
            scene_id: None,
            raw_text: None,
        })
        .await
        .unwrap();

    // Verify scene-scoped findings and metric deltas
    assert!(!drift_res.findings.is_empty());

    let findings_s1: Vec<_> = drift_res
        .findings
        .iter()
        .filter(|f| f.scene_id.as_ref() == Some(&scene_1.scene_id))
        .collect();
    let findings_s2: Vec<_> = drift_res
        .findings
        .iter()
        .filter(|f| f.scene_id.as_ref() == Some(&scene_2.scene_id))
        .collect();

    assert!(!findings_s1.is_empty(), "Should have findings for scene 1");
    assert!(!findings_s2.is_empty(), "Should have findings for scene 2");

    // Check for metric deltas
    let s1_sentence_finding = findings_s1
        .iter()
        .find(|f| f.metric_name.as_deref() == Some("average_sentence_words"))
        .unwrap();
    assert!(s1_sentence_finding.metric_delta.unwrap() > 10.0);

    // Check summary score
    assert_eq!(drift_res.summary_score, StyleDriftSummaryScore::StrongDrift);
}

#[tokio::test]
async fn test_compare_style_profiles() {
    let (tmp, svc) = fresh_service().await;

    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Compare Project".to_string(),
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

    // Profile A: Short sentences, no dialogue
    let file_a = tmp.path().join("a.md");
    std::fs::write(
        &file_a,
        "Jake ran. Jake jumped. Jake succeeded. Jake was quick. Jake won.",
    )
    .unwrap();
    let profile_a = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Short Sentences".to_string(),
            source_paths: vec![file_a.to_string_lossy().to_string()],
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
            force_apply: None,
        })
        .await
        .unwrap()
        .profile;

    // Profile B: Long sentences, lots of dialogue
    let file_b = tmp.path().join("b.md");
    std::fs::write(
        &file_b,
        "\"This is a very long dialogue sentence that we write to make sure the metrics delta between A and B is extremely high,\" he said to the other person who was standing right next to him. \"And this is another extremely long dialogue sentence to ensure consistency.\""
    )
    .unwrap();

    let profile_b = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Long Sentences Dialogue".to_string(),
            source_paths: vec![file_b.to_string_lossy().to_string()],
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
            force_apply: None,
        })
        .await
        .unwrap()
        .profile;

    let compare_res = svc
        .compare_style_profiles(CompareStyleProfilesInput {
            project_id: project_id.clone(),
            profile_id_a: profile_a.profile_id,
            profile_id_b: profile_b.profile_id,
        })
        .await
        .unwrap();

    assert!(compare_res.metric_deltas.average_sentence_words_delta.abs() > 3.0);
    assert!(compare_res.metric_deltas.dialogue_word_ratio_delta.abs() > 0.15);
    assert!(compare_res.likely_material_change);
    assert!(!compare_res.change_reasons.is_empty());
}

#[tokio::test]
async fn test_archive_style_profile_behavior() {
    let (tmp, svc) = fresh_service().await;

    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Archive Project".to_string(),
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

    let corpus_file = tmp.path().join("corpus.md");
    std::fs::write(
        &corpus_file,
        "Jake ran. Jake jumped. Jake succeeded. Jake was quick. Jake won.",
    )
    .unwrap();

    let profile = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Archive Target".to_string(),
            source_paths: vec![corpus_file.to_string_lossy().to_string()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(true), // active profile
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: Some(true),
            force_apply: Some(true),
        })
        .await
        .unwrap()
        .profile;

    // Try to archive active profile without force=true: should fail
    let archive_err = svc
        .archive_style_profile(ArchiveStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile.profile_id.clone(),
            force: None,
        })
        .await;
    assert!(archive_err.is_err());
    assert!(
        archive_err
            .unwrap_err()
            .to_string()
            .contains("Cannot archive the active style profile")
    );

    // Archive active profile with force=true: should succeed
    let archive_ok = svc
        .archive_style_profile(ArchiveStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile.profile_id.clone(),
            force: Some(true),
        })
        .await
        .unwrap();
    assert!(!archive_ok.archived_at.is_empty());

    // Verify it is no longer the active style profile
    let project = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(project.active_style_profile_id, None);

    // Verify it is excluded from list_style_profiles
    let list_res = svc
        .list_style_profiles(ListStyleProfilesInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap();
    assert!(
        list_res
            .profiles
            .iter()
            .all(|p| p.profile_id != profile.profile_id)
    );

    let drift_err = svc
        .check_style_against_profile(spindle_core::style::CheckStyleAgainstProfileInput {
            project_id: project_id.clone(),
            profile_id: Some(profile.profile_id.clone()),
            scene_id: None,
            raw_text: Some("A short style check sample.".to_string()),
            chapter_id: None,
        })
        .await
        .unwrap_err();
    assert!(drift_err.to_string().contains("style profile is archived"));

    let compare_err = svc
        .compare_style_profiles(CompareStyleProfilesInput {
            project_id,
            profile_id_a: profile.profile_id.clone(),
            profile_id_b: profile.profile_id,
        })
        .await
        .unwrap_err();
    assert!(
        compare_err
            .to_string()
            .contains("style profile is archived")
    );
}

#[tokio::test]
async fn test_privacy_metrics_only_prompt_construction() {
    use std::io::{Read, Write};

    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Spawn test TcpListener on a random local port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    // Write a spindle.toml file that routes `style_analyze` to our mock HTTP server
    let config_path = tmp.path().join("spindle.toml");
    std::fs::write(
        &config_path,
        format!(
            r####"
[health_check]
enabled = false

[[agents]]
id = "mock-analyze"
name = "Mock Analyze Agent"
provider = "openai-compatible"
endpoint = "http://{}/v1"
model = "mock-model"

[[routing]]
route = "style_analyze"
agent = "mock-analyze"
"####,
            addr
        ),
    )
    .unwrap();

    let router = ModelRouter::default();
    router
        .configure(Some(&config_path.display().to_string()))
        .unwrap();

    let repo = Repository::with_model_router(pool, data_dir, router);
    let svc = SqliteSpindleService::new(repo);

    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Privacy Project".to_string(),
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

    // Create a corpus file containing a distinctive "secret" prose sequence
    let corpus_file = tmp.path().join("secret_prose.md");
    let secret_prose = "Jake stared at the status screen. The glowing runes hovered in the dark air. It was a secret sequence of words.";
    std::fs::write(&corpus_file, secret_prose).unwrap();
    let corpus_path = corpus_file.to_string_lossy().to_string();

    // 1. Run in metrics_only mode: the captured prompt MUST NOT contain the secret prose
    let svc_clone = svc.clone();
    let project_id_clone = project_id.clone();
    let corpus_path_clone = corpus_path.clone();

    let listener_clone = listener.try_clone().unwrap();
    let thread_handle_1 = std::thread::spawn(move || {
        let (mut stream, _) = listener_clone.accept().unwrap();
        let mut buffer = [0_u8; 16384];
        let count = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..count]).to_string();

        let mock_response = r#"{
            "summary": "Mock style profile guidance summary",
            "pov": "third_person_close",
            "tense": "past",
            "narrator_distance": "close",
            "narrator_voice": {
              "comedy_density": "none",
              "pacing_feel": "contemplative",
              "interiority_ratio": "heavy interiority",
              "emotional_register": "brooding-and-reflective",
              "chapter_ending_style": "resolution",
              "notes": []
            },
            "pacing": [],
            "paragraphing": [],
            "sentence_rhythm": [],
            "diction": [],
            "dialogue": [],
            "exposition": [],
            "interiority": [],
            "humor_or_tension": [],
            "scene_structure": [],
            "do_rules": [],
            "avoid_rules": [],
            "prompt_snippet": "Snippet"
        }"#;

        let response_body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": mock_response
                }
            }]
        })
        .to_string();

        let response_http = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response_http.as_bytes()).unwrap();
        stream.flush().unwrap();

        request
    });

    let _create_out_1 = svc_clone
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id_clone,
            profile_name: "Metrics Only Profile".to_string(),
            source_paths: vec![corpus_path_clone],
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
            force_apply: None,
        })
        .await
        .unwrap();

    let captured_request_1 = thread_handle_1.join().unwrap();
    assert!(
        captured_request_1.contains("[METRICS ONLY MODE]"),
        "Request should be flagged as metrics only"
    );
    assert!(
        !captured_request_1.contains("Jake stared at the status screen"),
        "Source prose should not be included in metrics_only prompt"
    );

    // 2. Run in standard mode: the captured prompt MUST contain the secret prose
    let svc_clone_2 = svc.clone();
    let project_id_clone_2 = project_id.clone();
    let corpus_path_clone_2 = corpus_path.clone();

    // Spawn another listener accept (since we set connection: close)
    let thread_handle_2 = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 16384];
        let count = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..count]).to_string();

        let mock_response = r#"{
            "summary": "Mock style profile guidance summary",
            "pov": "third_person_close",
            "tense": "past",
            "narrator_distance": "close",
            "narrator_voice": {
              "comedy_density": "none",
              "pacing_feel": "contemplative",
              "interiority_ratio": "heavy interiority",
              "emotional_register": "brooding-and-reflective",
              "chapter_ending_style": "resolution",
              "notes": []
            },
            "pacing": [],
            "paragraphing": [],
            "sentence_rhythm": [],
            "diction": [],
            "dialogue": [],
            "exposition": [],
            "interiority": [],
            "humor_or_tension": [],
            "scene_structure": [],
            "do_rules": [],
            "avoid_rules": [],
            "prompt_snippet": "Snippet"
        }"#;

        let response_body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": mock_response
                }
            }]
        })
        .to_string();

        let response_http = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response_http.as_bytes()).unwrap();
        stream.flush().unwrap();

        request
    });

    let _create_out_2 = svc_clone_2
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id_clone_2,
            profile_name: "Standard Profile".to_string(),
            source_paths: vec![corpus_path_clone_2],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(false),
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: Some(false),
            force_apply: None,
        })
        .await
        .unwrap();

    let captured_request_2 = thread_handle_2.join().unwrap();
    assert!(
        captured_request_2.contains("[CORPUS CHUNKS]"),
        "Request should contain corpus chunks section"
    );
    assert!(
        captured_request_2.contains("Jake stared at the status screen"),
        "Source prose should be included in standard prompt"
    );
}

#[tokio::test]
async fn test_plan_style_revision_suite() {
    let (tmp, svc) = fresh_service().await;

    // 1. Create project 1
    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Style Revision Project".to_string(),
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
    let chapter_id = project_out.chapter_id;

    // 2. Create style profile with short sentences
    let corpus_file = tmp.path().join("corpus.md");
    std::fs::write(
        &corpus_file,
        "Jake ran. Jake jumped. Jake succeeded. Jake was quick. Jake won.",
    )
    .unwrap();

    let profile = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Short Sentences Profile".to_string(),
            source_paths: vec![corpus_file.to_string_lossy().to_string()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(true), // apply it to make it the active style profile
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: Some(true),
            force_apply: Some(true),
        })
        .await
        .unwrap()
        .profile;

    // Test 1: Active profile default behavior & Raw text planning
    // We pass profile_id: None. It should default to the active profile.
    let raw_text = "This is a remarkably long sentence designed specifically to mismatch with the style profile and produce a deterministic warning about sentence length which should be easily detected by the style drift checking service method. And this is another extremely long and winding sentence that just goes on and on to ensure the average sentence length exceeds the threshold significantly.";

    let raw_plan = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: None,
            raw_text: Some(raw_text.to_string()),
            scene_id: None,
            chapter_id: None,
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: None,
        })
        .await
        .unwrap();

    assert_eq!(raw_plan.project_id, project_id);
    assert_eq!(raw_plan.profile_id, profile.profile_id);
    assert!(raw_plan.target_summary.contains("Raw text target"));
    assert_eq!(
        raw_plan.drift_summary_score,
        spindle_core::style::StyleDriftSummaryScore::MildDrift
    );
    assert_eq!(raw_plan.mutates_prose, false);
    assert!(raw_plan.rewrite_examples.is_none());

    // Check findings and steps
    assert!(!raw_plan.findings.is_empty());
    assert_eq!(
        raw_plan.findings[0].severity,
        spindle_core::style::StyleRevisionSeverity::Warning
    );
    assert_eq!(raw_plan.findings[0].category, "sentence_length");
    assert!(
        raw_plan.findings[0]
            .evidence_summary
            .contains("sentence length")
    );

    assert!(!raw_plan.steps.is_empty());
    assert_eq!(raw_plan.steps[0].order, 1);
    assert_eq!(raw_plan.steps[0].finding_category, "sentence_length");
    assert_eq!(
        raw_plan.steps[0].target_scope,
        spindle_core::style::StyleRevisionTargetScope::RawText
    );
    assert_eq!(
        raw_plan.steps[0].confidence,
        spindle_core::style::StyleRevisionConfidence::High
    );

    // Test 2: Ambiguous target rejection (e.g. providing both raw_text and chapter_id)
    let ambig_err = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: None,
            raw_text: Some(raw_text.to_string()),
            scene_id: None,
            chapter_id: Some(chapter_id.clone()),
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: None,
        })
        .await
        .unwrap_err();
    assert!(
        ambig_err
            .to_string()
            .contains("provide only one of chapter_id, scene_id, or raw_text")
    );

    // Test 3: Project ownership guard for scenes
    // Create a different project
    let other_project_out = svc
        .create_project(CreateProjectInput {
            name: "Other Project".to_string(),
            project_type: "novel".to_string(),
            genre: "sci-fi".to_string(),
            reader_contract: ReaderContract {
                promise: "Spaceships".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let other_project_id = other_project_out.project_id;
    let other_chapter_id = other_project_out.chapter_id;

    let other_scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: other_project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: Some(other_chapter_id.clone()),
            scene_order: 1,
            full_text: "Other project scene text.".to_string(),
            summary: "Other Scene".to_string(),
            content_rating: ContentRating::Teen,
            ..Default::default()
        })
        .await
        .unwrap();

    // Planning with project 1's ID but other_scene's ID should fail ownership guard
    let ownership_err = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: None,
            raw_text: None,
            scene_id: Some(other_scene.scene_id.clone()),
            chapter_id: None,
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: None,
        })
        .await
        .unwrap_err();
    assert!(
        ownership_err
            .to_string()
            .contains("scene does not belong to project")
    );

    // Test 4: Scene planning
    let my_scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: Some(chapter_id.clone()),
            scene_order: 1,
            full_text: raw_text.to_string(),
            summary: "My Scene".to_string(),
            content_rating: ContentRating::Teen,
            ..Default::default()
        })
        .await
        .unwrap();

    let scene_plan = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: None,
            raw_text: None,
            scene_id: Some(my_scene.scene_id.clone()),
            chapter_id: None,
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: None,
        })
        .await
        .unwrap();

    assert!(scene_plan.target_summary.contains("Scene:"));
    assert_eq!(
        scene_plan.findings[0].scene_id,
        Some(my_scene.scene_id.clone())
    );
    assert_eq!(
        scene_plan.steps[0].target_scope,
        spindle_core::style::StyleRevisionTargetScope::Scene
    );
    assert_eq!(
        scene_plan.steps[0].target_id,
        Some(my_scene.scene_id.clone())
    );

    // Test 5: Chapter planning with scene-scoped suggestions
    let chapter_plan = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: None,
            raw_text: None,
            scene_id: None,
            chapter_id: Some(chapter_id.clone()),
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: None,
        })
        .await
        .unwrap();

    assert!(chapter_plan.target_summary.contains("Chapter:"));
    // Verify it preserves the scene-scoped finding for the scene in the chapter
    assert_eq!(
        chapter_plan.findings[0].scene_id,
        Some(my_scene.scene_id.clone())
    );
    assert_eq!(
        chapter_plan.steps[0].target_scope,
        spindle_core::style::StyleRevisionTargetScope::Scene
    );

    // Test 6: Archived profile rejection
    svc.archive_style_profile(ArchiveStyleProfileInput {
        project_id: project_id.clone(),
        profile_id: profile.profile_id.clone(),
        force: Some(true),
    })
    .await
    .unwrap();

    let archived_err = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: Some(profile.profile_id.clone()),
            raw_text: Some("Some text".to_string()),
            scene_id: None,
            chapter_id: None,
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: None,
        })
        .await
        .unwrap_err();
    assert!(
        archived_err
            .to_string()
            .contains("style profile is archived")
    );

    // Test 7: Optional rewrite examples (disabled by default)
    // Create another active style profile to replace the archived one
    let profile_2 = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Active Profile 2".to_string(),
            source_paths: vec![corpus_file.to_string_lossy().to_string()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(true),
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: Some(true),
            force_apply: Some(true),
        })
        .await
        .unwrap()
        .profile;

    // Plan without examples first
    let no_examples_plan = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: Some(profile_2.profile_id.clone()),
            raw_text: Some("This is a remarkably long sentence that triggers drift.".to_string()),
            scene_id: None,
            chapter_id: None,
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: Some(false),
        })
        .await
        .unwrap();
    assert!(no_examples_plan.rewrite_examples.is_none());

    // Plan with examples enabled
    let examples_plan = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: Some(profile_2.profile_id.clone()),
            raw_text: Some("This is a remarkably long sentence that triggers drift.".to_string()),
            scene_id: None,
            chapter_id: None,
            max_suggestions: None,
            metrics_only: None,
            include_rewrite_examples: Some(true),
        })
        .await
        .unwrap();

    assert!(examples_plan.rewrite_examples.is_some());
    let examples = examples_plan.rewrite_examples.unwrap();
    assert_eq!(examples.len(), 1);
    assert_eq!(
        examples[0].original_prose,
        "She went to the store. She bought some milk. She was happy."
    );
    assert!(
        examples[0]
            .revised_prose
            .contains("Walking down the dusty aisle")
    );

    // max_suggestions should cap findings, steps, and rewrite examples.
    let zero_suggestion_plan = svc
        .plan_style_revision(spindle_core::style::PlanStyleRevisionInput {
            project_id: project_id.clone(),
            profile_id: Some(profile_2.profile_id.clone()),
            raw_text: Some("This is a remarkably long sentence that triggers drift.".to_string()),
            scene_id: None,
            chapter_id: None,
            max_suggestions: Some(0),
            metrics_only: None,
            include_rewrite_examples: Some(true),
        })
        .await
        .unwrap();
    assert!(zero_suggestion_plan.findings.is_empty());
    assert!(zero_suggestion_plan.steps.is_empty());
    assert_eq!(
        zero_suggestion_plan
            .rewrite_examples
            .unwrap_or_default()
            .len(),
        0
    );

    // Test 8: No prose persistence
    let conn = Connection::open(tmp.path().join("svc.db")).unwrap();
    let query_res: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='style_revision_plan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(query_res, 0);
}

#[tokio::test]
async fn test_style_revision_patch_lifecycle() {
    let (tmp, svc) = fresh_service().await;

    // 1. Create project
    let project_out = svc
        .create_project(CreateProjectInput {
            name: "Patch Project".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "An adventurous tale".to_string(),
                style_notes: vec!["User-authored style note".to_string()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let project_id = project_out.project_id;

    // Create a chapter
    let book = svc
        .repository()
        .list_books_by_project(&project_id)
        .await
        .unwrap()[0]
        .clone();
    let chapter_out = svc
        .create_chapter(spindle_core::models::CreateChapterInput {
            project_id: project_id.clone(),
            book_number: Some(book.book_number),
            book_id: Some(book.id.clone()),
            chapter_number: Some(1),
            title: Some("Chapter One".to_string()),
        })
        .await
        .unwrap();
    let chapter_id = chapter_out.chapter_id;

    // Create two scenes in the chapter
    let scene_input_1 = SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: book.book_number,
        chapter_number: 1,
        chapter_id: Some(chapter_id.clone()),
        scene_order: 1,
        full_text: "This is a remarkably long sentence that triggers drift.".to_string(),
        summary: "Scene 1 summary".to_string(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        research_source_ids: Vec::new(),
        research_note_ids: Vec::new(),
        research_claim_ids: Vec::new(),
        research_query_pack_input: None,
        research_context_hash: None,
    };
    let scene_out_1 = svc.save_scene_draft(scene_input_1).await.unwrap();
    let scene_id_1 = scene_out_1.scene_id;

    let scene_input_2 = SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: book.book_number,
        chapter_number: 1,
        chapter_id: Some(chapter_id.clone()),
        scene_order: 2,
        full_text: "Another scene with some basic prose.".to_string(),
        summary: "Scene 2 summary".to_string(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        research_source_ids: Vec::new(),
        research_note_ids: Vec::new(),
        research_claim_ids: Vec::new(),
        research_query_pack_input: None,
        research_context_hash: None,
    };
    let scene_out_2 = svc.save_scene_draft(scene_input_2).await.unwrap();
    let scene_id_2 = scene_out_2.scene_id;

    // Create a style profile
    let dest_dir = tmp.path();
    let dest_file = dest_dir.join("profile.md");
    std::fs::write(&dest_file, "Jake ran. Jake won.").unwrap();
    let create_out = svc
        .create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
            project_id: project_id.clone(),
            profile_name: "Test Profile".to_string(),
            source_paths: vec![dest_file.to_string_lossy().to_string()],
            recursive: Some(false),
            include_globs: None,
            exclude_globs: None,
            max_files: None,
            max_bytes_per_file: None,
            max_total_words: None,
            apply: Some(true), // make it active style profile
            application_mode: None,
            source_sample_word_budget: None,
            metrics_only: None,
            force_apply: Some(true),
        })
        .await
        .unwrap();
    let profile = create_out.profile;

    // A. Preview for single scene
    let preview_scene_out = svc
        .preview_style_revision_patch(PreviewStyleRevisionPatchInput {
            project_id: project_id.clone(),
            scene_id: Some(scene_id_1.clone()),
            chapter_id: None,
            profile_id: Some(profile.profile_id.clone()),
            max_suggestions: None,
            instructions: Some("Align closely".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(preview_scene_out.scenes.len(), 1);
    assert_eq!(preview_scene_out.scenes[0].scene_id, scene_id_1);
    assert!(
        preview_scene_out.scenes[0]
            .revised_text
            .contains("This is a short sentence. It does not trigger drift.")
    );

    // Verify preview is non-mutating
    let current_scene_1 = svc.repository().get_scene(&scene_id_1).await.unwrap();
    assert_eq!(
        current_scene_1.full_text,
        "This is a remarkably long sentence that triggers drift."
    );

    // B. Preview for chapter (returns per-scene patches)
    let preview_chapter_out = svc
        .preview_style_revision_patch(PreviewStyleRevisionPatchInput {
            project_id: project_id.clone(),
            scene_id: None,
            chapter_id: Some(chapter_id.clone()),
            profile_id: Some(profile.profile_id.clone()),
            max_suggestions: None,
            instructions: None,
        })
        .await
        .unwrap();

    assert_eq!(preview_chapter_out.scenes.len(), 2);
    assert_eq!(preview_chapter_out.scenes[0].scene_id, scene_id_1);
    assert_eq!(preview_chapter_out.scenes[1].scene_id, scene_id_2);

    // C. Apply patch updates scene text
    let apply_input = ApplyStyleRevisionPatchInput {
        project_id: project_id.clone(),
        profile_id: profile.profile_id.clone(),
        scenes: preview_chapter_out.scenes.clone(),
        model_receipt: preview_chapter_out.model_receipt.clone(),
    };
    let apply_out = svc.apply_style_revision_patch(apply_input).await.unwrap();
    assert_eq!(apply_out.applied_scene_ids.len(), 2);

    // Check scene text is updated
    let updated_scene_1 = svc.repository().get_scene(&scene_id_1).await.unwrap();
    assert_eq!(
        updated_scene_1.full_text,
        "This is a short sentence. It does not trigger drift."
    );
    let updated_scene_2 = svc.repository().get_scene(&scene_id_2).await.unwrap();
    assert!(updated_scene_2.full_text.contains("(revised)"));

    // D. Stale patch rejection
    // Let's do another preview
    let preview_stale = svc
        .preview_style_revision_patch(PreviewStyleRevisionPatchInput {
            project_id: project_id.clone(),
            scene_id: Some(scene_id_1.clone()),
            chapter_id: None,
            profile_id: Some(profile.profile_id.clone()),
            max_suggestions: None,
            instructions: None,
        })
        .await
        .unwrap();

    // Now mutate the scene text so the hash changes
    let scene_mutate_input = SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: book.book_number,
        chapter_number: 1,
        chapter_id: Some(chapter_id.clone()),
        scene_order: 1,
        full_text: "Mutated scene text to trigger stale hash.".to_string(),
        summary: "Scene 1 summary".to_string(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        research_source_ids: Vec::new(),
        research_note_ids: Vec::new(),
        research_claim_ids: Vec::new(),
        research_query_pack_input: None,
        research_context_hash: None,
    };
    svc.save_scene_draft(scene_mutate_input).await.unwrap();

    // Apply the stale patch, should fail
    let apply_stale_input = ApplyStyleRevisionPatchInput {
        project_id: project_id.clone(),
        profile_id: profile.profile_id.clone(),
        scenes: preview_stale.scenes.clone(),
        model_receipt: preview_stale.model_receipt.clone(),
    };
    let apply_stale_res = svc.apply_style_revision_patch(apply_stale_input).await;
    assert!(apply_stale_res.is_err());
    assert!(
        apply_stale_res
            .unwrap_err()
            .to_string()
            .contains("patch is stale")
    );

    // E. Archived profile rejection
    svc.archive_style_profile(ArchiveStyleProfileInput {
        project_id: project_id.clone(),
        profile_id: profile.profile_id.clone(),
        force: Some(true),
    })
    .await
    .unwrap();

    let preview_archived_res = svc
        .preview_style_revision_patch(PreviewStyleRevisionPatchInput {
            project_id: project_id.clone(),
            scene_id: Some(scene_id_1.clone()),
            chapter_id: None,
            profile_id: Some(profile.profile_id.clone()),
            max_suggestions: None,
            instructions: None,
        })
        .await;
    assert!(preview_archived_res.is_err());
    assert!(
        preview_archived_res
            .unwrap_err()
            .to_string()
            .contains("is archived and cannot be used")
    );

    let apply_archived_res = svc
        .apply_style_revision_patch(ApplyStyleRevisionPatchInput {
            project_id: project_id.clone(),
            profile_id: profile.profile_id.clone(),
            scenes: preview_stale.scenes.clone(),
            model_receipt: preview_stale.model_receipt.clone(),
        })
        .await;
    assert!(apply_archived_res.is_err());
    assert!(
        apply_archived_res
            .unwrap_err()
            .to_string()
            .contains("is archived and cannot be used")
    );

    // F. Cross-project scene/chapter rejection
    // Create another project
    let project_other_out = svc
        .create_project(CreateProjectInput {
            name: "Other Project".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "Other promise".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let project_other_id = project_other_out.project_id;

    let preview_cross_res = svc
        .preview_style_revision_patch(PreviewStyleRevisionPatchInput {
            project_id: project_other_id.clone(),
            scene_id: Some(scene_id_1.clone()), // scene_1 belongs to project_id, not project_other_id
            chapter_id: None,
            profile_id: None,
            max_suggestions: None,
            instructions: None,
        })
        .await;
    assert!(preview_cross_res.is_err());

    // G. Audit stores hashes/metadata but not prose
    let audits = svc
        .repository()
        .list_style_revision_patch_audits(&project_id)
        .await
        .unwrap();
    assert_eq!(audits.len(), 1);
    let audit = &audits[0];
    assert_eq!(audit.profile_id, profile.profile_id);
    assert_eq!(
        audit.target_ids,
        vec![scene_id_1.clone(), scene_id_2.clone()]
    );
    assert!(audit.before_hashes.len() == 2);
    assert!(audit.after_hashes.len() == 2);

    let conn = Connection::open(tmp.path().join("svc.db")).unwrap();
    let audit_prose_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM style_revision_patch_audit \
             WHERE target_ids_json LIKE '%sentence%' \
                OR before_hashes_json LIKE '%sentence%' \
                OR after_hashes_json LIKE '%sentence%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_prose_count, 0);
}
