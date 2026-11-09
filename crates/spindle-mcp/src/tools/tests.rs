use tempfile::tempdir;
use tokio::time::{Duration, timeout};

use spindle_adapters::ModelRouter;
use spindle_adapters::SqlitePool;
use spindle_adapters::sqlite::Repository as SpindleRepository;

use super::*;

fn structured_json(result: CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(false));
    result.structured_content.expect("structured content")
}

async fn router() -> ToolRouter {
    let temp = tempdir().expect("temp dir");
    let db = SqlitePool::open(&temp.path().join("router.db"))
        .await
        .expect("db init");
    let data_dir = temp.keep();
    ToolRouter::with_tool_profile_and_serialization(
        SpindleService::new(SpindleRepository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        )),
        None,
        Arc::new(ToolSerializationState::default()),
    )
}

/// A fresh repository + a persisted `active` authoring run, for driving the
/// journal emitter directly (deterministic step-event trace tests).
async fn repo_with_run() -> (tempfile::TempDir, SpindleRepository, String) {
    use spindle_core::models::{CreateProjectInput, ReaderContract};
    let temp = tempdir().expect("temp dir");
    let db = SqlitePool::open(&temp.path().join("trace.db"))
        .await
        .expect("db init");
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = SpindleRepository::with_model_router(db, data_dir, ModelRouter::local_only());
    let (project, branch, _book, _chapter) = repo
        .create_project(&CreateProjectInput {
            name: "Trace".into(),
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
    let run_id = format!(
        "authoring_run:{}",
        ulid::Ulid::new().to_string().to_lowercase()
    );
    let now = chrono::Utc::now();
    repo.save_authoring_run(
        spindle_adapters::sqlite::records::AuthoringRun {
            id: run_id.clone(),
            project_id: project.id.clone(),
            active_branch_id: branch.id,
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
            mining_policy: Some("propose_all".into()),
            max_revise_attempts: Some(1),
            checkpoint_policy: None,
            replan_policy: None,
        },
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .await
    .unwrap();
    (temp, repo, run_id)
}

/// Build a single-scene [`HarnessState`] snapshot with the given per-scene
/// status fields, for driving `authoring_emit_step_events`.
fn trace_state(
    scene_id: &str,
    phase: ScenePhase,
    verify_status: Option<&str>,
    verify_detail: Option<&str>,
    mine_status: Option<&str>,
    mine_detail: Option<&str>,
    revise_attempts: i32,
) -> HarnessState {
    use spindle_harness::state::{ChapterState, ChapterStatus, SceneState};
    let scene = SceneState {
        scene_order: 1,
        location_id: "location:x".into(),
        phase,
        scene_id: Some(scene_id.into()),
        verify_status: verify_status.map(str::to_string),
        verify_detail: verify_detail.map(str::to_string),
        mine_status: mine_status.map(str::to_string),
        mine_detail: mine_detail.map(str::to_string),
        revise_attempts,
        ..SceneState::default()
    };
    let mut state = HarnessState::from_seed(
        spindle_harness::state::HarnessSeed {
            project_id: "project:x".into(),
            book_number: 1,
            range: spindle_harness::state::ChapterRange {
                start_chapter: 1,
                end_chapter: 1,
            },
            checkpoint_interval: 1,
            editorial_directives: Vec::new(),
            mining_policy: None,
            max_revise_attempts: None,
            checkpoint_policy: None,
            replan_policy: None,
            editorial: None,
            editorial_controller_path: None,
            chapters: Vec::new(),
        },
        "bible_branch:x".into(),
    );
    state.chapters = vec![ChapterState {
        chapter_number: 1,
        planned: true,
        synopsis: "s".into(),
        pov_character_id: None,
        status: ChapterStatus::InProgress,
        scenes: vec![scene],
        summary_saved: false,
        summary_artifact_path: None,
        replan_status: None,
        replan_detail: None,
    }];
    state
}

/// Recursively collect every `format` string value present anywhere in a
/// schema JSON tree, tagged with the JSON-pointer-ish path where it lives.
/// Used by the interop-sweep tests to name offenders precisely.
fn collect_null_enum_paths(value: &Value, path: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("enum")
                && items.iter().any(Value::is_null)
            {
                out.push(format!("{path}/enum"));
            }
            for (key, child) in map {
                collect_null_enum_paths(child, &format!("{path}/{key}"), out);
            }
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                collect_null_enum_paths(item, &format!("{path}[{idx}]"), out);
            }
        }
        _ => {}
    }
}

fn collect_format_values(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "format"
                    && let Value::String(s) = v
                {
                    out.push((format!("{path}/format"), s.clone()));
                }
                collect_format_values(v, &format!("{path}/{k}"), out);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                collect_format_values(v, &format!("{path}/{i}"), out);
            }
        }
        _ => {}
    }
}

/// The denylist of schemars' non-standard numeric `format` values that
/// strict JSON-Schema clients (Kimi Code et al.) warn about.
fn is_denylisted_numeric_format(s: &str) -> bool {
    matches!(s, "float" | "double")
        || (s
            .strip_prefix("int")
            .or_else(|| s.strip_prefix("uint"))
            .is_some_and(|rest| {
                // `int`/`uint` alone, or followed only by digits (int8..64, uint8..64)
                rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit())
            })
            && (s.starts_with("int") || s.starts_with("uint")))
}

async fn prepare_run_project(router: &ToolRouter, name: &str) -> String {
    let args = serde_json::to_value(CreateProjectInput {
        name: name.to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "Chapter overflow regression.".to_string(),
            style_notes: vec![],
            boundaries: vec![],
        },
    })
    .expect("project args");
    let project: CreateProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_project", Some(args.as_object().unwrap()))
            .await
            .expect("create project"),
    ))
    .expect("decode project");
    project.project_id
}

fn has_range_message(missing: &[serde_json::Value]) -> bool {
    missing.iter().any(|val| {
        val.as_str()
            .is_some_and(|text| text.contains("chapter range out of bounds"))
    })
}

fn has_span_message(missing: &[serde_json::Value]) -> bool {
    missing.iter().any(|val| {
        val.as_str()
            .is_some_and(|text| text.contains("exceeding the per-call maximum of 1000 chapters"))
    })
}

#[path = "authoring/tests.rs"]
mod authoring;
#[path = "canon/tests.rs"]
mod canon;
#[path = "catalog/tests.rs"]
mod catalog;
#[path = "dispatch/tests.rs"]
mod dispatch;
#[path = "export/tests.rs"]
mod export;
#[path = "journal/tests.rs"]
mod journal;
#[path = "schema/tests.rs"]
mod schema;
#[path = "serialization/tests.rs"]
mod serialization;
#[path = "session/tests.rs"]
mod session;
#[path = "skills/tests.rs"]
mod skills;
#[path = "style/tests.rs"]
mod style;

#[path = "alignment/tests.rs"]
mod alignment;

#[path = "quality/tests.rs"]
mod quality;

#[path = "profiles/tests.rs"]
mod profile_generation;

#[path = "schema/discovery_tests.rs"]
mod slim_discovery;

#[path = "prepare_scene/tests.rs"]
mod prepare_scene;

#[path = "commit_scene/tests.rs"]
mod commit_scene;
