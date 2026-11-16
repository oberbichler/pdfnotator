//! Remote viewers: the UI in a tablet's browser over the local network, for
//! annotating with a pen.
//!
//! Off by default. Once started (from the connect menu) it listens on all
//! interfaces next to the loopback-only MCP server and serves the bundled UI
//! plus a small HTTP API that mirrors the Tauri commands (`src/backend.ts`);
//! events are streamed as server-sent events. Every request needs the pairing
//! token: the QR code's URL carries it once, after which it lives in an
//! `HttpOnly`, `SameSite=Strict` cookie. Stopping (or restarting) the server
//! invalidates the token.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::document::DocId;
use crate::feedback::FeedbackItem;
use crate::session::Session;

/// Bundled UI files: path (e.g. `/assets/index.js`) → bytes and MIME type.
pub type Assets = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;

/// Preferred port (`PDFNOTATOR_REMOTE_PORT`); any free port if it is taken.
pub const DEFAULT_PORT: u16 = 4748;
const COOKIE: &str = "pdfnotator_remote";

/// What the desktop UI shows to pair a tablet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    /// URL including the pairing token.
    pub url: String,
    /// QR code of `url` as an SVG document.
    pub qr_svg: String,
}

struct Running {
    info: RemoteInfo,
    stop: tokio::sync::oneshot::Sender<()>,
}

/// The remote server, if running.
#[derive(Default)]
pub struct Remote(Mutex<Option<Running>>);

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Remote").field(&self.info()).finish()
    }
}

impl Remote {
    pub fn info(&self) -> Option<RemoteInfo> {
        self.0.lock().unwrap().as_ref().map(|r| r.info.clone())
    }

    /// Start serving on the local network (or return the running server).
    pub async fn start(
        &self,
        session: Arc<Session>,
        assets: Assets,
        port: u16,
    ) -> Result<RemoteInfo, String> {
        if let Some(info) = self.info() {
            return Ok(info);
        }
        let ip = local_ip_address::local_ip().map_err(|e| format!("no local network address: {e}"))?;
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind(("0.0.0.0", 0))
                .await
                .map_err(|e| e.to_string())?,
        };
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token: Arc<str> = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        )
        .into();
        let url = format!("http://{ip}:{port}/?t={token}");
        let info = RemoteInfo {
            qr_svg: qr_svg(&url)?,
            url,
        };

        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let app = router(session, token, assets);
        tokio::spawn(async move {
            let shutdown = async {
                let _ = stopped.await;
            };
            if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(shutdown).await {
                eprintln!("pdfnotator: remote server: {e}");
            }
        });
        let mut running = self.0.lock().unwrap();
        // a concurrent start won the race: keep that one
        if let Some(r) = running.as_ref() {
            let _ = stop.send(());
            return Ok(r.info.clone());
        }
        *running = Some(Running {
            info: info.clone(),
            stop,
        });
        Ok(info)
    }

    /// Stop serving; connected tablets lose access.
    pub fn stop(&self) {
        if let Some(r) = self.0.lock().unwrap().take() {
            let _ = r.stop.send(());
        }
    }
}

fn qr_svg(url: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build())
}

#[derive(Clone)]
struct AppState {
    session: Arc<Session>,
    token: Arc<str>,
    assets: Assets,
}

pub fn router(session: Arc<Session>, token: Arc<str>, assets: Assets) -> Router {
    let state = AppState {
        session,
        token,
        assets,
    };
    Router::new()
        .route("/remote/app-info", get(app_info))
        .route("/remote/status", get(status))
        .route("/remote/events", get(events))
        .route("/remote/pdf/{doc}", get(pdf))
        .route("/remote/feedback/{doc}", post(feedback))
        .route("/remote/view/{doc}", post(view))
        .route("/remote/close/{doc}", post(close))
        .fallback(get(asset))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

/// Pairing: `/?t=<token>` sets the cookie; everything else needs it.
async fn auth(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let query_token = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("t=")));
    if let Some(t) = query_token {
        if !same(t, &st.token) {
            return denied();
        }
        let mut resp = Redirect::to(req.uri().path()).into_response();
        let cookie = format!(
            "{COOKIE}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000",
            st.token
        );
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            resp.headers_mut().insert(header::SET_COOKIE, v);
        }
        return resp;
    }
    if cookie_token(req.headers()).is_some_and(|t| same(t, &st.token)) {
        next.run(req).await
    } else {
        denied()
    }
}

fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(&format!("{COOKIE}=")[..]))
}

/// Token comparison without an early exit.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn denied() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><meta name=viewport content='width=device-width'>\
         <body style='font:16px system-ui;margin:3em;color:#444'>\
         <h2>PDFnotator</h2><p>Not paired. In PDFnotator on your computer, open the agent menu → \
         <b>Connect tablet</b> and scan the QR code again.</p>",
    )
        .into_response()
}

async fn asset(State(st): State<AppState>, req: Request) -> Response {
    let path = match req.uri().path() {
        "/" | "" => "/index.html",
        p => p,
    };
    match (st.assets)(path) {
        Some((bytes, mime)) => {
            let cache = if path.starts_with("/assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, cache.to_owned()),
                ],
                bytes,
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn app_info() -> impl IntoResponse {
    Json(json!({ "version": env!("CARGO_PKG_VERSION"), "executable": null, "demo": null, "remote": true }))
}

async fn status(State(st): State<AppState>) -> impl IntoResponse {
    Json(st.session.status())
}

/// Server-sent events: the current status, then every UI event.
async fn events(
    State(st): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = st.session.subscribe();
    let first = Event::default()
        .event("status")
        .json_data(st.session.status())
        .ok();
    let session = st.session.clone();
    let live = BroadcastStream::new(rx).filter_map(move |msg| match msg {
        Ok((name, payload)) => Event::default().event(name).json_data(payload).ok(),
        // fell behind: a fresh status brings the tablet up to date
        Err(_) => Event::default().event("status").json_data(session.status()).ok(),
    });
    let stream = tokio_stream::iter(first).chain(live).map(Ok);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn pdf(State(st): State<AppState>, Path(doc): Path<DocId>) -> Response {
    let Some(path) = st.session.document(doc).and_then(|d| d.pdf()) else {
        return (StatusCode::NOT_FOUND, "document is not open").into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/pdf"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn feedback(
    State(st): State<AppState>,
    Path(doc): Path<DocId>,
    Json(items): Json<Vec<FeedbackItem>>,
) -> StatusCode {
    match st.session.document(doc) {
        Some(d) => {
            d.submit(items);
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct View {
    page: u32,
    page_count: u32,
}

async fn view(State(st): State<AppState>, Path(doc): Path<DocId>, Json(v): Json<View>) -> StatusCode {
    if let Some(d) = st.session.document(doc) {
        d.set_view(v.page, v.page_count);
    }
    StatusCode::NO_CONTENT
}

async fn close(State(st): State<AppState>, Path(doc): Path<DocId>) -> StatusCode {
    st.session.close(doc);
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    const TOKEN: &str = "secret";

    fn app() -> (Router, Arc<Session>) {
        let session = Session::new(0);
        let assets: Assets = Arc::new(|path| {
            (path == "/index.html").then(|| (b"<html>ui</html>".to_vec(), "text/html".to_owned()))
        });
        (router(session.clone(), TOKEN.into(), assets), session)
    }

    async fn send(app: &Router, req: axum::http::Request<Body>) -> Response {
        app.clone().oneshot(req).await.unwrap()
    }

    fn get_with_cookie(uri: &str, token: &str) -> axum::http::Request<Body> {
        axum::http::Request::get(uri)
            .header(header::COOKIE, format!("other=1; {COOKIE}={token}"))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn pairing_sets_a_cookie_and_everything_else_needs_it() {
        let (app, _) = app();
        let r = send(&app, axum::http::Request::get("/").body(Body::empty()).unwrap()).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let r = send(
            &app,
            axum::http::Request::get("/?t=wrong").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

        let r = send(
            &app,
            axum::http::Request::get("/?t=secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert!(r.status().is_redirection());
        assert_eq!(r.headers()[header::LOCATION], "/", "token leaves the address bar");
        let cookie = r.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("pdfnotator_remote=secret;") && cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));

        let r = send(&app, get_with_cookie("/", TOKEN)).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            r.into_body().collect().await.unwrap().to_bytes(),
            "<html>ui</html>"
        );
        assert_eq!(
            send(&app, get_with_cookie("/remote/status", "nope"))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            send(&app, get_with_cookie("/missing.js", TOKEN)).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn mirrors_the_ui_commands() {
        let (app, session) = app();
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF-1.7 test").unwrap();
        let (doc, _) = session.open(pdf.to_str().unwrap(), None).unwrap();

        let r = send(&app, get_with_cookie("/remote/status", TOKEN)).await;
        let body: serde_json::Value =
            serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["documents"][0]["id"], doc.id);

        let r = send(&app, get_with_cookie(&format!("/remote/pdf/{}", doc.id), TOKEN)).await;
        assert_eq!(r.into_body().collect().await.unwrap().to_bytes(), "%PDF-1.7 test");

        let post = |uri: String, body: serde_json::Value| {
            axum::http::Request::post(uri)
                .header(header::COOKIE, format!("{COOKIE}={TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let item = json!([{ "id": "c1", "kind": "general", "page": 0, "comment": "from the iPad" }]);
        let r = send(&app, post(format!("/remote/feedback/{}", doc.id), item)).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(doc.feedback.len(), 1);

        let r = send(
            &app,
            post(
                format!("/remote/view/{}", doc.id),
                json!({ "page": 2, "pageCount": 3 }),
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert_eq!(doc.status().current_page, 2);
    }

    #[test]
    fn compares_tokens_and_reads_cookies() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd") && !same("abc", "ab"));
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; pdfnotator_remote=xyz"),
        );
        assert_eq!(cookie_token(&h), Some("xyz"));
        assert!(qr_svg("http://192.168.1.2:4748/?t=abc").unwrap().contains("<svg"));
    }
}
