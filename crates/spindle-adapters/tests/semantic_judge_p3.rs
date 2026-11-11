use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateProjectInput,
    CreateWorldRuleInput, PrepareSemanticContinuityInput, ReaderContract, SaveSceneDraftInput,
};
use spindle_core::semantic_continuity::{
    SEMANTIC_CONTRACT_VERSION, SemanticCacheKeyV1, SemanticErrorKind, SemanticHostReportV1,
    SemanticJudgeErrorV1, SemanticJudgmentV1, SemanticMode, SemanticVerdict,
    semantic_candidate_set_hash,
};
use tempfile::TempDir;

async fn fresh_service() -> (TempDir, SqliteSpindleService) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, SqliteSpindleService::new(repo))
}

async fn seeded_scene() -> (TempDir, SqliteSpindleService, String) {
    let (tmp, svc) = fresh_service().await;
    let project_id = svc
        .create_project(CreateProjectInput {
            name: "Semantic judge".into(),
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
        .project_id;
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project_id.clone(),
        rule_name: "moon-key".into(),
        rule_type: "magic".into(),
        description: "The moon key cannot turn in daylight.".into(),
        scan_pattern: Some("moon key".into()),
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara turned the moon key in daylight.".into(),
        summary: "test".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    (tmp, svc, project_id)
}

fn host_report(
    svc_request: &spindle_core::semantic_continuity::SemanticHostRequestV1,
) -> SemanticHostReportV1 {
    let candidate = &svc_request.candidate_set.candidates[0];
    let quote = "moon key";
    let start = "Mara turned the moon key in daylight.".find(quote).unwrap();
    let end = start + quote.len();
    SemanticHostReportV1 {
        request_id: svc_request.request_id.clone(),
        candidate_set_hash: semantic_candidate_set_hash(&svc_request.candidate_set),
        scene_sha256: svc_request.candidate_set.scene_sha256.clone(),
        canon_version: svc_request.candidate_set.canon_version.clone(),
        judgments: vec![SemanticJudgmentV1 {
            candidate_id: candidate.candidate_id.clone(),
            verdict: SemanticVerdict::Contradiction,
            canon_refs: candidate.canon_refs.clone(),
            evidence: Some(spindle_core::semantic_continuity::SemanticEvidenceSpanV1 {
                scene_id: svc_request.candidate_set.scene_id.clone(),
                scene_sha256: svc_request.candidate_set.scene_sha256.clone(),
                start_byte: start,
                end_byte: end,
                exact_quote: quote.into(),
            }),
            confidence: 0.9,
            rationale: "the rule is contradicted".into(),
        }],
    }
}

#[tokio::test]
async fn host_report_emits_in_on_mode_and_shadow_counts_without_findings() {
    let (_tmp, svc, project_id) = seeded_scene().await;
    let prepared = svc
        .prepare_semantic_continuity(PrepareSemanticContinuityInput {
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            subjects: Vec::new(),
        })
        .await
        .unwrap();
    let report = host_report(&prepared.requests[0]);
    let on = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: Some(SemanticMode::On),
            semantic_host_reports: vec![report.clone()],
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(on.issues.iter().any(|issue| {
        issue.check_type == "world_rule_semantic_drift"
            && issue.message.contains("(semantic)")
            && issue
                .evidence
                .as_ref()
                .is_some_and(|span| span.exact_quote == "moon key")
    }));
    let summary = on.semantic_summary.expect("summary");
    assert_eq!(summary.warnings, 1);
    assert_eq!(summary.model_calls, 0);

    let shadow = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id,
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: Some(SemanticMode::Shadow),
            semantic_host_reports: vec![report],
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    assert!(
        shadow
            .issues
            .iter()
            .all(|issue| !issue.message.contains("(semantic)"))
    );
    assert_eq!(shadow.semantic_summary.expect("shadow summary").warnings, 1);
}

#[tokio::test]
async fn stale_host_report_and_local_stub_fail_closed() {
    let (_tmp, svc, project_id) = seeded_scene().await;
    let prepared = svc
        .prepare_semantic_continuity(PrepareSemanticContinuityInput {
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            subjects: Vec::new(),
        })
        .await
        .unwrap();
    let mut stale = host_report(&prepared.requests[0]);
    stale.request_id = "stale".into();
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id: project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: Some(SemanticMode::On),
            semantic_host_reports: vec![stale],
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    let summary = out.semantic_summary.expect("summary");
    assert!(
        summary
            .errors
            .iter()
            .any(|error| { error.kind == SemanticErrorKind::StaleHostRequest })
    );
    assert!(
        out.issues
            .iter()
            .all(|issue| !issue.message.contains("(semantic)"))
    );

    // No host report and no real route: local stub must fail closed rather
    // than being accepted as a judge.
    let out = svc
        .check_consistency(CheckConsistencyInput {
            deep_scan_offset: None,
            project_id,
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: Some(false),
            semantic_mode: Some(SemanticMode::On),
            semantic_host_reports: Vec::new(),
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();
    let summary = out.semantic_summary.expect("summary");
    assert!(
        summary
            .errors
            .iter()
            .any(|error| { error.kind == SemanticErrorKind::RouteUnavailable })
    );
    assert!(
        out.issues
            .iter()
            .all(|issue| !issue.message.contains("(semantic)"))
    );
}

#[tokio::test]
async fn cache_key_is_scene_canon_candidate_route_and_model_specific() {
    let (_tmp, svc, project_id) = seeded_scene().await;
    let prepared = svc
        .prepare_semantic_continuity(PrepareSemanticContinuityInput {
            project_id,
            scope: ConsistencyScopeInput::full(),
            subjects: Vec::new(),
        })
        .await
        .unwrap();
    let key = SemanticCacheKeyV1 {
        scene_id: prepared.requests[0].candidate_set.scene_id.clone(),
        scene_sha256: "scene-a".into(),
        canon_version: "canon-a".into(),
        candidate_set_hash: "cand-a".into(),
        contract_version: SEMANTIC_CONTRACT_VERSION.into(),
        route: "review".into(),
        model_identity: "model-a".into(),
    };
    svc.repository()
        .put_semantic_continuity_cache(&key, "{\"ok\":true}")
        .await
        .unwrap();
    assert_eq!(
        svc.repository()
            .get_semantic_continuity_cache(&key)
            .await
            .unwrap()
            .as_deref(),
        Some("{\"ok\":true}")
    );
    let mut changed = key.clone();
    changed.canon_version = "canon-b".into();
    assert!(
        svc.repository()
            .get_semantic_continuity_cache(&changed)
            .await
            .unwrap()
            .is_none()
    );
    svc.repository()
        .delete_semantic_continuity_cache_entry(&key)
        .await
        .unwrap();
    assert!(
        svc.repository()
            .get_semantic_continuity_cache(&key)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn typed_judge_errors_are_serializable_and_do_not_name_prose() {
    let error = SemanticJudgeErrorV1::new(
        SemanticErrorKind::RatingDenied,
        "route is not cleared for this rating",
    )
    .with_route("review")
    .with_rating("explicit");
    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(json["kind"], "rating_denied");
    assert!(!json.to_string().contains("scene"));
}
