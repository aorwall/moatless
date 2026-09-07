//! The Binding reads and deactivation the Task domain requires.

use std::collections::HashMap;

use async_trait::async_trait;

use moatless_api_schemas::bindings::TaskBindingSummary;

use moatless_types::store::StoreError;

/// The Bindings a Task carries.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TaskBindings: Send + Sync {
    /// Every Binding on the Task, each already labelled with the Loop that
    /// owns it. A Binding whose Loop cannot be read is still returned, labelled
    /// from the Binding alone.
    async fn summaries(&self, task_id: &str) -> Result<Vec<TaskBindingSummary>, StoreError>;

    /// The same, for a page of Tasks, keyed by task id. Carries the Loop label
    /// only — a listing shows which Loops a Task answers on, not each
    /// Binding's own subject.
    async fn list_summaries(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, Vec<TaskBindingSummary>>, StoreError>;

    /// Soft-deactivate every still-active Binding of a Task and report how many
    /// changed. Idempotent: a second call reports 0.
    async fn deactivate_for_task(&self, task_id: &str) -> Result<u64, StoreError>;
}

/// Stop inbound adapter events after a task reaches a terminal state.
/// Failures do not block cleanup because runtime admission also rejects events
/// for closed tasks.
pub(crate) async fn deactivate_task_bindings(bindings: &dyn TaskBindings, task_id: &str) {
    match bindings.deactivate_for_task(task_id).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(task_id, deactivated = n, "Deactivated task bindings"),
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to deactivate task bindings");
        }
    }
}
