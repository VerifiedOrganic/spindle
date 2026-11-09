use spindle_adapters::ModelRouter;
use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    ConfigureAgentsInput, ContentRating, CreateProjectInput, ReaderContract, SaveSceneDraftInput,
};
use spindle_core::style::{
    ApplyStyleProfileInput, CreateStyleProfileFromMarkdownInput, CreateStyleRevisionPatchInput,
    ListStyleProfileApplicationsInput, ListStyleProfilesInput, PreviewApplyStyleProfileInput,
    PreviewRefreshStyleProfileInput, PreviewStyleRevisionPatchInput, RefreshStyleProfileInput,
    StyleProfileApplyMode,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);

    let config_path = tmp.path().join("style-agent.toml");
    std::fs::write(
        &config_path,
        r#"
[health_check]
enabled = false

[[agents]]
id = "local-style"
name = "Local Style Stub"
provider = "local"
endpoint = "local"
model = "local-style"
ratings = ["general", "teen", "mature", "explicit"]

[[routing]]
route = "style_analyze"
agent = "local-style"

[[routing]]
route = "style_revise"
agent = "local-style"
"#,
    )
    .unwrap();
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.display().to_string()),
    })
    .unwrap();
    (tmp, svc)
}

async fn project(svc: &SqliteSpindleService, name: &str) -> String {
    svc.create_project(CreateProjectInput {
        name: name.to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "Fast-paced intrigue.".to_string(),
            style_notes: vec!["Existing note".to_string()],
            boundaries: Vec::new(),
        },
    })
    .await
    .unwrap()
    .project_id
}

async fn profile(svc: &SqliteSpindleService, tmp: &TempDir, project_id: &str) -> String {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/style");
    let dest1 = tmp.path().join("fast-serial-chapter-1.md");
    let dest2 = tmp.path().join("dialogue-heavy-scene.md");
    std::fs::copy(base.join("fast-serial-chapter-1.md"), &dest1).unwrap();
    std::fs::copy(base.join("dialogue-heavy-scene.md"), &dest2).unwrap();
    svc.create_style_profile_from_markdown(CreateStyleProfileFromMarkdownInput {
        project_id: project_id.to_string(),
        profile_name: "Dry Run Profile".to_string(),
        source_paths: vec![
            dest1.to_string_lossy().to_string(),
            dest2.to_string_lossy().to_string(),
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
    .unwrap()
    .profile
    .profile_id
}

#[tokio::test]
async fn apply_style_profile_dry_run_has_zero_side_effects() {
    let (tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Style Apply Dry Run").await;
    let profile_id = profile(&svc, &tmp, &project_id).await;
    let before_project = svc.repository().get_project(&project_id).await.unwrap();
    let before_rules = svc
        .repository()
        .list_world_rules_by_project_and_branch(
            &project_id,
            before_project.active_branch_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let before_applications = svc
        .list_style_profile_applications(ListStyleProfileApplicationsInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap()
        .applications;

    let dry = svc
        .apply_style_profile(ApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile_id.clone(),
            mode: StyleProfileApplyMode::Merge,
            force: None,
            dry_run: true,
        })
        .await
        .unwrap();
    assert!(dry.dry_run);
    let preview = dry
        .preview
        .clone()
        .expect("dry run returns the apply preview");
    assert!(!preview.added_style_notes.is_empty());
    assert_eq!(dry.narrator_voice, preview.after_narrator_voice);
    assert_eq!(
        dry.reader_contract_style_notes,
        before_project
            .reader_contract
            .style_notes
            .iter()
            .cloned()
            .chain(preview.added_style_notes.iter().cloned())
            .collect::<Vec<_>>()
    );

    let after_project = svc.repository().get_project(&project_id).await.unwrap();
    assert_eq!(
        serde_json::to_value(&after_project.narrator_voice).unwrap(),
        serde_json::to_value(&before_project.narrator_voice).unwrap(),
        "dry run must not change narrator voice"
    );
    assert_eq!(
        after_project.reader_contract.style_notes, before_project.reader_contract.style_notes,
        "dry run must not change style notes"
    );
    assert_eq!(
        after_project.active_style_profile_id, before_project.active_style_profile_id,
        "dry run must not activate the profile"
    );
    let after_rules = svc
        .repository()
        .list_world_rules_by_project_and_branch(
            &project_id,
            after_project.active_branch_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_rules.len(), before_rules.len());
    let after_applications = svc
        .list_style_profile_applications(ListStyleProfileApplicationsInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap()
        .applications;
    assert_eq!(after_applications.len(), before_applications.len());

    // The release-R preview twin returns the same diff and remains non-mutating.
    let preview_twin = svc
        .preview_apply_style_profile(PreviewApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile_id.clone(),
            mode: StyleProfileApplyMode::Merge,
        })
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&preview_twin).unwrap(),
        serde_json::to_value(&preview).unwrap()
    );

    // The live call still writes.
    let live = svc
        .apply_style_profile(ApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id,
            mode: StyleProfileApplyMode::Merge,
            force: None,
            dry_run: false,
        })
        .await
        .unwrap();
    assert!(!live.dry_run);
    assert!(live.preview.is_none());
    let after_live = svc.repository().get_project(&project_id).await.unwrap();
    assert_ne!(
        serde_json::to_value(&after_live.narrator_voice).unwrap(),
        serde_json::to_value(&before_project.narrator_voice).unwrap(),
        "a live apply must change narrator voice"
    );

    // A dry run against the already-applied profile reports the same no-op
    // style-rule id as the live receipt.
    let dry_again = svc
        .apply_style_profile(ApplyStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: live.profile_id.clone(),
            mode: StyleProfileApplyMode::Merge,
            force: None,
            dry_run: true,
        })
        .await
        .unwrap();
    assert_eq!(dry_again.style_rule_id, live.style_rule_id);
}

#[tokio::test]
async fn refresh_style_profile_dry_run_has_zero_side_effects() {
    let (tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Style Refresh Dry Run").await;
    let profile_id = profile(&svc, &tmp, &project_id).await;
    let persisted_before = svc
        .repository()
        .get_style_profile(&project_id, &profile_id)
        .await
        .unwrap()
        .unwrap();
    let before = svc
        .list_style_profiles(ListStyleProfilesInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap()
        .profiles;

    let dry = svc
        .refresh_style_profile(RefreshStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile_id.clone(),
            apply_after_refresh: None,
            force_apply: None,
            metrics_only: Some(true),
            dismiss_candidate_ids: Vec::new(),
            dry_run: true,
        })
        .await
        .unwrap();
    assert!(dry.dry_run);
    assert!(!dry.applied);
    assert!(dry.application.is_none());
    assert!(dry.consumed_candidate_ids.is_empty());
    assert!(dry.dismissed_candidate_ids.is_empty());
    let preview = dry
        .preview
        .clone()
        .expect("dry run returns the refresh preview");
    assert_eq!(preview.old_profile_id, profile_id);
    assert_eq!(preview.candidate_profile_id, dry.new_profile.profile_id);

    let persisted_after = svc
        .repository()
        .get_style_profile(&project_id, &profile_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted_after.source_policy.metrics_only, persisted_before.source_policy.metrics_only,
        "dry run must not rewrite the persisted source policy"
    );
    let after = svc
        .list_style_profiles(ListStyleProfilesInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap()
        .profiles;
    assert_eq!(
        after.len(),
        before.len(),
        "dry run must not insert a profile"
    );
    assert_eq!(
        after[0].version_number, before[0].version_number,
        "dry run must not create a new profile version"
    );

    let preview_twin = svc
        .preview_refresh_style_profile(PreviewRefreshStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile_id.clone(),
            metrics_only: Some(true),
        })
        .await
        .unwrap();
    assert_eq!(preview_twin.old_profile_summary.profile_id, profile_id);
    assert_eq!(
        serde_json::to_value(&preview_twin.candidate_profile_summary.guidance).unwrap(),
        serde_json::to_value(&dry.new_profile.guidance).unwrap()
    );
    assert_eq!(preview_twin.apply_safety, preview.apply_safety);
    assert_eq!(preview_twin.material_change, preview.material_change);
    assert_eq!(
        serde_json::to_value(preview_twin.quality_report.classification).unwrap(),
        serde_json::to_value(preview.candidate_quality.classification).unwrap()
    );
    assert_eq!(preview.metric_deltas[0].metric, "average_sentence_words");
    assert_eq!(
        preview.metric_deltas[0].delta,
        preview_twin.metric_deltas.average_sentence_words_delta
    );
}

#[tokio::test]
async fn refresh_style_profile_dry_run_rejects_mutating_options() {
    let (tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Style Refresh Dry Run Options").await;
    let profile_id = profile(&svc, &tmp, &project_id).await;

    let error = svc
        .refresh_style_profile(RefreshStyleProfileInput {
            project_id: project_id.clone(),
            profile_id: profile_id.clone(),
            apply_after_refresh: Some(true),
            force_apply: None,
            metrics_only: Some(true),
            dismiss_candidate_ids: Vec::new(),
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("apply_after_refresh"),
        "{error:#}"
    );

    let error = svc
        .refresh_style_profile(RefreshStyleProfileInput {
            project_id,
            profile_id,
            apply_after_refresh: None,
            force_apply: None,
            metrics_only: Some(true),
            dismiss_candidate_ids: vec!["candidate:x".to_string()],
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("dismiss_candidate_ids"),
        "{error:#}"
    );
}

#[tokio::test]
async fn create_style_revision_patch_is_non_mutating_and_matches_preview() {
    let (tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Style Patch Dry Run").await;
    let profile_id = profile(&svc, &tmp, &project_id).await;
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Jake stared at the status screen. The numbers refused to move.".to_string(),
            summary: "Jake checks the screen.".to_string(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
    let before = svc.repository().get_scene(&scene.scene_id).await.unwrap();
    let before_versions = svc
        .repository()
        .list_scene_versions(&scene.scene_id)
        .await
        .unwrap();

    let created = svc
        .create_style_revision_patch(CreateStyleRevisionPatchInput {
            project_id: project_id.clone(),
            scene_id: Some(scene.scene_id.clone()),
            chapter_id: None,
            profile_id: Some(profile_id.clone()),
            max_suggestions: None,
            instructions: None,
            run_evaluation: Some(false),
            run_validator_preflight: Some(false),
            minimum_improvement_score: None,
        })
        .await
        .unwrap();
    assert_eq!(created.scenes.len(), 1);
    assert_eq!(created.scenes[0].scene_id, scene.scene_id);

    let preview = svc
        .preview_style_revision_patch(PreviewStyleRevisionPatchInput {
            project_id,
            scene_id: Some(scene.scene_id.clone()),
            chapter_id: None,
            profile_id: Some(profile_id),
            max_suggestions: None,
            instructions: None,
            run_evaluation: Some(false),
            run_validator_preflight: Some(false),
            minimum_improvement_score: None,
        })
        .await
        .unwrap();
    assert_eq!(preview.scenes.len(), 1);
    assert_eq!(preview.scenes[0].scene_id, scene.scene_id);

    let after = svc.repository().get_scene(&scene.scene_id).await.unwrap();
    assert_eq!(
        after.full_text, before.full_text,
        "patch creation must not write prose"
    );
    let after_versions = svc
        .repository()
        .list_scene_versions(&scene.scene_id)
        .await
        .unwrap();
    assert_eq!(after_versions.len(), before_versions.len());
    assert_eq!(
        serde_json::to_value(&created).unwrap(),
        serde_json::to_value(&preview).unwrap()
    );
}

#[tokio::test]
async fn unified_undo_only_rolls_back_the_current_applied_style_head() {
    use spindle_core::models::UndoChangeInput;

    let (tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Style Undo Head").await;
    let first = profile(&svc, &tmp, &project_id).await;
    svc.apply_style_profile(ApplyStyleProfileInput {
        project_id: project_id.clone(),
        profile_id: first,
        mode: StyleProfileApplyMode::Merge,
        force: None,
        dry_run: false,
    })
    .await
    .unwrap();
    let second = profile(&svc, &tmp, &project_id).await;
    svc.apply_style_profile(ApplyStyleProfileInput {
        project_id: project_id.clone(),
        profile_id: second,
        mode: StyleProfileApplyMode::Merge,
        force: None,
        dry_run: false,
    })
    .await
    .unwrap();

    let apps = svc
        .list_style_profile_applications(ListStyleProfileApplicationsInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap()
        .applications;
    assert!(apps.len() >= 2, "expected two style applications");
    let latest = &apps[0];
    let older = &apps[1];

    let denied = svc
        .undo_change(UndoChangeInput {
            project_id: project_id.clone(),
            operation_id: format!("style_application:{}", older.id),
            expected_head: latest.id.clone(),
            dry_run: true,
        })
        .await
        .unwrap_err();
    assert!(
        format!("{denied:#}").contains("current applied head"),
        "older style application must be refused: {denied:#}"
    );

    svc.undo_change(UndoChangeInput {
        project_id: project_id.clone(),
        operation_id: format!("style_application:{}", latest.id),
        expected_head: latest.id.clone(),
        dry_run: false,
    })
    .await
    .unwrap();

    let after = svc
        .list_style_profile_applications(ListStyleProfileApplicationsInput {
            project_id: project_id.clone(),
        })
        .await
        .unwrap()
        .applications;
    let older_after = after
        .iter()
        .find(|app| app.id == older.id)
        .expect("older application");
    assert_eq!(older_after.rollback_status, "not_rolled_back");
    svc.undo_change(UndoChangeInput {
        project_id,
        operation_id: format!("style_application:{}", older.id),
        expected_head: older.id.clone(),
        dry_run: false,
    })
    .await
    .unwrap();
}
