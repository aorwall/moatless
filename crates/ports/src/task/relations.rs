use async_trait::async_trait;
use moatless_types::task::RelatedTask;

/// Recording that one Task relates to another.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait RelationRecorder: Send + Sync {
    /// Best-effort: a relation that cannot be written is logged, never raised.
    /// The Task it would have labelled is already there and usable.
    async fn record_related_to(&self, from_task_id: &str, related: &RelatedTask);
}
