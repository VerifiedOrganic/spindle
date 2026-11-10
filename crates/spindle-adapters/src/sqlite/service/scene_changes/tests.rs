use super::*;

/// The load-bearing guarantee: a temporal finding NEVER blocks a commit,
/// even under `BlockErrors` with `accept_continuity_risks = false`, and it
/// never leaks into `blocking_continuity_findings`.
#[tokio::test]
async fn commit_scene_changes_temporal_findings_never_block() {
    use spindle_core::models::{
        CommitContinuityGate, CommitSceneChangesInput, ContentRating, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "commit-temporal".into(),
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
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: TELEPORT_PROSE.into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let commit = |gate: CommitContinuityGate| CommitSceneChangesInput {
        project_id: proj.project_id.clone(),
        scene_id: scene.scene_id.clone(),
        character_states: Vec::new(),
        canonical_facts: Vec::new(),
        relationship_updates: Vec::new(),
        accept_world_rule_risks: false,
        accept_continuity_risks: false,
        continuity_gate: Some(gate),
    };

    // BlockErrors + no risk acceptance: a teleport must still NOT block.
    let out = svc
        .commit_scene_changes(commit(CommitContinuityGate::BlockErrors))
        .await
        .expect("a temporal finding must never block a commit");
    assert!(
        out.temporal_findings
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "commit must surface temporal findings: {:?}",
        out.temporal_findings
    );
    assert!(
        !out.blocking_continuity_findings
            .iter()
            .any(|i| i.check_type == "temporal_coherence"),
        "temporal findings must never appear in blocking_continuity_findings: {:?}",
        out.blocking_continuity_findings
    );

    // Gate Off skips the scan entirely.
    let off = svc
        .commit_scene_changes(commit(CommitContinuityGate::Off))
        .await
        .unwrap();
    assert!(
        off.temporal_findings.is_empty(),
        "continuity_gate Off must skip the temporal scan: {:?}",
        off.temporal_findings
    );
}

#[tokio::test]
async fn commit_scene_changes_blocks_contradicting_canonical_fact() {
    use spindle_core::models::{
        CanonicalFactEntry, CanonicalFactScope, CommitSceneChangesInput, ContentRating,
        SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "gate".into(),
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

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Her eyes were blue.".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let blue = CanonicalFactEntry {
        fact_type: None,
        key: None,
        value: None,
        subject_table: Some("character".into()),
        subject_id: Some("character:mara".into()),
        predicate: Some("eye_color".into()),
        value_kind: Some("string".into()),
        value_text: Some("blue".into()),
        value_number: None,
        value_unit: None,
        value_json: None,
        aliases: Vec::new(),
        scope: Some(CanonicalFactScope::Invariant),
        valid_from: None,
        valid_until: None,
        context: None,
        supersedes_fact_id: None,
    };

    // Establish eye_color = blue.
    svc.commit_scene_changes(CommitSceneChangesInput {
        project_id: proj.project_id.clone(),
        scene_id: scene.scene_id.clone(),
        character_states: Vec::new(),
        canonical_facts: vec![blue.clone()],
        relationship_updates: Vec::new(),
        accept_world_rule_risks: false,
        accept_continuity_risks: false,
        continuity_gate: None,
    })
    .await
    .unwrap();

    // An unbounded, non-superseding eye_color = brown contradicts canon and
    // must be blocked by the write-time continuity gate.
    let mut brown = blue.clone();
    brown.value_text = Some("brown".into());
    let blocked = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: proj.project_id.clone(),
            scene_id: scene.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: vec![brown.clone()],
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: None,
        })
        .await;
    assert!(
        blocked.is_err(),
        "contradicting canonical fact should be blocked by the continuity gate"
    );

    // The same commit goes through once the caller accepts the risk.
    let overridden = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: proj.project_id.clone(),
            scene_id: scene.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: vec![brown],
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: true,
            continuity_gate: None,
        })
        .await;
    assert!(
        overridden.is_ok(),
        "accept_continuity_risks should allow the commit: {:?}",
        overridden.err()
    );
}

#[tokio::test]
async fn commit_scene_changes_happy_path_state_and_fact() {
    use spindle_core::models::{
        CanonicalFactEntry, CanonicalFactScope, CharacterEmotionalProfileData, CharacterStatePatch,
        CharacterStatePatchEntry, CharacterVoiceProfileData, CommitSceneChangesInput,
        ContentRating, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "csc".into(),
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

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let tal = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Tal".into(),
            summary: "Scout".into(),
            role: "ally".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let out = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: proj.project_id.clone(),
            scene_id: scene.scene_id.clone(),
            character_states: vec![CharacterStatePatchEntry {
                character_id: mara.character_id.clone(),
                changes: CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: None,
                    status: Some(vec!["bracing for the dark".into()]),
                    notes: None,
                    source_summary: None,
                },
            }],
            canonical_facts: vec![CanonicalFactEntry {
                fact_type: None,
                key: None,
                value: None,
                subject_table: Some("character".into()),
                subject_id: Some(mara.character_id.clone()),
                predicate: Some("oath".into()),
                value_kind: Some("string".into()),
                value_text: Some("ash gate warden".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: Some(CanonicalFactScope::Invariant),
                valid_from: None,
                valid_until: None,
                context: None,
                supersedes_fact_id: None,
            }],
            relationship_updates: vec![spindle_core::models::RelationshipUpdateEntry {
                character_a_id: mara.character_id.clone(),
                character_b_id: tal.character_id.clone(),
                trust_delta: 2,
                tension_delta: 1,
                reason: "Mara accepts Tal's warning at the gate.".into(),
            }],
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(spindle_core::models::CommitContinuityGate::Off),
        })
        .await
        .unwrap();

    assert_eq!(out.scene_id, scene.scene_id);
    assert_eq!(out.character_states.len(), 1, "one character state entry");
    assert!(
        out.character_states[0].state_id.is_some(),
        "character state should commit: {:?}",
        out.character_states[0].error
    );
    assert!(out.character_states[0].error.is_none());
    assert_eq!(out.canonical_facts.len(), 1, "one canonical fact entry");
    assert!(
        out.canonical_facts[0].canonical_fact_id.is_some(),
        "fact should register: {:?}",
        out.canonical_facts[0].error
    );
    assert_eq!(out.canonical_facts[0].fact_type, "oath");
    assert_eq!(out.canonical_facts[0].key, "oath");
    assert_eq!(out.relationship_updates.len(), 1);
    assert!(
        out.relationship_updates[0].relationship_id.is_some(),
        "first-contact relationship update should create a relationship: {:?}",
        out.relationship_updates[0].error
    );
    assert_eq!(out.relationship_updates[0].trust, Some(2));
    assert_eq!(out.relationship_updates[0].tension, Some(1));
    assert!(out.world_rule_hits.is_empty(), "no world rules seeded");
    // Phase-4 fan-out runs but produces nothing: the only fact has
    // scene_id == this scene's id (validators skip same-scene facts)
    // and there are no other characters / world rules / interventions.
    assert_eq!(out.findings_summary.total_count, 0);
    assert_eq!(out.findings_summary.error_count, 0);
}

/// Phase-4 fan-out wiring inside `commit_scene_changes`: with a project
/// where scene 1 has registered the canonical fact `oath -> ash gate
/// warden`, committing changes on scene 2 — whose prose mentions
/// "oath" but not "ash gate warden" — must produce a
/// `canonical_fact_prose_drift` finding in `findings_summary`.
#[tokio::test]
async fn commit_scene_changes_populates_phase_four_findings_summary() {
    use spindle_core::models::{
        CanonicalFactEntry, CanonicalFactScope, CharacterEmotionalProfileData, CharacterStatePatch,
        CharacterStatePatchEntry, CharacterVoiceProfileData, CommitSceneChangesInput,
        ContentRating, CreateBookInput, CreateChapterInput, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ph4-commit".into(),
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

    svc.create_book(CreateBookInput {
        project_id: proj.project_id.clone(),
        title: Some("Book One".into()),
    })
    .await
    .unwrap();
    svc.create_chapter(CreateChapterInput {
        project_id: proj.project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(1),
        title: Some("Chapter One".into()),
    })
    .await
    .unwrap();
    svc.create_chapter(CreateChapterInput {
        project_id: proj.project_id.clone(),
        book_number: Some(1),
        book_id: None,
        chapter_number: Some(2),
        title: Some("Chapter Two".into()),
    })
    .await
    .unwrap();

    let mara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Warden".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    // Scene 1: register the canonical fact (mara.oath = "ash gate warden")
    // by saving the scene then committing the fact against scene 1.
    let scene1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara swore her oath at the Ash Gate.".into(),
            summary: "establishment".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    svc.commit_scene_changes(CommitSceneChangesInput {
        project_id: proj.project_id.clone(),
        scene_id: scene1.scene_id.clone(),
        character_states: Vec::new(),
        canonical_facts: vec![CanonicalFactEntry {
            fact_type: None,
            key: None,
            value: None,
            subject_table: Some("character".into()),
            subject_id: Some(mara.character_id.clone()),
            predicate: Some("eye color".into()),
            value_kind: Some("string".into()),
            value_text: Some("amber".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Invariant),
            valid_from: None,
            valid_until: None,
            context: None,
            supersedes_fact_id: None,
        }],
        relationship_updates: Vec::new(),
        accept_world_rule_risks: false,
        accept_continuity_risks: false,
        continuity_gate: Some(spindle_core::models::CommitContinuityGate::Off),
    })
    .await
    .unwrap();

    // Scene 2: prose gives a conflicting eye colour for the same character.
    // The Phase-4 canonical-fact validator should fire on positive
    // contradiction evidence, not merely on a key mention.
    let scene2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 2,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara's eyes were a flat brown in the lantern light.".into(),
            summary: "drift beat".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let out = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: proj.project_id.clone(),
            scene_id: scene2.scene_id.clone(),
            character_states: vec![CharacterStatePatchEntry {
                character_id: mara.character_id.clone(),
                changes: CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: None,
                    status: Some(vec!["bracing for the dark".into()]),
                    notes: None,
                    source_summary: None,
                },
            }],
            canonical_facts: Vec::new(),
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
            accept_continuity_risks: false,
            continuity_gate: Some(spindle_core::models::CommitContinuityGate::Off),
        })
        .await
        .unwrap();

    // The summary must reflect the Phase-4 drift finding.
    assert!(
        out.findings_summary.total_count >= 1,
        "expected at least one Phase-4 finding, got summary: {:?}",
        out.findings_summary
    );
    assert!(
        out.findings_summary
            .by_check
            .get("canonical_fact_prose_drift")
            .copied()
            .unwrap_or(0)
            >= 1,
        "expected a canonical_fact_prose_drift finding, got summary: {:?}",
        out.findings_summary
    );
}

#[tokio::test]
async fn commit_scene_changes_is_atomic_on_midway_failure() {
    use spindle_core::models::{
        CanonicalFactEntry, CanonicalFactScope, CharacterEmotionalProfileData, CharacterStatePatch,
        CharacterStatePatchEntry, CharacterVoiceProfileData, CommitContinuityGate,
        CommitSceneChangesInput, ContentRating, CreateCharacterInput, CreateProjectInput,
        RelationshipUpdateEntry, SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "atomic-commit".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "all or nothing".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let char_a = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Aria".to_string(),
            summary: "Protagonist.".to_string(),
            role: "protagonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Aria stood in the courtyard and learned the truth.".to_string(),
            summary: "Aria learns a truth.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Attempt a commit_scene_changes where the first character_state and canonical_fact are valid,
    // but a relationship_update references a nonexistent character, causing a mid-way failure.
    let res = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene.scene_id.clone(),
            character_states: vec![CharacterStatePatchEntry {
                character_id: char_a.character_id.clone(),
                changes: CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: Some(vec!["find the truth".to_string()]),
                    status: Some(vec!["unharmed".to_string()]),
                    notes: None,
                    source_summary: Some("courtyard scene".to_string()),
                },
            }],
            canonical_facts: vec![CanonicalFactEntry {
                fact_type: None,
                key: None,
                value: None,
                subject_table: Some("project".to_string()),
                subject_id: Some(project.project_id.clone()),
                predicate: Some("courtyard_fountain".to_string()),
                value_kind: Some("string".to_string()),
                value_text: Some("dry".to_string()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: Some(CanonicalFactScope::Invariant),
                valid_from: None,
                valid_until: None,
                context: None,
                supersedes_fact_id: None,
            }],
            relationship_updates: vec![RelationshipUpdateEntry {
                character_a_id: char_a.character_id.clone(),
                character_b_id: "nonexistent-character-id".to_string(),
                trust_delta: 10,
                tension_delta: -5,
                reason: "broken ref".to_string(),
            }],
            accept_world_rule_risks: true,
            accept_continuity_risks: true,
            continuity_gate: Some(CommitContinuityGate::WarnOnly),
        })
        .await;

    assert!(
        res.is_err(),
        "commit_scene_changes should fail on invalid character_b_id"
    );

    // Verify that the earlier scene character_state, progression_event, and canonical_fact were rolled back.
    let states = svc
        .repository
        .list_character_states_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert!(
        states.iter().all(|s| s.scene_id.is_none()),
        "scene character_state must be rolled back (only initial create_character state may exist)"
    );

    let events = svc
        .repository
        .list_all_progression_events(&project.project_id)
        .await
        .unwrap();
    assert!(events.is_empty(), "progression_event must be rolled back");

    let facts = svc
        .repository
        .list_canonical_facts_by_project_and_branch(&project.project_id, &project.branch_id)
        .await
        .unwrap();
    assert!(facts.is_empty(), "canonical_fact must be rolled back");
}

#[tokio::test]
async fn relationship_updates_are_idempotent_per_scene_and_replace_across_recommit_and_mining() {
    use crate::sqlite::repository::StageCanonDeltaParams;
    use spindle_core::models::{
        CanonDeltaDecisionInput, CharacterEmotionalProfileData, CharacterVoiceProfileData,
        CommitContinuityGate, CommitSceneChangesInput, ContentRating, CreateCharacterInput,
        CreateProjectInput, CreateRelationshipInput, DecideCanonDeltasInput,
        RelationshipUpdateEntry, SaveSceneDraftInput, UpdateRelationshipInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "rel-idempotency".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "no double counting".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let char_a = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Rowan".to_string(),
            summary: "Knight.".to_string(),
            role: "protagonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let char_b = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Sylvi".to_string(),
            summary: "Scout.".to_string(),
            role: "deuteragonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    svc.create_relationship(CreateRelationshipInput {
        character_a_id: char_a.character_id.clone(),
        character_b_id: char_b.character_id.clone(),
        relationship_type: "companions".to_string(),
        initial_trust: 50,
        initial_tension: 30,
        dynamics: vec!["wary allies".to_string()],
    })
    .await
    .unwrap();

    let scene1 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Rowan handed Sylvi a ration pack by the fire.".to_string(),
            summary: "Rowan and Sylvi share camp.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // First commit of scene 1 applies +10 trust, -5 tension (50 -> 60, 30 -> 25).
    let out1 = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene1.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: vec![RelationshipUpdateEntry {
                character_a_id: char_a.character_id.clone(),
                character_b_id: char_b.character_id.clone(),
                trust_delta: 10,
                tension_delta: -5,
                reason: "shared rations".to_string(),
            }],
            accept_world_rule_risks: true,
            accept_continuity_risks: true,
            continuity_gate: Some(CommitContinuityGate::WarnOnly),
        })
        .await
        .unwrap();
    assert_eq!(out1.relationship_updates[0].trust, Some(60));
    assert_eq!(out1.relationship_updates[0].tension, Some(25));

    // A save point must preserve the per-scene ledger alongside totals.
    let saved = svc
        .create_save_point(spindle_core::models::CreateSavePointInput {
            project_id: project.project_id.clone(),
            name: "relationship-ledger".into(),
            description: None,
        })
        .await
        .unwrap();
    svc.restore_save_point(spindle_core::models::RestoreSavePointInput {
        project_id: project.project_id.clone(),
        save_point_id: saved.save_point_id.clone(),
    })
    .await
    .unwrap();

    // Recommitting the same scene with the same delta must be idempotent (60, not 70).
    let out2 = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene1.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: vec![RelationshipUpdateEntry {
                character_a_id: char_a.character_id.clone(),
                character_b_id: char_b.character_id.clone(),
                trust_delta: 10,
                tension_delta: -5,
                reason: "shared rations".to_string(),
            }],
            accept_world_rule_risks: true,
            accept_continuity_risks: true,
            continuity_gate: Some(CommitContinuityGate::WarnOnly),
        })
        .await
        .unwrap();
    assert_eq!(out2.relationship_updates[0].trust, Some(60));
    assert_eq!(out2.relationship_updates[0].tension, Some(25));

    // Omitting the restored contribution must reverse it back to baseline.
    svc.commit_scene_changes(CommitSceneChangesInput {
        project_id: project.project_id.clone(),
        scene_id: scene1.scene_id.clone(),
        character_states: Vec::new(),
        canonical_facts: Vec::new(),
        relationship_updates: Vec::new(),
        accept_world_rule_risks: true,
        accept_continuity_risks: true,
        continuity_gate: Some(CommitContinuityGate::WarnOnly),
    })
    .await
    .unwrap();
    let removed = svc
        .repository
        .list_relationships_by_branch(&project.branch_id)
        .await
        .unwrap();
    assert_eq!((removed[0].trust, removed[0].tension), (50, 30));
    svc.restore_save_point(spindle_core::models::RestoreSavePointInput {
        project_id: project.project_id.clone(),
        save_point_id: saved.save_point_id,
    })
    .await
    .unwrap();

    // Recommitting the revised scene with an updated delta (+15 trust, -10 tension)
    // replaces the scene's prior (+10, -5) delta -> (65, 20).
    let out3 = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene1.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: vec![RelationshipUpdateEntry {
                character_a_id: char_a.character_id.clone(),
                character_b_id: char_b.character_id.clone(),
                trust_delta: 15,
                tension_delta: -10,
                reason: "saved from ambush".to_string(),
            }],
            accept_world_rule_risks: true,
            accept_continuity_risks: true,
            continuity_gate: Some(CommitContinuityGate::WarnOnly),
        })
        .await
        .unwrap();
    assert_eq!(out3.relationship_updates[0].trust, Some(65));
    assert_eq!(out3.relationship_updates[0].tension, Some(20));

    // Accepting a mined `relationship_shift` canon delta for the same scene replaces the scene's
    // effective delta (+20 trust, -8 tension -> 70, 22) rather than double-counting on top of the
    // continuity package.
    let staged_delta = svc
        .repository
        .stage_canon_delta(StageCanonDeltaParams {
            project_id: project.project_id.clone(),
            branch_id: project.branch_id.clone(),
            scene_id: scene1.scene_id.clone(),
            authoring_run_id: None,
            delta_class: "relationship_shift".to_string(),
            target_id: None,
            payload: serde_json::json!({
                "character_a_id": char_a.character_id,
                "character_b_id": char_b.character_id,
                "trust_delta": 20,
                "tension_delta": -8,
                "reason": "swore an oath",
                "scene_id": scene1.scene_id
            }),
            evidence: "Rowan handed Sylvi a ration pack by the fire.".to_string(),
            confidence: "high".to_string(),
        })
        .await
        .unwrap();

    let decide_out = svc
        .decide_canon_deltas(DecideCanonDeltasInput {
            project_id: project.project_id.clone(),
            decided_by: None,
            decisions: vec![CanonDeltaDecisionInput {
                delta_id: staged_delta.id,
                action: "apply".to_string(),
                edit: None,
                note: None,
            }],
        })
        .await
        .unwrap();
    assert_eq!(decide_out.applied_count, 1);

    let rels_after_mining = svc
        .repository
        .list_relationships_by_branch(&project.branch_id)
        .await
        .unwrap();
    assert_eq!(rels_after_mining.len(), 1);
    assert_eq!(rels_after_mining[0].trust, 70);
    assert_eq!(rels_after_mining[0].tension, 22);

    // Older snapshots have no ledger table. Restore their totals without
    // retaining ledger entries from the live branch's later state.
    let mut legacy = svc
        .build_project_export_payload(
            &project.project_id,
            "spindle-save-point-v1",
            std::collections::BTreeMap::new(),
        )
        .await
        .unwrap()
        .payload;
    legacy["tables"]
        .as_object_mut()
        .unwrap()
        .remove("relationship_scene_delta");
    let legacy_snapshot = build_branch_restore_snapshot(
        &svc.repository,
        &project.project_id,
        &project.branch_id,
        &legacy,
    )
    .await
    .unwrap();
    svc.repository
        .restore_branch_snapshot(&project.project_id, &project.branch_id, &legacy_snapshot)
        .await
        .unwrap();
    let legacy_rels = svc
        .repository
        .list_relationships_by_branch(&project.branch_id)
        .await
        .unwrap();
    assert_eq!((legacy_rels[0].trust, legacy_rels[0].tension), (70, 22));
    let legacy_ledger = svc
        .repository
        .dump_table_by_column("relationship_scene_delta", "branch_id", &project.branch_id)
        .await
        .unwrap();
    assert!(legacy_ledger.is_empty());

    // A second scene accumulates its delta on top of scene 1 (70 + 5 = 75, 22 - 2 = 20).
    let scene2 = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Rowan and Sylvi crossed the bridge together.".to_string(),
            summary: "Rowan and Sylvi cross the bridge.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let out_scene2 = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: char_a.character_id.clone(),
            character_b_id: char_b.character_id.clone(),
            trust_delta: 5,
            tension_delta: -2,
            reason: "crossed bridge".to_string(),
            scene_id: scene2.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(out_scene2.trust, 75);
    assert_eq!(out_scene2.tension, 20);
}

#[tokio::test]
async fn relationship_child_materialization_isolates_later_parent_updates() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating, CreateBranchInput,
        CreateCharacterInput, CreateProjectInput, CreateRelationshipInput, SaveSceneDraftInput,
        SwitchBranchInput, UpdateRelationshipInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let project = svc
        .create_project(CreateProjectInput {
            name: "rel-branch-isolation".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "branch-isolated relationship deltas".to_string(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let char_a = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Alden".to_string(),
            summary: "Knight.".to_string(),
            role: "protagonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    let char_b = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Bryn".to_string(),
            summary: "Mage.".to_string(),
            role: "deuteragonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: Vec::new(),
                example_lines: Vec::new(),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: Vec::new(),
                triggers: Vec::new(),
                defense_mechanisms: Vec::new(),
                flex_range: None,
            },
            initial_state: None,
        })
        .await
        .unwrap();

    // Parent (main) starts with trust 50, tension 20.
    svc.create_relationship(CreateRelationshipInput {
        character_a_id: char_a.character_id.clone(),
        character_b_id: char_b.character_id.clone(),
        relationship_type: "allies".to_string(),
        initial_trust: 50,
        initial_tension: 20,
        dynamics: Vec::new(),
    })
    .await
    .unwrap();

    let main_scene_a = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Alden and Bryn met at the camp.".to_string(),
            summary: "Scene A on main.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let main_scene_c = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 3,
            full_text: "Alden and Bryn shared watch.".to_string(),
            summary: "Scene C on main.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Parent applies +10 at scene C before child branches (parent trust = 60).
    let main_c_initial = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: char_a.character_id.clone(),
            character_b_id: char_b.character_id.clone(),
            trust_delta: 10,
            tension_delta: -2,
            reason: "parent scene C initial".to_string(),
            scene_id: main_scene_c.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(main_c_initial.trust, 60);
    assert_eq!(main_c_initial.tension, 18);

    // Create and switch to child branch `alt-rel`.
    let child_branch = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "alt-rel".to_string(),
            description: Some("child branch relationship test".to_string()),
            parent_branch_id: None,
            branch_type: "what_if".to_string(),
        })
        .await
        .unwrap();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child_branch.branch_id.clone(),
    })
    .await
    .unwrap();

    // Child applies +5 in scene B (materializing child relationship at 60 + 5 = 65).
    let child_scene_b = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Alden and Bryn crossed the river on child branch.".to_string(),
            summary: "Scene B on child.".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let child_after_b = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: char_a.character_id.clone(),
            character_b_id: char_b.character_id.clone(),
            trust_delta: 5,
            tension_delta: -1,
            reason: "child scene B".to_string(),
            scene_id: child_scene_b.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(child_after_b.trust, 65);
    assert_eq!(child_after_b.tension, 17);

    // Switch back to main (parent) and apply:
    // 1. +10 in scene A (which had no delta when child materialized)
    // 2. +25 in scene C (replacing parent's earlier +10 after child already materialized)
    let main_branch_id = child_branch.parent_branch_id.clone();
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: main_branch_id,
    })
    .await
    .unwrap();

    let main_after_a = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: char_a.character_id.clone(),
            character_b_id: char_b.character_id.clone(),
            trust_delta: 10,
            tension_delta: -5,
            reason: "parent scene A after child materialized".to_string(),
            scene_id: main_scene_a.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(main_after_a.trust, 70);

    let main_after_c_update = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: char_a.character_id.clone(),
            character_b_id: char_b.character_id.clone(),
            trust_delta: 25,
            tension_delta: -6,
            reason: "parent scene C updated after child materialized".to_string(),
            scene_id: main_scene_c.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(main_after_c_update.trust, 85);

    // Switch back to child branch.
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child_branch.branch_id.clone(),
    })
    .await
    .unwrap();

    // Child applies +3 at scene A: must NOT subtract parent's post-materialization +10.
    // Expected trust: 65 + 3 = 68 (not 58).
    let child_after_a = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: char_a.character_id.clone(),
            character_b_id: char_b.character_id.clone(),
            trust_delta: 3,
            tension_delta: 1,
            reason: "child scene A".to_string(),
            scene_id: main_scene_a.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        child_after_a.trust, 68,
        "child must not subtract parent's post-materialization scene A delta"
    );
    assert_eq!(child_after_a.tension, 18);

    // Child overrides scene C with +4: must subtract the +10 that WAS included in
    // child's materialized baseline, NOT parent's later +25.
    // Expected trust: 68 - 10 + 4 = 62.
    let child_after_c = svc
        .update_relationship(UpdateRelationshipInput {
            character_a_id: char_a.character_id.clone(),
            character_b_id: char_b.character_id.clone(),
            trust_delta: 4,
            tension_delta: -3,
            reason: "child scene C override".to_string(),
            scene_id: main_scene_c.scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        child_after_c.trust, 62,
        "child must subtract only the inherited +10 delta included at materialization"
    );
    assert_eq!(child_after_c.tension, 17);
}
