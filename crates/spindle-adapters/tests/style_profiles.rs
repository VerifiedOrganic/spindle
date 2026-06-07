use rusqlite::Connection;
use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{CreateProjectInput, ReaderContract};
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
