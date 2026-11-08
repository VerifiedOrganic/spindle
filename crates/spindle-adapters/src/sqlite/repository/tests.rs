use super::*;
use spindle_core::models::ReaderContract;
use tempfile::TempDir;

async fn fresh_repo() -> (TempDir, Repository) {
    let tmp = TempDir::new().unwrap();
    let pool = SqlitePool::open(&tmp.path().join("repo.db")).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);
    (tmp, repo)
}

/// Project + branch + one persisted scene, for FK-valid canon_delta rows.
async fn repo_with_scene() -> (TempDir, Repository, Project, BibleBranch, String) {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};
    let (tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "P".into(),
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
    let (scene, _) = repo
        .save_scene_draft(
            &project.id,
            &branch.id,
            &SaveSceneDraftInput {
                project_id: project.id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "She turned away without a word.".into(),
                summary: "turn".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let scene_id = scene.id.clone();
    (tmp, repo, project, branch, scene_id)
}

fn stage_params(
    project: &Project,
    branch: &BibleBranch,
    scene_id: &str,
    class: &str,
    evidence: &str,
) -> StageCanonDeltaParams {
    StageCanonDeltaParams {
        project_id: project.id.clone(),
        branch_id: branch.id.clone(),
        scene_id: scene_id.to_string(),
        authoring_run_id: None,
        delta_class: class.to_string(),
        target_id: None,
        payload: serde_json::json!({ "note": "x" }),
        evidence: evidence.to_string(),
        confidence: "high".to_string(),
    }
}

// ── Plan amendments (ADR 0003 — living-outline replanning) ───────────────

/// Project + main branch, for FK-valid plan_amendment rows (no scene needed
/// — amendments have no scene FK).
async fn repo_with_project_branch() -> (TempDir, Repository, Project, BibleBranch) {
    let (tmp, repo) = fresh_repo().await;
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "P".into(),
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
    (tmp, repo, project, branch)
}

fn plan_stage_params(
    project: &Project,
    branch: &BibleBranch,
    class: &str,
    target_chapter: Option<i32>,
    rationale: &str,
) -> StagePlanAmendmentParams {
    StagePlanAmendmentParams {
        project_id: project.id.clone(),
        branch_id: branch.id.clone(),
        source_chapter: 3,
        book_number: 1,
        authoring_run_id: None,
        amendment_class: class.to_string(),
        target_chapter,
        payload: serde_json::json!({ "synopsis": "revised" }),
        rationale: rationale.to_string(),
        confidence: "high".to_string(),
    }
}

// === Authoring-run event journal (ADR 0002, V0027) ======================

/// Persist a minimal `active` authoring run so events have a valid FK
/// target. Returns the run id.
async fn seed_authoring_run(repo: &Repository, project: &Project, branch: &BibleBranch) -> String {
    let run_id = format!("authoring_run:{}", Ulid::new().to_string().to_lowercase());
    let now = chrono::Utc::now();
    let run = crate::sqlite::records::AuthoringRun {
        id: run_id.clone(),
        project_id: project.id.clone(),
        active_branch_id: branch.id.clone(),
        book_number: 1,
        start_chapter: 1,
        end_chapter: 1,
        checkpoint_interval: 1,
        last_checkpoint_end_chapter: 0,
        artifacts_dir: "../artifacts".into(),
        editorial_directives: Vec::new(),
        status: "active".into(),
        created_at: now,
        updated_at: now,
        mining_policy: None,
        max_revise_attempts: None,
        checkpoint_policy: None,
        replan_policy: None,
    };
    repo.save_authoring_run(run, Vec::new(), Vec::new(), Vec::new())
        .await
        .unwrap();
    run_id
}

#[path = "authoring/tests.rs"]
mod authoring;
#[path = "branches/tests.rs"]
mod branches;
#[path = "canon/tests.rs"]
mod canon;
#[path = "characters/tests.rs"]
mod characters;
#[path = "entities/tests.rs"]
mod entities;
#[path = "export/tests.rs"]
mod export;
#[path = "migrations/tests.rs"]
mod migrations;
#[path = "narrative/tests.rs"]
mod narrative;
#[path = "plans/tests.rs"]
mod plans;
#[path = "projects/tests.rs"]
mod projects;
#[path = "relationships/tests.rs"]
mod relationships;
#[path = "research/tests.rs"]
mod research;
#[path = "reviews/tests.rs"]
mod reviews;
#[path = "scenes/tests.rs"]
mod scenes;
#[path = "search/tests.rs"]
mod search;
#[path = "snapshots/tests.rs"]
mod snapshots;
#[path = "validation/tests.rs"]
mod validation;
#[path = "writer/tests.rs"]
mod writer;
