//! Model Context Protocol (JSON-RPC 2.0) handling, transport independent.
//!
//! Used by the streamable-HTTP endpoint ([`crate::http`]) and the stdio bridge
//! ([`crate::bridge`]). Requests that do not need a running viewer are answered
//! by [`handle_static`], so the bridge can reply without launching the app.

pub mod tasks;
pub mod text;
mod tools;

use std::sync::Arc;

use serde_json::{Value, json};

use crate::session::Session;

/// Protocol revisions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &[
    STATELESS_VERSION,
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];
/// Revisions negotiated with `initialize` (all before 2026-07-28).
const INITIALIZE_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// Revision used when the client asks for one we don't know.
const FALLBACK_VERSION: &str = "2025-06-18";
/// From this revision on there is no `initialize`: every request names its
/// version, client and capabilities in `_meta`, and results carry `resultType`.
pub const STATELESS_VERSION: &str = "2026-07-28";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";

/// JSON-RPC error codes.
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const HEADER_MISMATCH: i64 = -32020;
    pub const MISSING_REQUIRED_CLIENT_CAPABILITY: i64 = -32021;
    pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
}

/// Result of a JSON-RPC method: a result value or `(code, message)`.
pub type RpcResult = Result<Value, (i64, String)>;

/// Build a JSON-RPC response object.
pub fn response(id: Value, result: RpcResult) -> Value {
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    }
}

/// A JSON-RPC error response with `data`.
pub fn error_response(id: Value, code: i64, message: &str, data: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message, "data": data } })
}

/// The protocol version a 2026-07-28+ request names in its `_meta`.
pub fn request_version(params: &Value) -> Option<&str> {
    params.get("_meta")?.get(META_VERSION)?.as_str()
}

/// A request as far as routing is concerned.
#[derive(Debug)]
pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

impl Request {
    /// Parse a message. Returns `None` for notifications and responses (no reply needed).
    pub fn parse(msg: &Value) -> Option<Self> {
        let id = msg.get("id")?.clone();
        let method = msg.get("method")?.as_str()?.to_owned();
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        Some(Self { id, method, params })
    }
}

/// The agent's display name (`clientInfo.title`, else `clientInfo.name`) from
/// `initialize` params or, since 2026-07-28, any request's `_meta`; reduced to
/// printable ASCII so it fits in a header.
pub fn client_name(params: &Value) -> Option<String> {
    let info = params
        .get("clientInfo")
        .or_else(|| params.get("_meta")?.get(META_CLIENT))?;
    let raw = ["title", "name"]
        .iter()
        .filter_map(|k| info.get(*k).and_then(Value::as_str))
        .find(|s| !s.trim().is_empty())?;
    let name: String = raw
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(60)
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn capabilities() -> Value {
    json!({
        "tools": { "listChanged": false },
        "prompts": { "listChanged": false },
        "extensions": { tasks::EXTENSION: {} },
    })
}

fn server_info() -> Value {
    json!({ "name": "pdfnotator", "title": "PDFnotator", "version": env!("CARGO_PKG_VERSION") })
}

/// `-32022` if the request names a protocol version we don't speak.
fn version_error(req: &Request) -> Option<Value> {
    let requested = request_version(&req.params)?;
    if PROTOCOL_VERSIONS.contains(&requested) {
        return None;
    }
    Some(error_response(
        req.id.clone(),
        code::UNSUPPORTED_PROTOCOL_VERSION,
        "Unsupported protocol version",
        json!({ "supported": PROTOCOL_VERSIONS, "requested": requested }),
    ))
}

/// Build the response; 2026-07-28 results carry `resultType`.
pub fn reply(req: &Request, result: RpcResult) -> Value {
    let stateless = request_version(&req.params) == Some(STATELESS_VERSION);
    let result = result.map(|mut v| {
        if stateless && let Some(obj) = v.as_object_mut() {
            obj.entry("resultType").or_insert_with(|| "complete".into());
        }
        v
    });
    response(req.id.clone(), result)
}

/// Answer a request that needs no viewer (or has an unsupported version).
/// `None` = needs the viewer.
pub fn answer_static(req: &Request) -> Option<Value> {
    version_error(req).or_else(|| handle_static(&req.method, &req.params).map(|r| reply(req, r)))
}

/// Handle methods that don't need the viewer. `None` = not a static method.
pub fn handle_static(method: &str, params: &Value) -> Option<RpcResult> {
    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = INITIALIZE_VERSIONS
                .iter()
                .find(|v| **v == requested)
                .copied()
                .unwrap_or(FALLBACK_VERSION);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": capabilities(),
                "serverInfo": server_info(),
                "instructions": text::INSTRUCTIONS,
            }))
        }
        "server/discover" => Ok(json!({
            "supportedVersions": PROTOCOL_VERSIONS,
            "capabilities": capabilities(),
            "instructions": text::INSTRUCTIONS,
            "ttlMs": 3_600_000,
            "cacheScope": "public",
            "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions_for(params) })),
        "prompts/list" => Ok(json!({ "prompts": [text::review_prompt_definition()] })),
        "prompts/get" => text::review_prompt(params),
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        _ => return None,
    };
    Some(result)
}

/// Handle one JSON-RPC message from the agent session `client`.
/// Returns `None` when no response is due.
pub async fn handle(session: &Arc<Session>, client: Option<&str>, msg: &Value) -> Option<Value> {
    if msg.get("id").is_none() && msg.get("method").and_then(Value::as_str) == Some("notifications/cancelled")
    {
        if let Some(request_id) = msg.pointer("/params/requestId") {
            session.cancel_call(client, request_id);
        }
        return None;
    }
    let req = Request::parse(msg)?;
    if let Some(resp) = answer_static(&req) {
        return Some(resp);
    }
    let result = match req.method.as_str() {
        "tools/call" if tools::runs_as_task(&req.params) => Ok(start_task(session, client, &req.params)),
        "tools/call" => {
            let call = session.begin_call(client, &req.id);
            Ok(tools::call(session, client, &req.params, &call.cancel).await)
        }
        "tasks/get" | "tasks/update" | "tasks/cancel" => return Some(task_request(session, &req)),
        _ => Err((
            code::METHOD_NOT_FOUND,
            format!("method not found: {}", req.method),
        )),
    };
    Some(reply(&req, result))
}

/// Run a long tool call as a task and return its `CreateTaskResult`.
fn start_task(session: &Arc<Session>, client: Option<&str>, params: &Value) -> Value {
    let params = tools::task_params(params);
    let (id, cancel, created) = session.tasks.create(tools::task_duration(&params));
    let (session, client) = (session.clone(), client.map(str::to_owned));
    tokio::spawn(async move {
        let mut result = tools::call(&session, client.as_deref(), &params, &cancel).await;
        result["resultType"] = "complete".into();
        session.tasks.complete(&id, result);
    });
    created
}

/// `tasks/get`, `tasks/update` and `tasks/cancel`.
fn task_request(session: &Session, req: &Request) -> Value {
    if !tasks::declared(&req.params) {
        return tasks::missing_capability(req.id.clone());
    }
    let id = tasks::task_id(&req.params).unwrap_or_default();
    let known = match req.method.as_str() {
        "tasks/get" => match session.tasks.get(id) {
            Some(task) => return response(req.id.clone(), Ok(task)),
            None => false,
        },
        "tasks/cancel" => session.tasks.cancel(id),
        // wait_for_feedback never asks for input; nothing to update
        _ => session.tasks.contains(id),
    };
    let result = if known {
        Ok(json!({ "resultType": "complete" }))
    } else {
        Err((
            code::INVALID_PARAMS,
            format!("task {id:?} not found (unknown or expired)"),
        ))
    };
    response(req.id.clone(), result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(method: &str, params: Value) -> Value {
        handle_static(method, &params).unwrap().unwrap()
    }

    #[test]
    fn reads_the_client_name() {
        let named = |info: Value| client_name(&json!({ "clientInfo": info }));
        assert_eq!(
            named(json!({ "name": "copilot", "title": "GitHub Copilot" })).as_deref(),
            Some("GitHub Copilot")
        );
        assert_eq!(
            named(json!({ "name": "claude-code" })).as_deref(),
            Some("claude-code")
        );
        assert_eq!(
            named(json!({ "name": "Visual Studio Code ✓" })).as_deref(),
            Some("Visual Studio Code")
        );
        assert_eq!(named(json!({ "name": " " })), None);
        assert_eq!(client_name(&json!({})), None);
    }

    #[test]
    fn negotiates_protocol_version() {
        assert_eq!(
            call("initialize", json!({ "protocolVersion": "2025-03-26" }))["protocolVersion"],
            "2025-03-26"
        );
        assert_eq!(
            call("initialize", json!({ "protocolVersion": "1999-01-01" }))["protocolVersion"],
            FALLBACK_VERSION
        );
        let init = call("initialize", json!({}));
        assert!(
            init["instructions"]
                .as_str()
                .unwrap()
                .contains("open in pdfnotator")
        );
        assert!(init["capabilities"]["prompts"].is_object());
    }

    #[test]
    fn lists_tools_with_schemas() {
        let tools = call("tools/list", json!({}));
        let names: Vec<_> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "open_pdf",
                "wait_for_feedback",
                "get_comments",
                "reply",
                "viewer_status"
            ]
        );
        for tool in tools["tools"].as_array().unwrap() {
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        }
    }

    #[test]
    fn review_prompt_uses_file_argument() {
        let p = call(
            "prompts/get",
            json!({ "name": "review", "arguments": { "file": "/a/b.typ" } }),
        );
        assert!(
            p["messages"][0]["content"]["text"]
                .as_str()
                .unwrap()
                .contains("`/a/b.typ`")
        );
        let p = call("prompts/get", json!({ "name": "review" }));
        assert!(
            p["messages"][0]["content"]["text"]
                .as_str()
                .unwrap()
                .contains("the Typst document")
        );
        assert!(
            handle_static("prompts/get", &json!({ "name": "nope" }))
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn notifications_and_responses_are_not_requests() {
        assert!(
            Request::parse(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none()
        );
        assert!(Request::parse(&json!({ "jsonrpc": "2.0", "id": 1, "result": {} })).is_none());
    }

    #[tokio::test]
    async fn unknown_method_is_an_error() {
        let s = Session::new(0);
        let r = handle(&s, None, &json!({ "jsonrpc": "2.0", "id": 7, "method": "nope" }))
            .await
            .unwrap();
        assert_eq!(r["id"], 7);
        assert_eq!(r["error"]["code"], code::METHOD_NOT_FOUND);
    }

    /// Open a PDF for `client` and start a blocking `wait_for_feedback` as request `id`.
    async fn start_wait(
        s: &Arc<Session>,
        client: &'static str,
        id: i64,
    ) -> tokio::task::JoinHandle<Option<Value>> {
        let before = s.documents()[0].feedback.waits_started();
        let task = tokio::spawn({
            let s = s.clone();
            async move {
                let msg = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
                    "params": { "name": "wait_for_feedback", "arguments": {} } });
                handle(&s, Some(client), &msg).await
            }
        });
        while s.documents()[0].feedback.waits_started() == before {
            tokio::task::yield_now().await;
        }
        task
    }

    /// All text blocks of a tool result, joined.
    fn text_of(v: &Value) -> String {
        let blocks = v["result"]["content"].as_array().cloned().unwrap_or_default();
        blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Regression: a cancelled wait used to keep listening and swallow the next comments.
    #[tokio::test]
    async fn cancelled_wait_stops_and_keeps_comments_for_the_next_call() {
        let s = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        s.open(pdf.to_str().unwrap(), None).unwrap();
        let doc = s.documents()[0].clone();

        let waiting = start_wait(&s, "agent", 1).await;
        // another client cannot cancel it
        let cancel = |client| {
            let s = s.clone();
            async move {
                let msg = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1 } });
                assert!(handle(&s, Some(client), &msg).await.is_none());
            }
        };
        cancel("someone-else").await;
        assert_eq!(doc.feedback.listeners(), 1);
        cancel("agent").await;
        assert_eq!(text_of(&waiting.await.unwrap().unwrap()), "Cancelled.");
        assert_eq!(doc.feedback.listeners(), 0);

        doc.submit(vec![crate::feedback::item("c1")]);
        let next = start_wait(&s, "agent", 2).await;
        assert!(text_of(&next.await.unwrap().unwrap()).contains("[c1]"));
    }

    fn meta(tasks: bool) -> Value {
        let caps = if tasks {
            json!({ "extensions": { tasks::EXTENSION: {} } })
        } else {
            json!({})
        };
        json!({
            "io.modelcontextprotocol/protocolVersion": STATELESS_VERSION,
            "io.modelcontextprotocol/clientInfo": { "name": "claude-code", "version": "1" },
            tasks::META_CAPABILITIES: caps,
        })
    }

    #[test]
    fn discovery_advertises_tasks_and_versions() {
        let req = Request::parse(&json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover",
            "params": { "_meta": meta(false) } }))
        .unwrap();
        let r = answer_static(&req).unwrap();
        assert_eq!(r["result"]["resultType"], "complete");
        assert_eq!(r["result"]["supportedVersions"][0], STATELESS_VERSION);
        assert!(r["result"]["capabilities"]["extensions"][tasks::EXTENSION].is_object());
        // initialize-based clients never get the stateless revision
        assert_eq!(
            call("initialize", json!({ "protocolVersion": STATELESS_VERSION }))["protocolVersion"],
            FALLBACK_VERSION
        );
        // older clients' results stay as they were
        assert!(call("tools/list", json!({})).get("resultType").is_none());
        assert_eq!(
            client_name(&json!({ "_meta": meta(false) })).as_deref(),
            Some("claude-code")
        );
    }

    #[test]
    fn marks_the_wait_as_task_only_for_clients_that_declare_tasks() {
        let support = |tasks: bool| {
            let tools = call("tools/list", json!({ "_meta": meta(tasks) }));
            let tools = tools["tools"].as_array().unwrap().clone();
            let marked: Vec<_> = tools.iter().filter(|t| t.get("execution").is_some()).collect();
            assert!(marked.len() <= 1);
            marked
                .first()
                .map(|t| (t["name"].clone(), t["execution"]["taskSupport"].clone()))
        };
        assert_eq!(
            support(true),
            Some((json!("wait_for_feedback"), json!("required")))
        );
        assert_eq!(support(false), None);
    }

    #[test]
    fn rejects_unknown_protocol_versions() {
        let req = Request::parse(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list",
            "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2099-01-01" } } }))
        .unwrap();
        let r = answer_static(&req).unwrap();
        assert_eq!(r["error"]["code"], code::UNSUPPORTED_PROTOCOL_VERSION);
        assert_eq!(r["error"]["data"]["requested"], "2099-01-01");
    }

    #[tokio::test]
    async fn wait_runs_as_a_task_for_clients_that_declare_it() {
        let s = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        s.open(pdf.to_str().unwrap(), None).unwrap();
        let doc = s.documents()[0].clone();
        let rpc = |method: &str, params: Value| {
            let s = s.clone();
            let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
            async move { handle(&s, Some("agent"), &msg).await.unwrap() }
        };
        let wait =
            |tasks: bool| json!({ "name": "wait_for_feedback", "arguments": {}, "_meta": meta(tasks) });

        let created = rpc("tools/call", wait(true)).await;
        assert_eq!(created["result"]["resultType"], "task");
        assert_eq!(created["result"]["status"], "working");
        let id = created["result"]["taskId"].as_str().unwrap().to_owned();
        let get = |tasks: bool| json!({ "taskId": id, "_meta": meta(tasks) });

        assert_eq!(
            rpc("tasks/get", get(false)).await["error"]["code"],
            code::MISSING_REQUIRED_CLIENT_CAPABILITY
        );
        assert_eq!(
            rpc("tasks/get", json!({ "taskId": "nope", "_meta": meta(true) })).await["error"]["code"],
            code::INVALID_PARAMS
        );
        while doc.feedback.listeners() == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(rpc("tasks/get", get(true)).await["result"]["status"], "working");

        doc.submit(vec![crate::feedback::item("c1")]);
        let done = loop {
            let r = rpc("tasks/get", get(true)).await;
            if r["result"]["status"] != "working" {
                break r;
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(done["result"]["status"], "completed");
        assert_eq!(done["result"]["result"]["resultType"], "complete");
        assert!(done["result"]["result"]["content"].to_string().contains("[c1]"));

        // cancelling releases the user's comments for the next wait
        let id = rpc("tools/call", wait(true)).await["result"]["taskId"]
            .as_str()
            .unwrap()
            .to_owned();
        while doc.feedback.listeners() == 0 {
            tokio::task::yield_now().await;
        }
        let cancel = json!({ "taskId": id, "_meta": meta(true) });
        assert_eq!(
            rpc("tasks/cancel", cancel.clone()).await["result"]["resultType"],
            "complete"
        );
        while doc.feedback.listeners() > 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(rpc("tasks/get", cancel).await["result"]["status"], "cancelled");
    }

    /// Regression: a stale wait that was never cancelled must not take the comments.
    #[tokio::test]
    async fn newer_wait_takes_over_from_a_stale_one() {
        let s = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        s.open(pdf.to_str().unwrap(), None).unwrap();
        let doc = s.documents()[0].clone();

        let stale = start_wait(&s, "agent", 1).await;
        let fresh = tokio::spawn({
            let s = s.clone();
            async move {
                let msg = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": "wait_for_feedback", "arguments": {} } });
                handle(&s, Some("agent"), &msg).await
            }
        });
        assert!(text_of(&stale.await.unwrap().unwrap()).contains("took over"));
        while doc.feedback.waits_started() < 2 {
            tokio::task::yield_now().await;
        }
        doc.submit(vec![crate::feedback::item("c1")]);
        assert!(text_of(&fresh.await.unwrap().unwrap()).contains("[c1]"));
    }
}
