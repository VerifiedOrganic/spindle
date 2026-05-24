use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
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

/// Build a router that serves both the MCP streamable HTTP transport at `/mcp`
/// and the existing read-only operational routes (`/health`, `/model-routes`,
/// `/events`).
pub fn mcp_router(service: SpindleService, cancellation_token: CancellationToken) -> Router {
    let mcp_service =
        SpindleMcpServer::streamable_http_service(service.clone(), cancellation_token);

    Router::new()
        .route("/health", get(health))
        .route("/model-routes", get(model_routes))
        .route("/events", get(event_stream))
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
        "read_only_endpoints": ["/health", "/model-routes", "/events"]
    }))
}

async fn model_routes(State(state): State<HttpState>) -> impl IntoResponse {
    Json(state.service.model_routes())
}

async fn event_stream(
    State(state): State<HttpState>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let service = state.service.clone();
    let stream =
        IntervalStream::new(tokio::time::interval(Duration::from_secs(2))).map(move |_| {
            let payload = serde_json::to_string(&snapshot_payload(&service))
                .unwrap_or_else(|_| "{}".to_string());
            Ok(Event::default().event("snapshot").data(payload))
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
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
}
