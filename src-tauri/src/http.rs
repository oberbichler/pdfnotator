//! Local HTTP server: MCP over streamable HTTP (JSON responses) and a small REST API.
//!
//! Only loopback clients are served. `Host` and `Origin` are validated to defend
//! against DNS-rebinding attacks from web pages.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp;
use crate::session::Session;

type AppState = Arc<Session>;

pub fn router(session: Arc<Session>) -> Router {
    Router::new()
        .route(
            "/mcp",
            post(mcp_post)
                // no server-initiated SSE stream; sessions are stateless
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .delete(|| async { StatusCode::OK }),
        )
        .route("/api/status", get(status))
        .route("/api/open", post(open))
        .route("/api/feedback", get(feedback))
        .layer(middleware::from_fn(guard))
        .with_state(session)
}

/// Reject requests whose `Host` or `Origin` is not loopback.
async fn guard(req: Request, next: Next) -> Response {
    if !host_ok(req.headers()) || !origin_ok(req.headers()) {
        return (StatusCode::FORBIDDEN, "forbidden: only local clients are allowed").into_response();
    }
    next.run(req).await
}

fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default() // [::1]:port
    } else if host.matches(':').count() > 1 {
        host // bare IPv6
    } else {
        host.split(':').next().unwrap_or_default() // name[:port]
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

fn host_ok(headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_none_or(is_loopback_host)
}

fn origin_ok(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) else {
        return true; // non-browser clients
    };
    if origin.starts_with("vscode-") || origin.starts_with("tauri://") || origin == "null" {
        return true;
    }
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .is_some_and(is_loopback_host)
}

/// Header identifying an MCP session (set by us on `initialize`, echoed by clients).
pub const SESSION_HEADER: &str = "mcp-session-id";

async fn mcp_post(State(session): State<AppState>, headers: HeaderMap, Json(msg): Json<Value>) -> Response {
    let client = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let client = client.as_deref();
    if let Value::Array(batch) = &msg {
        let mut out = Vec::new();
        for m in batch {
            out.extend(mcp::handle(&session, client, m).await);
        }
        return if out.is_empty() {
            StatusCode::ACCEPTED.into_response()
        } else {
            Json(out).into_response()
        };
    }
    let is_init = msg.get("method").and_then(Value::as_str) == Some("initialize");
    match mcp::handle(&session, client, &msg).await {
        None => StatusCode::ACCEPTED.into_response(),
        Some(reply) => {
            let mut resp = Json(reply).into_response();
            if is_init && let Ok(id) = HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()) {
                resp.headers_mut().insert(SESSION_HEADER, id);
            }
            resp
        }
    }
}

async fn status(State(session): State<AppState>) -> impl IntoResponse {
    Json(session.status())
}

#[derive(Debug, Deserialize)]
struct OpenRequest {
    path: String,
    root: Option<String>,
}

async fn open(State(session): State<AppState>, Json(req): Json<OpenRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        session.open(&req.path, req.root.as_deref().map(std::path::Path::new), None)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok((doc, opened)) => Json(json!({ "ok": true, "doc": doc.id, "path": opened.pdf })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct FeedbackQuery {
    wait: Option<u64>,
    /// Document path; optional when only one document is open.
    path: Option<String>,
}

async fn feedback(State(session): State<AppState>, Query(q): Query<FeedbackQuery>) -> Response {
    let doc = match session.resolve(q.path.as_deref(), None) {
        Ok(doc) => doc,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    };
    let wait = Duration::from_secs(q.wait.unwrap_or(0).min(3600));
    Json(
        doc.wait_for_feedback(wait, &tokio::sync::Notify::new())
            .await
            .unwrap_or_default(),
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn send(app: &Router, req: axum::http::Request<Body>) -> (StatusCode, HeaderMap, Value) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn rpc(body: &Value) -> axum::http::Request<Body> {
        axum::http::Request::post("/mcp")
            .header(header::HOST, "127.0.0.1:4747")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[test]
    fn loopback_detection() {
        for ok in ["127.0.0.1", "127.0.0.1:4747", "localhost:1", "[::1]:4747", "::1"] {
            assert!(is_loopback_host(ok), "{ok}");
        }
        for bad in [
            "evil.com",
            "evil.com:4747",
            "127.0.0.1.evil.com",
            "localhost.evil.com:80",
        ] {
            assert!(!is_loopback_host(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn rejects_foreign_host_and_origin() {
        let app = router(Session::new(0));
        let req = axum::http::Request::get("/api/status")
            .header(header::HOST, "evil.com:4747")
            .body(Body::empty());
        assert_eq!(send(&app, req.unwrap()).await.0, StatusCode::FORBIDDEN);

        let mut req = rpc(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
        req.headers_mut()
            .insert(header::ORIGIN, HeaderValue::from_static("https://evil.com"));
        assert_eq!(send(&app, req).await.0, StatusCode::FORBIDDEN);

        let mut req = rpc(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
        req.headers_mut()
            .insert(header::ORIGIN, HeaderValue::from_static("http://localhost:1420"));
        assert_eq!(send(&app, req).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn mcp_initialize_notification_and_batch() {
        let app = router(Session::new(0));
        let (st, headers, body) = send(
            &app,
            rpc(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(headers.contains_key("mcp-session-id"));
        assert_eq!(body["result"]["serverInfo"]["name"], "pdfnotator");

        let (st, ..) = send(
            &app,
            rpc(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED);

        let batch = json!([
            { "jsonrpc": "2.0", "id": 1, "method": "ping" },
            { "jsonrpc": "2.0", "method": "notifications/initialized" },
            { "jsonrpc": "2.0", "id": 2, "method": "tools/list" }
        ]);
        let (_, _, body) = send(&app, rpc(&batch)).await;
        assert_eq!(body.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rest_open_status_feedback() {
        let session = Session::new(4747);
        let app = router(session.clone());
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("r.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();

        let req = axum::http::Request::post("/api/open")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "path": pdf }).to_string()))
            .unwrap();
        let (st, _, body) = send(&app, req).await;
        assert_eq!(st, StatusCode::OK, "{body}");

        let (_, _, body) = send(
            &app,
            axum::http::Request::get("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(body["port"], 4747);
        assert!(body["documents"][0]["path"].as_str().unwrap().ends_with("r.pdf"));

        session.documents()[0].submit(vec![crate::feedback::item("c9")]);
        let (_, _, body) = send(
            &app,
            axum::http::Request::get("/api/feedback?wait=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(body[0]["id"], "c9");

        let req = axum::http::Request::post("/api/open")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "path": "/nope.pdf" }).to_string()))
            .unwrap();
        let (st, _, body) = send(&app, req).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(body["ok"], false);
    }
}
