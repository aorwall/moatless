use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::broadcast;
/// Events buffered per connected subscriber before the slowest one starts
/// losing them.
///
/// A lagged subscriber gets `RecvError::Lagged` and reconciles via
/// `preview.list` on the next mutation; deep enough that a burst of opens on
/// one Task cannot trigger it in practice.
pub(super) const EVENT_BUFFER: usize = 256;

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

#[derive(Debug, thiserror::Error)]
pub enum PreviewTabError {
    /// The store could not be reached or refused the command. Every operation
    /// here is safe to retry, because every write is a whole snapshot.
    #[error("the preview tab store is unavailable")]
    Unavailable(#[source] redis::RedisError),
    /// A stored snapshot could not be read back. Retrying cannot help; the
    /// value is skipped and the rest of the listing is still answered.
    #[error("a stored preview tab could not be decoded")]
    Undecodable(#[source] serde_json::Error),
}

/// The set of tabs open on each Task, and the fan-out that tells every replica
/// when one changes.
#[async_trait]
pub trait PreviewTabDao: Send + Sync {
    /// Identifies the store's *contents*, not the process reading them.
    ///
    /// A client compares it to decide whether held revisions are comparable at
    /// all, so it must survive a restart of this handle and change when the
    /// store is emptied.
    fn epoch(&self) -> &str;

    async fn list(&self, task_id: &str) -> Result<PreviewTabListing, PreviewTabError>;

    async fn get(&self, task_id: &str, tab_id: &str)
    -> Result<Option<PreviewTab>, PreviewTabError>;

    /// Write one whole tab and return the revision the write landed at.
    async fn put(&self, tab: &PreviewTab) -> Result<u64, PreviewTabError>;

    /// Remove one tab, or every tab on the Task when `tab_id` is `None`.
    async fn remove(
        &self,
        task_id: &str,
        tab_id: Option<&str>,
    ) -> Result<PreviewTabRemoval, PreviewTabError>;

    /// Fan one rendered `PreviewEvent` to every replica, including this one.
    async fn publish(&self, event: Value) -> Result<(), PreviewTabError>;

    /// Every event published from now on, by any replica.
    fn subscribe(&self) -> broadcast::Receiver<Value>;
}
