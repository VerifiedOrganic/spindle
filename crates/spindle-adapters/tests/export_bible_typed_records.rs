//! Integration test for Gap 5: `export_bible` must emit typed-record
//! JSON with RFC-3339 timestamps (not raw column-keyed rows with
//! unix-microsecond integers).

use spindle_adapters::sqlite::{Repository, SqlitePool, SqliteSpindleService};
use spindle_core::models::{CreateProjectInput, ExportBibleInput, ReaderContract};
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
async fn export_bible_emits_rfc3339_timestamps_in_typed_payload() {
    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "ExportInt".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "Wardens hold the line.".into(),
                style_notes: vec!["grim".into()],
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let out = svc
        .export_bible(ExportBibleInput {
            project_id: proj.project_id.clone(),
        })
        .await
        .unwrap();

    let bytes = std::fs::read(&out.file_path).expect("export file readable");
    let payload: serde_json::Value = serde_json::from_slice(&bytes).expect("export JSON parses");

    // Walk the tables map: every created_at timestamp must be an
    // RFC-3339 string (starts with "20" + contains 'T'), not an i64
    // count of microseconds.
    let tables = payload
        .get("tables")
        .and_then(|t| t.as_object())
        .expect("export payload has a tables object");
    let mut checked = 0;
    for (table_name, rows) in tables {
        let rows = match rows {
            serde_json::Value::Array(rows) => rows.clone(),
            // The "project" entry is a single object, not an array.
            other => vec![other.clone()],
        };
        for row in rows {
            if let Some(ts) = row.get("created_at") {
                assert!(
                    ts.is_string(),
                    "{table_name}.created_at must be a string, got {ts:?}"
                );
                let s = ts.as_str().unwrap();
                assert!(
                    s.starts_with("20") && s.contains('T'),
                    "{table_name}.created_at must look like RFC-3339, got {s}"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked > 0,
        "at least one created_at must have been checked"
    );
}
