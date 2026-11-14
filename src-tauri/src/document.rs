//! One open document (= one tab): file watching, typst compilation and the
//! feedback queue between the user and the agent working on it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::json;

use crate::events::{Emitter, name};
use crate::feedback::{FeedbackItem, FeedbackQueue, WaitOutcome};
use crate::paths::DocKind;
use crate::typst::{CompileEvent, TypstWatch};

/// Identifier of a document/tab, stable for the lifetime of the app.
pub type DocId = u64;

/// Debounce for file system events (typst writes the PDF in several steps).
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(120);
/// Maximum time to wait for the first compile of a freshly opened `.typ`.
const FIRST_COMPILE_TIMEOUT: Duration = Duration::from_secs(30);

/// Snapshot of one document for UI and agent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocStatus {
    pub id: DocId,
    pub path: Option<String>,
    /// `.typ` source compiled by pdfnotator itself, if any.
    pub source: Option<String>,
    pub compile_error: Option<String>,
    pub version: u64,
    /// Agents currently blocked in `wait_for_feedback` for this document.
    pub listeners: usize,
    /// Comments waiting to be fetched by the agent.
    pub pending: usize,
    pub current_page: u32,
    pub page_count: u32,
}

/// Result of [`Document::open`].
#[derive(Debug, Clone)]
pub struct Opened {
    pub pdf: PathBuf,
    pub source: Option<PathBuf>,
    pub compile_error: Option<String>,
}

/// Why `wait_for_feedback` returned without comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitEnd {
    TimedOut,
    Closed,
    /// A newer `wait_for_feedback` for this document took over.
    Superseded,
    Cancelled,
}

#[derive(Debug, Default)]
struct Inner {
    pdf: Option<PathBuf>,
    source: Option<PathBuf>,
    watcher: Option<RecommendedWatcher>,
    typst: Option<TypstWatch>,
    compile_error: Option<String>,
    current_page: u32,
    page_count: u32,
}

/// Called whenever the document's status changed (to refresh the UI).
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

/// Delivered comments kept for `get_comments` (newest last).
const HISTORY_LIMIT: usize = 200;

pub struct Document {
    pub id: DocId,
    pub feedback: FeedbackQueue,
    /// Comments already handed to the agent, so it can fetch full details later.
    history: Mutex<std::collections::VecDeque<FeedbackItem>>,
    inner: Mutex<Inner>,
    emitter: Arc<Emitter>,
    on_change: OnChange,
    version: AtomicU64,
    reload_generation: AtomicU64,
    compiles: AtomicU64,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl Document {
    pub fn new(id: DocId, emitter: Arc<Emitter>, on_change: OnChange) -> Arc<Self> {
        Arc::new(Self {
            id,
            feedback: FeedbackQueue::default(),
            history: Mutex::default(),
            inner: Mutex::new(Inner::default()),
            emitter,
            on_change,
            version: AtomicU64::new(0),
            reload_generation: AtomicU64::new(0),
            compiles: AtomicU64::new(0),
        })
    }

    pub fn status(&self) -> DocStatus {
        let inner = self.inner.lock().unwrap();
        DocStatus {
            id: self.id,
            path: inner.pdf.as_ref().map(|p| p.display().to_string()),
            source: inner.source.as_ref().map(|p| p.display().to_string()),
            compile_error: inner.compile_error.clone(),
            version: self.version.load(Ordering::SeqCst),
            listeners: self.feedback.listeners(),
            pending: self.feedback.len(),
            current_page: inner.current_page,
            page_count: inner.page_count,
        }
    }

    pub fn pdf(&self) -> Option<PathBuf> {
        self.inner.lock().unwrap().pdf.clone()
    }

    pub fn source(&self) -> Option<PathBuf> {
        self.inner.lock().unwrap().source.clone()
    }

    /// Does `path` (canonical `.typ` or `.pdf`) refer to this document?
    pub fn matches(&self, path: &Path) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.pdf.as_deref() == Some(path) || inner.source.as_deref() == Some(path)
    }

    pub fn set_view(&self, page: u32, page_count: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner.current_page = page;
        inner.page_count = page_count;
    }

    fn changed(&self) {
        (self.on_change)();
    }

    /// (Re)open the document from a canonical path. Blocking: for a `.typ`
    /// source this waits for the first compile.
    pub fn open(self: &Arc<Self>, path: &Path, kind: DocKind, root: Option<&Path>) -> Result<Opened, String> {
        let pdf = match kind {
            DocKind::Typst => {
                let pdf = path.with_extension("pdf");
                let already_watching = {
                    let inner = self.inner.lock().unwrap();
                    inner.source.as_deref() == Some(path) && inner.typst.is_some()
                };
                if !already_watching {
                    self.start_typst(path, &pdf, root)?;
                }
                if !pdf.exists() {
                    let err = self.inner.lock().unwrap().compile_error.clone();
                    return Err(format!(
                        "typst could not compile {}:\n{}",
                        path.display(),
                        err.unwrap_or_else(|| "no output was produced".into())
                    ));
                }
                pdf
            }
            DocKind::Pdf => {
                let mut inner = self.inner.lock().unwrap();
                // keep a running typst watch when it produces exactly this PDF
                if inner.source.as_ref().map(|s| s.with_extension("pdf")).as_deref() != Some(path) {
                    inner.typst = None;
                    inner.source = None;
                    inner.compile_error = None;
                }
                path.to_path_buf()
            }
        };

        let watcher = self.watch_file(&pdf)?;
        let (source, compile_error) = {
            let mut inner = self.inner.lock().unwrap();
            inner.pdf = Some(pdf.clone());
            inner.watcher = Some(watcher);
            if inner.current_page == 0 {
                inner.current_page = 1;
            }
            (inner.source.clone(), inner.compile_error.clone())
        };
        let version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        self.emitter.emit(
            name::PDF_OPENED,
            json!({ "doc": self.id, "path": pdf.display().to_string(), "version": version }),
        );
        self.changed();
        Ok(Opened {
            pdf,
            source,
            compile_error,
        })
    }

    /// Watch the PDF's directory (tools may replace the file atomically).
    fn watch_file(self: &Arc<Self>, pdf: &Path) -> Result<RecommendedWatcher, String> {
        let dir = pdf.parent().ok_or("PDF has no parent directory")?;
        let file_name = pdf.file_name().map(std::ffi::OsStr::to_os_string);
        let doc = Arc::downgrade(self);
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            if event.paths.iter().any(|p| p.file_name() == file_name.as_deref())
                && let Some(doc) = doc.upgrade()
            {
                doc.schedule_reload();
            }
        })
        .map_err(|e| e.to_string())?;
        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| e.to_string())?;
        Ok(watcher)
    }

    /// Debounce change events and wait for a stable file size, then notify the UI.
    fn schedule_reload(self: &Arc<Self>) {
        let generation = self.reload_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let doc = Arc::clone(self);
        std::thread::spawn(move || {
            let current = || doc.reload_generation.load(Ordering::SeqCst) == generation;
            std::thread::sleep(RELOAD_DEBOUNCE);
            if !current() {
                return;
            }
            let Some(pdf) = doc.pdf() else { return };
            let mut last = None;
            for _ in 0..20 {
                let len = std::fs::metadata(&pdf).map(|m| m.len()).ok();
                if len.is_some_and(|l| l > 0) && len == last {
                    break;
                }
                last = len;
                std::thread::sleep(Duration::from_millis(60));
            }
            if !current() {
                return;
            }
            let version = doc.version.fetch_add(1, Ordering::SeqCst) + 1;
            doc.emitter.emit(
                name::PDF_CHANGED,
                json!({ "doc": doc.id, "path": pdf.display().to_string(), "version": version }),
            );
        });
    }

    /// Start `typst watch` and block until the first compile finished.
    fn start_typst(self: &Arc<Self>, typ: &Path, pdf: &Path, root: Option<&Path>) -> Result<(), String> {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.typst = None; // stops a previous watcher
            inner.compile_error = None;
        }
        let before = self.compiles.load(Ordering::SeqCst);
        let doc = Arc::downgrade(self);
        let watch = TypstWatch::spawn(typ, pdf, root, move |event| {
            if let Some(doc) = doc.upgrade() {
                doc.on_compiled(event);
            }
        })?;
        {
            let mut inner = self.inner.lock().unwrap();
            inner.typst = Some(watch);
            inner.source = Some(typ.to_path_buf());
        }
        let deadline = Instant::now() + FIRST_COMPILE_TIMEOUT;
        while self.compiles.load(Ordering::SeqCst) == before && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(40));
        }
        Ok(())
    }

    fn on_compiled(&self, event: CompileEvent) {
        let error = match event {
            CompileEvent::Ok => None,
            CompileEvent::Error(e) => Some(e),
        };
        self.inner.lock().unwrap().compile_error.clone_from(&error);
        self.compiles.fetch_add(1, Ordering::SeqCst);
        self.emitter.emit(
            name::COMPILE,
            json!({ "doc": self.id, "ok": error.is_none(), "error": error }),
        );
        self.changed();
    }

    /// Give a compile triggered by a recent edit a moment to finish and return
    /// the current compile error (if pdfnotator compiles the document).
    pub async fn settle_compile(&self) -> Option<String> {
        self.inner.lock().unwrap().typst.as_ref()?;
        let before = self.compiles.load(Ordering::SeqCst);
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.compiles.load(Ordering::SeqCst) != before {
                break;
            }
        }
        self.inner.lock().unwrap().compile_error.clone()
    }

    /// Queue comments from the UI.
    pub fn submit(&self, items: Vec<FeedbackItem>) {
        self.feedback.push(items);
        self.changed();
    }

    /// Block until the user sends feedback, `timeout` elapses, the tab is
    /// closed, `cancel` fires or a newer wait for this document starts.
    pub async fn wait_for_feedback(
        &self,
        timeout: Duration,
        cancel: &tokio::sync::Notify,
    ) -> Result<Vec<FeedbackItem>, WaitEnd> {
        let items = match self.feedback.wait(timeout, cancel, || self.changed()).await {
            WaitOutcome::Items(items) => items,
            WaitOutcome::TimedOut => return Err(WaitEnd::TimedOut),
            WaitOutcome::Closed => return Err(WaitEnd::Closed),
            WaitOutcome::Superseded => return Err(WaitEnd::Superseded),
            WaitOutcome::Cancelled => return Err(WaitEnd::Cancelled),
        };
        {
            let mut history = self.history.lock().unwrap();
            history.extend(items.iter().cloned());
            let excess = history.len().saturating_sub(HISTORY_LIMIT);
            history.drain(..excess);
        }
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        self.emitter
            .emit(name::FEEDBACK_DELIVERED, json!({ "doc": self.id, "ids": ids }));
        self.changed();
        Ok(items)
    }

    /// Delivered comments with these ids (latest version of each), in the requested order.
    pub fn delivered(&self, ids: &[String]) -> Vec<FeedbackItem> {
        let history = self.history.lock().unwrap();
        ids.iter()
            .filter_map(|id| history.iter().rev().find(|i| &i.id == id).cloned())
            .collect()
    }

    /// Show an agent message in the UI and mark comments as resolved.
    pub fn agent_message(&self, message: &str, resolved: &[String]) {
        self.emitter.emit(
            name::AGENT_MESSAGE,
            json!({ "doc": self.id, "message": message, "resolved": resolved }),
        );
    }

    /// Stop background work and release waiting agents.
    pub fn close(&self) {
        self.feedback.close();
        let mut inner = self.inner.lock().unwrap();
        inner.typst = None;
        inner.watcher = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingSink;
    use crate::feedback::item;

    fn doc() -> (Arc<Document>, Arc<RecordingSink>) {
        let sink = Arc::new(RecordingSink::default());
        let emitter = Arc::new(Emitter::default());
        emitter.set(sink.clone());
        (Document::new(1, emitter, Arc::new(|| {})), sink)
    }

    fn wait_until(mut f: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !f() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        f()
    }

    #[test]
    fn change_on_disk_triggers_reload_event() {
        let (doc, sink) = doc();
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF-1.7 v1").unwrap();
        doc.open(&pdf.canonicalize().unwrap(), DocKind::Pdf, None)
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&pdf, b"%PDF-1.7 v2 changed").unwrap();
        assert!(
            wait_until(|| sink.last(name::PDF_CHANGED).is_some()),
            "{:?}",
            sink.names()
        );
        assert_eq!(sink.last(name::PDF_CHANGED).unwrap()["doc"], 1);
    }

    #[tokio::test]
    async fn feedback_roundtrip_and_close() {
        let (doc, sink) = doc();
        doc.submit(vec![item("c1")]);
        let got = doc
            .wait_for_feedback(Duration::from_secs(1), &tokio::sync::Notify::new())
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            sink.last(name::FEEDBACK_DELIVERED),
            Some(json!({ "doc": 1, "ids": ["c1"] }))
        );
        assert_eq!(
            doc.wait_for_feedback(Duration::ZERO, &tokio::sync::Notify::new())
                .await,
            Err(WaitEnd::TimedOut)
        );

        let waiter = {
            let doc = doc.clone();
            tokio::spawn(async move {
                doc.wait_for_feedback(Duration::from_secs(30), &tokio::sync::Notify::new())
                    .await
            })
        };
        while doc.feedback.listeners() == 0 {
            tokio::task::yield_now().await;
        }
        doc.close();
        assert_eq!(waiter.await.unwrap(), Err(WaitEnd::Closed));
    }

    #[test]
    fn typst_source_is_compiled_and_errors_reported() {
        if crate::typst::find_typst().is_none() {
            eprintln!("typst not installed — skipping");
            return;
        }
        let (doc, sink) = doc();
        let dir = tempfile::tempdir().unwrap();
        let typ = dir.path().join("doc.typ");
        std::fs::write(&typ, "= Title").unwrap();
        let typ = typ.canonicalize().unwrap();
        let opened = doc.open(&typ, DocKind::Typst, None).unwrap();
        assert!(opened.pdf.exists());
        assert!(opened.compile_error.is_none());
        assert!(doc.matches(&typ) && doc.matches(&opened.pdf));

        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(&typ, "= Title\n#broken(").unwrap();
        assert!(wait_until(|| doc.status().compile_error.is_some()));
        assert!(doc.status().compile_error.unwrap().contains("unclosed"));
        assert_eq!(sink.last(name::COMPILE).unwrap()["ok"], json!(false));

        // opening the produced PDF keeps the typst watch running
        doc.open(&opened.pdf, DocKind::Pdf, None).unwrap();
        assert!(doc.source().is_some());
        doc.close();
    }

    #[test]
    fn broken_typst_without_pdf_is_an_error() {
        if crate::typst::find_typst().is_none() {
            return;
        }
        let (doc, _) = doc();
        let dir = tempfile::tempdir().unwrap();
        let typ = dir.path().join("bad.typ");
        std::fs::write(&typ, "#broken(").unwrap();
        let err = doc
            .open(&typ.canonicalize().unwrap(), DocKind::Typst, None)
            .unwrap_err();
        assert!(err.contains("could not compile"), "{err}");
        doc.close();
    }
}
