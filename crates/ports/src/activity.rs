//! The read port over activities recorded by a Task.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_types::activity::Activity;
use moatless_types::store::StoreError;

/// The activities a Task recorded, for a read model that projects them.
///
/// arch-allow(port-admission): keeps the T3 projection independent of the Task implementation; remove when another read seam replaces it.
/// arch-allow(single-implementation-trait): the Task-owned read port keeps the T3 projection independent; remove when that projection uses another source.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait ActivitySource: Send + Sync {
    /// Top-level activities for a Task, oldest first, from `since` onward.
    async fn since(
        &self,
        task_id: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Activity>, StoreError>;

    /// The activities these Messages produced. An empty slice reads nothing.
    async fn for_messages(
        &self,
        task_id: &str,
        message_uuids: &[String],
    ) -> Result<Vec<Activity>, StoreError>;

    /// The activities these ids name. An empty slice reads nothing.
    async fn for_ids(
        &self,
        task_id: &str,
        activity_ids: &[String],
    ) -> Result<Vec<Activity>, StoreError>;

    /// The one tool call this `tool_use_id` names, whatever its state.
    async fn tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<Activity>, StoreError>;
}
