use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateCharacterInput,
    CreateProjectInput, CreateWorldRuleInput, PrepareSemanticContinuityInput, ReaderContract,
    RegisterCanonicalFactInput, SaveSceneDraftInput, SecrecyScope,
};
use spindle_core::semantic_continuity::{SemanticCandidateFamily, SemanticMode};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, SqliteSpindleService::new(repo))
}

async fn project(svc: &SqliteSpindleService, name: &str) -> String {
    svc.create_project(CreateProjectInput {
        name: name.into(),
        project_type: "novel".into(),
        genre: "fantasy".into(),
        reader_contract: ReaderContract {
            promise: "p".into(),
            style_notes: Vec::new(),
            boundaries: Vec::new(),
        },
    })
    .await
    .unwrap()
    .project_id
}

async fn character(svc: &SqliteSpindleService, project_id: &str, name: &str) -> String {
    svc.create_character(CreateCharacterInput {
        project_id: project_id.into(),
        name: name.into(),
        summary: "a test character".into(),
        role: "supporting".into(),
        realm: None,
        aliases: Vec::new(),
        voice_profile: Default::default(),
        emotional_profile: Default::default(),
        initial_state: None,
    })
    .await
    .unwrap()
    .character_id
}

async fn scene(svc: &SqliteSpindleService, project_id: &str, order: i32, text: &str) -> String {
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.into(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: order,
        full_text: text.into(),
        summary: "test scene".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap()
    .scene_id
}

#[tokio::test]
async fn secret_candidates_require_an_out_of_circle_character() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Secret candidates").await;
    let mara = character(&svc, &project_id, "Mara").await;
    let bran = character(&svc, &project_id, "Bran").await;
    let scene_id = scene(
        &svc,
        &project_id,
        1,
        "Mara held the moon key while Bran watched from the doorway.",
    )
    .await;

    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: project_id.clone(),
        scene_id: Some(scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: None,
        key: None,
        value: None,
        context: None,
        subject_table: Some("character".into()),
        subject_id: Some(mara.clone()),
        predicate: Some("hidden_key".into()),
        value_kind: Some("string".into()),
        value_text: Some("moon key".into()),
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
            holder_ids: vec![mara.clone()],
            concealment_note: Some("Mara never speaks of the key.".into()),
        }),
    })
    .await
    .unwrap();

    let prepared = svc
        .prepare_semantic_continuity(PrepareSemanticContinuityInput {
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            subjects: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(prepared.requests.len(), 1);
    let candidates = &prepared.requests[0].candidate_set.candidates;
    let secret = candidates
        .iter()
        .find(|candidate| candidate.family == SemanticCandidateFamily::Secret)
        .expect("out-of-circle Bran must produce a secret candidate");
    assert!(secret.subject_ids.contains(&bran));
    assert!(!secret.subject_ids.contains(&mara));
    assert_eq!(
        &prepared.requests[0].candidate_set.scene_id, &scene_id,
        "request must target the prepared scene"
    );

    // A second project scene with only the holder still sees the canon fact,
    // but must not produce a secret candidate.
    let _holder_only = scene(
        &svc,
        &project_id,
        2,
        "Mara turned the moon key over in her palm and said nothing.",
    )
    .await;
    let prepared = svc
        .prepare_semantic_continuity(PrepareSemanticContinuityInput {
            project_id,
            scope: ConsistencyScopeInput::full(),
            subjects: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(prepared.requests.len(), 2);
    assert!(
        prepared.requests[1]
            .candidate_set
            .candidates
            .iter()
            .all(|candidate| candidate.family != SemanticCandidateFamily::Secret),
        "holder-only scene must not emit a secret candidate"
    );
}

#[tokio::test]
async fn prepare_candidates_world_rules_and_keeps_modes_finding_neutral() {
    let (_tmp, svc) = fresh_service().await;
    let project_id = project(&svc, "Prepare candidates").await;
    let mara = character(&svc, &project_id, "Mara").await;
    let _ = scene(
        &svc,
        &project_id,
        1,
        "Mara crossed the yard. The moon key would not turn.",
    )
    .await;
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project_id.clone(),
        rule_name: "moon-key".into(),
        rule_type: "magic".into(),
        description: "The moon key cannot turn during daylight.".into(),
        scan_pattern: Some("moon key".into()),
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    let prepared = svc
        .prepare_semantic_continuity(PrepareSemanticContinuityInput {
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            subjects: vec![mara.clone()],
        })
        .await
        .unwrap();
    assert_eq!(prepared.coverage.scene_count, 1);
    assert_eq!(prepared.coverage.candidate_scene_count, 1);
    assert!(prepared.coverage.candidate_count >= 1);
    assert!(
        prepared.requests[0]
            .candidate_set
            .candidates
            .iter()
            .any(|candidate| {
                candidate.family == SemanticCandidateFamily::WorldRule
                    && candidate
                        .evidence_hints
                        .iter()
                        .any(|hint| hint.exact_quote == "moon key")
            })
    );

    let off = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    let shadow = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: Some(SemanticMode::Shadow),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    let on = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: Some(SemanticMode::On),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&off.issues).unwrap(),
        serde_json::to_value(&shadow.issues).unwrap(),
        "P2 shadow must not change findings"
    );
    assert_eq!(
        serde_json::to_value(&off.issues).unwrap(),
        serde_json::to_value(&on.issues).unwrap(),
        "P2 on must not change findings yet"
    );
    assert!(
        off.audit_coverage
            .iter()
            .all(|coverage| coverage.check_type != "semantic_continuity")
    );
    let semantic = shadow
        .audit_coverage
        .iter()
        .find(|coverage| coverage.check_type == "semantic_continuity")
        .expect("shadow mode must record semantic candidate coverage");
    assert_eq!(semantic.eligible, 1);
    assert_eq!(semantic.evaluated_ids.len(), 1);
    assert!(semantic.not_evaluated_ids.is_empty());
}
