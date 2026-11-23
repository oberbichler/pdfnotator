//! Comments created in the viewer and the queue that hands them to the agent.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};

/// What a comment refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    /// Selected text (quoted).
    Text,
    /// Rectangle on a page (with screenshot).
    Area,
    /// Free-form message not tied to a location.
    General,
}

/// Shape of a sketch drawn on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    /// Freehand stroke.
    Pen,
    Arrow,
    Rect,
    /// Note typed onto the page.
    Text,
}

/// One shape of a sketch, described for the agent. Positions in mm from the
/// page's top-left corner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackShape {
    #[serde(rename = "type")]
    pub kind: ShapeKind,
    /// Bounding box `[x, y, w, h]`.
    #[serde(default)]
    pub box_mm: [f64; 4],
    #[serde(default)]
    pub from_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub to_mm: Option<[f64; 2]>,
    /// Freehand stroke whose ends meet: it encircles something.
    #[serde(default)]
    pub closed: bool,
    /// Page text inside the shape (pen, rectangle).
    #[serde(default)]
    pub text: Option<String>,
    /// Page text at the arrow's tail and head.
    #[serde(default)]
    pub from_text: Option<String>,
    #[serde(default)]
    pub to_text: Option<String>,
    /// Typed note (text shapes).
    #[serde(default)]
    pub label: Option<String>,
}

/// One comment from the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackItem {
    /// Stable id like `c3`, used by the agent to resolve comments.
    pub id: String,
    pub kind: FeedbackKind,
    /// 1-based page number (0 for general comments).
    #[serde(default)]
    pub page: u32,
    /// Selected text, for [`FeedbackKind::Text`].
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub comment: String,
    /// Normalised rectangle `[x, y, w, h]` (0..1, top-left origin).
    #[serde(default)]
    pub rect: Option<[f64; 4]>,
    /// Full-resolution PNG screenshot as base64 (no `data:` prefix), for [`FeedbackKind::Area`].
    #[serde(default)]
    pub image: Option<String>,
    /// Downscaled PNG sent to the agent by default.
    #[serde(default)]
    pub preview: Option<String>,
    /// Page text around a text selection.
    #[serde(default)]
    pub context: Option<String>,
    /// Text inside an area selection.
    #[serde(default)]
    pub area_text: Option<String>,
    /// Sketch drawn on the page (area comments); `image` shows it, `clean` the region without it.
    #[serde(default)]
    pub shapes: Vec<FeedbackShape>,
    /// Full-resolution PNG of a sketch's region without the sketch.
    #[serde(default)]
    pub clean: Option<String>,
}

/// After the first comment arrives, wait this long so a burst is delivered together.
const BATCH_GRACE: Duration = Duration::from_millis(300);

/// How a [`FeedbackQueue::wait`] ended.
#[derive(Debug, Clone, PartialEq)]
pub enum WaitOutcome {
    /// Comments for the agent (removed from the queue).
    Items(Vec<FeedbackItem>),
    /// Nothing arrived within the timeout.
    TimedOut,
    /// The document was closed.
    Closed,
    /// A newer wait on the same queue took over. Older waits are stale — the
    /// agent abandoned them (cancelled tool call, restarted MCP server, …).
    Superseded,
    /// The agent cancelled the call (`notifications/cancelled`).
    Cancelled,
}

/// Queue between the viewer (producer) and the agent (consumer).
///
/// Only the most recent waiter may take comments: an agent never waits twice
/// for the same document at once, so an older waiter is always a leftover of an
/// abandoned call. Letting it drain the queue would hand comments to nobody.
#[derive(Debug)]
pub struct FeedbackQueue {
    items: Mutex<Vec<FeedbackItem>>,
    /// Bumped on every change (push, close, new waiter). A `watch` channel never
    /// loses a wake-up: `changed()` completes if anything happened since the
    /// waiter last looked.
    signal: watch::Sender<u64>,
    listeners: AtomicUsize,
    closed: AtomicBool,
    /// Incremented by every new waiter; only the waiter holding the latest value drains.
    generation: AtomicU64,
}

impl Default for FeedbackQueue {
    fn default() -> Self {
        Self {
            items: Mutex::default(),
            signal: watch::channel(0).0,
            listeners: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }
}

impl FeedbackQueue {
    /// Add comments and wake up the waiting agent. A comment whose id is
    /// already queued replaces the queued one, so a double send (or a resend
    /// before the agent picked it up) reaches the agent once.
    pub fn push(&self, items: impl IntoIterator<Item = FeedbackItem>) {
        let mut queue = self.items.lock().unwrap();
        let mut added = false;
        for item in items {
            if let Some(queued) = queue.iter_mut().find(|q| q.id == item.id) {
                *queued = item;
            } else {
                queue.push(item);
            }
            added = true;
        }
        if added {
            drop(queue);
            self.wake();
        }
    }

    fn wake(&self) {
        self.signal.send_modify(|v| *v = v.wrapping_add(1));
    }

    /// Take all queued comments.
    pub fn drain(&self) -> Vec<FeedbackItem> {
        std::mem::take(&mut *self.items.lock().unwrap())
    }

    /// Number of queued comments.
    pub fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    /// Release all waiting agents for good (the document was closed).
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.wake();
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Number of waits started so far (only grows; lets tests sync on a new waiter).
    #[cfg(test)]
    pub(crate) fn waits_started(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Number of agents currently blocked in [`Self::wait`].
    pub fn listeners(&self) -> usize {
        self.listeners.load(Ordering::SeqCst)
    }

    /// Wait until comments are available and take them. Ends early when the
    /// timeout elapses, the queue is closed, `cancel` is notified or a newer
    /// wait starts. `on_listeners_changed` runs when listening starts and stops.
    pub async fn wait(
        &self,
        timeout: Duration,
        cancel: &Notify,
        on_listeners_changed: impl Fn() + Sync,
    ) -> WaitOutcome {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // wake older waiters so they notice they are superseded
        self.wake();
        let mut changes = self.signal.subscribe();
        let _guard = ListenerGuard::new(self, &on_listeners_changed);
        let current = || self.generation.load(Ordering::SeqCst) == generation;
        let deadline = tokio::time::Instant::now() + timeout;
        let cancelled = cancel.notified();
        tokio::pin!(cancelled);
        loop {
            // `changes` was marked as seen before these checks, so a push in between is not missed.
            if !current() {
                return WaitOutcome::Superseded;
            }
            if self.is_closed() {
                return WaitOutcome::Closed;
            }
            if self.len() > 0 {
                tokio::select! {
                    () = tokio::time::sleep(BATCH_GRACE) => {}
                    () = &mut cancelled => return WaitOutcome::Cancelled,
                }
                if !current() {
                    return WaitOutcome::Superseded;
                }
                return WaitOutcome::Items(self.drain());
            }
            if timeout.is_zero() {
                return WaitOutcome::TimedOut;
            }
            tokio::select! {
                _ = changes.changed() => {}
                () = &mut cancelled => return WaitOutcome::Cancelled,
                () = tokio::time::sleep_until(deadline) => {
                    let items = self.drain();
                    return if items.is_empty() { WaitOutcome::TimedOut } else { WaitOutcome::Items(items) };
                }
            }
        }
    }
}

struct ListenerGuard<'a> {
    queue: &'a FeedbackQueue,
    changed: &'a (dyn Fn() + Sync),
}

impl<'a> ListenerGuard<'a> {
    fn new(queue: &'a FeedbackQueue, changed: &'a (dyn Fn() + Sync)) -> Self {
        queue.listeners.fetch_add(1, Ordering::SeqCst);
        changed();
        Self { queue, changed }
    }
}

impl Drop for ListenerGuard<'_> {
    fn drop(&mut self) {
        self.queue.listeners.fetch_sub(1, Ordering::SeqCst);
        (self.changed)();
    }
}

#[cfg(test)]
pub(crate) fn item(id: &str) -> FeedbackItem {
    FeedbackItem {
        id: id.into(),
        kind: FeedbackKind::General,
        page: 0,
        quote: None,
        comment: format!("comment {id}"),
        rect: None,
        image: None,
        preview: None,
        context: None,
        area_text: None,
        shapes: Vec::new(),
        clean: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ids(outcome: &WaitOutcome) -> Vec<&str> {
        match outcome {
            WaitOutcome::Items(items) => items.iter().map(|i| i.id.as_str()).collect(),
            other => panic!("expected items, got {other:?}"),
        }
    }

    async fn wait(q: &FeedbackQueue, secs: u64) -> WaitOutcome {
        q.wait(Duration::from_secs(secs), &Notify::new(), || {}).await
    }

    /// Start a waiter in the background and wait until it is listening.
    async fn spawn_waiter(
        q: &Arc<FeedbackQueue>,
        cancel: Arc<Notify>,
    ) -> tokio::task::JoinHandle<WaitOutcome> {
        // sync on the start counter: the listener count may not change net
        // when the new waiter immediately supersedes an older one
        let before = q.waits_started();
        let handle = tokio::spawn({
            let q = q.clone();
            async move { q.wait(Duration::from_secs(10), &cancel, || {}).await }
        });
        while q.waits_started() == before {
            tokio::task::yield_now().await;
        }
        handle
    }

    #[tokio::test]
    async fn returns_queued_items_immediately() {
        let q = FeedbackQueue::default();
        q.push([item("c1"), item("c2")]);
        assert_eq!(ids(&wait(&q, 5).await), ["c1", "c2"]);
        assert_eq!(q.len(), 0);
    }

    #[tokio::test]
    async fn a_comment_sent_twice_is_queued_once() {
        let q = FeedbackQueue::default();
        q.push([item("c1"), item("c2")]);
        let mut again = item("c1");
        again.comment = "edited".into();
        q.push([again]);
        q.push([item("c3"), item("c3")]);
        let WaitOutcome::Items(items) = wait(&q, 5).await else {
            panic!("expected items")
        };
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["c1", "c2", "c3"]
        );
        assert_eq!(items[0].comment, "edited");
    }

    #[tokio::test]
    async fn zero_timeout_does_not_block() {
        let q = FeedbackQueue::default();
        assert_eq!(wait(&q, 0).await, WaitOutcome::TimedOut);
    }

    #[tokio::test(start_paused = true)]
    async fn times_out_empty() {
        let q = FeedbackQueue::default();
        assert_eq!(wait(&q, 60).await, WaitOutcome::TimedOut);
    }

    #[tokio::test]
    async fn wakes_up_on_push_and_tracks_listeners() {
        let q = Arc::new(FeedbackQueue::default());
        let waiter = spawn_waiter(&q, Arc::new(Notify::new())).await;
        q.push([item("c7")]);
        assert_eq!(ids(&waiter.await.unwrap()), ["c7"]);
        assert_eq!(q.listeners(), 0);
    }

    /// Regression: an abandoned wait must not swallow comments meant for the agent's next call.
    #[tokio::test]
    async fn newer_wait_supersedes_older_ones() {
        let q = Arc::new(FeedbackQueue::default());
        let stale = spawn_waiter(&q, Arc::new(Notify::new())).await;
        let fresh = spawn_waiter(&q, Arc::new(Notify::new())).await;
        assert_eq!(stale.await.unwrap(), WaitOutcome::Superseded);
        q.push([item("c1")]);
        assert_eq!(ids(&fresh.await.unwrap()), ["c1"]);
        assert_eq!(q.listeners(), 0);
    }

    #[tokio::test]
    async fn cancelled_wait_leaves_comments_queued() {
        let q = Arc::new(FeedbackQueue::default());
        let cancel = Arc::new(Notify::new());
        let waiter = spawn_waiter(&q, cancel.clone()).await;
        cancel.notify_one();
        assert_eq!(waiter.await.unwrap(), WaitOutcome::Cancelled);
        assert_eq!(q.listeners(), 0);
        q.push([item("c2")]);
        assert_eq!(ids(&wait(&q, 1).await), ["c2"], "comments wait for the next call");
    }

    #[tokio::test]
    async fn closing_releases_waiters() {
        let q = Arc::new(FeedbackQueue::default());
        let waiter = spawn_waiter(&q, Arc::new(Notify::new())).await;
        q.close();
        assert_eq!(waiter.await.unwrap(), WaitOutcome::Closed);
    }

    #[test]
    fn deserializes_frontend_payload() {
        let json = r#"{"id":"c1","kind":"area","page":2,"comment":"bigger","rect":[0.1,0.2,0.3,0.4],
                       "image":"iVBOR","areaText":"Hello","quote":null}"#;
        let it: FeedbackItem = serde_json::from_str(json).unwrap();
        assert_eq!(it.kind, FeedbackKind::Area);
        assert_eq!(it.rect, Some([0.1, 0.2, 0.3, 0.4]));
        assert_eq!(it.area_text.as_deref(), Some("Hello"));
    }
}
