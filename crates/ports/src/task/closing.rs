use async_trait::async_trait;
use moatless_types::store::StoreError;

/// Why a provider closed a Task, and what its `Closed` event carries.
///
/// The task band does not read `payload`: the provider owns its shape.
#[derive(Debug, Clone)]
pub struct CloseReason {
    pub actor: String,
    pub payload: serde_json::Value,
}

/// Closing a Task from the provider path.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait TaskFinisher: Send + Sync {
    /// Close `task_id` as the system. A Task that does not exist and a Task the
    /// caller may not control answer alike, so a close never discloses that a
    /// Task is there.
    async fn close(&self, task_id: &str, reason: CloseReason) -> Result<(), StoreError>;
}
