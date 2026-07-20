use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::IntervalStream;
use tokio_util::sync::CancellationToken;

use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;

use crate::server::SpindleMcpServer;

#[derive(Clone)]
pub struct HttpState {
    service: Arc<SpindleService>,
}

impl HttpState {
    pub fn new(service: SpindleService) -> Self {
        Self {
            service: Arc::new(service),
        }
    }
}

/// The read-only operator console page (evolution §3.7 console v1). ONE committed
/// HTML file — inline CSS + JS, no external requests, no build step — embedded at
/// compile time and served under `/console`. Security posture: localhost-bound
/// like the whole HTTP surface, no auth, read-only; the manuscript/delta panes
/// show prose the operator already owns (same trust context as their own
/// terminal), while journal payloads it streams are prose-free by ADR 0002.
const CONSOLE_HTML: &str = include_str!("console.html");

/// Build a router that serves the MCP streamable HTTP transport at `/mcp`, the
/// existing read-only operational routes (`/health`, `/model-routes`, `/events`),
/// and the read-only operator console (`/console` + `/console/api/*`).
pub fn mcp_router(service: SpindleService, cancellation_token: CancellationToken) -> Router {
    let mcp_service =
        SpindleMcpServer::streamable_http_service(service.clone(), cancellation_token);

    Router::new()
        .route("/health", get(health))
        .route("/model-routes", get(model_routes))
        .route("/events", get(event_stream))
        // Read-only operator console v1 (evolution §3.7). The page is one static
        // file; its data comes from the localhost-only `/console/api/*` reads
        // below, which call the service layer thinly and never mutate. The MCP
        // streamable-HTTP transport's initialize+session handshake is
        // impractical to speak from zero-dependency browser JS, so this is the
        // sanctioned GET-fallback documented in the console file's header.
        .route("/console", get(console_page))
        .route("/console/api/projects", get(console_projects))
        .route("/console/api/status", get(console_status))
        .route("/console/api/manuscript", get(console_manuscript))
        .route("/console/api/canon_deltas", get(console_canon_deltas))
        .route("/console/api/plan_amendments", get(console_plan_amendments))
        .with_state(HttpState::new(service))
        .nest_service("/mcp", mcp_service)
}

pub async fn serve(service: SpindleService, addr: SocketAddr) -> anyhow::Result<()> {
    let ct = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("spindle mcp listening on http://{addr}/mcp");
    axum::serve(listener, mcp_router(service, ct.clone()))
        .with_graceful_shutdown(async move { ct.cancelled_owned().await })
        .await?;
    Ok(())
}

fn snapshot_payload(service: &SpindleService) -> serde_json::Value {
    serde_json::json!({
        "model_routes": service.model_routes(),
    })
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "mode": "mcp-http",
        "mcp_endpoint": "/mcp",
        "read_only_endpoints": ["/health", "/model-routes", "/events"],
        "console": "/console"
    }))
}

async fn model_routes(State(state): State<HttpState>) -> impl IntoResponse {
    Json(state.service.model_routes())
}

// ── Read-only operator console (evolution §3.7 console v1) ──────────────────
//
// These endpoints are the sanctioned browser fallback: the console page cannot
// practically speak the rmcp streamable-HTTP MCP transport (initialize +
// `Mcp-Session-Id` negotiation + SSE tool responses) from zero-dependency JS, so
// it reads through these thin, read-only GETs instead. Each calls the service
// layer directly (never the repository), returns exactly the underlying tool's
// serialization (no extra leakage surface — those tools are already gated), and
// performs NO mutation. Localhost-bound like the rest of the HTTP surface.

/// Serve the embedded console page (evolution §3.7). `text/html; charset=utf-8`.
async fn console_page() -> axum::response::Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        CONSOLE_HTML,
    )
        .into_response()
}

/// Map any read error to a JSON 500 with a prose-free message. A malformed input
/// (missing required query param) is a 400 so the defensive JS renders an error
/// state, never a blank pane.
fn read_error(err: anyhow::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": err.to_string() })),
    )
        .into_response()
}

fn bad_request(message: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// `GET /console/api/projects` → `list_projects` output verbatim (I5 viewer).
async fn console_projects(State(state): State<HttpState>) -> axum::response::Response {
    match state.service.list_projects().await {
        Ok(output) => Json(output).into_response(),
        Err(err) => read_error(err),
    }
}

/// Query params shared by the project-scoped console reads.
#[derive(Debug, Deserialize)]
struct ConsoleStatusParams {
    project_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
}

/// `GET /console/api/status` → read-only authoring status for a project's latest
/// run (or the given `run_id`). Wraps the tool's status in `{ run_present, run }`
/// so an empty project (no run) is an honest `run_present: false` 200, not a 500.
/// Read-only: unlike `authoring_status` the tool, this never persists the
/// reconcile (evolution I5 — the console is a viewer, never a second writer).
async fn console_status(
    State(state): State<HttpState>,
    Query(params): Query<ConsoleStatusParams>,
) -> axum::response::Response {
    let Some(project_id) = params.project_id.filter(|p| !p.is_empty()) else {
        return bad_request("project_id is required");
    };
    let router = crate::tools::ToolRouter::with_tool_profile_and_serialization(
        (*state.service).clone(),
        None,
        std::sync::Arc::new(crate::tools::ToolSerializationState::default()),
    );
    let input = spindle_core::models::AuthoringStatusInput {
        project_id,
        run_id: params.run_id.filter(|r| !r.is_empty()),
    };
    match router.authoring_status_readonly(input).await {
        Ok(Some(status)) => {
            Json(serde_json::json!({ "run_present": true, "run": status })).into_response()
        }
        Ok(None) => {
            Json(serde_json::json!({ "run_present": false, "run": serde_json::Value::Null }))
                .into_response()
        }
        Err(err) => read_error(err),
    }
}

/// Query params for the manuscript read.
#[derive(Debug, Deserialize)]
struct ConsoleManuscriptParams {
    project_id: Option<String>,
    book_number: Option<i32>,
    #[serde(default)]
    start_chapter: Option<i32>,
    #[serde(default)]
    end_chapter: Option<i32>,
}

/// `GET /console/api/manuscript` → `compile_manuscript` output. Always
/// `write_to_workspace: false` — the console display path never writes an
/// artifact (read-only).
async fn console_manuscript(
    State(state): State<HttpState>,
    Query(params): Query<ConsoleManuscriptParams>,
) -> axum::response::Response {
    let Some(project_id) = params.project_id.filter(|p| !p.is_empty()) else {
        return bad_request("project_id is required");
    };
    let Some(book_number) = params.book_number else {
        return bad_request("book_number is required");
    };
    let input = spindle_core::models::CompileManuscriptInput {
        project_id,
        book_number,
        start_chapter: params.start_chapter,
        end_chapter: params.end_chapter,
        write_to_workspace: false,
    };
    match state.service.compile_manuscript(input).await {
        Ok(output) => Json(output).into_response(),
        Err(err) => read_error(err),
    }
}

/// Query params for the canon-delta queue read.
#[derive(Debug, Deserialize)]
struct ConsoleCanonParams {
    project_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// `GET /console/api/canon_deltas` → `list_canon_deltas` output verbatim
/// (defaults to `status=staged` when the query omits it — the ratify queue).
async fn console_canon_deltas(
    State(state): State<HttpState>,
    Query(params): Query<ConsoleCanonParams>,
) -> axum::response::Response {
    let Some(project_id) = params.project_id.filter(|p| !p.is_empty()) else {
        return bad_request("project_id is required");
    };
    let input = spindle_core::models::ListCanonDeltasInput {
        project_id,
        status: Some(params.status.unwrap_or_else(|| "staged".to_string())),
        scene_id: None,
        chapter_range: None,
    };
    match state.service.list_canon_deltas(input).await {
        Ok(output) => Json(output).into_response(),
        Err(err) => read_error(err),
    }
}

/// Query params for the plan-amendment queue read.
#[derive(Debug, Deserialize)]
struct ConsolePlanParams {
    project_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// `GET /console/api/plan_amendments` → `list_plan_amendments` output verbatim
/// (defaults to `status=staged` — the ratify queue).
async fn console_plan_amendments(
    State(state): State<HttpState>,
    Query(params): Query<ConsolePlanParams>,
) -> axum::response::Response {
    let Some(project_id) = params.project_id.filter(|p| !p.is_empty()) else {
        return bad_request("project_id is required");
    };
    let input = spindle_core::models::ListPlanAmendmentsInput {
        project_id,
        status: Some(params.status.unwrap_or_else(|| "staged".to_string())),
        book_number: None,
        source_chapter: None,
    };
    match state.service.list_plan_amendments(input).await {
        Ok(output) => Json(output).into_response(),
        Err(err) => read_error(err),
    }
}

/// Query parameters for `/events`. `topic` selects the stream shape (ADR 0002
/// D4): absent → the existing model-routes snapshot stream (I1); `run:<run_id>`
/// → that run's journal, replayed from `Last-Event-ID`+1 then followed live.
#[derive(Debug, Deserialize)]
struct EventStreamParams {
    topic: Option<String>,
}

/// Live-follow poll cadence for the journal stream — matches the existing
/// snapshot stream's 2s interval (ADR D4: "polling the table on the existing
/// snapshot cadence is acceptable").
const JOURNAL_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How many rows a single replay/poll batch reads. Bounds memory on a long run;
/// the follow loop drains in successive batches until caught up.
const JOURNAL_BATCH_LIMIT: usize = 256;

async fn event_stream(
    State(state): State<HttpState>,
    Query(params): Query<EventStreamParams>,
    headers: HeaderMap,
) -> axum::response::Response {
    let Some(topic) = params.topic else {
        // No topic → the existing model-routes snapshot stream, byte-identical
        // to the pre-journal behavior (ADR D4 / I1).
        let service = state.service.clone();
        let stream =
            IntervalStream::new(tokio::time::interval(JOURNAL_POLL_INTERVAL)).map(move |_| {
                let payload = serde_json::to_string(&snapshot_payload(&service))
                    .unwrap_or_else(|_| "{}".to_string());
                Ok::<_, std::convert::Infallible>(Event::default().event("snapshot").data(payload))
            });
        return Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response();
    };

    // Topic present: only `run:<run_id>` is understood, and the run id is
    // validated shape-wise (`authoring_run:` prefix). No existence check — an
    // empty stream is a valid answer (ADR D4). Anything else is a 400.
    let Some(run_id) = topic.strip_prefix("run:") else {
        return (
            StatusCode::BAD_REQUEST,
            "unknown topic; expected run:<run_id>",
        )
            .into_response();
    };
    if !run_id.starts_with("authoring_run:") || run_id.len() <= "authoring_run:".len() {
        return (
            StatusCode::BAD_REQUEST,
            "malformed run topic; expected run:authoring_run:<id>",
        )
            .into_response();
    }

    let after_seq = parse_last_event_id(&headers);
    let stream = run_journal_stream(state.service.clone(), run_id.to_string(), after_seq);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Parse the SSE `Last-Event-ID` header into a resume cursor. The header value
/// is the seq of the last event the client saw; replay resumes at `seq+1`
/// (handled by passing this as `after_seq` to `list_run_events`). A missing or
/// unparseable header replays from the beginning (ADR D4).
fn parse_last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i64>().ok())
}

/// Build the run-journal SSE stream: replay rows with `seq > after_seq`, then
/// follow live appends by polling the journal on [`JOURNAL_POLL_INTERVAL`].
/// Each SSE frame carries `id = seq`, `event = kind`, `data = payload JSON`
/// (ADR D4). A per-connection `cursor` advances past every delivered row so no
/// row is re-sent and the follow loop only ever emits newly-appended rows.
///
/// Implemented as a producer task feeding a bounded channel, consumed as a
/// [`ReceiverStream`] — the channel backpressures the poller if a slow client
/// falls behind, and the task ends (dropping the sender, closing the stream)
/// when the client disconnects and the receiver is dropped.
fn run_journal_stream(
    service: Arc<SpindleService>,
    run_id: String,
    after_seq: Option<i64>,
) -> impl futures_core::Stream<Item = Result<Event, std::convert::Infallible>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);

    tokio::spawn(async move {
        // `cursor` is the highest seq delivered so far; the next batch reads
        // `seq > cursor`. Seeded from Last-Event-ID (0 = from the beginning).
        let mut cursor = after_seq.unwrap_or(0);

        // Immediate replay batch(es), draining until caught up.
        loop {
            let rows = service
                .repository()
                .list_run_events(&run_id, Some(cursor), Some(JOURNAL_BATCH_LIMIT))
                .await
                .unwrap_or_default();
            let batch_len = rows.len();
            for row in &rows {
                cursor = row.seq;
                if tx.send(Ok(run_event_frame(row))).await.is_err() {
                    return; // client disconnected
                }
            }
            if batch_len < JOURNAL_BATCH_LIMIT {
                break;
            }
        }

        // Live follow: poll for newly-appended rows on the snapshot cadence.
        let mut ticks = IntervalStream::new(tokio::time::interval(JOURNAL_POLL_INTERVAL));
        while ticks.next().await.is_some() {
            let rows = service
                .repository()
                .list_run_events(&run_id, Some(cursor), Some(JOURNAL_BATCH_LIMIT))
                .await
                .unwrap_or_default();
            for row in &rows {
                cursor = row.seq;
                if tx.send(Ok(run_event_frame(row))).await.is_err() {
                    return; // client disconnected
                }
            }
        }
    });

    tokio_stream::wrappers::ReceiverStream::new(rx)
}

/// Render one stored journal row as an SSE frame (ADR D4): `id = seq`,
/// `event = kind`, `data = payload JSON`.
fn run_event_frame(row: &spindle_adapters::sqlite::records::StoredRunEvent) -> Event {
    let data = serde_json::to_string(&row.payload).unwrap_or_else(|_| "{}".to_string());
    Event::default()
        .id(row.seq.to_string())
        .event(row.kind.clone())
        .data(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Method;
    use spindle_adapters::sqlite::Repository;
    use spindle_adapters::{ModelRouter, SqlitePool};
    use tempfile::tempdir;
    use tower::util::ServiceExt;

    async fn app() -> Router {
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("http.db"))
            .await
            .expect("db init");
        let data_dir = temp.keep();
        let service = SpindleService::new(Repository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ));
        mcp_router(service, CancellationToken::new())
    }

    /// A router plus the service backing it, so tests can seed journal rows on
    /// the same database the router reads. Returns the created project so the
    /// authoring_run FK target is valid.
    async fn app_with_service() -> (Router, SpindleService, String) {
        use spindle_core::models::{CreateProjectInput, ReaderContract};
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("http.db"))
            .await
            .expect("db init");
        let data_dir = temp.keep();
        let service = SpindleService::new(Repository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ));
        let project = service
            .create_project(CreateProjectInput {
                name: "SSE".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .expect("project");
        let router = mcp_router(service.clone(), CancellationToken::new());
        (router, service, project.project_id)
    }

    /// Persist a minimal `active` authoring run so events have a valid FK
    /// target, returning its id.
    async fn seed_run(service: &SpindleService, project_id: &str) -> String {
        let branch = service
            .repository()
            .get_active_branch(project_id)
            .await
            .expect("active branch");
        let run_id = format!(
            "authoring_run:{}",
            ulid::Ulid::new().to_string().to_lowercase()
        );
        let now = chrono::Utc::now();
        let run = spindle_adapters::sqlite::records::AuthoringRun {
            id: run_id.clone(),
            project_id: project_id.to_string(),
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
            mining_policy: None,
            max_revise_attempts: None,
            checkpoint_policy: None,
            replan_policy: None,
        };
        service
            .repository()
            .save_authoring_run(run, Vec::new(), Vec::new(), Vec::new())
            .await
            .expect("save run");
        run_id
    }

    /// Read available SSE frames from a response body within a short timeout,
    /// then drop the body (the stream never closes). Returns the raw text so a
    /// test can assert on `id:` / `event:` / `data:` lines.
    async fn read_sse_frames(response: axum::response::Response) -> String {
        use tokio_stream::StreamExt;
        let mut body = response.into_body().into_data_stream();
        let mut text = String::new();
        // Collect whatever the replay batch produced; the follow poll cadence is
        // 2s, so a sub-second budget captures replay-only without live rows.
        let _ = tokio::time::timeout(Duration::from_millis(400), async {
            while let Some(Ok(chunk)) = body.next().await {
                text.push_str(&String::from_utf8_lossy(&chunk));
            }
        })
        .await;
        text
    }

    async fn service() -> SpindleService {
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("service.db"))
            .await
            .expect("db init");
        let data_dir = temp.keep();
        SpindleService::new(Repository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn health_route_returns_ok() {
        let app = app().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("health json");
        assert_eq!(body["status"], "ok");
        assert_eq!(body["mode"], "mcp-http");
        assert_eq!(body["mcp_endpoint"], "/mcp");
        assert_eq!(
            body["read_only_endpoints"]
                .as_array()
                .expect("endpoints")
                .len(),
            3
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn model_routes_route_returns_current_route_snapshot() {
        let app = app().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/model-routes")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("model routes json");
        let routes = body.as_array().expect("route array");
        assert!(routes.iter().any(|route| route.get("route_name")
            == Some(&serde_json::Value::String("draft".to_string()))));
        assert!(routes.iter().any(|route| route.get("route_name")
            == Some(&serde_json::Value::String("import_extract".to_string()))));
        assert!(routes.iter().any(|route| route.get("route_name")
            == Some(&serde_json::Value::String("import_synthesize".to_string()))));
        assert!(routes.iter().any(|route| route.get("route_name")
            == Some(&serde_json::Value::String("import_validate".to_string()))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn events_route_exposes_sse_content_type() {
        let app = app().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/events")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("content type");
        assert!(content_type.starts_with("text/event-stream"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn events_run_topic_replays_journal_rows_with_id_and_event_fields() {
        let (app, service, project_id) = app_with_service().await;
        let run_id = seed_run(&service, &project_id).await;
        for kind in ["run_started", "scene_drafted", "scene_committed"] {
            service
                .repository()
                .append_run_event(&run_id, kind, serde_json::json!({ "k": kind }))
                .await
                .expect("append");
        }

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/events?topic=run:{run_id}"))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let frames = read_sse_frames(response).await;
        // All three replayed, in seq order, with id = seq and event = kind.
        assert!(frames.contains("id: 1"), "missing id 1 in {frames:?}");
        assert!(frames.contains("id: 2"));
        assert!(frames.contains("id: 3"));
        assert!(frames.contains("event: run_started"));
        assert!(frames.contains("event: scene_drafted"));
        assert!(frames.contains("event: scene_committed"));
        assert!(frames.contains("\"k\":\"run_started\""));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn events_run_topic_resumes_after_last_event_id() {
        let (app, service, project_id) = app_with_service().await;
        let run_id = seed_run(&service, &project_id).await;
        for i in 1..=4 {
            service
                .repository()
                .append_run_event(&run_id, "scene_committed", serde_json::json!({ "n": i }))
                .await
                .expect("append");
        }

        // Last-Event-ID = 2 → only seq 3 and 4 replay (resume-from token).
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/events?topic=run:{run_id}"))
                    .header("Last-Event-ID", "2")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let frames = read_sse_frames(response).await;
        assert!(
            !frames.contains("id: 1"),
            "seq 1 must not replay: {frames:?}"
        );
        assert!(
            !frames.contains("id: 2"),
            "seq 2 must not replay: {frames:?}"
        );
        assert!(frames.contains("id: 3"), "seq 3 must replay: {frames:?}");
        assert!(frames.contains("id: 4"), "seq 4 must replay: {frames:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn events_malformed_topic_returns_400() {
        // Unknown topic scheme.
        let response = app()
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/events?topic=garbage")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

        // Right scheme, wrong run-id shape.
        let response = app()
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/events?topic=run:not-a-run")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn events_no_topic_stream_is_unchanged_model_route_snapshot() {
        // No topic param → the existing snapshot stream (I1): SSE content-type
        // and a `snapshot` event carrying model_routes.
        let response = app()
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/events")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("content type");
        assert!(content_type.starts_with("text/event-stream"));
        let frames = read_sse_frames(response).await;
        assert!(
            frames.contains("event: snapshot"),
            "no-topic stream must emit snapshot events: {frames:?}"
        );
        assert!(frames.contains("model_routes"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn events_unknown_run_topic_is_valid_empty_stream() {
        // A well-formed run id with no rows is a valid empty stream (ADR D4: no
        // existence check), not a 400.
        let response = app()
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/events?topic=run:authoring_run:doesnotexist")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_surface_is_get_only() {
        let app = app().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method(Method::POST)
                    .uri("/model-routes")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(
            response.status(),
            axum::http::StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sse_snapshot_payload_stays_read_only_and_model_route_focused() {
        let service = service().await;
        let payload = snapshot_payload(&service);
        let payload = payload.as_object().expect("snapshot payload object");

        assert_eq!(payload.len(), 1);
        assert!(payload.contains_key("model_routes"));
        assert!(payload["model_routes"].is_array());
    }

    // ── P5.1 console (read-only operator console v1) ─────────────────────────

    /// Stable marker string every console page carries; asserted by the serving
    /// test so a future edit that swaps the page for something empty is caught.
    const CONSOLE_MARKER: &str = "Spindle Operator Console";

    #[tokio::test(flavor = "current_thread")]
    async fn console_route_serves_embedded_page() {
        let response = app()
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/console")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("content type");
        assert_eq!(content_type, "text/html; charset=utf-8");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body = String::from_utf8(body.to_vec()).expect("utf-8 html");
        assert!(
            body.contains(CONSOLE_MARKER),
            "console page missing stable marker: {CONSOLE_MARKER}"
        );
        // No external requests may leave localhost: the page embeds everything
        // (STRICT DEPENDENCY RULE — no CDNs, fonts, or remote assets).
        assert!(
            !body.contains("http://") && !body.contains("https://"),
            "console page must make no external requests"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_health_route_reports_console_endpoint() {
        // /health advertises the console so an operator can discover it.
        let response = app()
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("health json");
        assert_eq!(body["console"], "/console");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_api_projects_matches_service_serialization() {
        let (app, service, project_id) = app_with_service().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/console/api/projects")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("projects json");

        // The endpoint must equal the underlying service call's serialization —
        // it leaks nothing beyond what `list_projects` already returns.
        let expected =
            serde_json::to_value(service.list_projects().await.expect("list")).expect("serialize");
        assert_eq!(body, expected);
        assert!(
            body["projects"]
                .as_array()
                .expect("projects array")
                .iter()
                .any(|p| p["project_id"] == project_id)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_api_canon_deltas_is_read_only_and_shape_stable() {
        let (app, service, project_id) = app_with_service().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/console/api/canon_deltas?project_id={project_id}&status=staged"
                    ))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("deltas json");

        // Equals the gated service call verbatim (no extra leakage surface).
        let expected = serde_json::to_value(
            service
                .list_canon_deltas(spindle_core::models::ListCanonDeltasInput {
                    project_id: project_id.clone(),
                    status: Some("staged".into()),
                    scene_id: None,
                    chapter_range: None,
                })
                .await
                .expect("list deltas"),
        )
        .expect("serialize");
        assert_eq!(body, expected);
        assert!(body["deltas"].is_array());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_api_plan_amendments_is_read_only_and_shape_stable() {
        let (app, service, project_id) = app_with_service().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/console/api/plan_amendments?project_id={project_id}&status=staged"
                    ))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("amendments json");

        let expected = serde_json::to_value(
            service
                .list_plan_amendments(spindle_core::models::ListPlanAmendmentsInput {
                    project_id: project_id.clone(),
                    status: Some("staged".into()),
                    book_number: None,
                    source_chapter: None,
                })
                .await
                .expect("list amendments"),
        )
        .expect("serialize");
        assert_eq!(body, expected);
        assert!(body["amendments"].is_array());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_api_manuscript_matches_service_serialization() {
        let (app, service, project_id) = app_with_service().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/console/api/manuscript?project_id={project_id}&book_number=1"
                    ))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("manuscript json");

        // The console never triggers a workspace write: write_to_workspace is
        // always false, so artifact_path is absent (read-only display path).
        let expected = serde_json::to_value(
            service
                .compile_manuscript(spindle_core::models::CompileManuscriptInput {
                    project_id: project_id.clone(),
                    book_number: 1,
                    start_chapter: None,
                    end_chapter: None,
                    write_to_workspace: false,
                })
                .await
                .expect("compile"),
        )
        .expect("serialize");
        assert_eq!(body, expected);
        assert!(body.get("artifact_path").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_api_status_without_run_reports_no_run_honestly() {
        // Empty project, no runs: the status endpoint reports an honest empty
        // state (200 with a `no_run` marker), never a 500 or blank body.
        let (app, _service, project_id) = app_with_service().await;

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/console/api/status?project_id={project_id}"))
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("status json");
        assert_eq!(body["run"], serde_json::Value::Null);
        assert_eq!(body["run_present"], serde_json::Value::Bool(false));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_api_status_reports_run_without_persisting() {
        // With a seeded run, the status endpoint returns the reconciled status
        // for the latest run (run-id discovery: run_id omitted → latest run) —
        // and is read-only: two calls do not change stored run state.
        let (app, service, project_id) = app_with_service().await;
        let run_id = seed_run(&service, &project_id).await;

        let call = |uri: String| {
            let app = app.clone();
            async move {
                let response = app
                    .oneshot(
                        axum::http::Request::builder()
                            .uri(uri)
                            .body(axum::body::Body::empty())
                            .expect("request"),
                    )
                    .await
                    .expect("response");
                assert_eq!(response.status(), axum::http::StatusCode::OK);
                let body = to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("body bytes");
                serde_json::from_slice::<serde_json::Value>(&body).expect("status json")
            }
        };

        let uri = format!("/console/api/status?project_id={project_id}");
        let first = call(uri.clone()).await;
        assert_eq!(first["run_present"], serde_json::Value::Bool(true));
        assert_eq!(first["run"]["run_id"], run_id);
        assert!(first["run"]["chapters"].is_array());

        // Read-only: the stored run's updated_at is unchanged by a status read
        // (the tool path persists a reconcile; the console path must not).
        let (before, ..) = service
            .repository()
            .get_authoring_run(&run_id)
            .await
            .expect("get run")
            .expect("run present");
        let _ = call(uri).await;
        let (after, ..) = service
            .repository()
            .get_authoring_run(&run_id)
            .await
            .expect("get run")
            .expect("run present");
        assert_eq!(
            before.updated_at, after.updated_at,
            "console status read must not persist a reconcile"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn console_api_missing_project_id_is_bad_request() {
        // A defensive read endpoint returns 400 (not 500) when its required
        // query parameter is absent.
        let response = app()
            .await
            .oneshot(
                axum::http::Request::builder()
                    .uri("/console/api/status")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}
