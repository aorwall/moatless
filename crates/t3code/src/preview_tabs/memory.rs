use super::contracts::{
    EVENT_BUFFER, PreviewTab, PreviewTabDao, PreviewTabError, PreviewTabListing, PreviewTabRemoval,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::broadcast;
use tracing::warn;
use uuid::Uuid;
// =============================================================================
// In memory
// =============================================================================

/// The single-process store, for a deployment that configures no Redis.
///
/// Agrees with the Redis one about everything a caller can observe — enough to
/// run the same test body against both — but cannot agree with a *second*
/// process, so it is not the production implementation.
pub struct InMemoryPreviewTabDao {
    epoch: String,
    state: Mutex<HashMap<String, TaskTabs>>,
    events: broadcast::Sender<Value>,
}

#[derive(Debug, Default)]
struct TaskTabs {
    tabs: HashMap<String, PreviewTab>,
    revision: u64,
}

impl InMemoryPreviewTabDao {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            // A fresh epoch per process is correct here and not a shortcut:
            // the contents die with the process, so a restart genuinely is a
            // different store and clients should discard what they hold.
            epoch: Uuid::new_v4().to_string(),
            state: Mutex::new(HashMap::new()),
            events,
        }
    }
}

impl InMemoryPreviewTabDao {
    /// The guard never crosses an `.await`, so a poison only comes from a panic
    /// in one of the short critical sections below.
    ///
    /// The map stays coherent after one — every mutation is a single insert,
    /// remove or drain — so recovering beats propagating the panic to every
    /// later caller.
    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<String, TaskTabs>> {
        self.state.lock().unwrap_or_else(|poisoned| {
            warn!("The in-process preview tab lock was poisoned; continuing");
            poisoned.into_inner()
        })
    }
}

impl Default for InMemoryPreviewTabDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PreviewTabDao for InMemoryPreviewTabDao {
    fn epoch(&self) -> &str {
        &self.epoch
    }

    async fn list(&self, task_id: &str) -> Result<PreviewTabListing, PreviewTabError> {
        let state = self.locked();
        let Some(task) = state.get(task_id) else {
            return Ok(PreviewTabListing {
                tabs: Vec::new(),
                revision: 0,
            });
        };
        let mut tabs: Vec<PreviewTab> = task.tabs.values().cloned().collect();
        tabs.sort_by(|left, right| left.tab_id.cmp(&right.tab_id));
        Ok(PreviewTabListing {
            tabs,
            revision: task.revision,
        })
    }

    async fn get(
        &self,
        task_id: &str,
        tab_id: &str,
    ) -> Result<Option<PreviewTab>, PreviewTabError> {
        let state = self.locked();
        Ok(state
            .get(task_id)
            .and_then(|task| task.tabs.get(tab_id))
            .cloned())
    }

    async fn put(&self, tab: &PreviewTab) -> Result<u64, PreviewTabError> {
        let mut state = self.locked();
        let task = state.entry(tab.thread_id.clone()).or_default();
        task.tabs.insert(tab.tab_id.clone(), tab.clone());
        task.revision += 1;
        Ok(task.revision)
    }

    async fn remove(
        &self,
        task_id: &str,
        tab_id: Option<&str>,
    ) -> Result<PreviewTabRemoval, PreviewTabError> {
        let mut state = self.locked();
        let task = state.entry(task_id.to_string()).or_default();
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
        Ok(PreviewTabRemoval {
            tab_ids,
            revision: task.revision,
        })
    }

    async fn publish(&self, event: Value) -> Result<(), PreviewTabError> {
        drop(self.events.send(event));
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }
}
