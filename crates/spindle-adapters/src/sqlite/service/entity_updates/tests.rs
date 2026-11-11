use super::*;

#[tokio::test]
async fn update_entity_world_rule_resolves_world_rule_semantic_drift_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) =
        project_with_scene(&svc, "generic-world-update-cache").await;
    let rule = svc
        .create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "Moon glass cannot be forged twice".to_string(),
            rule_type: "magic".to_string(),
            description: "Moon glass cracks when reforged.".to_string(),
            scan_pattern: Some("moon glass".to_string()),
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::WorldRuleSemanticDrift,
    )
    .await;

    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "world_rule".to_string(),
        entity_id: rule.world_rule_id,
        changes: serde_json::json!({
            "description": "Moon glass cracks when reforged or reheated."
        }),
    })
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::WorldRuleSemanticDrift,
        0,
    )
    .await;
}

#[tokio::test]
async fn update_entity_project_style_fields_resolve_style_compliance_cache() {
    let (_tmp, svc) = fresh_service().await;
    let (project, scene_id, full_text) = project_with_scene(&svc, "project-style-cache").await;
    let scene_text_hash = seed_phase_four_cache(
        &svc,
        &project.project_id,
        &project.branch_id,
        &scene_id,
        &full_text,
        PhaseFourCacheId::StyleCompliance,
    )
    .await;

    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "project".to_string(),
        entity_id: project.project_id.clone(),
        changes: serde_json::json!({
            "genre": "comedy fantasy"
        }),
    })
    .await
    .unwrap();

    assert_active_phase_four_cache(
        &svc,
        &project.branch_id,
        &scene_id,
        &scene_text_hash,
        PhaseFourCacheId::StyleCompliance,
        0,
    )
    .await;
}

/// Regression for the SQL crash where update_entity on a book row failed
/// with `no such column: updated_at`. The book table in V0001 was missing
/// the `updated_at` column that the generic update path sets on every
/// UPDATE. V0004 backfills the column; this test would have failed before
/// that migration and the matching repository code changes landed.
#[tokio::test]
async fn update_entity_on_book_title_succeeds_after_updated_at_backfill() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Book Rename".into(),
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

    // create_project auto-mints book #1; grab its id straight from the repo.
    let books = svc
        .repository()
        .list_books_by_project(&proj.project_id)
        .await
        .unwrap();
    let book_id = books
        .first()
        .map(|b| b.id.clone())
        .expect("bootstrap book exists");

    // Drive the bug repro: update entity_type="book", field="title".
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "book".into(),
        entity_id: book_id.clone(),
        changes: serde_json::json!({ "title": "Renamed Book" }),
    })
    .await
    .expect("update_entity on book.title must not crash");

    // Confirm the rename persisted and updated_at is now populated.
    let books = svc
        .repository()
        .list_books_by_project(&proj.project_id)
        .await
        .unwrap();
    let renamed = books.first().expect("book row");
    assert_eq!(renamed.title.as_deref(), Some("Renamed Book"));
    assert!(
        renamed.updated_at.is_some(),
        "updated_at must be populated after update_entity"
    );
}

/// Regression for the asymmetric allowlist where create_conflict accepted
/// `try_fail_cycles` but update_entity rejected it. The continuity-editor
/// path explicitly tells callers to add more cycles after creation, so
/// this update must succeed.
#[tokio::test]
async fn update_entity_on_conflict_try_fail_cycles_now_allowlisted() {
    use spindle_core::models::{CreateConflictInput, TryFailCycleStep};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Conflict Update".into(),
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

    let conflict = svc
        .create_conflict(CreateConflictInput {
            project_id: proj.project_id.clone(),
            name: "Mara vs Wraith".into(),
            conflict_type: "person-vs-supernatural".into(),
            stakes: "Life of the village.".into(),
            escalation_stages: Vec::new(),
            expected_total_cycles: None,
            try_fail_cycles: Vec::new(),
            stated_consequences: Vec::new(),
        })
        .await
        .unwrap();

    // Drive the bug repro: retroactively populate try_fail_cycles via
    // update_entity. Pre-fix this errored with "column 'try_fail_cycles'
    // on 'conflict' is not in the update allowlist".
    let cycles = vec![
        TryFailCycleStep {
            attempt_order: 1,
            label: "Salt ward at the gate".into(),
            outcome: "Wraith slips through".into(),
            cost: Some("Mara loses her warding charm".into()),
            revelation: None,
        },
        TryFailCycleStep {
            attempt_order: 2,
            label: "Ambush in the alley".into(),
            outcome: "Wraith reveals it can phase".into(),
            cost: Some("Mara's shoulder torn".into()),
            revelation: Some("Wraith is incorporeal at night".into()),
        },
    ];

    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "conflict".into(),
        entity_id: conflict.conflict_id.clone(),
        changes: serde_json::json!({
            "try_fail_cycles": cycles,
            "expected_total_cycles": 3,
        }),
    })
    .await
    .expect("update_entity on conflict array fields must succeed");

    let updated = svc
        .repository()
        .get_conflict(&conflict.conflict_id)
        .await
        .unwrap();
    assert_eq!(updated.try_fail_cycles.len(), 2);
    assert_eq!(updated.try_fail_cycles[0].attempt_order, 1);
    assert_eq!(updated.try_fail_cycles[1].attempt_order, 2);
    assert_eq!(updated.expected_total_cycles, Some(3));
}

/// Regression for the allowlist gap that made planned_payoff
/// unactionable: after a chapter renumber every promise still pointed at
/// its old payoff chapter, and narrative_promise_tracking emitted
/// permanent "past planned payoff" warnings with no API path to retarget
/// the promise (update_promise_status takes only status/note).
#[tokio::test]
async fn update_entity_on_narrative_promise_planned_payoff_now_allowlisted() {
    use spindle_core::models::{CreateNarrativePromiseInput, StoryPlacement};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Promise Retarget".into(),
            project_type: "novel".into(),
            genre: "litRPG".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let promise = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: proj.project_id.clone(),
            promise_type: "foreshadowing".into(),
            description: "The coerced yes will detonate.".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 4,
                scene_order: None,
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 30,
                scene_order: None,
                note: None,
            }),
            notes: Vec::new(),
        })
        .await
        .unwrap();

    // Drive the bug repro: retarget the payoff after a restructure.
    // Pre-fix this errored with "column 'planned_payoff' on
    // 'narrative_promise' is not in the update allowlist".
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "narrative_promise".into(),
        entity_id: promise.narrative_promise_id.clone(),
        changes: serde_json::json!({
            "planned_payoff": { "book_number": 1, "chapter_number": 65 }
        }),
    })
    .await
    .expect("update_entity on narrative_promise.planned_payoff must succeed");

    let updated = svc
        .repository()
        .get_narrative_promise(&promise.narrative_promise_id)
        .await
        .unwrap();
    let payoff = updated
        .planned_payoff
        .expect("planned_payoff persists")
        .into_core();
    assert_eq!(payoff.book_number, 1);
    assert_eq!(payoff.chapter_number, 65);
}

/// Regression for the allowlist gap on scene.summary: scene summaries
/// were fixed at save_scene_draft time, so a renumber left stale
/// cross-references that get_chapter_briefing keeps feeding into future
/// drafting context.
#[tokio::test]
async fn update_entity_on_scene_summary_now_allowlisted() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Scene Summary".into(),
            project_type: "novel".into(),
            genre: "litRPG".into(),
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
            full_text: "The knife changed hands under the table.".into(),
            summary: "Nate pockets the combat knife from Chapter 3.".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // Drive the bug repro: correct the stale cross-reference after the
    // renumber moved the knife scene to Chapter 8. Pre-fix this errored
    // with "column 'summary' on 'scene' is not in the update allowlist".
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "scene".into(),
        entity_id: scene.scene_id.clone(),
        changes: serde_json::json!({
            "summary": "Nate pockets the combat knife from Chapter 8."
        }),
    })
    .await
    .expect("update_entity on scene.summary must succeed");

    let updated = svc.repository().get_scene(&scene.scene_id).await.unwrap();
    assert_eq!(
        updated.summary,
        "Nate pockets the combat knife from Chapter 8."
    );
}

/// Regression: renaming a character through update_entity used to be
/// impossible — `name` is not in the column allowlist — leaving authors
/// who created a character before its in-world naming scene with no way
/// to apply the name. The rename is now a controlled operation gated by
/// `allow_rename`; without the flag the call must fail BEFORE writing.
#[tokio::test]
async fn update_entity_character_rename_requires_allow_rename_flag() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Rename Gate".into(),
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
    let character_id = create_character_named(&svc, &proj.project_id, "Placeholder Co-Lead").await;

    // No flag: rejected, with guidance, and nothing changed.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character".into(),
            entity_id: character_id.clone(),
            changes: serde_json::json!({ "name": "Kaelen Voss" }),
        })
        .await
        .expect_err("rename without allow_rename must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("allow_rename"),
        "error must name the gate, got: {msg}"
    );
    let unchanged = svc.repository().get_character(&character_id).await.unwrap();
    assert_eq!(unchanged.name, "Placeholder Co-Lead");
    assert!(unchanged.aliases.is_empty());

    // Flag set: the rename lands.
    let out = svc
        .update_entity(UpdateEntityInput {
            allow_rename: Some(true),
            entity_type: "character".into(),
            entity_id: character_id.clone(),
            changes: serde_json::json!({ "name": "Kaelen Voss" }),
        })
        .await
        .expect("rename with allow_rename must succeed");
    let report = out
        .rename_report
        .expect("character rename must return a rename_report");
    assert_eq!(report.old_name, "Placeholder Co-Lead");
    assert_eq!(report.new_name, "Kaelen Voss");
    assert!(report.old_name_kept_as_alias);

    let renamed = svc.repository().get_character(&character_id).await.unwrap();
    assert_eq!(renamed.name, "Kaelen Voss");
    assert_eq!(renamed.normalized_name, "kaelen voss");
    assert_eq!(renamed.aliases, vec!["Placeholder Co-Lead".to_string()]);
}

/// The rename report must surface every record still referencing the old
/// name — scenes (prose), canonical facts, knowledge, arcs — so the
/// author can fix them. A silent rename leaving stale references is worse
/// than the old hard error.
#[tokio::test]
async fn update_entity_character_rename_reports_stale_references() {
    use spindle_core::models::{
        ContentRating, CreateCharacterArcInput, RecordKnowledgeInput, RegisterCanonicalFactInput,
        SaveSceneDraftInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Rename Report".into(),
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
    let character_id = create_character_named(&svc, &proj.project_id, "Mara").await;

    // Scene prose mentioning the old name.
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara drew the salt blade and waited.".into(),
            summary: "Mara stands watch.".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    // A canonical fact whose value text carries the old name.
    let fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(scene.scene_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".to_string()),
            subject_id: Some(character_id.clone()),
            predicate: Some("true_name".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("Mara of the Salt Flats".to_string()),
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

    // Knowledge prose mentioning the old name.
    let other_holder = create_character_named(&svc, &proj.project_id, "Bran").await;
    svc.record_knowledge(RecordKnowledgeInput {
        project_id: proj.project_id.clone(),
        branch_id: None,
        character_id: other_holder.clone(),
        fact: "Bran knows Mara smuggles salt.".into(),
        source_summary: "declared".into(),
        learned_at: None,
        confidence: None,
        tags: Vec::new(),
        reader_visible: true,
        secret_of_fact_id: None,
    })
    .await
    .unwrap();

    // An arc whose notes mention the old name.
    let arc = svc
        .create_character_arc(CreateCharacterArcInput {
            project_id: proj.project_id.clone(),
            character_id: character_id.clone(),
            arc_type: "positive".into(),
            starting_state: "Guarded smuggler.".into(),
            ending_state: "Open leader.".into(),
            milestones: Vec::new(),
            thematic_purpose: "Trust.".into(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "character_arc".into(),
        entity_id: arc.character_arc_id.clone(),
        changes: serde_json::json!({ "notes": "Mara's trust arc." }),
    })
    .await
    .unwrap();

    // The rename itself.
    let out = svc
        .update_entity(UpdateEntityInput {
            allow_rename: Some(true),
            entity_type: "character".into(),
            entity_id: character_id.clone(),
            changes: serde_json::json!({ "name": "Marah" }),
        })
        .await
        .expect("rename must succeed");
    let report = out.rename_report.expect("rename_report present");

    assert!(
        report.scenes_referencing_old_name.contains(&scene.scene_id),
        "scene prose mention must be reported: {:?}",
        report.scenes_referencing_old_name
    );
    assert!(
        report
            .canonical_facts_referencing_old_name
            .contains(&fact.canonical_fact_id),
        "canonical fact value text must be reported: {:?}",
        report.canonical_facts_referencing_old_name
    );
    assert!(
        !report.knowledge_facts_referencing_old_name.is_empty(),
        "knowledge mention must be reported"
    );
    assert!(
        report
            .arcs_referencing_old_name
            .contains(&arc.character_arc_id),
        "arc notes mention must be reported: {:?}",
        report.arcs_referencing_old_name
    );

    // Resolution keeps working under BOTH names: the new one via the FTS
    // name column, the old one via the preserved alias (V0037 fts index).
    let by_new = svc
        .find_entity(spindle_core::models::FindEntityInput {
            project_id: proj.project_id.clone(),
            query: "Marah".into(),
            table: Some(spindle_core::subject::SubjectTable::Character),
            branch_id: None,
            limit: Some(5),
        })
        .await
        .unwrap();
    assert!(
        by_new.matches.iter().any(|m| m.entity_id == character_id),
        "find_entity must resolve the NEW name"
    );
    let by_old = svc
        .find_entity(spindle_core::models::FindEntityInput {
            project_id: proj.project_id.clone(),
            query: "Mara".into(),
            table: Some(spindle_core::subject::SubjectTable::Character),
            branch_id: None,
            limit: Some(5),
        })
        .await
        .unwrap();
    assert!(
        by_old.matches.iter().any(|m| m.entity_id == character_id),
        "find_entity must still resolve the OLD name via aliases"
    );
}

/// Rename collisions are checked project-wide (the unique index is on
/// project_id + normalized_name, not branch-scoped).
#[tokio::test]
async fn update_entity_character_rename_rejects_name_collision() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Rename Collision".into(),
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
    let first = create_character_named(&svc, &proj.project_id, "Kael").await;
    let second = create_character_named(&svc, &proj.project_id, "Wren").await;

    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: Some(true),
            entity_type: "character".into(),
            entity_id: second.clone(),
            changes: serde_json::json!({ "name": "KAEL" }),
        })
        .await
        .expect_err("rename onto an existing name must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("already exists"),
        "error must name the collision, got: {msg}"
    );
    assert!(msg.contains(&first), "error must name the holder id: {msg}");
    let unchanged = svc.repository().get_character(&second).await.unwrap();
    assert_eq!(unchanged.name, "Wren");
}

/// Aliases on characters (V0037): settable at create time and amendable
/// through the update_entity allowlist. This is the rename-free path for
/// the "named after the record exists" case.
#[tokio::test]
async fn character_aliases_create_and_update_round_trip() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Aliases".into(),
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

    let created = svc
        .create_character(CreateCharacterInput {
            aliases: vec!["The Salt Blade".to_string()],
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Smuggler of the flats.".into(),
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
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: None,
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

    let character = svc
        .repository()
        .get_character(&created.character_id)
        .await
        .unwrap();
    assert_eq!(character.aliases, vec!["The Salt Blade".to_string()]);

    // Amend via the allowlisted column; the primary name stays put.
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "character".into(),
        entity_id: created.character_id.clone(),
        changes: serde_json::json!({ "aliases": ["The Salt Blade", "Gull"] }),
    })
    .await
    .expect("character.aliases must be allowlisted");
    let updated = svc
        .repository()
        .get_character(&created.character_id)
        .await
        .unwrap();
    assert_eq!(
        updated.aliases,
        vec!["The Salt Blade".to_string(), "Gull".to_string()]
    );
    assert_eq!(updated.name, "Mara");

    // And the alias resolves through find_entity's FTS path.
    let hits = svc
        .find_entity(spindle_core::models::FindEntityInput {
            project_id: proj.project_id.clone(),
            query: "Gull".into(),
            table: Some(spindle_core::subject::SubjectTable::Character),
            branch_id: None,
            limit: Some(5),
        })
        .await
        .unwrap();
    assert!(
        hits.matches
            .iter()
            .any(|m| m.entity_id == created.character_id),
        "find_entity must resolve characters by alias"
    );
}

#[tokio::test]
async fn character_aliases_reject_non_string_values() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
        CreateCharacterInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Aliases Shape".into(),
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
    let created = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "Smuggler.".into(),
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
            initial_state: Some(CharacterStatePatch {
                emotional_state: std::collections::BTreeMap::new(),
                goals: None,
                status: None,
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character".into(),
            entity_id: created.character_id.clone(),
            changes: serde_json::json!({ "aliases": [1, 2] }),
        })
        .await
        .expect_err("non-string aliases must not persist");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("array of strings"),
        "error must name the required shape, got: {msg}"
    );
    let unchanged = svc
        .repository()
        .get_character(&created.character_id)
        .await
        .unwrap();
    assert!(unchanged.aliases.is_empty());
}

/// Bug 1: an arc's destination is among the likeliest fields to change
/// during planning — recording it early so it can be revised is the
/// reason to write an arc down before drafting. Pre-fix, every planning
/// field except milestones was frozen at create time and a live record
/// kept reading "OPEN, deliberately so" after the decision was made.
#[tokio::test]
async fn update_entity_character_arc_planning_fields_now_updatable() {
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterArcInput,
        CreateCharacterInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Arc Planning".into(),
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
    let character = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
            summary: "s".into(),
            role: "r".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData::default(),
            emotional_profile: CharacterEmotionalProfileData::default(),
            initial_state: None,
        })
        .await
        .unwrap();
    let arc = svc
        .create_character_arc(CreateCharacterArcInput {
            project_id: proj.project_id.clone(),
            character_id: character.character_id.clone(),
            arc_type: "flat".into(),
            starting_state: "Guarded.".into(),
            ending_state: "OPEN, deliberately so".into(),
            milestones: Vec::new(),
            thematic_purpose: "Placeholder until the theme firms up.".into(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();

    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "character_arc".into(),
        entity_id: arc.character_arc_id.clone(),
        changes: serde_json::json!({
            "arc_type": "positive",
            "starting_state": "Guarded smuggler.",
            "ending_state": "Open leader who trusts the crew.",
            "thematic_purpose": "Trust is chosen, not earned.",
            "connected_theme_ids": ["theme:one"],
            "status": "active"
        }),
    })
    .await
    .expect("arc planning fields must be updatable");

    let updated = svc
        .repository()
        .get_character_arc(&arc.character_arc_id)
        .await
        .unwrap();
    assert_eq!(updated.arc_type, "positive");
    assert_eq!(updated.starting_state, "Guarded smuggler.");
    assert_eq!(updated.ending_state, "Open leader who trusts the crew.");
    assert_eq!(updated.thematic_purpose, "Trust is chosen, not earned.");
    assert_eq!(updated.connected_theme_ids, vec!["theme:one".to_string()]);
    assert_eq!(updated.status, "active");

    // The boundary: computed/identity fields stay locked.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character_arc".into(),
            entity_id: arc.character_arc_id.clone(),
            changes: serde_json::json!({ "progress": 0.5 }),
        })
        .await
        .expect_err("progress is computed and must stay locked");
    assert!(format!("{err:#}").contains("not in the update allowlist"));
}

/// Bug 2: emotional profiles were write-once at create_character, so any
/// character scaffolded before their psychology was worked out (most
/// supporting cast) was permanently stuck with an empty profile.
#[tokio::test]
async fn update_entity_emotional_profile_columns_now_updatable() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Emotional Profiles".into(),
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
    let character_id = create_character_named(&svc, &proj.project_id, "Mara").await;
    let profile_id = svc
        .repository()
        .get_character_emotional_profile(&character_id)
        .await
        .unwrap()
        .id;

    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "character_emotional_profile".into(),
        entity_id: profile_id.clone(),
        changes: serde_json::json!({
            "base_emotions": { "grief": 0.7, "defiance": 0.5 },
            "suppressed": ["anger"],
            "triggers": ["betrayal", "salt"],
            "defense_mechanisms": ["deflection"],
            "flex_range": { "low": "guarded", "high": "volatile" }
        }),
    })
    .await
    .expect("emotional profile columns must be updatable");

    let updated = svc
        .repository()
        .get_character_emotional_profile_by_id(&profile_id)
        .await
        .unwrap();
    assert_eq!(
        updated.base_emotions.get("grief"),
        Some(&serde_json::json!(0.7))
    );
    assert_eq!(updated.suppressed, vec!["anger".to_string()]);
    assert_eq!(
        updated.triggers,
        vec!["betrayal".to_string(), "salt".to_string()]
    );
    assert_eq!(updated.defense_mechanisms, vec!["deflection".to_string()]);
    let flex = updated.flex_range.expect("flex_range set");
    assert_eq!(flex.low.as_deref(), Some("guarded"));
    assert_eq!(flex.high.as_deref(), Some("volatile"));

    // flex_range can be cleared back to NULL.
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "character_emotional_profile".into(),
        entity_id: profile_id.clone(),
        changes: serde_json::json!({ "flex_range": null }),
    })
    .await
    .expect("flex_range must accept null");
    let cleared = svc
        .repository()
        .get_character_emotional_profile_by_id(&profile_id)
        .await
        .unwrap();
    assert!(cleared.flex_range.is_none());
}

/// The emotional-profile columns only carry json_valid CHECKs, so the
/// service validates shapes BEFORE writing: a type-wrong value must be
/// rejected and leave the stored profile untouched (a poisoned row would
/// break every later read of the character's briefing context).
#[tokio::test]
async fn update_entity_emotional_profile_rejects_wrong_shapes_before_writing() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Profile Shapes".into(),
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
    let character_id = create_character_named(&svc, &proj.project_id, "Mara").await;
    let profile_id = svc
        .repository()
        .get_character_emotional_profile(&character_id)
        .await
        .unwrap()
        .id;

    // suppressed must be an array of strings, not numbers.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character_emotional_profile".into(),
            entity_id: profile_id.clone(),
            changes: serde_json::json!({ "suppressed": [1, 2] }),
        })
        .await
        .expect_err("non-string array must be rejected");
    assert!(format!("{err:#}").contains("array of strings"));

    // base_emotions must be an object.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character_emotional_profile".into(),
            entity_id: profile_id.clone(),
            changes: serde_json::json!({ "base_emotions": ["grief"] }),
        })
        .await
        .expect_err("non-object base_emotions must be rejected");
    assert!(format!("{err:#}").contains("JSON object"));

    // flex_range must be null or {low?, high?} with string fields.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character_emotional_profile".into(),
            entity_id: profile_id.clone(),
            changes: serde_json::json!({ "flex_range": { "low": 3 } }),
        })
        .await
        .expect_err("non-string flex_range fields must be rejected");
    assert!(format!("{err:#}").contains("flex_range"));

    // Validation runs before ANY column write: a mixed change set with one
    // valid and one invalid field must not half-apply.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "character_emotional_profile".into(),
            entity_id: profile_id.clone(),
            changes: serde_json::json!({
                "triggers": ["salt"],
                "suppressed": [1]
            }),
        })
        .await
        .expect_err("mixed valid/invalid change set must be rejected whole");
    assert!(format!("{err:#}").contains("array of strings"));
    let untouched = svc
        .repository()
        .get_character_emotional_profile_by_id(&profile_id)
        .await
        .unwrap();
    assert!(untouched.triggers.is_empty());
    assert!(untouched.suppressed.is_empty());
}

/// Bug 4 verification: a mixed-type payload — one string field plus one
/// array field in the same changes object — applies every field. An
/// earlier failure rejected the whole call with "changes must be a JSON
/// object" despite changes being a valid object; this guards the
/// regression at the service boundary.
#[tokio::test]
async fn update_entity_mixed_string_and_array_payload_applies_both() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Mixed Payload".into(),
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
    let character_id = create_character_named(&svc, &proj.project_id, "Mara").await;

    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "character".into(),
        entity_id: character_id.clone(),
        changes: serde_json::json!({
            "summary": "A smuggler with a salt blade.",
            "aliases": ["The Salt Blade", "Gull"]
        }),
    })
    .await
    .expect("mixed string + array payload must succeed");

    let updated = svc.repository().get_character(&character_id).await.unwrap();
    assert_eq!(updated.summary, "A smuggler with a salt blade.");
    assert_eq!(
        updated.aliases,
        vec!["The Salt Blade".to_string(), "Gull".to_string()]
    );
}

/// Audit spot-check: the allowlist sweep opened the locked planning
/// fields on the remaining entity tables. world_rule goes through its
/// dedicated tool (same allowlist underneath), timeline_event through
/// update_entity directly.
#[tokio::test]
async fn update_entity_audit_opened_world_rule_and_timeline_planning_fields() {
    use spindle_core::models::{
        CreateTimelineEventInput, CreateWorldRuleInput, StoryPlacement, UpdateWorldRuleInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Audit Sweep".into(),
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

    let rule = svc
        .create_world_rule(CreateWorldRuleInput {
            project_id: proj.project_id.clone(),
            rule_name: "Salt Price".into(),
            rule_type: "constraint".into(),
            description: "Salt is worth its weight in iron.".into(),
            scan_pattern: None,
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();
    svc.update_world_rule(UpdateWorldRuleInput {
        world_rule_id: rule.world_rule_id.clone(),
        changes: serde_json::json!({
            "rule_type": "style",
            "scan_pattern": "salt",
            "relevance_tags": ["trade"]
        }),
    })
    .await
    .expect("world_rule planning fields must be updatable via update_world_rule");
    let rule_after = svc
        .repository()
        .get_world_rule(&rule.world_rule_id)
        .await
        .unwrap();
    assert_eq!(rule_after.rule_type, "style");
    assert_eq!(rule_after.scan_pattern.as_deref(), Some("salt"));
    assert_eq!(rule_after.relevance_tags, Some(vec!["trade".to_string()]));

    let event = svc
        .create_timeline_event(CreateTimelineEventInput {
            project_id: proj.project_id.clone(),
            title: "The Harbor Fire".into(),
            event_type: "backstory".into(),
            placement: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: None,
                note: None,
            },
            summary: "Original summary.".into(),
            related_entity_ids: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "timeline_event".into(),
        entity_id: event.timeline_event_id.clone(),
        changes: serde_json::json!({
            "title": "The Harbor Fire (corrected)",
            "summary": "Corrected after the renumber.",
            "placement": { "book_number": 1, "chapter_number": 3 }
        }),
    })
    .await
    .expect("timeline_event planning fields must be updatable");
    let event_after = svc
        .repository()
        .get_timeline_event(&event.timeline_event_id)
        .await
        .unwrap();
    assert_eq!(event_after.title, "The Harbor Fire (corrected)");
    assert_eq!(event_after.summary, "Corrected after the renumber.");
    assert_eq!(event_after.placement.chapter_number, 3);
}

// ── Word-count range extraction (contradiction check primitives) ────────

#[test]
fn extract_word_count_ranges_parses_common_formats() {
    assert_eq!(
        extract_word_count_ranges("One beat per chapter, 1,200–2,000 words."),
        vec![(1200, 2000)]
    );
    assert_eq!(
        extract_word_count_ranges("Chapters run 1200-2000 words."),
        vec![(1200, 2000)]
    );
    assert_eq!(
        extract_word_count_ranges("Target 3,500 to 5,000 words per chapter."),
        vec![(3500, 5000)]
    );
    assert_eq!(
        extract_word_count_ranges("Aim for 1200–2000-word chapters."),
        vec![(1200, 2000)]
    );
    // No "words" nearby -> not a length target.
    assert!(extract_word_count_ranges("Between 1200 and 2000 pages.").is_empty());
    // A lone number is not a range.
    assert!(extract_word_count_ranges("Chapters are 2000 words.").is_empty());
    assert!(extract_word_count_ranges("No numbers here.").is_empty());
}

#[test]
fn word_ranges_contradict_detects_disjoint_targets() {
    // Disjoint -> contradiction.
    assert!(word_ranges_contradict(&[(1200, 2000)], &[(3500, 5000)]));
    // Overlapping -> no contradiction.
    assert!(!word_ranges_contradict(&[(1200, 2000)], &[(1800, 2500)]));
    assert!(!word_ranges_contradict(&[(1200, 2000)], &[(1200, 2000)]));
    // Empty side never contradicts.
    assert!(!word_ranges_contradict(&[], &[(3500, 5000)]));
    assert!(!word_ranges_contradict(&[(1200, 2000)], &[]));
}

// ── reader_contract update via update_entity ───────────────────────────

/// The reader contract is set once at create_project and was previously
/// immutable — a stale style note silently overrode an explicit author
/// directive and corrupted drafted chapters. update_entity must now merge
/// reader_contract / promise / style_notes / boundaries, preserving fields
/// the caller did not send.
#[tokio::test]
async fn update_entity_project_reader_contract_is_updatable_and_partial_merge_preserves_rest() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Contract Update".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Epic adventure".into(),
                style_notes: vec!["One beat per chapter, 1,200–2,000 words.".into()],
                boundaries: vec!["No gore".into()],
            },
        })
        .await
        .unwrap();

    // Update ONLY style_notes; promise and boundaries must survive.
    let out = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "project".into(),
            entity_id: proj.project_id.clone(),
            changes: serde_json::json!({
                "style_notes": ["One beat per chapter, 3,500–5,000 words."]
            }),
        })
        .await
        .expect("style_notes must be updatable");
    assert!(out.warnings.is_empty());

    let project = svc
        .repository()
        .get_project(&proj.project_id)
        .await
        .unwrap();
    let contract = project.reader_contract.into_core();
    assert_eq!(contract.promise, "Epic adventure");
    assert_eq!(contract.boundaries, vec!["No gore".to_string()]);
    assert_eq!(
        contract.style_notes,
        vec!["One beat per chapter, 3,500–5,000 words.".to_string()]
    );

    // Update promise via a top-level key.
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "project".into(),
        entity_id: proj.project_id.clone(),
        changes: serde_json::json!({ "promise": "A heist with heart" }),
    })
    .await
    .expect("promise must be updatable");
    let project = svc
        .repository()
        .get_project(&proj.project_id)
        .await
        .unwrap();
    assert_eq!(
        project.reader_contract.into_core().promise,
        "A heist with heart"
    );

    // Update the whole reader_contract object (nested partial).
    svc.update_entity(UpdateEntityInput {
        allow_rename: None,
        entity_type: "project".into(),
        entity_id: proj.project_id.clone(),
        changes: serde_json::json!({
            "reader_contract": { "boundaries": ["No gore", "No character death"] }
        }),
    })
    .await
    .expect("reader_contract object must be updatable");
    let project = svc
        .repository()
        .get_project(&proj.project_id)
        .await
        .unwrap();
    let contract = project.reader_contract.into_core();
    assert_eq!(
        contract.boundaries,
        vec!["No gore".to_string(), "No character death".to_string()]
    );
    // Nested partial must not wipe promise/style_notes.
    assert_eq!(contract.promise, "A heist with heart");
    assert_eq!(contract.style_notes.len(), 1);
}

/// A wrong-shaped reader-contract value must be rejected before any write,
/// leaving the stored contract untouched.
#[tokio::test]
async fn update_entity_project_reader_contract_rejects_wrong_shapes_before_writing() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Contract Shapes".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: vec!["keep me".into()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // style_notes must be an array of strings.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "project".into(),
            entity_id: proj.project_id.clone(),
            changes: serde_json::json!({ "style_notes": ["ok", 42] }),
        })
        .await
        .expect_err("non-string style note must be rejected");
    assert!(format!("{err:#}").contains("array of strings"));

    // promise must be a string.
    let err = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "project".into(),
            entity_id: proj.project_id.clone(),
            changes: serde_json::json!({ "promise": { "not": "a string" } }),
        })
        .await
        .expect_err("non-string promise must be rejected");
    assert!(format!("{err:#}").contains("must be a string"));

    // The stored contract is untouched after the rejected writes.
    let project = svc
        .repository()
        .get_project(&proj.project_id)
        .await
        .unwrap();
    let contract = project.reader_contract.into_core();
    assert_eq!(contract.promise, "p");
    assert_eq!(contract.style_notes, vec!["keep me".to_string()]);
}

// ── Style-surface contradiction warnings ───────────────────────────────

/// Updating a style surface to a word-count target that contradicts
/// another surface must surface a warning — the stale-contract failure
/// that corrupted chapters.
#[tokio::test]
async fn update_entity_reader_contract_contradiction_warns_against_narrator_voice() {
    use spindle_core::models::SetNarratorVoiceInput;
    use spindle_core::style::NarratorVoice;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Contradiction".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: vec!["One beat per chapter, 1,200–2,000 words.".into()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    // Narrator voice agrees with the contract -> no warning.
    let agree = svc
        .set_narrator_voice(SetNarratorVoiceInput {
            project_id: proj.project_id.clone(),
            narrator_voice: NarratorVoice {
                pacing_feel: Some("chapters land 1,200–2,000 words".to_string()),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    assert!(agree.warnings.is_empty());

    // Narrator voice moves to a disjoint target -> warning at the update.
    let disagree = svc
        .set_narrator_voice(SetNarratorVoiceInput {
            project_id: proj.project_id.clone(),
            narrator_voice: NarratorVoice {
                pacing_feel: Some("chapters land 3,500–5,000 words".to_string()),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    assert_eq!(disagree.warnings.len(), 1);
    assert!(
        disagree.warnings[0].contains("style-surface contradiction"),
        "expected contradiction warning, got: {:?}",
        disagree.warnings
    );

    // Fixing the reader contract to match clears the contradiction; the
    // update that fixes it reports no warning.
    let fixed = svc
        .update_entity(UpdateEntityInput {
            allow_rename: None,
            entity_type: "project".into(),
            entity_id: proj.project_id.clone(),
            changes: serde_json::json!({
                "style_notes": ["One beat per chapter, 3,500–5,000 words."]
            }),
        })
        .await
        .unwrap();
    assert!(
        fixed.warnings.is_empty(),
        "matching targets must not warn, got: {:?}",
        fixed.warnings
    );
}

/// Targeted milestone update: patch placement and reached_at by label
/// without resending (or even being able to read) the rest of the
/// milestone array.
#[tokio::test]
async fn update_arc_milestone_patches_only_the_supplied_fields() {
    use spindle_core::models::{
        CharacterArcMilestone, CharacterEmotionalProfileData, CharacterVoiceProfileData,
        CreateCharacterArcInput, CreateCharacterInput, StoryPlacement, UpdateArcMilestoneInput,
    };

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "Milestone Update".into(),
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
    let character = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: proj.project_id.clone(),
            name: "Mara Oathkeeper".into(),
            summary: "Warden of the Ash Gate.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData::default(),
            emotional_profile: CharacterEmotionalProfileData::default(),
            initial_state: None,
        })
        .await
        .unwrap();
    let arc = svc
        .create_character_arc(CreateCharacterArcInput {
            project_id: proj.project_id.clone(),
            character_id: character.character_id.clone(),
            arc_type: "belief_shift".into(),
            starting_state: "obeys".into(),
            ending_state: "defies".into(),
            milestones: vec![
                CharacterArcMilestone {
                    label: "first doubt".into(),
                    placement: Some(StoryPlacement {
                        book_number: 1,
                        chapter_number: 10,
                        scene_order: None,
                        note: None,
                    }),
                    description: "Mara questions the oath.".into(),
                    unlocks: vec!["inner monologue".into()],
                    reached_at: None,
                },
                CharacterArcMilestone {
                    label: "open defiance".into(),
                    placement: Some(StoryPlacement {
                        book_number: 1,
                        chapter_number: 30,
                        scene_order: None,
                        note: None,
                    }),
                    description: "Mara refuses the order.".into(),
                    unlocks: Vec::new(),
                    reached_at: None,
                },
            ],
            thematic_purpose: "Claim agency.".into(),
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();

    let out = svc
        .update_arc_milestone(UpdateArcMilestoneInput {
            arc_id: arc.character_arc_id.clone(),
            label: "first doubt".into(),
            placement: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 12,
                scene_order: None,
                note: None,
            }),
            reached_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 12,
                scene_order: Some(2),
                note: None,
            }),
        })
        .await
        .expect("update_arc_milestone must succeed");

    assert_eq!(
        out.milestone.placement.as_ref().map(|p| p.chapter_number),
        Some(12)
    );
    assert_eq!(
        out.milestone
            .reached_at
            .as_ref()
            .map(|p| (p.chapter_number, p.scene_order)),
        Some((12, Some(2)))
    );
    // Untouched fields survive the targeted patch.
    assert_eq!(out.milestone.description, "Mara questions the oath.");
    assert_eq!(out.milestone.unlocks, vec!["inner monologue".to_string()]);

    // The write persisted, and the sibling milestone is untouched.
    let stored = svc
        .repository()
        .get_character_arc(&arc.character_arc_id)
        .await
        .unwrap();
    assert_eq!(stored.milestones.len(), 2);
    let sibling = stored
        .milestones
        .iter()
        .find(|m| m.label == "open defiance")
        .unwrap();
    assert_eq!(
        sibling.placement.as_ref().map(|p| p.chapter_number),
        Some(30)
    );
    assert!(sibling.reached_at.is_none());

    // Unknown label: error names the valid labels.
    let err = svc
        .update_arc_milestone(UpdateArcMilestoneInput {
            arc_id: arc.character_arc_id.clone(),
            label: "no such milestone".into(),
            placement: None,
            reached_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: None,
                note: None,
            }),
        })
        .await
        .expect_err("unknown label must fail");
    let msg = err.to_string();
    assert!(msg.contains("first doubt"), "error lists labels: {msg}");
    assert!(msg.contains("open defiance"), "error lists labels: {msg}");

    // No-op guard: neither field supplied.
    svc.update_arc_milestone(UpdateArcMilestoneInput {
        arc_id: arc.character_arc_id.clone(),
        label: "first doubt".into(),
        placement: None,
        reached_at: None,
    })
    .await
    .expect_err("empty update must fail");
}

/// The arc-update path can set `reached_at` on a milestone by resending the
/// milestones array through update_entity; the audit then goes silent.
#[tokio::test]
async fn arc_milestone_reached_at_settable_via_update_entity() {
    use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput, UpdateEntityInput};

    let (_tmp, svc, project_id, _scene, character_id) = convergence_fixture_scene(1).await;
    for ch in [2, 3, 4] {
        seed_scene_at(&svc, &project_id, ch).await;
    }
    let arc_id = seed_arc_with_milestone(
        &svc,
        &project_id,
        &character_id,
        Some(placement(1, 2)),
        None,
    )
    .await;

    // Overdue before the update.
    let before = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["arc_milestone_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        before
            .issues
            .iter()
            .any(|i| i.check_type == "arc_milestone_audit"),
        "milestone should be overdue before it is reached"
    );

    // Resend the whole milestones array with reached_at set.
    svc.update_entity(UpdateEntityInput { allow_rename: None,
            entity_type: "character_arc".into(),
            entity_id: arc_id.clone(),
            changes: serde_json::json!({
                "milestones": [{
                    "label": "the reckoning",
                    "placement": {"book_number": 1, "chapter_number": 2, "scene_order": null, "note": null},
                    "description": "faces the truth",
                    "unlocks": [],
                    "reached_at": {"book_number": 1, "chapter_number": 3, "scene_order": null, "note": null}
                }]
            }),
        })
        .await
        .unwrap();

    let after = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: vec!["arc_milestone_audit".to_string()],
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        !after
            .issues
            .iter()
            .any(|i| i.check_type == "arc_milestone_audit"),
        "after setting reached_at the milestone must be silent: {:?}",
        after.issues
    );
}
