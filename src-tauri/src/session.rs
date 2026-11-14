//! Application state: all open documents (tabs) and which agent session works
//! on which document.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde::Serialize;
use serde_json::json;
use tokio::sync::Notify;

use crate::document::{DocId, DocStatus, Document, Opened};
use crate::events::{Emitter, EventSink, name};
use crate::paths;

/// Snapshot for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub port: u16,
    pub server_error: Option<String>,
    /// Open documents in tab order.
    pub documents: Vec<DocStatus>,
}

pub struct Session {
    emitter: Arc<Emitter>,
    docs: Mutex<Vec<Arc<Document>>>,
    /// Agent session id → document it opened last.
    clients: Mutex<HashMap<String, DocId>>,
    next_id: AtomicU64,
    port: u16,
    server_error: Mutex<Option<String>>,
    /// Running tool calls by `(client, request id)`, so `notifications/cancelled` can stop them.
    inflight: Mutex<HashMap<String, Arc<Notify>>>,
    this: Weak<Session>,
}

/// A running tool call; unregisters itself when dropped (also if the HTTP
/// request is aborted and its future discarded).
pub struct CallGuard<'a> {
    session: &'a Session,
    key: String,
    pub cancel: Arc<Notify>,
}

impl Drop for CallGuard<'_> {
    fn drop(&mut self) {
        self.session.inflight.lock().unwrap().remove(&self.key);
    }
}

fn call_key(client: Option<&str>, request_id: &serde_json::Value) -> String {
    format!("{}\u{0}{request_id}", client.unwrap_or_default())
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl Session {
    pub fn new(port: u16) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            emitter: Arc::new(Emitter::default()),
            docs: Mutex::new(Vec::new()),
            clients: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            port,
            server_error: Mutex::new(None),
            inflight: Mutex::new(HashMap::new()),
            this: this.clone(),
        })
    }

    /// Connect the session to the UI.
    pub fn set_sink(&self, sink: Arc<dyn EventSink>) {
        self.emitter.set(sink);
    }

    pub fn emit_status(&self) {
        self.emitter.emit(
            name::STATUS,
            serde_json::to_value(self.status()).unwrap_or_default(),
        );
    }

    pub fn status(&self) -> Status {
        Status {
            port: self.port,
            server_error: self.server_error.lock().unwrap().clone(),
            documents: self.documents().iter().map(|d| d.status()).collect(),
        }
    }

    pub fn set_server_error(&self, err: impl Into<String>) {
        *self.server_error.lock().unwrap() = Some(err.into());
        self.emit_status();
    }

    pub fn documents(&self) -> Vec<Arc<Document>> {
        self.docs.lock().unwrap().clone()
    }

    pub fn document(&self, id: DocId) -> Option<Arc<Document>> {
        self.docs.lock().unwrap().iter().find(|d| d.id == id).cloned()
    }

    fn find_by_path(&self, path: &Path) -> Option<Arc<Document>> {
        let pdf = path.with_extension("pdf");
        self.docs
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.matches(path) || d.matches(&pdf))
            .cloned()
    }

    /// Open a document — in its existing tab if it is already open — and bind
    /// the agent session `client` to it. Blocking (first typst compile).
    pub fn open(
        &self,
        raw: &str,
        root: Option<&Path>,
        client: Option<&str>,
    ) -> Result<(Arc<Document>, Opened), String> {
        let path = paths::normalize(raw)?;
        let kind = paths::kind_of(&path)?;
        let path = path
            .canonicalize()
            .map_err(|_| format!("file does not exist: {}", path.display()))?;

        let (doc, created) = match self.find_by_path(&path) {
            Some(doc) => (doc, false),
            None => (self.create_document(), true),
        };
        let opened = match doc.open(&path, kind, root) {
            Ok(opened) => opened,
            Err(e) => {
                if created && let Some(doc) = self.remove(doc.id) {
                    doc.close();
                }
                return Err(e);
            }
        };
        if let Some(client) = client {
            self.clients.lock().unwrap().insert(client.to_owned(), doc.id);
        }
        self.emitter.emit(name::FOCUS, json!({ "doc": doc.id }));
        self.emit_status();
        let file = opened.pdf.file_name().unwrap_or_default().to_string_lossy();
        self.emitter.reveal(&format!("PDFnotator — {file}"));
        Ok((doc, opened))
    }

    fn create_document(&self) -> Arc<Document> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let session = self.this.clone();
        let doc = Document::new(
            id,
            self.emitter.clone(),
            Arc::new(move || {
                if let Some(s) = session.upgrade() {
                    s.emit_status();
                }
            }),
        );
        self.docs.lock().unwrap().push(doc.clone());
        doc
    }

    fn remove(&self, id: DocId) -> Option<Arc<Document>> {
        let mut docs = self.docs.lock().unwrap();
        let idx = docs.iter().position(|d| d.id == id)?;
        let doc = docs.remove(idx);
        drop(docs);
        self.clients.lock().unwrap().retain(|_, d| *d != id);
        Some(doc)
    }

    /// Close a tab: stops watchers and releases agents waiting on it.
    pub fn close(&self, id: DocId) {
        if let Some(doc) = self.remove(id) {
            doc.close();
            self.emitter.emit(name::DOC_CLOSED, json!({ "doc": id }));
            self.emit_status();
        }
    }

    /// Find the document an agent call refers to:
    /// explicit `path` → the client's last opened document → the only open document.
    pub fn resolve(&self, path: Option<&str>, client: Option<&str>) -> Result<Arc<Document>, String> {
        if let Some(raw) = path.filter(|p| !p.trim().is_empty()) {
            let path = paths::normalize(raw)?;
            let path = path.canonicalize().unwrap_or(path);
            return self.find_by_path(&path).ok_or_else(|| {
                format!(
                    "{} is not open in pdfnotator — call open_pdf first.{}",
                    path.display(),
                    self.open_list()
                )
            });
        }
        let bound = client
            .and_then(|c| self.clients.lock().unwrap().get(c).copied())
            .and_then(|id| self.document(id));
        if let Some(doc) = bound {
            return Ok(doc);
        }
        let docs = self.documents();
        match docs.as_slice() {
            [only] => Ok(only.clone()),
            [] => Err("No document is open — call open_pdf first.".into()),
            _ => Err(format!(
                "Several documents are open; pass `path` to say which one.{}",
                self.open_list()
            )),
        }
    }

    fn open_list(&self) -> String {
        let paths: Vec<String> = self
            .documents()
            .iter()
            .filter_map(|d| d.source().or_else(|| d.pdf()))
            .map(|p: PathBuf| format!("\n- {}", p.display()))
            .collect();
        if paths.is_empty() {
            String::new()
        } else {
            format!("\nOpen documents:{}", paths.concat())
        }
    }

    /// Register a running tool call so it can be cancelled.
    pub fn begin_call(&self, client: Option<&str>, request_id: &serde_json::Value) -> CallGuard<'_> {
        let key = call_key(client, request_id);
        let cancel = Arc::new(Notify::new());
        self.inflight.lock().unwrap().insert(key.clone(), cancel.clone());
        CallGuard {
            session: self,
            key,
            cancel,
        }
    }

    /// Cancel a running tool call (MCP `notifications/cancelled`). Returns whether it was found.
    pub fn cancel_call(&self, client: Option<&str>, request_id: &serde_json::Value) -> bool {
        let token = self
            .inflight
            .lock()
            .unwrap()
            .get(&call_key(client, request_id))
            .cloned();
        // notify_one stores a permit, so a cancel that races ahead of the wait is not lost
        token.inspect(|t| t.notify_one()).is_some()
    }

    /// Stop background work before exit.
    pub fn shutdown(&self) {
        for doc in self.documents() {
            doc.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingSink;

    fn pdf_in(dir: &tempfile::TempDir, name: &str) -> String {
        let p = dir.path().join(name);
        std::fs::write(&p, b"%PDF-1.7").unwrap();
        p.to_str().unwrap().to_owned()
    }

    #[test]
    fn opens_tabs_and_reuses_existing() {
        let s = Session::new(0);
        let sink = Arc::new(RecordingSink::default());
        s.set_sink(sink.clone());
        let dir = tempfile::tempdir().unwrap();
        let (a, _) = s.open(&pdf_in(&dir, "a.pdf"), None, Some("agent-1")).unwrap();
        let (b, _) = s.open(&pdf_in(&dir, "b.pdf"), None, Some("agent-2")).unwrap();
        assert_ne!(a.id, b.id);
        let (a2, _) = s
            .open(dir.path().join("a.pdf").to_str().unwrap(), None, None)
            .unwrap();
        assert_eq!(a.id, a2.id, "same file → same tab");
        assert_eq!(s.status().documents.len(), 2);
        assert_eq!(sink.last(name::FOCUS).unwrap()["doc"], a.id);
    }

    #[test]
    fn resolves_documents_for_agents() {
        let s = Session::new(0);
        let dir = tempfile::tempdir().unwrap();
        assert!(s.resolve(None, None).unwrap_err().contains("No document"));
        let (a, _) = s.open(&pdf_in(&dir, "a.pdf"), None, Some("one")).unwrap();
        assert_eq!(
            s.resolve(None, None).unwrap().id,
            a.id,
            "single tab is the default"
        );
        let (b, _) = s.open(&pdf_in(&dir, "b.pdf"), None, Some("two")).unwrap();

        assert_eq!(s.resolve(None, Some("one")).unwrap().id, a.id);
        assert_eq!(s.resolve(None, Some("two")).unwrap().id, b.id);
        let err = s.resolve(None, Some("unknown")).unwrap_err();
        assert!(
            err.contains("pass `path`") && err.contains("a.pdf") && err.contains("b.pdf"),
            "{err}"
        );
        let by_path = s
            .resolve(Some(dir.path().join("b.pdf").to_str().unwrap()), Some("one"))
            .unwrap();
        assert_eq!(by_path.id, b.id, "explicit path wins over the client binding");
        assert!(
            s.resolve(Some("/nope/x.pdf"), None)
                .unwrap_err()
                .contains("not open")
        );
    }

    #[tokio::test]
    async fn closing_a_tab_releases_waiting_agents() {
        let s = Session::new(0);
        let sink = Arc::new(RecordingSink::default());
        s.set_sink(sink.clone());
        let dir = tempfile::tempdir().unwrap();
        let (doc, _) = s.open(&pdf_in(&dir, "a.pdf"), None, Some("c")).unwrap();
        let waiter = {
            let doc = doc.clone();
            tokio::spawn(async move {
                doc.wait_for_feedback(std::time::Duration::from_secs(30), &tokio::sync::Notify::new())
                    .await
            })
        };
        while doc.feedback.listeners() == 0 {
            tokio::task::yield_now().await;
        }
        s.close(doc.id);
        assert!(waiter.await.unwrap().is_err());
        assert!(s.documents().is_empty());
        assert!(s.resolve(None, Some("c")).is_err());
        assert_eq!(sink.last(name::DOC_CLOSED).unwrap()["doc"], doc.id);
    }

    #[test]
    fn failed_open_does_not_leave_an_empty_tab() {
        let s = Session::new(0);
        assert!(
            s.open("/definitely/not/here.pdf", None, None)
                .unwrap_err()
                .contains("does not exist")
        );
        let dir = tempfile::tempdir().unwrap();
        let docx = dir.path().join("a.docx");
        std::fs::write(&docx, b"x").unwrap();
        assert!(
            s.open(docx.to_str().unwrap(), None, None)
                .unwrap_err()
                .contains("unsupported")
        );
        assert!(s.documents().is_empty());
    }
}
