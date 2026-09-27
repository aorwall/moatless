use async_trait::async_trait;
use moatless_types::store::StoreError;

/// Which Tasks on a Sandbox are waiting on the agent.
///
/// Several Tasks can share one Sandbox, so delivering its wake signal means
/// naming which of them the work is actually for — a Task-domain fact the
/// control plane never reads `task_inbox` to answer itself.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait PendingWorkReader: Send + Sync {
    /// Every open Task sharing `task_id`'s Sandbox holding an unclaimed user
    /// message, `task_id` itself included when it does.
    ///
    /// Empty when the work settled between the wake and this read; the caller
    /// then notifies nobody rather than guessing.
    async fn members_awaiting_dispatch(&self, task_id: &str) -> Result<Vec<String>, StoreError>;
}
