//! Events sent from the backend to the viewer UI.
//!
//! The core logic only depends on the [`EventSink`] trait, so it can be tested
//! without a running Tauri application.

use serde_json::Value;

/// Event names shared with the frontend (`src/backend.ts`).
pub mod name {
    /// A document was opened (or re-opened) — payload `{ doc, path, version }`.
    pub const PDF_OPENED: &str = "pdf-opened";
    /// The PDF on disk changed — payload `{ doc, path, version }`.
    pub const PDF_CHANGED: &str = "pdf-changed";
    /// A document tab was closed — payload `{ doc }`.
    pub const DOC_CLOSED: &str = "doc-closed";
    /// The UI should switch to this tab — payload `{ doc }`.
    pub const FOCUS: &str = "focus";
    /// Typst finished a compile — payload `{ doc, ok, error }`.
    pub const COMPILE: &str = "compile";
    /// Server / agent status changed — payload [`crate::session::Status`].
    pub const STATUS: &str = "status";
    /// The agent fetched comments — payload `{ doc, ids }`.
    pub const FEEDBACK_DELIVERED: &str = "feedback-delivered";
    /// The agent replied — payload `{ doc, message, resolved }`.
    pub const AGENT_MESSAGE: &str = "agent-message";
}

/// Receiver for backend → UI notifications.
pub trait EventSink: Send + Sync + 'static {
    /// Emit an event to the UI.
    fn emit(&self, event: &str, payload: Value);

    /// Bring the viewer window to the front and update its title.
    fn reveal(&self, _title: &str) {}
}

impl EventSink for tauri::AppHandle {
    fn emit(&self, event: &str, payload: Value) {
        if let Err(err) = tauri::Emitter::emit(self, event, payload) {
            eprintln!("pdfnotator: failed to emit {event}: {err}");
        }
    }

    fn reveal(&self, title: &str) {
        use tauri::Manager;
        if let Some(window) = self.get_webview_window("main") {
            let _ = window.set_title(title);
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// Shared, replaceable sink: documents keep a handle while the app connects
/// the real UI sink once it is ready. Every event is also broadcast to remote
/// viewers (tablets, see [`crate::remote`]).
pub struct Emitter {
    sink: std::sync::RwLock<std::sync::Arc<dyn EventSink>>,
    remote: tokio::sync::broadcast::Sender<(String, Value)>,
}

/// Events buffered per remote viewer before a slow one starts missing some.
const REMOTE_BACKLOG: usize = 256;

impl Default for Emitter {
    fn default() -> Self {
        Self {
            sink: std::sync::RwLock::new(std::sync::Arc::new(NoopSink)),
            remote: tokio::sync::broadcast::channel(REMOTE_BACKLOG).0,
        }
    }
}

impl std::fmt::Debug for Emitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Emitter")
    }
}

impl Emitter {
    pub fn set(&self, sink: std::sync::Arc<dyn EventSink>) {
        *self.sink.write().unwrap() = sink;
    }

    fn sink(&self) -> std::sync::Arc<dyn EventSink> {
        self.sink.read().unwrap().clone()
    }

    pub fn emit(&self, event: &str, payload: Value) {
        if self.remote.receiver_count() > 0 {
            let _ = self.remote.send((event.to_owned(), payload.clone()));
        }
        self.sink().emit(event, payload);
    }

    /// Receive every event from now on (for a remote viewer).
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<(String, Value)> {
        self.remote.subscribe()
    }

    pub fn reveal(&self, title: &str) {
        self.sink().reveal(title);
    }
}

/// Sink that drops all events (used before the app is ready and in tests).
#[derive(Debug, Default)]
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: &str, _payload: Value) {}
}

/// Sink that records events, for assertions in tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct RecordingSink(pub std::sync::Mutex<Vec<(String, Value)>>);

#[cfg(test)]
impl RecordingSink {
    /// Names of all recorded events, in order.
    pub fn names(&self) -> Vec<String> {
        self.0.lock().unwrap().iter().map(|(n, _)| n.clone()).collect()
    }

    /// Payload of the most recent event with this name.
    pub fn last(&self, event: &str) -> Option<Value> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(n, _)| n == event)
            .map(|(_, v)| v.clone())
    }
}

#[cfg(test)]
impl EventSink for RecordingSink {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
}
