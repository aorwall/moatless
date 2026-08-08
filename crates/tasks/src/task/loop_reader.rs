//! The one loop read the task presenter needs, declared in the tasks band.
//!
//! `task/presenter.rs` labels a task's bindings with the triggering loop's
//! subscription. That is its only reach into `loops`, so instead of holding the
//! whole `LoopDao` (loops band, upward) it holds this single-question port; the
//! concrete `LoopDao` in `loops/dao.rs` implements it.

use moatless_adapter_model::models::SubscriptionMatch;

#[async_trait::async_trait]
pub trait LoopSubscriptionReader: Send + Sync {
    /// Resolve a loop id to its subscription match, or `None` when the loop has
    /// none (e.g. a schedule loop). Used only for display labelling.
    async fn find_subscription_by_id(
        &self,
        id: &str,
    ) -> Result<Option<SubscriptionMatch>, sqlx::Error>;
}
