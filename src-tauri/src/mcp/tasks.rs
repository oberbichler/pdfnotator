//! MCP Tasks extension (`io.modelcontextprotocol/tasks`, revision 2026-07-28).
//!
//! A client that declares the extension in a request's `_meta` gets a task
//! handle for `wait_for_feedback` instead of a blocking call: it polls
//! `tasks/get` until the user has commented. Nothing has to stay open, so
//! client timeouts (often ~60 s) no longer cut the wait short.
//!
//! Spec: <https://github.com/modelcontextprotocol/ext-tasks>

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::sync::Notify;

/// Extension identifier, in `extensions` of client and server capabilities.
pub const EXTENSION: &str = "io.modelcontextprotocol/tasks";
/// Per-request client capabilities (`params._meta`).
pub const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// Suggested `tasks/get` interval: comments should reach the agent promptly.
pub const POLL_INTERVAL_MS: u64 = 1000;
/// How long a task can be fetched after its wait ended at the latest.
const KEEP_FINISHED: Duration = Duration::from_secs(600);

/// Did the client declare the Tasks extension on this request?
pub fn declared(params: &Value) -> bool {
    params
        .get("_meta")
        .and_then(|m| m.get(META_CAPABILITIES))
        .and_then(|c| c.get("extensions"))
        .and_then(|e| e.get(EXTENSION))
        .is_some_and(Value::is_object)
}

/// `-32021` for `tasks/*` requests from clients that did not declare the extension.
pub fn missing_capability(id: Value) -> Value {
    super::error_response(
        id,
        super::code::MISSING_REQUIRED_CLIENT_CAPABILITY,
        "Missing required client capability",
        json!({ "requiredCapabilities": { "extensions": { EXTENSION: {} } } }),
    )
}

/// The `taskId` param of a `tasks/*` request.
pub fn task_id(params: &Value) -> Option<&str> {
    params.get("taskId").and_then(Value::as_str)
}

#[derive(Debug, Clone)]
enum State {
    Working,
    Completed(Value),
    Cancelled,
}

#[derive(Debug)]
struct Entry {
    created: SystemTime,
    updated: SystemTime,
    ttl: Duration,
    state: State,
    cancel: Arc<Notify>,
}

impl Entry {
    fn expired(&self, now: SystemTime) -> bool {
        now.duration_since(self.created).unwrap_or_default() > self.ttl
    }

    /// The task as `tasks/get` and `CreateTaskResult` describe it.
    fn describe(&self, id: &str, result_type: &str) -> Value {
        let mut v = json!({
            "resultType": result_type,
            "taskId": id,
            "createdAt": rfc3339(self.created),
            "lastUpdatedAt": rfc3339(self.updated),
            "ttlMs": u64::try_from(self.ttl.as_millis()).unwrap_or(u64::MAX),
            "pollIntervalMs": POLL_INTERVAL_MS,
        });
        let (status, result) = match &self.state {
            State::Working => ("working", None),
            State::Completed(result) => ("completed", Some(result.clone())),
            State::Cancelled => ("cancelled", None),
        };
        v["status"] = status.into();
        if let Some(result) = result {
            v["result"] = result;
        }
        v
    }
}

/// Running and recently finished tasks of this viewer.
#[derive(Debug, Default)]
pub struct Tasks {
    map: Mutex<HashMap<String, Entry>>,
}

impl Tasks {
    /// Register a task whose work takes at most `work`. Returns its id, the
    /// cancel signal for the work and the `CreateTaskResult`.
    pub fn create(&self, work: Duration) -> (String, Arc<Notify>, Value) {
        let id = uuid::Uuid::new_v4().simple().to_string(); // 122 random bits: unguessable
        let now = SystemTime::now();
        let entry = Entry {
            created: now,
            updated: now,
            ttl: work + KEEP_FINISHED,
            state: State::Working,
            cancel: Arc::new(Notify::new()),
        };
        let created = entry.describe(&id, "task");
        let cancel = entry.cancel.clone();
        let mut map = self.map.lock().unwrap();
        map.retain(|_, e| !e.expired(now));
        map.insert(id.clone(), entry);
        (id, cancel, created)
    }

    /// Store the result of a finished task (ignored once it was cancelled).
    pub fn complete(&self, id: &str, result: Value) {
        if let Some(e) = self.map.lock().unwrap().get_mut(id)
            && matches!(e.state, State::Working)
        {
            e.state = State::Completed(result);
            e.updated = SystemTime::now();
        }
    }

    /// `tasks/get` result, `None` for unknown or expired tasks.
    pub fn get(&self, id: &str) -> Option<Value> {
        let now = SystemTime::now();
        let mut map = self.map.lock().unwrap();
        map.retain(|_, e| !e.expired(now));
        map.get(id).map(|e| e.describe(id, "complete"))
    }

    /// Cancel a task; `false` if unknown.
    pub fn cancel(&self, id: &str) -> bool {
        let mut map = self.map.lock().unwrap();
        let Some(e) = map.get_mut(id) else {
            return false;
        };
        if matches!(e.state, State::Working) {
            e.state = State::Cancelled;
            e.updated = SystemTime::now();
            // notify_one keeps a permit, so a cancel that races ahead of the wait is not lost
            e.cancel.notify_one();
        }
        true
    }

    /// Whether a task with this id exists (for `tasks/update`).
    pub fn contains(&self, id: &str) -> bool {
        self.map.lock().unwrap().contains_key(id)
    }
}

/// ISO 8601 / RFC 3339 UTC timestamp, e.g. `2026-11-22T10:30:00Z`.
pub fn rfc3339(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil-from-days (Howard Hinnant), valid for all dates after 1970
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timestamps() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(1_795_336_200); // 2026-11-22 08:30 UTC
        assert_eq!(rfc3339(t), "2026-11-22T08:30:00Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert_eq!(rfc3339(leap), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn detects_the_declared_extension() {
        let with = json!({ "_meta": { META_CAPABILITIES: { "extensions": { EXTENSION: {} } } } });
        assert!(declared(&with));
        assert!(!declared(&json!({ "_meta": { META_CAPABILITIES: {} } })));
        assert!(!declared(&json!({})));
    }

    #[test]
    fn task_lifecycle() {
        let tasks = Tasks::default();
        let (id, _cancel, created) = tasks.create(Duration::from_secs(60));
        assert_eq!(created["resultType"], "task");
        assert_eq!(created["status"], "working");
        assert_eq!(created["ttlMs"], 660_000);
        assert_eq!(tasks.get(&id).unwrap()["resultType"], "complete");
        assert_eq!(tasks.get(&id).unwrap()["status"], "working");

        tasks.complete(&id, json!({ "content": [] }));
        let done = tasks.get(&id).unwrap();
        assert_eq!(done["status"], "completed");
        assert_eq!(done["result"]["content"], json!([]));
        assert!(tasks.get("nope").is_none());
    }

    #[tokio::test]
    async fn cancelling_signals_the_work_and_wins_over_a_late_result() {
        let tasks = Tasks::default();
        let (id, cancel, _) = tasks.create(Duration::from_secs(60));
        assert!(tasks.cancel(&id));
        cancel.notified().await; // permit stored
        tasks.complete(&id, json!({ "content": [] }));
        assert_eq!(tasks.get(&id).unwrap()["status"], "cancelled");
        assert!(!tasks.cancel("nope"));
    }
}
