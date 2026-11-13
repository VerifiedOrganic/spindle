//! Regression coverage for detached deep `check_consistency` jobs.
//!
//! The bug: a deep audit can run for tens of minutes without any MCP progress,
//! so a client timeout discarded a result the server had already paid for.
//! These tests prove the start/join/collect contract, the per-stage progress
//! stream, and the documented restart behaviour (a new service instance has an
//! empty registry and starts a fresh job).

use super::*;
use crate::ai::ModelRouter;
use crate::sqlite::{Repository, SqlitePool};
use spindle_core::models::{
    CheckConsistencyInput, ConsistencyScopeInput, ConsistencyStatus, ContentRating,
    CreateProjectInput, ReaderContract, SaveSceneDraftInput,
};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const DEEP_PROSE: &str = "He sat at the long table and turned the matter over and over in his \
                          mind, the figures refusing to balance. MOCK_TEMPORAL_JUMP. The lamps were \
                          lit and the room had gone cold around him, the cup at his elbow holding \
                          nothing but a skin of dregs.";

async fn service_with_scene(name: &str) -> (TempDir, SqliteSpindleService, String) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("deep-job.db"))
        .await
        .unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
    let svc = SqliteSpindleService::new(repo);
    let project = svc
        .create_project(CreateProjectInput {
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
        .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: DEEP_PROSE.to_string(),
        summary: "a quiet accounting".to_string(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    (tmp, svc, project.project_id)
}

fn deep_input(project_id: &str) -> CheckConsistencyInput {
    CheckConsistencyInput {
        deep_scan_offset: None,
        project_id: project_id.to_string(),
        scope: ConsistencyScopeInput::full(),
        checks: vec!["temporal_coherence".to_string()],
        severity_filter: Vec::new(),
        deep_check: Some(true),
        semantic_mode: None,
        semantic_host_reports: Vec::new(),
        subjects: Vec::new(),
        format: None,
        budget_tokens: None,
    }
}

#[tokio::test]
async fn deep_check_starts_joins_and_collects_one_detached_job() {
    let (_tmp, mut svc, project_id) = service_with_scene("deep-job").await;
    svc.set_consistency_job_join_wait(Duration::from_millis(50));
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    svc.set_consistency_job_gate(Some(gate.clone()));

    // Start: the job is blocked before its first check, so the caller gets a
    // prompt in-progress response instead of silence.
    let first = svc
        .check_consistency(deep_input(&project_id))
        .await
        .unwrap();
    assert_eq!(first.status, ConsistencyStatus::InProgress);
    let job_id = first.job_id.clone().expect("in-progress job id");
    assert!(job_id.starts_with("consistency-"));
    assert!(first.checks_completed.is_empty());
    assert_eq!(first.checks_pending, vec!["temporal_coherence.deep"]);
    assert!(first.issues.is_empty());

    // Join: a second call over the same scope and text returns the SAME job,
    // not a duplicate run.
    let second = svc
        .check_consistency(deep_input(&project_id))
        .await
        .unwrap();
    assert_eq!(second.status, ConsistencyStatus::InProgress);
    assert_eq!(second.job_id.as_deref(), Some(job_id.as_str()));

    // Release the job and collect its finished audit.
    gate.add_permits(1);
    let deadline = Instant::now() + Duration::from_secs(10);
    let collected = loop {
        let output = svc
            .check_consistency(deep_input(&project_id))
            .await
            .unwrap();
        if output.status == ConsistencyStatus::Complete {
            break output;
        }
        assert!(
            Instant::now() < deadline,
            "detached job did not finish: {output:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(collected.job_id.as_deref(), Some(job_id.as_str()));
    assert!(
        collected
            .checks_completed
            .iter()
            .any(|check| check == "temporal_coherence.deep")
    );
    assert!(collected.checks_pending.is_empty());
    assert!(
        collected
            .issues
            .iter()
            .any(|issue| issue.check_type == "temporal_coherence"),
        "the finished audit must carry the deep-tier finding: {:?}",
        collected.issues
    );

    // Collect: a later retry returns the finished result without re-running.
    let again = svc
        .check_consistency(deep_input(&project_id))
        .await
        .unwrap();
    assert_eq!(again.status, ConsistencyStatus::Complete);
    assert_eq!(again.job_id.as_deref(), Some(job_id.as_str()));
    assert_eq!(again.issues.len(), collected.issues.len());
}

#[tokio::test]
async fn progress_sink_receives_a_completion_event_per_deep_check() {
    let (_tmp, svc, project_id) = service_with_scene("deep-progress").await;
    let (sink, mut events) = tokio::sync::mpsc::unbounded_channel();

    let output = svc
        .check_consistency_with_progress(deep_input(&project_id), Some(sink))
        .await
        .unwrap();
    assert_eq!(output.status, ConsistencyStatus::Complete);
    let job_id = output.job_id.clone().expect("job id");

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        assert_eq!(event.job_id, job_id);
        seen.push(event);
    }
    assert!(
        !seen.is_empty(),
        "a deep check with a progress token must emit progress events"
    );
    let last = seen.last().unwrap();
    assert!(
        last.checks_completed
            .iter()
            .any(|check| check == "temporal_coherence.deep"),
        "progress must name the finished deep check: {seen:?}"
    );
    assert!(last.checks_pending.is_empty());
    assert!(last.issue_count >= 1);
}

#[tokio::test]
async fn fast_checks_and_deep_checks_without_deep_tiers_stay_synchronous() {
    let (_tmp, svc, project_id) = service_with_scene("fast-job").await;

    let mut fast = deep_input(&project_id);
    fast.deep_check = Some(false);
    let output = svc.check_consistency(fast).await.unwrap();
    assert_eq!(output.status, ConsistencyStatus::Complete);
    assert!(output.job_id.is_none());
    assert!(output.checks_completed.is_empty());
    assert!(output.checks_pending.is_empty());

    let mut no_deep_tier = deep_input(&project_id);
    no_deep_tier.deep_check = Some(true);
    no_deep_tier.checks = vec!["scene_spine_integrity".to_string()];
    let output = svc.check_consistency(no_deep_tier).await.unwrap();
    assert_eq!(output.status, ConsistencyStatus::Complete);
    assert!(
        output.job_id.is_none(),
        "no detached job exists when the request asks for no deep-tier check"
    );
}

#[tokio::test]
async fn a_new_service_instance_does_not_join_the_old_job() {
    let (_tmp, mut first, project_id) = service_with_scene("restart-job").await;
    first.set_consistency_job_join_wait(Duration::from_millis(50));
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    first.set_consistency_job_gate(Some(gate.clone()));

    let before_restart = first
        .check_consistency(deep_input(&project_id))
        .await
        .unwrap();
    assert_eq!(before_restart.status, ConsistencyStatus::InProgress);
    let old_job_id = before_restart.job_id.clone().expect("old job id");

    // A fresh service over the same repository models a server restart: the
    // in-memory registry is empty, so the retry starts its own job.
    let mut restarted = SqliteSpindleService::new(first.repository().clone());
    restarted.set_consistency_job_join_wait(Duration::from_secs(10));
    let after_restart = restarted
        .check_consistency(deep_input(&project_id))
        .await
        .unwrap();
    assert_eq!(after_restart.status, ConsistencyStatus::Complete);
    let new_job_id = after_restart.job_id.clone().expect("new job id");
    assert_ne!(
        new_job_id, old_job_id,
        "a restart must not resurrect the in-memory job"
    );

    // Release the old job so the test leaves no task blocked on the gate.
    gate.add_permits(1);
}
