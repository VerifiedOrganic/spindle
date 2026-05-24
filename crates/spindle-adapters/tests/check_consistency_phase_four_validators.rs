//! Integration test for Gap 1: the Phase-4 validator suite must produce
//! real findings through `check_consistency`. Before the port the
//! suite was stubbed out and `report_sections` came back empty; this
//! test seeds a world rule violation and asserts the corresponding
//! `world_rule_semantic_drift` finding lands in the response.

use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{
    CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateProjectInput,
    CreateWorldRuleInput, ReaderContract, SaveSceneDraftInput,
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

#[tokio::test]
async fn check_consistency_emits_phase_four_validator_findings() {
    let (_tmp, svc) = fresh_service().await;

    let proj = svc
        .create_project(CreateProjectInput {
            name: "Validators".into(),
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

    svc.create_world_rule(CreateWorldRuleInput {
        project_id: proj.project_id.clone(),
        rule_name: "no-resurrection".into(),
        rule_type: "magic".into(),
        description: "Resurrection magic is impossible in this world.".into(),
        scan_pattern: Some("resurrect".into()),
        relevance_tags: Vec::new(),
        established_in: None,
    })
    .await
    .unwrap();

    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "Mara watched her brother resurrect from the ashes, breath returning.".into(),
        summary: "violation".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
    })
    .await
    .unwrap();

    let out = svc
        .check_consistency(CheckConsistencyInput {
            project_id: proj.project_id.clone(),
            scope: ConsistencyScopeInput::full(),
            checks: Vec::new(),
            severity_filter: Vec::new(),
            deep_check: None,
            subjects: Vec::new(),
            format: None,
            budget_tokens: None,
        })
        .await
        .unwrap();

    // Phase-4 fan-out is no longer gated: the world_rule_semantic_drift
    // validator must surface the pattern hit in the scene prose.
    assert!(
        out.issues
            .iter()
            .any(|i| i.check_type == "world_rule_semantic_drift"),
        "expected world_rule_semantic_drift finding, got: {:?}",
        out.issues
            .iter()
            .map(|i| i.check_type.clone())
            .collect::<Vec<_>>()
    );
    // report_sections is populated, not empty.
    assert!(
        !out.report_sections.is_empty(),
        "report_sections must be populated by the Phase-4 fan-out"
    );
}
