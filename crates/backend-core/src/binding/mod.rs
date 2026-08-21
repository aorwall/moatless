//! The seam the Binding domain publishes downward.
//!
//! Binding belongs to `moatless-loops`, which sits above `moatless-tasks`, so
//! the Task band cannot name the store that holds it. What it needs is two
//! answers — what a Task is bound to, and stop listening — and those are what
//! crosses. The Adapters sit beside `loops` and name its own seams directly;
//! only this downward edge goes through `backend-core`.

use std::collections::HashMap;

use async_trait::async_trait;

pub use moatless_api_schemas::bindings::TaskBindingSummary;

use crate::store::StoreError;

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
