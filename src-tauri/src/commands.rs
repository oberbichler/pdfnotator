//! Tauri commands invoked by the viewer UI (`src/backend.ts`).

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::document::{DocId, Document};
use crate::feedback::FeedbackItem;
use crate::session::{Session, Status};

type SessionState<'a> = State<'a, Arc<Session>>;

fn doc(session: &Session, id: DocId) -> Result<Arc<Document>, String> {
    session
        .document(id)
        .ok_or_else(|| format!("document {id} is not open"))
}

/// Open a `.typ` or `.pdf` file chosen in the UI (dialog or drag & drop).
/// Returns the id of its tab.
#[tauri::command]
pub async fn open_document(session: SessionState<'_>, path: String) -> Result<DocId, String> {
    let session = Arc::clone(&session);
    tauri::async_runtime::spawn_blocking(move || session.open(&path, None))
        .await
        .map_err(|e| e.to_string())?
        .map(|(doc, _)| doc.id)
}

/// Close a tab.
#[tauri::command]
pub fn close_document(session: SessionState<'_>, doc: DocId) {
    session.close(doc);
}

/// Raw bytes of a document's PDF (sent as binary IPC payload).
#[tauri::command]
pub fn read_pdf(session: SessionState<'_>, doc: DocId) -> Result<tauri::ipc::Response, String> {
    let path = self::doc(&session, doc)?.pdf().ok_or("document has no PDF yet")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub fn get_status(session: SessionState<'_>) -> Status {
    session.status()
}

/// Queue comments for the agent working on `doc`.
#[tauri::command]
pub fn submit_feedback(
    session: SessionState<'_>,
    doc: DocId,
    items: Vec<FeedbackItem>,
) -> Result<(), String> {
    self::doc(&session, doc)?.submit(items);
    Ok(())
}

/// Page the user is looking at (reported to the agent).
#[tauri::command]
pub fn set_view(session: SessionState<'_>, doc: DocId, page: u32, page_count: u32) {
    if let Some(d) = session.document(doc) {
        d.set_view(page, page_count);
    }
}

/// Static information for the "connect your agent" menu.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    /// Command line that runs the stdio MCP bridge (`<exe> --mcp`).
    pub executable: Option<String>,
    /// `PDFNOTATOR_DEMO`: `1` stages a screenshot, `animate` plays the scripted demo.
    pub demo: Option<String>,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        executable: std::env::current_exe().ok().map(|p| p.display().to_string()),
        demo: std::env::var("PDFNOTATOR_DEMO").ok().filter(|v| !v.is_empty()),
    }
}
