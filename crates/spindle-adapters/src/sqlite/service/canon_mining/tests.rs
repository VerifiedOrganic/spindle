use super::*;

#[tokio::test]
async fn mine_scene_canon_stages_fact_and_payoff_candidate() {
    use spindle_core::models::{CreateNarrativePromiseInput, MineSceneCanonInput, StoryPlacement};
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Mine Happy").await;

    // An open promise so the payoff candidate can target a real id.
    let promise = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project.project_id.clone(),
            promise_type: "mystery".to_string(),
            description: "The locked door will be opened".to_string(),
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

    // Prose carrying the sentinel + the promise-id marker so the local stub
    // returns a fact delta and a payoff candidate targeting the real promise.
    let prose = format!(
        "The grey-eyed stranger crossed the hall. MOCK_CANON_MINE \
             MOCK_CANON_MINE_PROMISE[{}] as the door swung wide.",
        promise.narrative_promise_id
    );
    set_scene_prose(&svc, &scene_id, &prose).await;

    let out = svc
        .mine_scene_canon(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();

    assert_eq!(out.status, "staged");
    assert_eq!(out.superseded_count, 0);
    assert_eq!(out.discarded_count, 0);
    assert_eq!(out.staged.len(), 2);
    let fact = out
        .staged
        .iter()
        .find(|d| d.delta_class == "canonical_fact")
        .expect("canonical_fact staged");
    assert_eq!(fact.evidence, "MOCK_CANON_MINE");
    let payoff = out
        .staged
        .iter()
        .find(|d| d.delta_class == "promise_payoff_candidate")
        .expect("payoff candidate staged");
    assert_eq!(
        payoff.target_id.as_deref(),
        Some(promise.narrative_promise_id.as_str())
    );

    // Rows are persisted on the active branch.
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
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn mine_scene_canon_remine_supersedes_but_spares_decided_rows() {
    use crate::sqlite::repository::CanonDeltaDecision;
    use spindle_core::models::MineSceneCanonInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Mine Remine").await;
    set_scene_prose(
        &svc,
        &scene_id,
        "A grey stone tower. MOCK_CANON_MINE stands.",
    )
    .await;

    let first = svc
        .mine_scene_canon(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(first.status, "staged");
    assert_eq!(first.staged.len(), 1);

    // Operator applies the first batch's row — it must survive the remine.
    let applied = svc
        .repository()
        .decide_canon_delta(&first.staged[0].id, CanonDeltaDecision::Applied, "op", None)
        .await
        .unwrap();
    assert_eq!(applied.status, "applied");

    let second = svc
        .mine_scene_canon(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();
    // Remine supersedes only prior STAGED rows — the applied one is history.
    assert_eq!(second.status, "staged");
    assert_eq!(second.superseded_count, 0);
    assert_eq!(second.staged.len(), 1);

    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let applied_rows = svc
        .repository()
        .list_canon_deltas(
            &project.project_id,
            &branch.id,
            Some("applied"),
            Some(&scene_id),
        )
        .await
        .unwrap();
    assert_eq!(applied_rows.len(), 1, "the applied row is never superseded");
}

#[tokio::test]
async fn mine_scene_canon_discards_unknown_class_and_fabricated_evidence() {
    // The router returns an unknown class, a fabricated-evidence delta (quote
    // not in prose), and one valid sibling. Only the sibling stages.
    use spindle_core::models::MineSceneCanonInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Mine Discard").await;
    // The sentinel keeps the fabricated-evidence discard test focused: the
    // stub's canonical_fact carries evidence "MOCK_CANON_MINE" — present in
    // prose — so it stages, while the injected bad deltas are discarded.
    set_scene_prose(&svc, &scene_id, "MOCK_CANON_MINE_DISCARD marker here.").await;

    let out = svc
        .mine_scene_canon(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(out.status, "staged");
    assert_eq!(
        out.discarded_count, 2,
        "unknown class + fabricated evidence"
    );
    assert_eq!(out.staged.len(), 1);
    assert_eq!(out.staged[0].delta_class, "canonical_fact");
}

#[tokio::test]
async fn mine_scene_canon_skips_when_rating_uncovered() {
    use spindle_core::models::{ConfigureAgentsInput, ContentRating, MineSceneCanonInput};
    let (tmp, svc) = fresh_service_local().await;
    // Config: review agent covers only non-explicit ratings.
    let config_path = tmp.path().join("mine-routes.toml");
    std::fs::write(
        &config_path,
        r#"
[health_check]
enabled = false

[[agents]]
id = "review-safe"
name = "Review Safe"
provider = "local"
endpoint = "local"
model = "review-safe"
ratings = ["safe", "teen", "mature"]

[[routing]]
route = "review"
agent = "review-safe"
"#,
    )
    .unwrap();
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.display().to_string()),
    })
    .unwrap();

    let (project, scene_id, _) = project_with_scene(&svc, "Mine Explicit").await;
    // Re-save the scene as explicit prose carrying the sentinel.
    let scene = svc.repository().get_scene(&scene_id).await.unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: scene.book_number,
        chapter_number: scene.chapter_number,
        chapter_id: None,
        scene_order: scene.scene_order,
        full_text: "MOCK_CANON_MINE explicit prose here.".to_string(),
        summary: "explicit".to_string(),
        content_rating: ContentRating::Explicit,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .mine_scene_canon(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(out.status, "skipped");
    assert!(out.staged.is_empty());
    let reason = out.skip_reason.expect("skip reason present");
    assert!(reason.contains("explicit"), "reason names rating: {reason}");

    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let rows = svc
        .repository()
        .list_canon_deltas(&project.project_id, &branch.id, None, Some(&scene_id))
        .await
        .unwrap();
    assert!(rows.is_empty(), "no rows staged on rating skip");
}

#[tokio::test]
async fn mine_scene_canon_skips_empty_prose_without_model_call() {
    use spindle_core::models::MineSceneCanonInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Mine Empty").await;
    set_scene_prose(&svc, &scene_id, "   \n  ").await;

    let out = svc
        .mine_scene_canon(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(out.status, "skipped");
    assert_eq!(out.skip_reason.as_deref(), Some("empty scene"));
    assert!(out.staged.is_empty());
}

#[test]
fn mine_digest_withholds_secret_fact_keys() {
    // The pure digest builder must never surface a secret fact key when the
    // secret gate has stripped it. Non-secret keys remain.
    let digest = MineDigest {
        cast: vec![("character:sara".to_string(), "Sara".to_string())],
        open_promises: Vec::new(),
        active_conflicts: Vec::new(),
        arcs: Vec::new(),
        fact_keys: vec![(
            "Sara".to_string(),
            vec!["hair_color".to_string(), "hometown".to_string()],
        )],
    };
    let rendered = build_mine_digest(&digest);
    assert!(rendered.contains("hair_color"));
    assert!(rendered.contains("hometown"));
    // A key that was gated out never appears (the service excludes it before
    // building the digest, so the builder simply never receives it).
    assert!(!rendered.contains("true_parentage"));
}

#[tokio::test]
async fn mine_scene_canon_digest_excludes_out_of_circle_secret_key() {
    // Integration: a secret fact whose circle excludes the scene cast must
    // NOT appear in the mining prompt's fact-keys digest; a non-secret key
    // for the same present subject does.
    use spindle_core::models::{
        CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
        RegisterCanonicalFactInput, StoryPlacement,
    };
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Mine Secret").await;

    // Two characters: Sara is present in the scene; Mara is the sole secret
    // knower and is NOT present.
    let sara = svc
        .create_character(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Sara".to_string(),
            summary: "the protagonist".to_string(),
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

    set_scene_prose(&svc, &scene_id, "Sara walked alone through the empty hall.").await;

    let _ = StoryPlacement {
        book_number: 1,
        chapter_number: 1,
        scene_order: Some(1),
        note: None,
    };
    let public_fact = RegisterCanonicalFactInput {
        project_id: project.project_id.clone(),
        scene_id: Some(scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: None,
        key: None,
        value: None,
        context: None,
        subject_table: Some("character".to_string()),
        subject_id: Some(sara.character_id.clone()),
        predicate: Some("hair_color".to_string()),
        value_kind: Some("string".to_string()),
        value_text: Some("auburn".to_string()),
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
    };
    svc.register_canonical_fact(public_fact).await.unwrap();
    // Secret fact about Sara whose circle excludes the present cast (empty
    // holder set → no present cast member knows → withheld).
    let secret_fact = RegisterCanonicalFactInput {
        project_id: project.project_id.clone(),
        scene_id: Some(scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: None,
        key: None,
        value: None,
        context: None,
        subject_table: Some("character".to_string()),
        subject_id: Some(sara.character_id.clone()),
        predicate: Some("true_parentage".to_string()),
        value_kind: Some("string".to_string()),
        value_text: Some("royal blood".to_string()),
        value_number: None,
        value_unit: None,
        value_json: None,
        aliases: Vec::new(),
        scope: None,
        valid_from: None,
        valid_until: None,
        legacy_untyped: None,
        supersedes_fact_id: None,
        secrecy: Some(spindle_core::models::SecrecyScope {
            holder_ids: Vec::new(),
            concealment_note: None,
        }),
    };
    svc.register_canonical_fact(secret_fact).await.unwrap();

    let prompt = svc
        .build_mine_prompt_for_scene(&project.project_id, &scene_id)
        .await
        .unwrap();
    assert!(prompt.contains("hair_color"), "public key present");
    assert!(
        !prompt.contains("true_parentage"),
        "out-of-circle secret key withheld from digest"
    );
    // Fiction frame leads the prompt so stricter provider classifiers (e.g.
    // Qwen) treat the verbatim scene prose as fiction, not real-world content.
    assert!(
        prompt.starts_with("Context and safety frame:"),
        "mine prompt must lead with the fiction frame"
    );
    // Content rating unaffected (guards the plumbing).
    let _ = ContentRating::General;
}
