//! MCP tools exposed to the agent.
//!
//! Several agents can work on several documents at once. A call is routed to a
//! document by its `path` argument (see [`Session::resolve`]), never by the
//! agent session: chats may share one MCP connection. Tool results always
//! repeat the path so that agents pass it along.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Notify;

use std::fmt::Write as _;

use super::text::{compile_note, details_content, feedback_content, quit_note, text};
use crate::document::{Document, WaitEnd, Waiter};
use crate::session::Session;

/// Below the ~60 s after which MCP clients commonly abort a tool call: an
/// aborted wait surfaces as an error, and agents then often stop the loop.
/// An empty result just tells them to call again.
const DEFAULT_WAIT: Duration = Duration::from_secs(50);
const MAX_WAIT: Duration = Duration::from_secs(3600);
/// Longest wait when running as an MCP task. The client polls, but Copilot
/// CLI fails a task that is still working after 600 s ("did not complete
/// before its deadline"), so finish before that with an empty result.
const TASK_WAIT: Duration = Duration::from_secs(540);

const PATH_PARAM: &str = "Document path, as returned by open_pdf";

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
                        "description": "Max wait in seconds (default 50; 0 = only fetch queued comments)"
                    }
                },
                "required": ["path"]
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
                "required": ["ids", "path"]
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
                "required": ["path", "message"]
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

/// `tools/list` for this request. Clients that declare the Tasks extension
/// always get `wait_for_feedback` as a task, so it is marked
/// `taskSupport: "required"` for them: Copilot CLI only accepts a task
/// result for tools marked that way. Everyone else sees the plain tool.
pub fn definitions_for(params: &Value) -> Value {
    let mut tools = definitions();
    if super::tasks::declared(params)
        && let Some(wait) = tools
            .as_array_mut()
            .and_then(|t| t.iter_mut().find(|t| t["name"] == "wait_for_feedback"))
    {
        wait["execution"] = json!({ "taskSupport": "required" });
    }
    tools
}

/// Whether this `tools/call` runs as an MCP task: only the long
/// `wait_for_feedback`, and only for clients that declared the extension.
pub fn runs_as_task(params: &Value) -> bool {
    params.get("name").and_then(Value::as_str) == Some("wait_for_feedback") && super::tasks::declared(params)
}

/// Call params for a task: without a timeout of its own the wait is long,
/// but never longer than [`TASK_WAIT`].
pub fn task_params(params: &Value) -> Value {
    let mut params = params.clone();
    let requested = params
        .pointer("/arguments/timeout_seconds")
        .and_then(Value::as_u64);
    if !params.get("arguments").is_some_and(Value::is_object) {
        params["arguments"] = json!({});
    }
    params["arguments"]["timeout_seconds"] = requested
        .map_or(TASK_WAIT.as_secs(), |s| s.min(TASK_WAIT.as_secs()))
        .into();
    params
}

/// Longest time the (task) call can take.
pub fn task_duration(params: &Value) -> Duration {
    params
        .pointer("/arguments/timeout_seconds")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_WAIT, Duration::from_secs)
        .min(MAX_WAIT)
}

/// Execute a `tools/call` request for the agent session `client`. Tool errors
/// are reported in the result (`isError: true`) as the MCP spec recommends.
pub async fn call(session: &Arc<Session>, client: Option<&str>, params: &Value, cancel: &Notify) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let path = args.get("path").and_then(Value::as_str);
    let result = match name {
        "open_pdf" => open_pdf(session, &args).await,
        "wait_for_feedback" => match resolve_or_reopen(session, path).await {
            Ok(doc) => {
                let waiter = Waiter {
                    client: client.map(str::to_owned),
                    name: session.agent_name(client),
                };
                Ok(wait_for_feedback(&doc, &args, cancel, waiter).await)
            }
            Err(e) => Err(e),
        },
        "reply" => match resolve_or_reopen(session, path).await {
            Ok(doc) => Ok(reply(&doc, &args).await),
            Err(e) => Err(e),
        },
        "get_comments" => match resolve_or_reopen(session, path).await {
            Ok(doc) => get_comments(&doc, &args),
            Err(e) => Err(e),
        },
        "viewer_status" => Ok(viewer_status(session, path)),
        _ => Err(format!("unknown tool: {name}")),
    };
    match result {
        Ok(content) => json!({ "content": content, "isError": false }),
        Err(e) => json!({ "content": [text(e)], "isError": true }),
    }
}

/// The document for `path`; if pdfnotator was restarted (or the tab closed by
/// a crash) and the file still exists, it is opened again so the agent's loop
/// survives instead of failing with "not open".
async fn resolve_or_reopen(session: &Arc<Session>, path: Option<&str>) -> Result<Arc<Document>, String> {
    let err = match session.resolve(path) {
        Ok(doc) => return Ok(doc),
        Err(e) => e,
    };
    let Some(raw) = path.filter(|p| std::path::Path::new(p).is_file()) else {
        return Err(err);
    };
    let (session, raw) = (Arc::clone(session), raw.to_owned());
    tokio::task::spawn_blocking(move || session.open(&raw, None))
        .await
        .map_err(|e| e.to_string())?
        .map(|(doc, _)| doc)
}

/// Path the agent should pass in follow-up calls.
fn doc_path(doc: &Document) -> String {
    doc.source()
        .or_else(|| doc.pdf())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

async fn open_pdf(session: &Arc<Session>, args: &Value) -> Result<Vec<Value>, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("missing 'path'")?
        .to_owned();
    let root = args.get("root").and_then(Value::as_str).map(PathBuf::from);
    // `open` blocks while typst compiles for the first time.
    let session = Arc::clone(session);
    let (doc, opened) = tokio::task::spawn_blocking(move || session.open(&path, root.as_deref()))
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
         (wait → edit → reply → wait) until the user says they are done. Pass this exact path to every call — \
         the user may have several documents open with other agents. Do not end your turn.{}",
        opened.pdf.display(),
        doc_path(&doc),
        compile_note(opened.compile_error.as_deref())
    ))])
}

async fn wait_for_feedback(doc: &Document, args: &Value, cancel: &Notify, waiter: Waiter) -> Vec<Value> {
    let timeout = args
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_WAIT, Duration::from_secs)
        .min(MAX_WAIT);
    let items = match doc.wait_for_feedback_as(timeout, cancel, waiter).await {
        Ok(items) => items,
        Err(WaitEnd::Quit) => return vec![text(quit_note(&doc_path(doc)))],
        Err(WaitEnd::Closed) => {
            return vec![text(format!(
                "The user closed {} in pdfnotator. The review is finished — stop calling wait_for_feedback.",
                doc_path(doc)
            ))];
        }
        Err(WaitEnd::Superseded { by: None }) => {
            return vec![text(
                "A newer wait_for_feedback call for this document took over — ignore this result.",
            )];
        }
        Err(WaitEnd::Superseded { by: Some(agent) }) => {
            return vec![text(format!(
                "{agent} is now waiting for feedback on {} — it took this document over. Stop calling \
                 wait_for_feedback for it and tell the user, unless they ask you to take it back.",
                doc_path(doc)
            ))];
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
fn viewer_status(session: &Session, path: Option<&str>) -> Vec<Value> {
    let mut out = String::new();
    if let Ok(doc) = session.resolve(path) {
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

    #[test]
    fn task_waits_end_before_copilots_deadline() {
        let timeout = |args: Value| {
            task_params(&json!({ "name": "wait_for_feedback", "arguments": args }))["arguments"]["timeout_seconds"]
                .as_u64()
                .unwrap()
        };
        assert_eq!(timeout(json!({})), 540);
        assert_eq!(timeout(json!({ "timeout_seconds": 3600 })), 540);
        assert_eq!(timeout(json!({ "timeout_seconds": 30 })), 30);
        assert_eq!(
            task_params(&json!({ "name": "wait_for_feedback" }))["arguments"]["timeout_seconds"],
            540
        );
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
            &json!({ "name": "wait_for_feedback", "arguments": { "timeout_seconds": 0, "path": file } }),
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

        let wait = |p: &PathBuf| json!({ "name": "wait_for_feedback", "arguments": { "timeout_seconds": 0, "path": p } });
        // chats sharing one connection ("same") still reach their own document
        assert!(result_text(&call(&Notify::new(), &session, Some("same"), &wait(&b)).await).contains("[c2]"));
        assert!(result_text(&call(&Notify::new(), &session, Some("same"), &wait(&a)).await).contains("[c1]"));

        // without a path there is no guessing, not even for the agent that opened a document
        let no_path = json!({ "name": "wait_for_feedback", "arguments": { "timeout_seconds": 0 } });
        let r = call(&Notify::new(), &session, Some("one"), &no_path).await;
        assert_eq!(r["isError"], true);
        assert!(result_text(&r).contains("pass `path`"));

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
    async fn a_restarted_viewer_reopens_the_document() {
        let session = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        let file = pdf(&dir, "x.pdf");
        // pdfnotator was restarted: the agent still waits with the path it got from open_pdf
        assert!(session.documents().is_empty());
        let wait =
            json!({ "name": "wait_for_feedback", "arguments": { "path": file, "timeout_seconds": 0 } });
        let r = call(&Notify::new(), &session, Some("a"), &wait).await;
        assert_eq!(r["isError"], false, "{r}");
        assert!(result_text(&r).contains("Keep waiting"), "{r}");
        assert!(result_text(&r).contains(&format!("path \"{}\"", file.display())));
        assert_eq!(session.documents().len(), 1, "the tab is back");

        let missing = dir.path().join("gone.pdf");
        let r = call(
            &Notify::new(),
            &session,
            Some("a"),
            &json!({ "name": "wait_for_feedback", "arguments": { "path": missing, "timeout_seconds": 0 } }),
        )
        .await;
        assert_eq!(r["isError"], true, "a missing file is not reopened");
    }

    #[tokio::test]
    async fn quitting_the_app_does_not_end_the_review() {
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
            let (session, file) = (session.clone(), file.clone());
            tokio::spawn(async move {
                let wait = json!({ "name": "wait_for_feedback", "arguments": { "path": file } });
                call(&Notify::new(), &session, Some("a"), &wait).await
            })
        };
        while doc.feedback.listeners() == 0 {
            tokio::task::yield_now().await;
        }
        session.shutdown(); // app quits (update, Cmd+Q, logout)
        let text = result_text(&waiter.await.unwrap());
        assert!(
            text.contains("was quit") && text.contains("Call wait_for_feedback again"),
            "{text}"
        );
        assert!(!text.contains("stop calling"), "{text}");
    }

    #[tokio::test]
    async fn another_agent_taking_over_is_reported() {
        let session = Session::new(0);
        session.set_agent_name("cli", "GitHub Copilot CLI");
        let dir = tempfile::tempdir().unwrap();
        let file = pdf(&dir, "x.pdf");
        call(
            &Notify::new(),
            &session,
            Some("vscode"),
            &json!({ "name": "open_pdf", "arguments": { "path": file } }),
        )
        .await;
        let doc = session.documents()[0].clone();
        let wait = json!({ "name": "wait_for_feedback", "arguments": { "path": file } });
        let first = {
            let (session, wait) = (session.clone(), wait.clone());
            tokio::spawn(async move { call(&Notify::new(), &session, Some("vscode"), &wait).await })
        };
        while doc.feedback.listeners() == 0 {
            tokio::task::yield_now().await;
        }
        let second = {
            let (session, wait) = (session.clone(), wait.clone());
            tokio::spawn(async move { call(&Notify::new(), &session, Some("cli"), &wait).await })
        };
        let text = result_text(&first.await.unwrap());
        assert!(text.contains("GitHub Copilot CLI is now waiting"), "{text}");
        assert!(text.contains("Stop calling wait_for_feedback"), "{text}");
        assert_eq!(doc.status().agents, vec!["GitHub Copilot CLI".to_owned()]);
        doc.submit(vec![item("c1")]);
        assert!(result_text(&second.await.unwrap()).contains("[c1]"));
        assert_eq!(doc.status().agents, Vec::<String>::new());
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
