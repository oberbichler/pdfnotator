//! `pdfnotator --mcp`: stdio MCP server for agents (VS Code Copilot, Copilot CLI,
//! Claude Code, …).
//!
//! Static requests (`initialize`, `server/discover`, `tools/list`, …) are
//! answered locally, so the agent can connect without the viewer running. Only
//! tool calls are forwarded to the viewer's HTTP endpoint; the viewer is
//! launched for the first one. MCP task requests (`tasks/*`) go to a running
//! viewer only: if it was quit, the bridge answers for the tasks it lost.

use std::io::{BufRead, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::mcp::{self, Request};

const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);
/// How often a running tool call reports progress (when the client asked for
/// it): clients that reset their timeout on progress keep long waits alive.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(15);

/// Run the bridge on stdin/stdout until stdin closes.
pub fn run(port: u16) {
    let bridge = Arc::new(Bridge::new(port));
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let write = move |v: &Value| {
        let mut out = stdout.lock().unwrap();
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    };
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        bridge.note_client(&line);
        match Bridge::route(&line) {
            Route::Done(Some(resp)) => write(&resp),
            Route::Done(None) => {}
            Route::Task(req) => {
                let bridge = bridge.clone();
                let write = write.clone();
                std::thread::spawn(move || write(&bridge.task_request(&line, &req)));
            }
            Route::Forward(req) => {
                // tool calls may block for minutes (wait_for_feedback) → own thread
                let bridge = bridge.clone();
                let write = write.clone();
                std::thread::spawn(move || {
                    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    if let Some(token) = req.params.pointer("/_meta/progressToken").cloned() {
                        let (done, write) = (done.clone(), write.clone());
                        std::thread::spawn(move || keep_alive(&token, &done, PROGRESS_INTERVAL, write));
                    }
                    let resp = bridge.forward(&line, req.id.clone());
                    bridge.track_task(&req, &resp);
                    done.store(true, std::sync::atomic::Ordering::SeqCst);
                    // the client must not get a response for a call it cancelled
                    if !bridge.take_cancelled(&req.id) {
                        write(&resp);
                    }
                });
            }
            Route::Cancel(request_id) => {
                bridge.mark_cancelled(&request_id);
                let bridge = bridge.clone();
                std::thread::spawn(move || bridge.notify_viewer(&line));
            }
        }
    }
}

/// Send `notifications/progress` for `token` until `done`.
fn keep_alive(
    token: &Value,
    done: &std::sync::atomic::AtomicBool,
    interval: Duration,
    write: impl Fn(&Value),
) {
    let mut ticks = 0u64;
    loop {
        // sleep in short steps so the thread ends soon after the call does
        for _ in 0..interval.as_millis() / 50 {
            if done.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        ticks += 1;
        write(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": { "progressToken": token, "progress": ticks, "message": "waiting for the user" }
        }));
    }
}

/// What to do with one incoming line.
#[derive(Debug)]
pub enum Route {
    /// Answered locally (or nothing to answer).
    Done(Option<Value>),
    /// Needs the viewer.
    Forward(Request),
    /// `tasks/get` / `tasks/update` / `tasks/cancel`: needs a running viewer.
    Task(Request),
    /// `notifications/cancelled` for this request id: stop it in the viewer too.
    Cancel(Value),
}

#[derive(Debug)]
pub struct Bridge {
    port: u16,
    http: ureq::Agent,
    launch: Mutex<()>,
    /// Identifies this agent session towards the viewer (one bridge process per session).
    client_id: String,
    /// Request ids the client cancelled (their responses are dropped).
    cancelled: Mutex<std::collections::HashSet<String>>,
    /// The agent's name from `initialize` (`clientInfo`), passed on so the viewer can show it.
    agent: Mutex<Option<String>>,
    /// MCP tasks the viewer created for this client: task id → (document path,
    /// `createdAt`), to answer for them if the viewer quits.
    tasks: Mutex<std::collections::HashMap<String, (String, String)>>,
}

impl Bridge {
    pub fn new(port: u16) -> Self {
        // no global timeout: wait_for_feedback legitimately blocks for minutes
        let http = ureq::Agent::config_builder()
            .timeout_global(None)
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            port,
            http,
            launch: Mutex::new(()),
            client_id: uuid::Uuid::new_v4().to_string(),
            cancelled: Mutex::default(),
            agent: Mutex::default(),
            tasks: Mutex::default(),
        }
    }

    /// Remember the agent's name when the client introduces itself (in
    /// `initialize`, or since 2026-07-28 in every request's `_meta`).
    fn note_client(&self, line: &str) {
        if !line.contains("clientInfo") {
            return;
        }
        if let Ok(msg) = serde_json::from_str::<Value>(line)
            && let Some(name) = mcp::client_name(msg.get("params").unwrap_or(&Value::Null))
        {
            *self.agent.lock().unwrap() = Some(name);
        }
    }

    pub fn route(line: &str) -> Route {
        if line.trim().is_empty() {
            return Route::Done(None);
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return Route::Done(Some(mcp::response(
                Value::Null,
                Err((mcp::code::PARSE_ERROR, "parse error".into())),
            )));
        };
        if msg.get("method").and_then(Value::as_str) == Some("notifications/cancelled")
            && let Some(id) = msg.pointer("/params/requestId")
        {
            return Route::Cancel(id.clone());
        }
        let Some(req) = Request::parse(&msg) else {
            return Route::Done(None);
        };
        if let Some(resp) = mcp::answer_static(&req) {
            return Route::Done(Some(resp));
        }
        match req.method.as_str() {
            // only tool calls need the viewer; anything else must not launch it
            // (clients send e.g. `logging/setLevel` when a session starts)
            "tools/call" => Route::Forward(req),
            "tasks/get" | "tasks/update" | "tasks/cancel" => Route::Task(req),
            _ => Route::Done(Some(mcp::response(
                req.id,
                Err((
                    mcp::code::METHOD_NOT_FOUND,
                    format!("method not found: {}", req.method),
                )),
            ))),
        }
    }

    /// Remember a task the viewer created for a forwarded tool call.
    fn track_task(&self, req: &Request, resp: &Value) {
        if resp.pointer("/result/resultType").and_then(Value::as_str) != Some("task") {
            return;
        }
        let field = |p: &str| {
            resp.pointer(p)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let path = req
            .params
            .pointer("/arguments/path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        self.tasks
            .lock()
            .unwrap()
            .insert(field("/result/taskId"), (path, field("/result/createdAt")));
    }

    /// Pass a `tasks/*` request to a running viewer. Never launches it: a
    /// freshly started viewer would not know the task anyway.
    pub fn task_request(&self, line: &str, req: &Request) -> Value {
        if !mcp::tasks::declared(&req.params) {
            return mcp::tasks::missing_capability(req.id.clone());
        }
        let id = mcp::tasks::task_id(&req.params).unwrap_or_default();
        if self.alive()
            && let Ok(resp) = self.post(line)
            && resp.pointer("/error/code").and_then(Value::as_i64) != Some(mcp::code::INVALID_PARAMS)
        {
            let finished = req.method == "tasks/cancel"
                || resp
                    .pointer("/result/status")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s != "working" && s != "input_required");
            if finished {
                self.tasks.lock().unwrap().remove(id);
            }
            return resp;
        }
        self.lost_task(req, id)
    }

    /// Answer for a task whose viewer is gone: the wait ended like any wait
    /// interrupted by quitting, so the agent calls `wait_for_feedback` again.
    fn lost_task(&self, req: &Request, id: &str) -> Value {
        let Some((path, created)) = self.tasks.lock().unwrap().remove(id) else {
            return mcp::response(
                req.id.clone(),
                Err((
                    mcp::code::INVALID_PARAMS,
                    format!("task {id:?} not found (unknown or expired)"),
                )),
            );
        };
        if req.method != "tasks/get" {
            return mcp::response(req.id.clone(), Ok(json!({ "resultType": "complete" })));
        }
        let now = mcp::tasks::rfc3339(std::time::SystemTime::now());
        mcp::response(
            req.id.clone(),
            Ok(json!({
                "resultType": "complete",
                "taskId": id,
                "status": "completed",
                "createdAt": if created.is_empty() { now.clone() } else { created },
                "lastUpdatedAt": now,
                "ttlMs": 0,
                "result": {
                    "resultType": "complete",
                    "content": [{ "type": "text", "text": mcp::text::quit_note(&path) }],
                    "isError": false
                }
            })),
        )
    }

    fn mark_cancelled(&self, id: &Value) {
        self.cancelled.lock().unwrap().insert(id.to_string());
    }

    /// Was this request cancelled? (Forgets the id.)
    fn take_cancelled(&self, id: &Value) -> bool {
        self.cancelled.lock().unwrap().remove(&id.to_string())
    }

    /// Pass a notification on to a running viewer (never launches it).
    fn notify_viewer(&self, line: &str) {
        if self.alive() {
            let _ = self.post_raw(line);
        }
    }

    /// Forward a raw request to the viewer, launching it if necessary.
    pub fn forward(&self, line: &str, id: Value) -> Value {
        let attempt = || {
            {
                let _launching = self.launch.lock().unwrap();
                self.ensure_viewer()
            }
            .and_then(|()| self.post(line))
        };
        // The viewer may quit mid-call (update, restart). Calls that only read
        // are retried once against a freshly started viewer.
        let result = attempt().or_else(|e| {
            if retry_safe(line) {
                std::thread::sleep(Duration::from_millis(500));
                attempt()
            } else {
                Err(e)
            }
        });
        result.unwrap_or_else(|e| {
            mcp::response(
                id,
                Ok(json!({
                    "content": [{ "type": "text", "text": format!("pdfnotator viewer not reachable: {e}") }],
                    "isError": true
                })),
            )
        })
    }

    fn post(&self, body: &str) -> Result<Value, String> {
        let mut resp = self.post_raw(body)?;
        if resp.status() != 200 {
            return Err(format!("viewer answered HTTP {}", resp.status()));
        }
        resp.body_mut().read_json::<Value>().map_err(|e| e.to_string())
    }

    fn post_raw(&self, body: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
        let url = format!("http://127.0.0.1:{}/mcp", self.port);
        // copy the name: a guard held across `send` would block every other
        // request while a wait_for_feedback is in flight
        let agent = self.agent.lock().unwrap().clone().unwrap_or_default();
        self.http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header(crate::http::SESSION_HEADER, &self.client_id)
            .header(crate::http::AGENT_HEADER, &agent)
            .send(body)
            .map_err(|e| e.to_string())
    }

    fn alive(&self) -> bool {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
    }

    fn ensure_viewer(&self) -> Result<(), String> {
        if self.alive() {
            return Ok(());
        }
        launch_viewer()?;
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        while Instant::now() < deadline {
            if self.alive() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        Err(format!(
            "viewer did not start within {} s",
            LAUNCH_TIMEOUT.as_secs()
        ))
    }
}

/// Tool calls that can run twice without side effects.
fn retry_safe(line: &str) -> bool {
    serde_json::from_str::<Value>(line).is_ok_and(|msg| {
        matches!(
            msg.pointer("/params/name").and_then(Value::as_str),
            Some("wait_for_feedback" | "get_comments" | "viewer_status")
        )
    })
}

/// Start the GUI. Inside a macOS bundle `LaunchServices` is used so the app
/// behaves like a normally started application.
fn launch_viewer() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.to_string_lossy();
    if cfg!(target_os = "macos")
        && let Some(idx) = exe_str.find(".app/Contents/MacOS/")
    {
        let bundle = &exe_str[..idx + ".app".len()];
        let mut open = std::process::Command::new("open");
        // LaunchServices does not inherit our environment; forward the settings explicitly
        for var in ["PDFNOTATOR_PORT", "PDFNOTATOR_TYPST"] {
            if let Ok(value) = std::env::var(var) {
                open.arg("--env").arg(format!("{var}={value}"));
            }
        }
        return open
            .arg(bundle)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(format!("open {bundle} failed"))
                }
            });
    }
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // keep running when the agent stops the MCP server
    }
    cmd.spawn()
        .map(drop)
        .map_err(|e| format!("failed to launch viewer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    #[test]
    fn answers_static_requests_locally() {
        let Route::Done(Some(r)) = Bridge::route(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#) else {
            panic!("expected local answer")
        };
        assert_eq!(r["id"], 1);
        assert!(r["result"]["tools"].is_array());
        assert!(matches!(
            Bridge::route(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Route::Done(None)
        ));
        assert!(matches!(Bridge::route(""), Route::Done(None)));
        let Route::Done(Some(err)) = Bridge::route("{not json") else {
            panic!()
        };
        assert_eq!(err["error"]["code"], mcp::code::PARSE_ERROR);
        assert!(matches!(
            Bridge::route(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"viewer_status"}}"#
            ),
            Route::Forward(_)
        ));
    }

    const TASKS_META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}}}"#;

    #[test]
    fn answers_discovery_locally_and_routes_tasks_to_the_viewer() {
        let Route::Done(Some(r)) = Bridge::route(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{{{TASKS_META}}}}}"#
        )) else {
            panic!("discovery must not launch the viewer")
        };
        assert_eq!(r["result"]["resultType"], "complete");
        assert!(r["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"].is_object());
        let get = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tasks/get","params":{{"taskId":"x",{TASKS_META}}}}}"#
        );
        assert!(matches!(Bridge::route(&get), Route::Task(_)));
    }

    /// End to end: `wait_for_feedback` as a task, polled through the bridge; when
    /// the viewer is gone the bridge tells the agent to wait again.
    #[test]
    fn task_wait_is_polled_and_survives_the_viewer() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        let session = Session::new(port);
        let server = rt.spawn({
            let session = session.clone();
            async move { axum::serve(listener, crate::http::router(session)).await }
        });
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        let path = pdf.to_str().unwrap().to_owned();
        session.open(&path, None).unwrap();
        let doc = session.documents()[0].clone();
        let bridge = Bridge::new(port);

        let call = |id: i64| {
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"wait_for_feedback","arguments":{{"path":{path:?}}},{TASKS_META}}}}}"#
            );
            let Route::Forward(req) = Bridge::route(&line) else {
                panic!()
            };
            let resp = bridge.forward(&line, req.id.clone());
            bridge.track_task(&req, &resp);
            resp
        };
        let task = |method: &str, id: &str| {
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":9,"method":"{method}","params":{{"taskId":"{id}",{TASKS_META}}}}}"#
            );
            let Route::Task(req) = Bridge::route(&line) else {
                panic!()
            };
            bridge.task_request(&line, &req)
        };

        let created = call(1);
        assert_eq!(created["result"]["resultType"], "task", "{created}");
        let id = created["result"]["taskId"].as_str().unwrap().to_owned();
        assert_eq!(task("tasks/get", &id)["result"]["status"], "working");

        doc.submit(vec![crate::feedback::item("c1")]);
        let deadline = Instant::now() + Duration::from_secs(5);
        let done = loop {
            let r = task("tasks/get", &id);
            if r["result"]["status"] != "working" || Instant::now() > deadline {
                break r;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(done["result"]["status"], "completed", "{done}");
        let text = done["result"]["result"]["content"].to_string();
        assert!(text.contains("[c1]"), "{text}");

        // a second wait, then the viewer quits for good
        let id = call(2)["result"]["taskId"].as_str().unwrap().to_owned();
        session.shutdown();
        server.abort();
        rt.shutdown_background();
        let deadline = Instant::now() + Duration::from_secs(5);
        while bridge.alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let lost = task("tasks/get", &id);
        assert_eq!(lost["result"]["status"], "completed", "{lost}");
        let text = lost["result"]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("was quit") && text.contains(&path), "{text}");
        assert_eq!(
            task("tasks/get", &id)["error"]["code"],
            mcp::code::INVALID_PARAMS,
            "answered once"
        );
    }

    #[test]
    fn unknown_methods_do_not_launch_the_viewer() {
        for method in ["logging/setLevel", "completion/complete", "frobnicate"] {
            let line = format!(r#"{{"jsonrpc":"2.0","id":3,"method":"{method}","params":{{}}}}"#);
            let Route::Done(Some(r)) = Bridge::route(&line) else {
                panic!("{method} must be answered locally")
            };
            assert_eq!(r["error"]["code"], mcp::code::METHOD_NOT_FOUND, "{method}");
        }
    }

    #[test]
    fn retries_only_calls_without_side_effects() {
        let call = |name: &str| {
            format!(r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{name}"}}}}"#)
        };
        assert!(retry_safe(&call("wait_for_feedback")));
        assert!(retry_safe(&call("get_comments")));
        assert!(!retry_safe(&call("reply")), "a retried reply could show twice");
        assert!(!retry_safe(&call("open_pdf")));
    }

    #[test]
    fn reports_progress_until_the_call_ends() {
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let worker = {
            let (done, sent) = (done.clone(), sent.clone());
            std::thread::spawn(move || {
                keep_alive(&json!("tok"), &done, Duration::from_millis(300), |v: &Value| {
                    sent.lock().unwrap().push(v.clone());
                });
            })
        };
        // let one interval pass
        std::thread::sleep(Duration::from_millis(450));
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        worker.join().unwrap();
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["method"], "notifications/progress");
        assert_eq!(sent[0]["params"]["progressToken"], "tok");
    }

    #[test]
    fn recognises_cancellations() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7,"reason":"timeout"}}"#;
        assert!(matches!(Bridge::route(line), Route::Cancel(id) if id == 7));
        let b = Bridge::new(1);
        b.mark_cancelled(&json!(7));
        assert!(b.take_cancelled(&json!(7)));
        assert!(!b.take_cancelled(&json!(7)), "forgotten after use");
    }

    /// End to end: a cancel sent through the bridge stops the wait in the viewer.
    #[test]
    fn cancellation_reaches_the_viewer() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        let session = Session::new(port);
        rt.spawn({
            let session = session.clone();
            async move { axum::serve(listener, crate::http::router(session)).await }
        });
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        session.open(pdf.to_str().unwrap(), None).unwrap();
        let doc = session.documents()[0].clone();

        let bridge = Arc::new(Bridge::new(port));
        let wait = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"wait_for_feedback","arguments":{}}}"#;
        let waiter = std::thread::spawn({
            let bridge = bridge.clone();
            move || bridge.forward(wait, json!(3))
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while doc.feedback.listeners() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(doc.feedback.listeners(), 1);

        let cancel = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}"#;
        let Route::Cancel(id) = Bridge::route(cancel) else {
            panic!()
        };
        bridge.mark_cancelled(&id);
        bridge.notify_viewer(cancel);
        let resp = waiter.join().unwrap();
        assert_eq!(resp["result"]["content"][0]["text"], "Cancelled.");
        assert!(bridge.take_cancelled(&json!(3)), "response would be dropped");
        assert_eq!(doc.feedback.listeners(), 0);
    }

    #[test]
    fn forwards_tool_calls_to_running_viewer() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        let session = Session::new(port);
        rt.spawn({
            let session = session.clone();
            async move { axum::serve(listener, crate::http::router(session)).await }
        });

        let b = Bridge::new(port);
        b.note_client(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"copilot-cli","title":"GitHub Copilot CLI"}}}"#,
        );
        let line = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"viewer_status"}}"#;
        let Route::Forward(req) = Bridge::route(line) else {
            panic!()
        };
        let resp = b.forward(line, req.id);
        assert_eq!(resp["id"], 5);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Open documents (0)"), "{text}");
        assert_eq!(
            session.agent_name(Some(&b.client_id)).as_deref(),
            Some("GitHub Copilot CLI"),
            "the viewer learns the agent's name"
        );
    }
}
