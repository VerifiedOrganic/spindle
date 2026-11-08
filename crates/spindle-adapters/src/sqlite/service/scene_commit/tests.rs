use super::*;

use spindle_core::models::{CharacterEmotionalProfileData, CharacterVoiceProfileData};
use spindle_core::models::{
    CommitSceneCanonicalFactEntry, CommitSceneChapterSummary, CommitSceneCharacterStateEntry,
    CommitSceneCharacterStatePatch, CommitSceneContinuityPackage, CommitSceneInput,
    CommitSceneKnowledgeEntry, CommitSceneQuantityStateEntry, CommitSceneRelationshipEntry,
    CommitSceneStoryPlacement, CommitSceneTimelineEventEntry, ContentRating, CreateBranchInput,
    CreateCharacterInput, CreateLocationInput, CreateProjectInput, CreateRelationshipInput,
    PrepareSceneInput, PrepareSceneIntent, ReaderContract, RegisterCanonicalFactInput,
    SaveSceneDraftInput, SceneAuthorship, SceneBeatAnnotationInput, SwitchBranchInput,
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
    replay_token.expected_scene_version = Some(1);
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
