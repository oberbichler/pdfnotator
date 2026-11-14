//! Model Context Protocol (JSON-RPC 2.0) handling, transport independent.
//!
//! Used by the streamable-HTTP endpoint ([`crate::http`]) and the stdio bridge
//! ([`crate::bridge`]). Requests that do not need a running viewer are answered
//! by [`handle_static`], so the bridge can reply without launching the app.

mod text;
mod tools;

use std::sync::Arc;

use serde_json::{Value, json};

use crate::session::Session;

/// Protocol revisions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// Revision used when the client asks for one we don't know.
const FALLBACK_VERSION: &str = "2025-06-18";

/// JSON-RPC error codes.
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
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

/// Handle methods that don't need the viewer. `None` = not a static method.
pub fn handle_static(method: &str, params: &Value) -> Option<RpcResult> {
    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|v| **v == requested)
                .copied()
                .unwrap_or(FALLBACK_VERSION);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false }, "prompts": { "listChanged": false } },
                "serverInfo": { "name": "pdfnotator", "title": "PDFnotator", "version": env!("CARGO_PKG_VERSION") },
                "instructions": text::INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions() })),
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
    let result = match handle_static(&req.method, &req.params) {
        Some(result) => result,
        None if req.method == "tools/call" => {
            let call = session.begin_call(client, &req.id);
            Ok(tools::call(session, client, &req.params, &call.cancel).await)
        }
        None => Err((
            code::METHOD_NOT_FOUND,
            format!("method not found: {}", req.method),
        )),
    };
    Some(response(req.id, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(method: &str, params: Value) -> Value {
        handle_static(method, &params).unwrap().unwrap()
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
        s.open(pdf.to_str().unwrap(), None, Some("agent")).unwrap();
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

    /// Regression: a stale wait that was never cancelled must not take the comments.
    #[tokio::test]
    async fn newer_wait_takes_over_from_a_stale_one() {
        let s = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        s.open(pdf.to_str().unwrap(), None, Some("agent")).unwrap();
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
