use super::*;

mod ratify {
    use super::*;
    use crate::sqlite::repository::StageCanonDeltaParams;
    use spindle_core::models::{
        CanonDeltaDecisionInput, CommitQuantityStateInput, CreateCharacterArcInput,
        CreateCharacterInput, CreateConflictInput, CreateMotifInput, CreateNarrativePromiseInput,
        DecideCanonDeltasInput, ListCanonDeltasInput, QuantityState, RegisterCanonicalFactInput,
        SecrecyScope, StatedConsequence, StoryPlacement,
    };

    /// Stage one delta directly on the active branch, bypassing the model, so
    /// the dispatcher can be driven deterministically per class.
    async fn stage(
        svc: &SqliteSpindleService,
        project_id: &str,
        scene_id: &str,
        delta_class: &str,
        target_id: Option<&str>,
        payload: serde_json::Value,
    ) -> String {
        let branch = svc
            .repository()
            .get_active_branch(project_id)
            .await
            .unwrap();
        let stored = svc
            .repository()
            .stage_canon_delta(StageCanonDeltaParams {
                project_id: project_id.to_string(),
                branch_id: branch.id,
                scene_id: scene_id.to_string(),
                authoring_run_id: None,
                delta_class: delta_class.to_string(),
                target_id: target_id.map(str::to_string),
                payload,
                evidence: "verbatim evidence".to_string(),
                confidence: "high".to_string(),
            })
            .await
            .unwrap();
        stored.id
    }

    fn voice() -> spindle_core::models::CharacterVoiceProfileData {
        spindle_core::models::CharacterVoiceProfileData {
            tone: None,
            vocabulary: Vec::new(),
            sentence_structure: Vec::new(),
            tics: Vec::new(),
            forbidden_words: Vec::new(),
            example_lines: Vec::new(),
            established_in_scene_id: None,
            updated_at: None,
        }
    }

    fn emotions() -> spindle_core::models::CharacterEmotionalProfileData {
        spindle_core::models::CharacterEmotionalProfileData {
            base_emotions: std::collections::BTreeMap::new(),
            suppressed: Vec::new(),
            triggers: Vec::new(),
            defense_mechanisms: Vec::new(),
            flex_range: None,
        }
    }

    async fn make_character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
        svc.create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project_id.to_string(),
            name: name.to_string(),
            summary: format!("{name} exists."),
            role: "supporting".to_string(),
            realm: None,
            voice_profile: voice(),
            emotional_profile: emotions(),
            initial_state: None,
        })
        .await
        .unwrap()
        .character_id
    }

    // ── Miner-payload normalization (defect item 4) ─────────────────────
    // The miner emits payload shapes the strict DTOs reject: bare ULIDs
    // without table prefixes, plural subject tables, `description` instead
    // of `summary`, missing role, null voice/emotional profiles, and a
    // free-string planted_at. The dispatcher normalizes these so already-
    // staged rows stay ratifiable.

    #[tokio::test]
    async fn bare_ulid_promise_id_is_prefixed_and_applies() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Bare Promise").await;
        let promise = svc
            .create_narrative_promise(CreateNarrativePromiseInput {
                project_id: project.project_id.clone(),
                promise_type: "mystery".to_string(),
                description: "The drawer will open".to_string(),
                planted_at: StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(1),
                    note: None,
                },
                planned_payoff: None,
                notes: Vec::new(),
            })
            .await
            .unwrap();
        let bare_key = promise
            .narrative_promise_id
            .strip_prefix("narrative_promise:")
            .expect("promise ids are table-prefixed")
            .to_string();
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "promise_payoff_candidate",
            None,
            serde_json::json!({
                "narrative_promise_id": bare_key,
                "proposed_status": "paid_off"
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(
            out.applied_count, 1,
            "bare ULID must be normalized to the class's table prefix: {out:?}"
        );
        let updated = svc
            .repository()
            .get_narrative_promise(&promise.narrative_promise_id)
            .await
            .unwrap();
        assert_eq!(updated.status, "paid_off");
    }

    #[tokio::test]
    async fn bare_ulid_conflict_id_is_prefixed_and_applies() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Bare Conflict").await;
        let conflict = svc
            .create_conflict(CreateConflictInput {
                project_id: project.project_id.clone(),
                name: "The siege".to_string(),
                conflict_type: "external".to_string(),
                stakes: "the gate".to_string(),
                escalation_stages: Vec::new(),
                expected_total_cycles: None,
                try_fail_cycles: Vec::new(),
                stated_consequences: Vec::new(),
            })
            .await
            .unwrap();
        let bare_key = conflict
            .conflict_id
            .strip_prefix("conflict:")
            .expect("conflict ids are table-prefixed")
            .to_string();
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "try_fail_cycle",
            None,
            serde_json::json!({
                "conflict_id": bare_key,
                "label": "storm the wall",
                "outcome": "fail"
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(
            out.applied_count, 1,
            "bare conflict ULID must be normalized: {out:?}"
        );
        let updated = svc
            .repository()
            .get_conflict(&conflict.conflict_id)
            .await
            .unwrap();
        assert_eq!(updated.try_fail_cycles.len(), 1);
    }

    #[tokio::test]
    async fn plural_subject_table_is_normalized_to_singular() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Plural Table").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({
                "subject_table": "characters",
                "predicate": "eye_color",
                "value_text": "grey"
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(out.applied_count, 1, "{out:?}");
        let fact_id = out.results[0].applied_record_id.as_deref().unwrap();
        let fact = svc.repository().get_canonical_fact(fact_id).await.unwrap();
        assert_eq!(
            fact.subject_table, "character",
            "plural subject_table must be stored singular"
        );
    }

    #[tokio::test]
    async fn minimal_miner_entity_candidate_character_applies() {
        // The miner emits {kind,name,description} with explicit null
        // profiles. The dispatcher maps description→summary, defaults
        // role to "minor", and tolerates null/missing voice_profile and
        // emotional_profile.
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Miner Entity").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "entity_candidate",
            None,
            serde_json::json!({
                "kind": "characters",
                "name": "The grey-eyed stranger",
                "description": "A stranger with grey eyes seen at the door.",
                "voice_profile": null,
                "emotional_profile": null
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(
            out.applied_count, 1,
            "minimal miner payload must be normalized and applied: {out:?}"
        );
        let character_id = out.results[0].applied_record_id.as_deref().unwrap();
        let character = svc.repository().get_character(character_id).await.unwrap();
        assert_eq!(character.name, "The grey-eyed stranger");
        assert_eq!(
            character.summary,
            "A stranger with grey eyes seen at the door."
        );
        assert_eq!(character.role, "minor");
    }

    #[tokio::test]
    async fn string_planted_at_is_coerced_to_mined_scene_placement() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify String Plant").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "promise_planted",
            None,
            serde_json::json!({
                "promise_type": "vow",
                "description": "She will make the call.",
                "planted_at": "Tomorrow I call. Once."
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(
            out.applied_count, 1,
            "free-string planted_at must coerce to the mined scene's placement: {out:?}"
        );
        let promise_id = out.results[0].applied_record_id.as_deref().unwrap();
        let promise = svc
            .repository()
            .get_narrative_promise(promise_id)
            .await
            .unwrap();
        assert_eq!(promise.planted_at.book_number, 1);
        assert_eq!(promise.planted_at.chapter_number, 1);
        assert_eq!(promise.planted_at.scene_order, Some(1));
        assert_eq!(
            promise.planted_at.note.as_deref(),
            Some("Tomorrow I call. Once."),
            "the miner's free-string placement is preserved as the note"
        );
    }

    #[tokio::test]
    async fn apply_canonical_fact_creates_the_fact() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Fact").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({
                "subject_table": "character",
                "predicate": "eye_color",
                "value_text": "grey"
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id: delta_id.clone(),
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();

        assert_eq!(out.applied_count, 1);
        assert_eq!(out.rejected_count, 0);
        assert_eq!(out.failed_count, 0);
        let result = &out.results[0];
        assert_eq!(result.outcome, "applied");
        let fact_id = result
            .applied_record_id
            .as_deref()
            .expect("fact id returned");
        assert!(fact_id.starts_with("canonical_fact:"));

        // The fact is real canon.
        let fact = svc.repository().get_canonical_fact(fact_id).await.unwrap();
        assert_eq!(fact.predicate, "eye_color");

        // The row is now applied and stamped with the default decider.
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let rows = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("applied"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].decided_by.as_deref(), Some("operator"));
    }

    #[tokio::test]
    async fn apply_promise_payoff_candidate_flips_promise_to_paid_off() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Payoff").await;
        let promise = svc
            .create_narrative_promise(CreateNarrativePromiseInput {
                project_id: project.project_id.clone(),
                promise_type: "mystery".to_string(),
                description: "The door will open".to_string(),
                planted_at: StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(1),
                    note: None,
                },
                planned_payoff: None,
                notes: Vec::new(),
            })
            .await
            .unwrap();
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "promise_payoff_candidate",
            Some(&promise.narrative_promise_id),
            serde_json::json!({
                "narrative_promise_id": promise.narrative_promise_id,
                "proposed_status": "paid_off"
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: Some("editor".to_string()),
            })
            .await
            .unwrap();
        assert_eq!(out.applied_count, 1);
        assert_eq!(out.results[0].outcome, "applied");

        let updated = svc
            .repository()
            .get_narrative_promise(&promise.narrative_promise_id)
            .await
            .unwrap();
        assert_eq!(updated.status, "paid_off");
    }

    #[tokio::test]
    async fn apply_knowledge_learned_with_secret_link_expands_circle() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Reveal").await;
        let keeper = make_character(&svc, &project.project_id, "Mara").await;
        let confidant = make_character(&svc, &project.project_id, "Bran").await;

        // A secret fact whose sole holder is the keeper.
        let secret = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project.project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".to_string()),
                subject_id: Some(keeper.clone()),
                predicate: Some("reincarnation".to_string()),
                value_kind: Some("string".to_string()),
                value_text: Some("is a reincarnated warden".to_string()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: Some(SecrecyScope {
                    holder_ids: vec![keeper.clone()],
                    concealment_note: None,
                }),
            })
            .await
            .unwrap();

        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "knowledge_learned",
            Some(&confidant),
            serde_json::json!({
                "character_id": confidant,
                "fact": "Mara is a reincarnated warden.",
                "source_summary": "she finally told him",
                "reader_visible": true,
                "secret_of_fact_id": secret.canonical_fact_id,
            }),
        )
        .await;

        svc.decide_canon_deltas(DecideCanonDeltasInput {
            project_id: project.project_id.clone(),
            decisions: vec![CanonDeltaDecisionInput {
                delta_id,
                action: "apply".to_string(),
                edit: None,
                note: None,
            }],
            decided_by: None,
        })
        .await
        .unwrap();

        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let circle = svc
            .repository()
            .secret_circle_members(&project.project_id, &branch.id, &secret.canonical_fact_id)
            .await
            .unwrap();
        assert_eq!(circle.len(), 2, "circle expanded to keeper + confidant");
        assert!(circle.iter().any(|(id, _)| id == &confidant));
    }

    #[tokio::test]
    async fn apply_escalation_demonstrated_sets_index_one_leaving_zero_none() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Escalation").await;
        let conflict = svc
            .create_conflict(CreateConflictInput {
                project_id: project.project_id.clone(),
                name: "The Siege".to_string(),
                conflict_type: "external".to_string(),
                stakes: "the city".to_string(),
                escalation_stages: vec!["walls breached".to_string(), "keep falls".to_string()],
                expected_total_cycles: None,
                try_fail_cycles: Vec::new(),
                stated_consequences: Vec::new(),
            })
            .await
            .unwrap();

        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "escalation_demonstrated",
            Some(&conflict.conflict_id),
            serde_json::json!({ "conflict_id": conflict.conflict_id, "stage_index": 1 }),
        )
        .await;

        svc.decide_canon_deltas(DecideCanonDeltasInput {
            project_id: project.project_id.clone(),
            decisions: vec![CanonDeltaDecisionInput {
                delta_id,
                action: "apply".to_string(),
                edit: None,
                note: None,
            }],
            decided_by: None,
        })
        .await
        .unwrap();

        let updated = svc
            .repository()
            .get_conflict(&conflict.conflict_id)
            .await
            .unwrap();
        assert_eq!(updated.escalation_demonstrated.len(), 2);
        assert!(
            updated.escalation_demonstrated[0].is_none(),
            "index 0 stays None"
        );
        let stamp = updated.escalation_demonstrated[1]
            .as_ref()
            .expect("index 1 demonstrated");
        assert_eq!(stamp.chapter_number, 1);
    }

    #[tokio::test]
    async fn apply_arc_milestone_reached_sets_reached_at_by_label() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Arc").await;
        let hero = make_character(&svc, &project.project_id, "Kael").await;
        let arc = svc
            .create_character_arc(CreateCharacterArcInput {
                project_id: project.project_id.clone(),
                character_id: hero,
                arc_type: "growth".to_string(),
                starting_state: "naive".to_string(),
                ending_state: "wise".to_string(),
                milestones: vec![
                    spindle_core::models::CharacterArcMilestone {
                        label: "first blood".to_string(),
                        placement: None,
                        description: "kills for the first time".to_string(),
                        unlocks: Vec::new(),
                        reached_at: None,
                    },
                    spindle_core::models::CharacterArcMilestone {
                        label: "the reckoning".to_string(),
                        placement: None,
                        description: "faces the villain".to_string(),
                        unlocks: Vec::new(),
                        reached_at: None,
                    },
                ],
                thematic_purpose: "coming of age".to_string(),
                connected_theme_ids: Vec::new(),
            })
            .await
            .unwrap();

        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "arc_milestone_reached",
            Some(&arc.character_arc_id),
            serde_json::json!({
                "character_arc_id": arc.character_arc_id,
                "milestone_label": "the reckoning"
            }),
        )
        .await;

        svc.decide_canon_deltas(DecideCanonDeltasInput {
            project_id: project.project_id.clone(),
            decisions: vec![CanonDeltaDecisionInput {
                delta_id,
                action: "apply".to_string(),
                edit: None,
                note: None,
            }],
            decided_by: None,
        })
        .await
        .unwrap();

        let updated = svc
            .repository()
            .get_character_arc(&arc.character_arc_id)
            .await
            .unwrap();
        let first = updated
            .milestones
            .iter()
            .find(|m| m.label == "first blood")
            .unwrap();
        assert!(first.reached_at.is_none(), "unrelated milestone untouched");
        let reckoning = updated
            .milestones
            .iter()
            .find(|m| m.label == "the reckoning")
            .unwrap();
        let stamp = reckoning.reached_at.as_ref().expect("milestone reached");
        assert_eq!(stamp.chapter_number, 1);
    }

    #[tokio::test]
    async fn apply_arc_milestone_unknown_label_is_preflight_failure() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Arc Bad").await;
        let hero = make_character(&svc, &project.project_id, "Kael").await;
        let arc = svc
            .create_character_arc(CreateCharacterArcInput {
                project_id: project.project_id.clone(),
                character_id: hero,
                arc_type: "growth".to_string(),
                starting_state: "naive".to_string(),
                ending_state: "wise".to_string(),
                milestones: vec![spindle_core::models::CharacterArcMilestone {
                    label: "first blood".to_string(),
                    placement: None,
                    description: "kills".to_string(),
                    unlocks: Vec::new(),
                    reached_at: None,
                }],
                thematic_purpose: "coming of age".to_string(),
                connected_theme_ids: Vec::new(),
            })
            .await
            .unwrap();
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "arc_milestone_reached",
            Some(&arc.character_arc_id),
            serde_json::json!({
                "character_arc_id": arc.character_arc_id,
                "milestone_label": "no such milestone"
            }),
        )
        .await;

        let err = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id: delta_id.clone(),
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("milestone"),
            "err names the label: {err}"
        );
        // Row untouched — still staged.
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let rows = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("staged"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn apply_beat_annotation_lands_intensity_and_motif() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Beat").await;
        let motif = svc
            .create_motif(CreateMotifInput {
                project_id: project.project_id.clone(),
                name: "mirrors".to_string(),
                description: "reflections recur".to_string(),
                max_uses_per_chapter: None,
                connected_theme_ids: Vec::new(),
            })
            .await
            .unwrap();
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "beat_annotation",
            Some(&scene_id),
            serde_json::json!({
                "motif_ids": [motif.motif_id],
                "intensity": 0.7
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(out.applied_count, 1);
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let annotation = svc
            .repository()
            .get_scene_beat_annotation(&branch.id, &scene_id)
            .await
            .unwrap()
            .expect("annotation written");
        assert_eq!(annotation.intensity, Some(0.7));
        assert_eq!(annotation.motif_ids, vec![motif.motif_id]);
    }

    #[tokio::test]
    async fn apply_quantity_change_commits_state() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Quantity").await;
        let hero = make_character(&svc, &project.project_id, "Rich").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "quantity_change",
            Some(&hero),
            serde_json::json!({
                "subject_table": "character",
                "subject_id": hero,
                "measure": "wealth",
                "state": { "band": "poor", "change_reason": "spent it all" }
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(out.applied_count, 1);
        assert_eq!(out.results[0].outcome, "applied");
    }

    #[tokio::test]
    async fn apply_entity_candidate_character_creates_character() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Entity").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "entity_candidate",
            None,
            serde_json::json!({
                "kind": "character",
                "name": "The Stranger",
                "summary": "A grey-eyed newcomer.",
                "role": "minor",
                "voice_profile": {},
                "emotional_profile": {}
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(out.applied_count, 1);
        let new_char = out.results[0]
            .applied_record_id
            .as_deref()
            .expect("character id");
        assert!(new_char.starts_with("character:"));
        let character = svc.repository().get_character(new_char).await.unwrap();
        assert_eq!(character.name, "The Stranger");
    }

    #[tokio::test]
    async fn edit_before_apply_uses_and_records_edited_payload() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Edit").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({
                "subject_table": "character",
                "predicate": "eye_color",
                "value_text": "grey"
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id: delta_id.clone(),
                    action: "apply".to_string(),
                    edit: Some(serde_json::json!({
                        "subject_table": "character",
                        "predicate": "eye_color",
                        "value_text": "hazel"
                    })),
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        let fact_id = out.results[0].applied_record_id.as_deref().unwrap();
        // Applied value is the EDITED one.
        let fact = svc.repository().get_canonical_fact(fact_id).await.unwrap();
        assert_eq!(fact.value_text.as_deref(), Some("hazel"));

        // The edited payload is recorded on the row.
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let rows = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("applied"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        assert_eq!(rows[0].payload["value_text"], "hazel");
    }

    #[tokio::test]
    async fn reject_marks_row_and_writes_no_canon() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Reject").await;
        let delta_id = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({
                "subject_table": "character",
                "predicate": "eye_color",
                "value_text": "grey"
            }),
        )
        .await;

        let out = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![CanonDeltaDecisionInput {
                    delta_id,
                    action: "reject".to_string(),
                    edit: None,
                    note: Some("not canon — narrator is unreliable here".to_string()),
                }],
                decided_by: None,
            })
            .await
            .unwrap();
        assert_eq!(out.rejected_count, 1);
        assert_eq!(out.applied_count, 0);
        assert_eq!(out.results[0].outcome, "rejected");
        // The note is echoed in the output (no row column for it).
        assert_eq!(
            out.results[0].note.as_deref(),
            Some("not canon — narrator is unreliable here")
        );

        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let facts = svc
            .repository()
            .list_canonical_facts_by_project(&project.project_id)
            .await
            .unwrap();
        assert!(facts.is_empty(), "reject writes no canon");
        let rejected = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("rejected"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        assert_eq!(rejected.len(), 1);
    }

    #[tokio::test]
    async fn preflight_rejects_whole_batch_on_one_malformed_payload() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Atomic").await;
        let good1 = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({"subject_table": "character", "predicate": "a", "value_text": "1"}),
        )
        .await;
        // Malformed: value_text should be a string, not a number.
        let bad = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({"subject_table": "character", "predicate": "b", "value_text": 42}),
        )
        .await;
        let good2 = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({"subject_table": "character", "predicate": "c", "value_text": "3"}),
        )
        .await;

        let err = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![
                    CanonDeltaDecisionInput {
                        delta_id: good1.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                    CanonDeltaDecisionInput {
                        delta_id: bad.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                    CanonDeltaDecisionInput {
                        delta_id: good2.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                ],
                decided_by: None,
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains(&bad),
            "error names the offender: {err}"
        );

        // Zero writes; all three still staged.
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let staged = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("staged"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        assert_eq!(staged.len(), 3);
        let facts = svc
            .repository()
            .list_canonical_facts_by_project(&project.project_id)
            .await
            .unwrap();
        assert!(facts.is_empty(), "no fact written");
    }

    #[tokio::test]
    async fn decision_on_already_decided_row_errors_whole_call() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Final").await;
        let already = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({"subject_table": "character", "predicate": "x", "value_text": "1"}),
        )
        .await;
        // Apply it first so it becomes terminal.
        svc.decide_canon_deltas(DecideCanonDeltasInput {
            project_id: project.project_id.clone(),
            decisions: vec![CanonDeltaDecisionInput {
                delta_id: already.clone(),
                action: "apply".to_string(),
                edit: None,
                note: None,
            }],
            decided_by: None,
        })
        .await
        .unwrap();

        let fresh = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({"subject_table": "character", "predicate": "y", "value_text": "2"}),
        )
        .await;
        let err = svc
            .decide_canon_deltas(DecideCanonDeltasInput {
                project_id: project.project_id.clone(),
                decisions: vec![
                    CanonDeltaDecisionInput {
                        delta_id: fresh.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                    CanonDeltaDecisionInput {
                        delta_id: already.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                ],
                decided_by: None,
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains(&already),
            "names the decided row: {err}"
        );
        // The fresh row was NOT applied (pre-flight aborted the whole call).
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let staged = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("staged"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        assert_eq!(staged.len(), 1, "the fresh row stays staged");
        assert_eq!(staged[0].id, fresh);
    }

    #[tokio::test]
    async fn mid_apply_failure_stops_and_reports_honestly() {
        // Pre-flight passes for a knowledge_learned reveal (the secret fact
        // exists and is secret), but between pre-flight and apply the fact's
        // secret flag is cleared, so `record_knowledge` rejects the reveal at
        // apply time (it re-checks the flag). Earlier applies in the same
        // batch stay applied; the failing row stays staged; the later row
        // reads as not_reached. The clear runs through the injectable
        // after-pre-flight hook on `decide_canon_deltas_with_seam`, so the
        // failure is driven through the genuine dispatch path, not a mock.
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify Mid").await;
        let confidant = make_character(&svc, &project.project_id, "Bran").await;
        let keeper = make_character(&svc, &project.project_id, "Mara").await;

        // A plain fact we apply first (must survive).
        let first = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({"subject_table": "character", "predicate": "k", "value_text": "v"}),
        )
        .await;

        // A secret fact + a reveal delta that pre-flights clean.
        let secret = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project.project_id.clone(),
                scene_id: Some(scene_id.clone()),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".to_string()),
                subject_id: Some(keeper.clone()),
                predicate: Some("secret".to_string()),
                value_kind: Some("string".to_string()),
                value_text: Some("hidden".to_string()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
                secrecy: Some(SecrecyScope {
                    holder_ids: vec![keeper.clone()],
                    concealment_note: None,
                }),
            })
            .await
            .unwrap();
        let reveal = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "knowledge_learned",
            Some(&confidant),
            serde_json::json!({
                "character_id": confidant,
                "fact": "the secret",
                "source_summary": "told",
                "reader_visible": true,
                "secret_of_fact_id": secret.canonical_fact_id,
            }),
        )
        .await;
        let third = stage(
            &svc,
            &project.project_id,
            &scene_id,
            "canonical_fact",
            None,
            serde_json::json!({"subject_table": "character", "predicate": "t", "value_text": "3"}),
        )
        .await;

        let fact_to_clear = secret.canonical_fact_id.clone();
        let repo = svc.repository().clone();
        let out = svc
            .decide_canon_deltas_with_seam(
                DecideCanonDeltasInput {
                    project_id: project.project_id.clone(),
                    decisions: vec![
                        CanonDeltaDecisionInput {
                            delta_id: first.clone(),
                            action: "apply".to_string(),
                            edit: None,
                            note: None,
                        },
                        CanonDeltaDecisionInput {
                            delta_id: reveal.clone(),
                            action: "apply".to_string(),
                            edit: None,
                            note: None,
                        },
                        CanonDeltaDecisionInput {
                            delta_id: third.clone(),
                            action: "apply".to_string(),
                            edit: None,
                            note: None,
                        },
                    ],
                    decided_by: None,
                },
                move || {
                    let repo = repo.clone();
                    let fact_to_clear = fact_to_clear.clone();
                    async move {
                        repo.clear_canonical_fact_secret_for_test(&fact_to_clear)
                            .await
                            .unwrap();
                    }
                },
            )
            .await
            .unwrap();

        assert_eq!(out.applied_count, 1);
        assert_eq!(out.failed_count, 1);
        assert_eq!(out.results[0].outcome, "applied");
        assert_eq!(out.results[1].outcome, "failed");
        assert!(out.results[1].error.is_some());
        assert_eq!(out.results[2].outcome, "not_reached");

        // The first apply is real canon (not rolled back).
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let applied = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("applied"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].id, first);
        // The failed + not-reached rows stay staged.
        let staged = svc
            .repository()
            .list_canon_deltas(
                &project.project_id,
                &branch.id,
                Some("staged"),
                Some(&scene_id),
            )
            .await
            .unwrap();
        let staged_ids: Vec<_> = staged.iter().map(|d| d.id.clone()).collect();
        assert!(staged_ids.contains(&reveal));
        assert!(staged_ids.contains(&third));
    }

    #[tokio::test]
    async fn list_canon_deltas_resolves_chapter_range() {
        let (_tmp, svc) = fresh_service_local().await;
        let (project, scene_id, _) = project_with_scene(&svc, "Ratify List").await;
        // Stage two deltas on the ch.1 scene.
        for predicate in ["a", "b"] {
            stage(
                    &svc,
                    &project.project_id,
                    &scene_id,
                    "canonical_fact",
                    None,
                    serde_json::json!({"subject_table": "character", "predicate": predicate, "value_text": "x"}),
                )
                .await;
        }

        // By scene.
        let by_scene = svc
            .list_canon_deltas(ListCanonDeltasInput {
                project_id: project.project_id.clone(),
                status: None,
                scene_id: Some(scene_id.clone()),
                chapter_range: None,
            })
            .await
            .unwrap();
        assert_eq!(by_scene.deltas.len(), 2);

        // By chapter range covering chapter 1.
        let by_range = svc
            .list_canon_deltas(ListCanonDeltasInput {
                project_id: project.project_id.clone(),
                status: None,
                scene_id: None,
                chapter_range: Some(spindle_core::models::ChapterRange {
                    book_number: 1,
                    start: 1,
                    end: 1,
                }),
            })
            .await
            .unwrap();
        assert_eq!(by_range.deltas.len(), 2);

        // A range with no scenes yields nothing.
        let empty = svc
            .list_canon_deltas(ListCanonDeltasInput {
                project_id: project.project_id.clone(),
                status: None,
                scene_id: None,
                chapter_range: Some(spindle_core::models::ChapterRange {
                    book_number: 1,
                    start: 5,
                    end: 9,
                }),
            })
            .await
            .unwrap();
        assert!(empty.deltas.is_empty());

        // scene_id + chapter_range together is an input error.
        let err = svc
            .list_canon_deltas(ListCanonDeltasInput {
                project_id: project.project_id.clone(),
                status: None,
                scene_id: Some(scene_id.clone()),
                chapter_range: Some(spindle_core::models::ChapterRange {
                    book_number: 1,
                    start: 1,
                    end: 1,
                }),
            })
            .await;
        assert!(
            err.is_err(),
            "scene_id + chapter_range is mutually exclusive"
        );

        // Silence unused-import warnings for helpers not used in this arm.
        let _ = (
            CommitQuantityStateInput {
                project_id: String::new(),
                subject_table: String::new(),
                subject_id: String::new(),
                measure: String::new(),
                book_number: 0,
                chapter_number: 0,
                scene_order: 0,
                scene_id: None,
                state: QuantityState {
                    amount: None,
                    unit: None,
                    band: None,
                    change_reason: None,
                },
            },
            StatedConsequence {
                description: String::new(),
                stated_at: None,
                must_demonstrate_by: None,
                delivered: false,
            },
        );
    }
}
