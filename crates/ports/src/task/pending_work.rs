use async_trait::async_trait;
use moatless_types::store::StoreError;

/// Which Tasks on a Sandbox are waiting on the agent.
///
/// A Sandbox's wake signal says that work is queued somewhere on it, and
/// several Tasks can share one Sandbox. Delivering that wake means notifying
/// the pod per Task, so the control plane has to name the Tasks the work is
/// actually for. The predicate is a Task-domain fact, so the Task band answers
/// it and the control plane never reads `task_messages`.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait PendingWorkReader: Send + Sync {
    /// Every open Task sharing `task_id`'s Sandbox that holds a user message
    /// the agent has not claimed, `task_id` itself included when it does.
    ///
    /// Empty when the work settled between the wake and this read, which is
    /// ordinary: the caller notifies nobody rather than guessing.
    async fn members_awaiting_dispatch(&self, task_id: &str) -> Result<Vec<String>, StoreError>;
}
