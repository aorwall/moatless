//! The preview tabs open on each Task, and the fan-out that tells subscribers
//! when one changes.
//!
//! Held in this process only, so a tab written on one replica is invisible to
//! another. Deliberately a placeholder for a Postgres-backed store — see
//! `docs/deferred-changes.md#preview-tabs-are-per-replica`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::warn;
use uuid::Uuid;

#[cfg(test)]
mod tests;

/// Events buffered per connected subscriber before the slowest one starts
/// losing them.
///
/// A lagged subscriber gets `RecvError::Lagged` and reconciles via
/// `preview.list` on the next mutation; deep enough that a burst of opens on
/// one Task cannot trigger it in practice.
const EVENT_BUFFER: usize = 256;

/// How long a Task's tabs outlive their last mutation.
///
/// Passed in so it can track the sandbox idle TTL — a tab should not outlive
/// the servers it points at. Refreshed on every write.
#[derive(Debug, Clone, Copy)]
pub struct PreviewTabTtl(pub Duration);

/// One browser tab, in the shape `PreviewSessionSnapshot` has on the wire
/// (`packages/contracts/src/preview.ts`).
///
/// `nav_status` and `viewport` stay as JSON: both are tagged unions this
/// backend never inspects, only stores and returns, so a Rust enum would just
/// be a second place for the contract to drift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTab {
    pub thread_id: String,
    pub tab_id: String,
    pub nav_status: Value,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Value>,
    pub updated_at: String,
}

/// Every tab on one Task, and the position that read was taken at.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTabListing {
    pub tabs: Vec<PreviewTab>,
    pub revision: u64,
}

/// What a close removed, and the revision it removed them at.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewTabRemoval {
    pub tab_ids: Vec<String>,
    pub revision: u64,
}

/// The set of tabs open on each Task, and the fan-out that tells subscribers
/// when one changes.
pub struct PreviewTabs {
    epoch: String,
    ttl: Duration,
    state: Mutex<HashMap<String, TaskTabs>>,
    events: broadcast::Sender<Value>,
}

struct TaskTabs {
    tabs: HashMap<String, PreviewTab>,
    revision: u64,
    /// Refreshed on every access; the sweep in [`PreviewTabs::locked`] drops
    /// the Task once it passes. Without it a long-lived replica accumulates
    /// tabs for every Task it has ever served.
    expires_at: Instant,
}

impl PreviewTabs {
    pub fn new(ttl: PreviewTabTtl) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            // A fresh epoch per process is correct here and not a shortcut:
            // the contents die with the process, so a restart genuinely is a
            // different store and clients should discard what they hold.
            epoch: Uuid::new_v4().to_string(),
            ttl: ttl.0,
            state: Mutex::new(HashMap::new()),
            events,
        }
    }

    /// Identifies this store, which a client compares to decide whether the
    /// revisions it holds are comparable at all.
    ///
    /// Fixed for the life of the handle, so a TTL sweep resets a Task's
    /// revision to 0 without changing it: a client must read a lower revision
    /// as a reset rather than as stale.
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn list(&self, task_id: &str) -> PreviewTabListing {
        let state = self.locked();
        let Some(task) = state.get(task_id) else {
            return PreviewTabListing {
                tabs: Vec::new(),
                revision: 0,
            };
        };
        let mut tabs: Vec<PreviewTab> = task.tabs.values().cloned().collect();
        tabs.sort_by(|left, right| left.tab_id.cmp(&right.tab_id));
        PreviewTabListing {
            tabs,
            revision: task.revision,
        }
    }

    pub fn get(&self, task_id: &str, tab_id: &str) -> Option<PreviewTab> {
        self.locked()
            .get(task_id)
            .and_then(|task| task.tabs.get(tab_id))
            .cloned()
    }

    /// Write one whole tab and return the revision the write landed at.
    pub fn put(&self, tab: &PreviewTab) -> u64 {
        let expires_at = self.deadline();
        let mut state = self.locked();
        let task = state
            .entry(tab.thread_id.clone())
            .or_insert_with(|| TaskTabs::new(expires_at));
        task.tabs.insert(tab.tab_id.clone(), tab.clone());
        task.revision += 1;
        task.expires_at = expires_at;
        task.revision
    }

    /// Remove one tab, or every tab on the Task when `tab_id` is `None`.
    pub fn remove(&self, task_id: &str, tab_id: Option<&str>) -> PreviewTabRemoval {
        let expires_at = self.deadline();
        let mut state = self.locked();
        // No entry means nothing to close, and creating one here would make a
        // closed tab the reason a Task first appears in the map.
        let Some(task) = state.get_mut(task_id) else {
            return PreviewTabRemoval {
                tab_ids: Vec::new(),
                revision: 0,
            };
        };
        let mut tab_ids: Vec<String> = match tab_id {
            Some(tab_id) => task
                .tabs
                .remove(tab_id)
                .map(|tab| vec![tab.tab_id])
                .unwrap_or_default(),
            None => task.tabs.drain().map(|(tab_id, _)| tab_id).collect(),
        };
        tab_ids.sort();
        task.revision += 1;
        task.expires_at = expires_at;
        PreviewTabRemoval {
            tab_ids,
            revision: task.revision,
        }
    }

    /// Fan one rendered `PreviewEvent` to every subscriber on this replica.
    pub fn publish(&self, event: Value) {
        drop(self.events.send(event));
    }

    /// Every event published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    fn deadline(&self) -> Instant {
        Instant::now() + self.ttl
    }

    /// Take the lock, dropping any Task whose TTL has passed.
    ///
    /// The guard never crosses an `.await`, so a poison only comes from a panic
    /// in one of the short critical sections above. The map stays coherent
    /// after one — every mutation is a single insert, remove or drain — so
    /// recovering beats propagating the panic to every later caller.
    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<String, TaskTabs>> {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| {
            warn!("The in-process preview tab lock was poisoned; continuing");
            poisoned.into_inner()
        });
        let now = Instant::now();
        state.retain(|_, task| task.expires_at > now);
        state
    }
}

impl TaskTabs {
    fn new(expires_at: Instant) -> Self {
        Self {
            tabs: HashMap::new(),
            revision: 0,
            expires_at,
        }
    }
}
