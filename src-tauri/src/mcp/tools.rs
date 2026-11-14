//! MCP tools exposed to the agent.
//!
//! Several agents can work on several documents at once. A call is routed to a
//! document by its `path` argument, otherwise by the agent session that opened
//! it (see [`Session::resolve`]). Tool results always repeat the path so that
//! agents pass it along.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Notify;

use std::fmt::Write as _;

use super::text::{compile_note, details_content, feedback_content, text};
use crate::document::{Document, WaitEnd};
use crate::session::Session;

const DEFAULT_WAIT: Duration = Duration::from_secs(600);
const MAX_WAIT: Duration = Duration::from_secs(3600);

const PATH_PARAM: &str = "Document path from open_pdf; needed only when several documents are open.";

pub fn definitions() -> Value {
    json!([
        {
            "name": "open_pdf",
            "title": "Open in pdfnotator",
            "description": "Open a document in the pdfnotator viewer. Use when the user says 'open in pdfnotator' / \
                'öffne in pdfnotator' or wants to review a Typst/PDF document. Pass the .typ source: pdfnotator \
                compiles it on every save, so never run typst compile yourself. Then call wait_for_feedback.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the .typ source (preferred) or a .pdf" },
                    "root": { "type": "string", "description": "typst --root, only if the document imports files outside its folder" }
                },
                "required": ["path"]
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "wait_for_feedback",
            "title": "Wait for pdfnotator feedback",
            "description": "Block until the user comments in pdfnotator; returns the comments (compact: quote, page, \
                comment, small screenshot for areas and sketches — a sketch is drawn in red by the user and comes \
                with a description of each shape and its size in mm). Handle them, call reply, then call this again until the user \
                is done.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": PATH_PARAM },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Max wait (default 600; 0 = only fetch queued comments)"
                    }
                }
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "get_comments",
            "title": "Comment details",
            "description": "Full details of delivered comments: complete quote, page text around it (to locate \
                ambiguous quotes in the source), full area text and full-resolution screenshot (for sketches also without the sketch). Use only when the \
                compact feedback is not enough.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "description": "Comment ids, e.g. [\"c3\"]" },
                    "path": { "type": "string", "description": PATH_PARAM }
                },
                "required": ["ids"]
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "reply",
            "title": "Reply in pdfnotator",
            "description": "After editing: show a short message to the user and mark comment ids as done. Reports \
                compile errors — fix them before waiting again.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": PATH_PARAM },
                    "message": { "type": "string", "description": "One line, e.g. what you changed" },
                    "resolved_ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["message"]
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "viewer_status",
            "title": "pdfnotator status",
            "description": "Compile status, current page and queued comments of a document, plus all open documents.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": PATH_PARAM } }
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

/// Execute a `tools/call` request for the agent session `client`. Tool errors
/// are reported in the result (`isError: true`) as the MCP spec recommends.
pub async fn call(session: &Arc<Session>, client: Option<&str>, params: &Value, cancel: &Notify) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let path = args.get("path").and_then(Value::as_str);
    let result = match name {
        "open_pdf" => open_pdf(session, client, &args).await,
        "wait_for_feedback" => match session.resolve(path, client) {
            Ok(doc) => Ok(wait_for_feedback(&doc, &args, cancel).await),
            Err(e) => Err(e),
        },
        "reply" => match session.resolve(path, client) {
            Ok(doc) => Ok(reply(&doc, &args).await),
            Err(e) => Err(e),
        },
        "get_comments" => match session.resolve(path, client) {
            Ok(doc) => get_comments(&doc, &args),
            Err(e) => Err(e),
        },
        "viewer_status" => Ok(viewer_status(session, path, client)),
        _ => Err(format!("unknown tool: {name}")),
    };
    match result {
        Ok(content) => json!({ "content": content, "isError": false }),
        Err(e) => json!({ "content": [text(e)], "isError": true }),
    }
}

/// Path the agent should pass in follow-up calls.
fn doc_path(doc: &Document) -> String {
    doc.source()
        .or_else(|| doc.pdf())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

async fn open_pdf(session: &Arc<Session>, client: Option<&str>, args: &Value) -> Result<Vec<Value>, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("missing 'path'")?
        .to_owned();
    let root = args.get("root").and_then(Value::as_str).map(PathBuf::from);
    let client = client.map(str::to_owned);
    // `open` blocks while typst compiles for the first time.
    let session = Arc::clone(session);
    let (doc, opened) =
        tokio::task::spawn_blocking(move || session.open(&path, root.as_deref(), client.as_deref()))
            .await
            .map_err(|e| e.to_string())??;
    let how = match &opened.source {
        Some(src) => format!(
            "pdfnotator compiles {} itself (typst watch): just edit the source, the viewer updates automatically.",
            src.display()
        ),
        None => {
            "The viewer reloads automatically whenever this PDF changes — recompile it after edits.".into()
        }
    };
    Ok(vec![text(format!(
        "Opened {} in pdfnotator. {how}\nNEXT: call wait_for_feedback with path \"{}\" now and keep looping \
         (wait → edit → reply → wait) until the user says they are done. Always pass this path — the user may \
         have several documents open. Do not end your turn.{}",
        opened.pdf.display(),
        doc_path(&doc),
        compile_note(opened.compile_error.as_deref())
    ))])
}

async fn wait_for_feedback(doc: &Document, args: &Value, cancel: &Notify) -> Vec<Value> {
    let timeout = args
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_WAIT, Duration::from_secs)
        .min(MAX_WAIT);
    let items = match doc.wait_for_feedback(timeout, cancel).await {
        Ok(items) => items,
        Err(WaitEnd::Closed) => {
            return vec![text(format!(
                "The user closed {} in pdfnotator. The review is finished — stop calling wait_for_feedback.",
                doc_path(doc)
            ))];
        }
        Err(WaitEnd::Superseded) => {
            return vec![text(
                "A newer wait_for_feedback call for this document took over — ignore this result.",
            )];
        }
        Err(WaitEnd::Cancelled) => return vec![text("Cancelled.")],
        Err(WaitEnd::TimedOut) => Vec::new(),
    };
    feedback_content(&items, &doc.status(), &image_dir())
}

async fn reply(doc: &Document, args: &Value) -> Vec<Value> {
    let message = args.get("message").and_then(Value::as_str).unwrap_or_default();
    let resolved: Vec<String> = args
        .get("resolved_ids")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    doc.agent_message(message, &resolved);
    let error = doc.settle_compile().await;
    let ok = if doc.source().is_some() && error.is_none() {
        " Document compiles fine."
    } else {
        ""
    };
    vec![text(format!(
        "Shown to the user.{ok} Now call wait_for_feedback (path \"{}\") again.{}",
        doc_path(doc),
        compile_note(error.as_deref())
    ))]
}

fn get_comments(doc: &Document, args: &Value) -> Result<Vec<Value>, String> {
    let ids: Vec<String> = args
        .get("ids")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Err("pass the comment ids, e.g. {\"ids\": [\"c3\"]}".into());
    }
    let found = doc.delivered(&ids);
    if found.is_empty() {
        return Err(format!(
            "no delivered comments with ids {ids:?} for {}",
            doc_path(doc)
        ));
    }
    let mut content = details_content(&found, &image_dir());
    let missing: Vec<&String> = ids
        .iter()
        .filter(|id| !found.iter().any(|f| &f.id == *id))
        .collect();
    if !missing.is_empty() {
        content.push(text(format!("Unknown ids: {missing:?}")));
    }
    Ok(content)
}

/// Compact one-line-per-field status (cheaper than pretty JSON).
fn viewer_status(session: &Session, path: Option<&str>, client: Option<&str>) -> Vec<Value> {
    let mut out = String::new();
    if let Ok(doc) = session.resolve(path, client) {
        let st = doc.status();
        let _ = writeln!(out, "Document: {}", doc_path(&doc));
        let _ = writeln!(out, "User is on page {}/{}", st.current_page, st.page_count);
        let _ = writeln!(out, "Queued comments: {}", st.pending);
        let _ = writeln!(
            out,
            "Compiles: {}",
            if st.source.is_none() {
                "n/a (PDF is watched, not compiled by pdfnotator)"
            } else if st.compile_error.is_some() {
                "NO"
            } else {
                "yes"
            }
        );
        out.push_str(&compile_note(st.compile_error.as_deref()));
    }
    let open: Vec<String> = session.documents().iter().map(|d| doc_path(d)).collect();
    let _ = write!(out, "\nOpen documents ({}):", open.len());
    for p in open {
        let _ = write!(out, "\n- {p}");
    }
    vec![text(out.trim_start().to_owned())]
}

fn image_dir() -> PathBuf {
    std::env::temp_dir().join("pdfnotator")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{RecordingSink, name};
    use crate::feedback::item;

    /// Test shim: tool call without cancellation.
    async fn call(cancel: &Notify, session: &Arc<Session>, client: Option<&str>, params: &Value) -> Value {
        super::call(session, client, params, cancel).await
    }

    fn result_text(v: &Value) -> String {
        v["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn pdf(dir: &tempfile::TempDir, file: &str) -> PathBuf {
        let p = dir.path().join(file);
        std::fs::write(&p, b"%PDF").unwrap();
        p.canonicalize().unwrap()
    }

    #[tokio::test]
    async fn open_wait_reply_cycle() {
        let session = Session::new(0);
        let sink = Arc::new(RecordingSink::default());
        session.set_sink(sink.clone());
        let dir = tempfile::tempdir().unwrap();
        let file = pdf(&dir, "x.pdf");

        let r = call(
            &Notify::new(),
            &session,
            Some("a"),
            &json!({ "name": "open_pdf", "arguments": { "path": file } }),
        )
        .await;
        assert_eq!(r["isError"], false, "{r}");
        assert!(result_text(&r).contains(&format!("wait_for_feedback with path \"{}\"", file.display())));

        session.documents()[0].submit(vec![item("c1")]);
        let r = call(
            &Notify::new(),
            &session,
            Some("a"),
            &json!({ "name": "wait_for_feedback", "arguments": { "timeout_seconds": 0 } }),
        )
        .await;
        assert!(result_text(&r).contains("[c1] general"));

        let args = json!({ "name": "reply", "arguments": { "message": "done", "resolved_ids": ["c1"] } });
        let r = call(&Notify::new(), &session, Some("a"), &args).await;
        assert!(result_text(&r).contains("Shown to the user"));
        let msg = sink.last(name::AGENT_MESSAGE).unwrap();
        assert_eq!(msg["resolved"], json!(["c1"]));
        assert_eq!(msg["doc"], session.documents()[0].id);
    }

    #[tokio::test]
    async fn two_agents_two_documents() {
        let session = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (pdf(&dir, "a.pdf"), pdf(&dir, "b.pdf"));
        call(
            &Notify::new(),
            &session,
            Some("one"),
            &json!({ "name": "open_pdf", "arguments": { "path": a } }),
        )
        .await;
        call(
            &Notify::new(),
            &session,
            Some("two"),
            &json!({ "name": "open_pdf", "arguments": { "path": b } }),
        )
        .await;
        let docs = session.documents();
        docs[0].submit(vec![item("c1")]);
        docs[1].submit(vec![item("c2")]);

        let wait = json!({ "name": "wait_for_feedback", "arguments": { "timeout_seconds": 0 } });
        assert!(result_text(&call(&Notify::new(), &session, Some("two"), &wait).await).contains("[c2]"));
        assert!(result_text(&call(&Notify::new(), &session, Some("one"), &wait).await).contains("[c1]"));

        // an unknown client must say which document it means
        let r = call(&Notify::new(), &session, Some("three"), &wait).await;
        assert_eq!(r["isError"], true);
        assert!(result_text(&r).contains("pass `path`"));
        docs[1].submit(vec![item("c3")]);
        let by_path =
            json!({ "name": "wait_for_feedback", "arguments": { "timeout_seconds": 0, "path": b } });
        assert!(result_text(&call(&Notify::new(), &session, Some("three"), &by_path).await).contains("[c3]"));

        let r = call(
            &Notify::new(),
            &session,
            None,
            &json!({ "name": "viewer_status", "arguments": { "path": a } }),
        )
        .await;
        let status = result_text(&r);
        assert!(status.contains("Open documents (2)"), "{status}");
        assert!(status.contains(&format!("Document: {}", a.display())), "{status}");
    }

    #[tokio::test]
    async fn details_are_available_after_delivery() {
        let session = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let file = pdf(&dir, "x.pdf");
        call(
            &Notify::new(),
            &session,
            Some("a"),
            &json!({ "name": "open_pdf", "arguments": { "path": file } }),
        )
        .await;
        let mut long = item("c9");
        long.quote = Some("lorem ".repeat(200));
        long.context = Some("intro lorem outro".into());
        session.documents()[0].submit(vec![long]);

        let wait = json!({ "name": "wait_for_feedback", "arguments": { "timeout_seconds": 0 } });
        let compact = result_text(&call(&Notify::new(), &session, Some("a"), &wait).await);
        assert!(compact.contains("get_comments") && !compact.contains("intro lorem outro"));

        let get = json!({ "name": "get_comments", "arguments": { "ids": ["c9", "c404"] } });
        let full = result_text(&call(&Notify::new(), &session, Some("a"), &get).await);
        assert!(full.matches("lorem").count() >= 200);
        assert!(full.contains("intro lorem outro"));
        assert!(full.contains("Unknown ids: [\"c404\"]"));

        let r = call(
            &Notify::new(),
            &session,
            Some("a"),
            &json!({ "name": "get_comments", "arguments": {} }),
        )
        .await;
        assert_eq!(r["isError"], true);
    }

    #[tokio::test]
    async fn closed_document_ends_the_loop() {
        let session = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let file = pdf(&dir, "x.pdf");
        call(
            &Notify::new(),
            &session,
            Some("a"),
            &json!({ "name": "open_pdf", "arguments": { "path": file } }),
        )
        .await;
        let doc = session.documents()[0].clone();
        let waiter = {
            let session = session.clone();
            tokio::spawn(async move {
                call(
                    &Notify::new(),
                    &session,
                    Some("a"),
                    &json!({ "name": "wait_for_feedback", "arguments": {} }),
                )
                .await
            })
        };
        while doc.feedback.listeners() == 0 {
            tokio::task::yield_now().await;
        }
        session.close(doc.id);
        assert!(result_text(&waiter.await.unwrap()).contains("stop calling wait_for_feedback"));
    }

    #[tokio::test]
    async fn tool_errors_are_reported_in_result() {
        let session = Session::new(0);
        let r = call(
            &Notify::new(),
            &session,
            None,
            &json!({ "name": "open_pdf", "arguments": {} }),
        )
        .await;
        assert_eq!(r["isError"], true);
        let r = call(
            &Notify::new(),
            &session,
            None,
            &json!({ "name": "open_pdf", "arguments": { "path": "relative.pdf" } }),
        )
        .await;
        assert!(result_text(&r).contains("absolute"));
        let r = call(
            &Notify::new(),
            &session,
            None,
            &json!({ "name": "wait_for_feedback", "arguments": {} }),
        )
        .await;
        assert!(result_text(&r).contains("No document is open"));
        let r = call(&Notify::new(), &session, None, &json!({ "name": "frobnicate" })).await;
        assert!(result_text(&r).contains("unknown tool"));
    }
}
