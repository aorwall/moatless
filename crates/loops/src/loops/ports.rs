//! The seam the Loop domain publishes to the Adapters.
//!
//! An Adapter reads Subscriptions to decide whether an inbound event is work
//! it should do. It never writes one, and it never reads a Schedule, a
//! receipt, or a Loop's config rows — so what crosses is the five reads below,
//! not `LoopDao` and its twenty-six functions.
//!
//! Declared here rather than in `moatless-backend-core`: every Adapter already
//! depends on this crate for the Adapter kernel, so a `backend-core` hop would
//! buy nothing. Same reasoning as `bindings::ports::BindingProbe`.

use async_trait::async_trait;
use moatless_backend_core::store::StoreError;

use moatless_adapter_model::models::SubscriptionMatch;
use crate::loops::dao::LoopDao;

/// The Subscriptions an inbound provider event is matched against.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait LoopReader: Send + Sync {
    /// The Subscription with this id, whatever its state. A Binding whose
    /// Subscription was deleted keeps working, so `None` means "no
    /// Subscription context", never "stop".
    async fn subscription(&self, loop_id: &str) -> Result<Option<SubscriptionMatch>, StoreError>;

    /// The same Subscription, but only when it is active, undeleted, and names
    /// the identity it runs as — the state a firing needs.
    async fn runnable_subscription(
        &self,
        loop_id: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError>;

    /// The oldest runnable Subscription an Adapter's source matcher names.
    async fn subscription_for_source(
        &self,
        adapter_kind: &str,
        source_matcher: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError>;

    /// Every runnable Subscription an Adapter's source matcher names, oldest
    /// first.
    async fn subscriptions_for_source(
        &self,
        adapter_kind: &str,
        source_matcher: &str,
    ) -> Result<Vec<SubscriptionMatch>, StoreError>;

    /// The Adapter's catch-all Subscription, for a source no matcher named.
    async fn wildcard_subscription(
        &self,
        adapter_kind: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError>;
}

/// The Loop tables answer the Subscription reads directly: `LoopDao` is the
/// only store over them, so a wrapper here would be a second surface over one
/// table. Same shape as [`crate::bindings::ports::BindingProbe`].
#[async_trait]
impl LoopReader for LoopDao {
    async fn subscription(&self, loop_id: &str) -> Result<Option<SubscriptionMatch>, StoreError> {
        self.find_subscription_by_id(loop_id)
            .await
            .map_err(|e| StoreError::with_source(format!("read subscription {loop_id}"), e))
    }

    async fn runnable_subscription(
        &self,
        loop_id: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError> {
        self.find_runnable_subscription_by_id(loop_id)
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read runnable subscription {loop_id}"), e)
            })
    }

    async fn subscription_for_source(
        &self,
        adapter_kind: &str,
        source_matcher: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError> {
        self.find_subscription_by_source_matcher(adapter_kind, source_matcher)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the {adapter_kind} subscription for {source_matcher}"),
                    e,
                )
            })
    }

    async fn subscriptions_for_source(
        &self,
        adapter_kind: &str,
        source_matcher: &str,
    ) -> Result<Vec<SubscriptionMatch>, StoreError> {
        self.list_subscriptions_by_source_matcher(adapter_kind, source_matcher)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("list the {adapter_kind} subscriptions for {source_matcher}"),
                    e,
                )
            })
    }

    async fn wildcard_subscription(
        &self,
        adapter_kind: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError> {
        self.find_subscription_wildcard(adapter_kind)
            .await
            .map_err(|e| {
                StoreError::with_source(
                    format!("read the {adapter_kind} wildcard subscription"),
                    e,
                )
            })
    }
}
