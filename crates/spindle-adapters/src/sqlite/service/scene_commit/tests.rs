use super::*;

use spindle_core::models::{CharacterEmotionalProfileData, CharacterVoiceProfileData};
use spindle_core::models::{
    CommitSceneCanonicalFactEntry, CommitSceneChapterSummary, CommitSceneCharacterStateEntry,
    CommitSceneCharacterStatePatch, CommitSceneContinuityPackage, CommitSceneInput,
    CommitSceneKnowledgeEntry, CommitSceneProseSource, CommitSceneQuantityStateEntry,
    CommitSceneRelationshipEntry, CommitSceneStoryPlacement, CommitSceneTimelineEventEntry,
    ContentRating, CreateBranchInput,
    CreateCharacterInput, CreateFutureKnowledgeInput, CreateLocationInput, CreateProjectInput,
    CreateRelationshipInput, PrepareSceneInput, PrepareSceneIntent, ReaderContract,
    RegisterCanonicalFactInput, SaveSceneDraftInput, SceneAuthorship, SceneBeatAnnotationInput,
    SwitchBranchInput,
};

fn base_commit(project_id: &str, text: &str) -> CommitSceneInput {
    CommitSceneInput {
        project_id: project_id.to_string(),
        branch_id: None,
        preparation_id: None,
        context_revision_token: None,
        dependency_tokens: None,
        scene_id: None,
        book_number: Some(1),
        chapter_number: Some(1),
        scene_order: Some(1),
        expected_scene_version: None,
        full_text: text.to_string(),
        prose_path: None,
        expected_sha256: None,
        summary: "scene summary".to_string(),
        content_rating: ContentRating::General,
        location_id: None,
        tone: None,
        authorship: SceneAuthorship::Human,
        generation_receipt: None,
        continuity: CommitSceneContinuityPackage::default(),
        beat_annotations: Vec::new(),
        chapter_summary: None,
        run_id: None,
        expected_phase: None,
        dry_run: false,
        idempotency_key: None,
    }
}

async fn fresh_workspace_service() -> (tempfile::TempDir, SqliteSpindleService) {
    use crate::ai::ModelRouter;
    use crate::sqlite::{Repository, SqlitePool};

    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("book");
    let data_dir = root.join(".spindle");
    std::fs::create_dir_all(&data_dir).unwrap();
    let pool = SqlitePool::open(&data_dir.join("spindle.db"))
        .await
        .unwrap();
    let repository = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    (temp, SqliteSpindleService::new(repository))
}

async fn project(svc: &SqliteSpindleService, name: &str) -> CreateProjectOutput {
    svc.create_project(CreateProjectInput {
        name: name.to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "clean continuity".to_string(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    })
    .await
    .unwrap()
}

async fn character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
    svc.create_character(CreateCharacterInput {
        aliases: Vec::new(),
        project_id: project_id.to_string(),
        name: name.to_string(),
        summary: "A traveller.".to_string(),
        role: "protagonist".to_string(),
        realm: None,
        voice_profile: CharacterVoiceProfileData::default(),
        emotional_profile: CharacterEmotionalProfileData::default(),
        initial_state: None,
    })
    .await
    .unwrap()
    .character_id
}

async fn location(svc: &SqliteSpindleService, project_id: &str) -> String {
    svc.create_location(CreateLocationInput {
        project_id: project_id.to_string(),
        name: "The Lockhouse".to_string(),
        kind: "harbor".to_string(),
        realm: None,
        summary: "A lamplit stone room above the canal.".to_string(),
        initial_state: Default::default(),
    })
    .await
    .unwrap()
    .location_id
}

#[tokio::test]
async fn commit_scene_file_prose_receipt_matches_inline() {
    let (temp, svc) = fresh_workspace_service().await;
    let root = temp.path().join("book");
    std::fs::create_dir_all(root.join("scenes")).unwrap();
    let prose = "The lockhouse was quiet.\nMara barred the door.\n";
    std::fs::write(root.join("scenes/ch1.md"), prose).unwrap();
    let project = project(&svc, "prose-path-receipt").await;

    let mut inline = base_commit(&project.project_id, prose);
    inline.scene_order = Some(1);
    let inline = svc.commit_scene(inline).await.unwrap();
    let inline_receipt = inline.prose.expect("inline prose receipt");

    let mut file = base_commit(&project.project_id, "");
    file.scene_order = Some(2);
    file.prose_path = Some("scenes/ch1.md".to_string());
    let file = svc.commit_scene(file).await.unwrap();
    let file_receipt = file.prose.expect("file prose receipt");

    assert_eq!(inline_receipt.source, CommitSceneProseSource::Inline);
    assert_eq!(inline_receipt.path, None);
    assert_eq!(file_receipt.source, CommitSceneProseSource::File);
    assert_eq!(file_receipt.path.as_deref(), Some("scenes/ch1.md"));
    assert_eq!(file_receipt.byte_count, prose.len());
    assert_eq!(file_receipt.sha256, inline_receipt.sha256);
    assert_eq!(file_receipt.byte_count, inline_receipt.byte_count);
}

#[tokio::test]
async fn commit_scene_dry_run_has_zero_side_effects_and_replays_idempotently() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-idempotency").await;
    let character_id = character(&svc, &project.project_id, "Mara").await;

    let mut input = base_commit(&project.project_id, "Mara locked the door.");
    input.idempotency_key = Some("scene-1".to_string());
    input.continuity.character_states = vec![CommitSceneCharacterStateEntry {
        character_id: character_id.clone(),
        changes: CommitSceneCharacterStatePatch {
            notes: Some(vec!["wary".to_string()]),
            source_summary: Some("the lockhouse exchange".to_string()),
            ..Default::default()
        },
    }];
    input.beat_annotations = vec![SceneBeatAnnotationInput {
        beat_type: "decision".to_string(),
        summary: "Mara chooses to bar the door.".to_string(),
    }];
    input.chapter_summary = Some(CommitSceneChapterSummary {
        summary: "Mara secures the lockhouse.".to_string(),
        ..Default::default()
    });
    input.dry_run = true;

    let preview = svc.commit_scene(input.clone()).await.unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.blocking_count, 0);
    assert_eq!(preview.applied.total_applied, 1);
    assert_eq!(preview.beat_count, 1);
    assert_eq!(preview.version_number, 1);
    let branch_id = svc
        .repository
        .active_branch_id_public(&project.project_id)
        .await
        .unwrap();
    assert!(
        svc.repository
            .find_scene_by_natural_key(&project.project_id, &branch_id, 1, 1, 1)
            .await
            .unwrap()
            .is_none(),
        "dry run must not create a scene"
    );
    assert!(
        svc.repository
            .find_scene_commit_receipt("scene-1")
            .await
            .unwrap()
            .is_none(),
        "dry run must not claim the idempotency key"
    );

    input.dry_run = false;
    let committed = svc.commit_scene(input.clone()).await.unwrap();
    assert!(!committed.dry_run);
    assert!(!committed.idempotent_replay);
    assert_eq!(committed.applied.character_state_ids.len(), 1);
    assert_eq!(committed.beat_count, 1);
    assert!(committed.chapter_summary_saved);
    let cursor = svc
        .repository
        .get_writer_position(&project.project_id, &branch_id)
        .await
        .unwrap()
        .expect("writer cursor moves on a live commit");
    assert_eq!(
        cursor.scene_id.as_deref(),
        Some(committed.scene_id.as_str())
    );
    assert!(
        svc.repository
            .get_scene_beat_annotation(&branch_id, &committed.scene_id)
            .await
            .unwrap()
            .is_some(),
        "beats land with the commit"
    );
    assert!(
        svc.repository
            .get_chapter_summary(&project.project_id, &branch_id, 1, 1)
            .await
            .unwrap()
            .is_some(),
        "chapter summary lands with the commit"
    );

    // Same key + payload: the stored receipt returns, with no second write.
    let replay = svc.commit_scene(input.clone()).await.unwrap();
    assert!(replay.idempotent_replay);
    assert_eq!(replay.operation_id, committed.operation_id);
    assert_eq!(replay.scene_id, committed.scene_id);
    assert_eq!(
        replay.applied.total_applied,
        committed.applied.total_applied
    );

    // Same key + different payload: rejected.
    let mut conflicting = input;
    conflicting.summary = "a different summary".to_string();
    let error = svc.commit_scene(conflicting).await.unwrap_err();
    assert!(
        error.to_string().contains("different commit payload"),
        "{error:#}"
    );
}

#[tokio::test]
async fn commit_scene_rejects_stale_version_and_preparation_token_without_writing() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-stale").await;
    let location_id = location(&svc, &project.project_id).await;
    let saved = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The lockhouse was quiet.".to_string(),
            summary: "quiet".to_string(),
            content_rating: ContentRating::General,
            location_id: Some(location_id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut input = base_commit(&project.project_id, "The lockhouse was not quiet.");
    input.scene_id = Some(saved.scene_id.clone());
    input.expected_scene_version = Some(42);
    let stale = svc.commit_scene(input.clone()).await.unwrap_err();
    assert!(stale.to_string().contains("stale expected_scene_version"));
    let after = svc.repository.get_scene(&saved.scene_id).await.unwrap();
    assert_eq!(after.full_text, "The lockhouse was quiet.");

    let prepared = svc
        .prepare_scene(PrepareSceneInput {
            project_id: project.project_id.clone(),
            scene_id: Some(saved.scene_id.clone()),
            location_id: Some(location_id),
            intent: Some(PrepareSceneIntent::Revise),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stale_token = input.clone();
    stale_token.expected_scene_version = Some(1);
    stale_token.context_revision_token = Some("bogus-token".to_string());
    let error = svc.commit_scene(stale_token).await.unwrap_err();
    assert!(error.to_string().contains("stale context_revision_token"));

    let mut live = input;
    live.expected_scene_version = Some(1);
    live.context_revision_token = Some(prepared.context_revision_token.clone());
    let output = svc.commit_scene(live).await.unwrap();
    assert_eq!(output.blocking_count, 0);
    let after = svc.repository.get_scene(&saved.scene_id).await.unwrap();
    assert_eq!(after.full_text, "The lockhouse was not quiet.");

    // The token prepared before the commit now fails the freshness check
    // because the scene text dependency changed.
    let mut replay_token = base_commit(&project.project_id, "Another pass.");
    replay_token.scene_id = Some(saved.scene_id);
    replay_token.expected_scene_version = Some(2);
    replay_token.context_revision_token = Some(prepared.context_revision_token);
    let error = svc.commit_scene(replay_token).await.unwrap_err();
    assert!(error.to_string().contains("stale context_revision_token"));
}

#[tokio::test]
async fn commit_scene_blocking_gate_writes_nothing() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-blocked").await;
    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: project.project_id.clone(),
        scene_id: None,
        book_number: 1,
        chapter_number: 1,
        fact_type: Some("world".to_string()),
        key: Some("sky_color".to_string()),
        value: Some("blue".to_string()),
        context: None,
        subject_table: Some("project".to_string()),
        subject_id: Some(project.project_id.clone()),
        predicate: Some("sky_color".to_string()),
        value_kind: Some("string".to_string()),
        value_text: Some("blue".to_string()),
        value_number: None,
        value_unit: None,
        value_json: None,
        aliases: Vec::new(),
        scope: None,
        valid_from: None,
        valid_until: None,
        legacy_untyped: None,
        supersedes_fact_id: None,
        secrecy: None,
    })
    .await
    .unwrap();

    let mut input = base_commit(&project.project_id, "The sky was green over the harbor.");
    input.continuity.canonical_facts = vec![CommitSceneCanonicalFactEntry {
        subject_table: Some("project".to_string()),
        subject_id: Some(project.project_id.clone()),
        predicate: Some("sky_color".to_string()),
        value_text: Some("green".to_string()),
        value_kind: Some("string".to_string()),
        ..Default::default()
    }];

    let output = svc.commit_scene(input).await.unwrap();
    assert!(output.blocking_count > 0);
    assert!(!output.dry_run);
    assert!(
        output
            .issues
            .iter()
            .any(|issue| issue.check_type == "canonical_fact_consistency")
    );
    let branch_id = svc
        .repository
        .active_branch_id_public(&project.project_id)
        .await
        .unwrap();
    assert!(
        svc.repository
            .find_scene_by_natural_key(&project.project_id, &branch_id, 1, 1, 1)
            .await
            .unwrap()
            .is_none(),
        "a blocking gate must not create the scene"
    );
    let facts = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(&project.project_id, &branch_id)
        .await
        .unwrap();
    assert_eq!(
        facts.len(),
        1,
        "the declared contradicting fact was not written"
    );
}

#[tokio::test]
async fn commit_scene_replaces_child_branch_continuity_deltas_and_canon() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-child-branch").await;
    let a = character(&svc, &project.project_id, "Rowan").await;
    let b = character(&svc, &project.project_id, "Sylvi").await;
    svc.create_relationship(CreateRelationshipInput {
        character_a_id: a.clone(),
        character_b_id: b.clone(),
        relationship_type: "companions".to_string(),
        initial_trust: 50,
        initial_tension: 30,
        dynamics: vec!["wary allies".to_string()],
    })
    .await
    .unwrap();
    let child = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "child".to_string(),
            branch_type: "alternate".to_string(),
            description: None,
            parent_branch_id: Some(project.branch_id.clone()),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child.branch_id.clone(),
    })
    .await
    .unwrap();

    let mut first = base_commit(&project.project_id, "Rowan shared the rations.");
    first.continuity.relationship_updates = vec![CommitSceneRelationshipEntry {
        character_a_id: a.clone(),
        character_b_id: b.clone(),
        trust_delta: 10,
        tension_delta: -5,
        reason: "shared rations".to_string(),
    }];
    let first = svc.commit_scene(first).await.unwrap();
    let rels = svc
        .repository
        .list_relationships_by_branch(&child.branch_id)
        .await
        .unwrap();
    assert_eq!((rels[0].trust, rels[0].tension), (60, 25));

    // Recommitting with a different delta replaces the scene's prior
    // contribution on the child branch instead of stacking on top.
    let mut second = base_commit(&project.project_id, "Rowan saved Sylvi from the ambush.");
    second.scene_id = Some(first.scene_id.clone());
    second.continuity.relationship_updates = vec![CommitSceneRelationshipEntry {
        character_a_id: a.clone(),
        character_b_id: b.clone(),
        trust_delta: 15,
        tension_delta: -10,
        reason: "saved from ambush".to_string(),
    }];
    svc.commit_scene(second).await.unwrap();
    let rels = svc
        .repository
        .list_relationships_by_branch(&child.branch_id)
        .await
        .unwrap();
    assert_eq!((rels[0].trust, rels[0].tension), (65, 20));

    // Omitting the relationship this time reverses the scene's contribution.
    let mut third = base_commit(&project.project_id, "They walked in silence.");
    third.scene_id = Some(first.scene_id);
    svc.commit_scene(third).await.unwrap();
    let rels = svc
        .repository
        .list_relationships_by_branch(&child.branch_id)
        .await
        .unwrap();
    assert_eq!((rels[0].trust, rels[0].tension), (50, 30));

    // Canon replacement on the child branch: a superseding fact retires the
    // earlier one without leaking to the parent branch.
    let mut fact_one = base_commit(&project.project_id, "The blade was iron.");
    fact_one.continuity.canonical_facts = vec![CommitSceneCanonicalFactEntry {
        subject_table: Some("project".to_string()),
        subject_id: Some(project.project_id.clone()),
        predicate: Some("blade_metal".to_string()),
        value_text: Some("iron".to_string()),
        value_kind: Some("string".to_string()),
        ..Default::default()
    }];
    let fact_one = svc.commit_scene(fact_one).await.unwrap();
    let first_fact_id = fact_one.applied.canonical_fact_ids[0].clone();
    let mut fact_two = base_commit(&project.project_id, "The blade was reforged in steel.");
    fact_two.continuity.canonical_facts = vec![CommitSceneCanonicalFactEntry {
        subject_table: Some("project".to_string()),
        subject_id: Some(project.project_id.clone()),
        predicate: Some("blade_metal".to_string()),
        value_text: Some("steel".to_string()),
        value_kind: Some("string".to_string()),
        supersedes_fact_id: Some(first_fact_id.clone()),
        ..Default::default()
    }];
    svc.commit_scene(fact_two).await.unwrap();
    let child_facts = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(&project.project_id, &child.branch_id)
        .await
        .unwrap();
    assert!(
        child_facts.iter().all(|fact| fact.id != first_fact_id),
        "the superseded fact is inactive on the child branch"
    );
    assert!(
        child_facts
            .iter()
            .any(|fact| fact.predicate == "blade_metal"
                && fact.value_text.as_deref() == Some("steel")),
        "the replacement fact is active on the child branch"
    );
    let parent_facts = svc
        .repository
        .list_active_canonical_facts_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert!(
        parent_facts.is_empty(),
        "child-branch canon must not leak to the parent branch"
    );
}

#[tokio::test]
async fn commit_scene_version_advances_on_each_revision_and_matches_prepare() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-version").await;
    let location_id = location(&svc, &project.project_id).await;
    let saved = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The lockhouse was quiet.".to_string(),
            summary: "quiet".to_string(),
            content_rating: ContentRating::General,
            location_id: Some(location_id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut first = base_commit(&project.project_id, "The lockhouse was not quiet.");
    first.scene_id = Some(saved.scene_id.clone());
    first.expected_scene_version = Some(1);
    let first = svc.commit_scene(first).await.unwrap();
    assert_eq!(first.version_number, 2);

    let prepared = svc
        .prepare_scene(PrepareSceneInput {
            project_id: project.project_id.clone(),
            scene_id: Some(saved.scene_id.clone()),
            location_id: Some(location_id),
            intent: Some(PrepareSceneIntent::Revise),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(prepared.scene_version, Some(2));

    // The same expected version can no longer overwrite the revision.
    let mut stale = base_commit(&project.project_id, "A second overwrite attempt.");
    stale.scene_id = Some(saved.scene_id.clone());
    stale.expected_scene_version = Some(1);
    let error = svc.commit_scene(stale).await.unwrap_err();
    assert!(
        error.to_string().contains("stale expected_scene_version"),
        "{error:#}"
    );

    let mut second = base_commit(&project.project_id, "The lockhouse stayed quiet.");
    second.scene_id = Some(saved.scene_id.clone());
    second.expected_scene_version = Some(2);
    let second = svc.commit_scene(second).await.unwrap();
    assert_eq!(second.version_number, 3);
    let prepared = svc
        .prepare_scene(PrepareSceneInput {
            project_id: project.project_id.clone(),
            scene_id: Some(saved.scene_id),
            intent: Some(PrepareSceneIntent::Revise),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(prepared.scene_version, Some(3));
}

#[tokio::test]
async fn commit_scene_concurrent_creates_with_expected_zero_only_one_wins() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-concurrent").await;

    let mut first = base_commit(&project.project_id, "The first create attempt landed.");
    first.expected_scene_version = Some(0);
    let mut second = base_commit(&project.project_id, "The second create attempt must lose.");
    second.expected_scene_version = Some(0);

    let (first_result, second_result) =
        tokio::join!(svc.commit_scene(first), svc.commit_scene(second));
    let successes = [first_result.is_ok(), second_result.is_ok()]
        .into_iter()
        .filter(|ok| *ok)
        .count();
    assert_eq!(
        successes,
        1,
        "exactly one racing create with expected version 0 may win: {:?} / {:?}",
        first_result.as_ref().err().map(|error| error.to_string()),
        second_result.as_ref().err().map(|error| error.to_string())
    );
    let error = if first_result.is_err() {
        first_result.unwrap_err()
    } else {
        second_result.unwrap_err()
    };
    assert!(
        error.to_string().contains("stale expected_scene_version"),
        "{error:#}"
    );
    let branch_id = svc
        .repository
        .active_branch_id_public(&project.project_id)
        .await
        .unwrap();
    let scene = svc
        .repository
        .find_scene_by_natural_key(&project.project_id, &branch_id, 1, 1, 1)
        .await
        .unwrap()
        .expect("the winning create persisted one scene");
    assert!(
        scene.full_text == "The first create attempt landed."
            || scene.full_text == "The second create attempt must lose."
    );
}

#[tokio::test]
async fn commit_scene_rejects_stale_authoring_handoff_plan_revision() {
    use crate::sqlite::records::{AuthoringRun, AuthoringRunChapter, AuthoringRunScene};

    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-handoff").await;
    let branch_id = svc
        .repository
        .active_branch_id_public(&project.project_id)
        .await
        .unwrap();
    let run_id = format!(
        "authoring_run:{}",
        ulid::Ulid::new().to_string().to_lowercase()
    );
    let now = chrono::Utc::now();
    svc.repository
        .save_authoring_run(
            AuthoringRun {
                id: run_id.clone(),
                project_id: project.project_id.clone(),
                active_branch_id: branch_id,
                book_number: 1,
                start_chapter: 1,
                end_chapter: 1,
                checkpoint_interval: 1,
                last_checkpoint_end_chapter: 0,
                artifacts_dir: "../artifacts".to_string(),
                editorial_directives: Vec::new(),
                status: "active".to_string(),
                created_at: now,
                updated_at: now,
                mining_policy: None,
                max_revise_attempts: None,
                checkpoint_policy: None,
                replan_policy: None,
            },
            vec![AuthoringRunChapter {
                authoring_run_id: run_id.clone(),
                chapter_number: 1,
                planned: true,
                synopsis: "stale handoff".to_string(),
                pov_character_id: None,
                status: "in_progress".to_string(),
                summary_saved: false,
                summary_artifact_path: None,
                replan_status: None,
                replan_detail: None,
            }],
            vec![AuthoringRunScene {
                authoring_run_id: run_id.clone(),
                chapter_number: 1,
                scene_order: 1,
                character_ids: Vec::new(),
                location_id: "location:x".to_string(),
                content_rating: "general".to_string(),
                tone: None,
                source_path: None,
                phase: "draft_saved".to_string(),
                scene_id: None,
                scene_artifact_path: None,
                draft_diagnostics: Some(serde_json::json!({
                    "superseded_handoff_plan_revision": 1,
                    "handoff_plan_revision": 1
                })),
                blocked_reason: None,
                research_required: None,
                research_tags: Vec::new(),
                explicit_query: None,
                mine_status: None,
                mine_detail: None,
                verify_status: None,
                verify_detail: None,
                revise_attempts: 0,
                last_finding_fingerprint: None,
            }],
            Vec::new(),
        )
        .await
        .unwrap();

    let mut input = base_commit(&project.project_id, "Stale handoff prose must not commit.");
    input.run_id = Some(run_id);
    let error = svc.commit_scene(input).await.unwrap_err();
    assert!(
        error.to_string().contains("stale host draft handoff"),
        "{error:#}"
    );
    assert!(
        svc.repository
            .find_scene_by_natural_key(
                &project.project_id,
                &svc.repository
                    .active_branch_id_public(&project.project_id)
                    .await
                    .unwrap(),
                1,
                1,
                1
            )
            .await
            .unwrap()
            .is_none(),
        "a rejected handoff must not create a scene"
    );
}

#[tokio::test]
async fn commit_scene_retcon_scan_uses_proposed_revision_text() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-retcon").await;
    let character_id = character(&svc, &project.project_id, "Mara").await;
    svc.create_future_knowledge(CreateFutureKnowledgeInput {
        project_id: project.project_id.clone(),
        character_id,
        knowledge_summary: "the hidden vault".to_string(),
        source: "an old letter".to_string(),
        learned_at: spindle_core::models::StoryPlacement {
            book_number: 3,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        expires_at: None,
        notes: Vec::new(),
    })
    .await
    .unwrap();

    // A revision that INTRODUCES future knowledge must be blocked even though
    // the persisted text was clean.
    let clean = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara waited by the door.".to_string(),
            summary: "waiting".to_string(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
    let mut introducing = base_commit(&project.project_id, "Mara mentioned the hidden vault.");
    introducing.scene_id = Some(clean.scene_id.clone());
    introducing.expected_scene_version = Some(1);
    let blocked = svc.commit_scene(introducing).await.unwrap();
    assert!(blocked.blocking_count > 0);
    assert!(
        blocked
            .issues
            .iter()
            .any(|issue| issue.check_type == "retcon_reachability"),
        "{:?}",
        blocked.issues
    );
    let unchanged = svc.repository.get_scene(&clean.scene_id).await.unwrap();
    assert_eq!(unchanged.full_text, "Mara waited by the door.");

    // A revision that REMOVES an existing violation must not inherit the
    // persisted text's finding.
    let offending = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Mara mentioned the hidden vault.".to_string(),
            summary: "telling".to_string(),
            content_rating: ContentRating::General,
            ..Default::default()
        })
        .await
        .unwrap();
    let mut removing = base_commit(&project.project_id, "Mara waited in silence.");
    removing.scene_id = Some(offending.scene_id.clone());
    removing.expected_scene_version = Some(1);
    let clean_output = svc.commit_scene(removing).await.unwrap();
    assert_eq!(clean_output.blocking_count, 0);
    assert!(
        clean_output
            .issues
            .iter()
            .all(|issue| issue.check_type != "retcon_reachability")
    );
}

#[tokio::test]
async fn commit_scene_applies_quantity_promises_and_timeline_atomically() {
    let (_tmp, svc) = fresh_service_local().await;
    let project = project(&svc, "commit-package").await;
    let character_id = character(&svc, &project.project_id, "Mara").await;

    let mut input = base_commit(&project.project_id, "Mara spent two coins at the lock.");
    input.continuity.quantity_states = vec![CommitSceneQuantityStateEntry {
        subject_table: "character".to_string(),
        subject_id: character_id.clone(),
        measure: "coin".to_string(),
        state: Default::default(),
    }];
    input.continuity.narrative_promises = vec![spindle_core::models::CommitScenePromiseEntry {
        promise_type: "mystery".to_string(),
        description: "The lock-keeper knows more than she admits.".to_string(),
        planted_at: CommitSceneStoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        planned_payoff: None,
        notes: Vec::new(),
    }];
    input.continuity.timeline_events = vec![CommitSceneTimelineEventEntry {
        title: "The lockhouse bargain".to_string(),
        event_type: "deal".to_string(),
        placement: CommitSceneStoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        summary: "Mara trades the cipher coin.".to_string(),
        related_entity_ids: vec![character_id.clone()],
    }];
    input.continuity.knowledge_learned = vec![CommitSceneKnowledgeEntry {
        character_id: character_id.clone(),
        fact: "The lock-keeper is lying.".to_string(),
        source_summary: Some("the too-quick answer".to_string()),
        secret_of_fact_id: None,
        reader_visible: Some(true),
    }];

    let output = svc.commit_scene(input).await.unwrap();
    assert_eq!(output.blocking_count, 0);
    assert_eq!(output.applied.quantity_state_ids.len(), 1);
    assert_eq!(output.applied.narrative_promise_ids.len(), 1);
    assert_eq!(output.applied.timeline_event_ids.len(), 1);
    assert_eq!(output.applied.knowledge_fact_ids.len(), 1);
    assert_eq!(output.applied.total_applied, 4);
    assert_eq!(output.version_number, 1);
}
