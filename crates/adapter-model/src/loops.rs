//! The seam the Loop domain publishes to the Adapters.
//!
//! An Adapter reads Subscriptions to decide whether an inbound event is work
//! it should do. It never writes one, and it never reads a Schedule, a receipt,
//! or a Loop's config rows — so what crosses is the five reads below, not the
//! Loop store and its twenty-six functions.
//!
//! Declared in the Adapter kernel: every consumer already names this crate for
//! the Adapter contract, and [`SubscriptionMatch`] — the value every read
//! returns — is a kernel model, so a lower home would drag the model down or
//! force a second shape over the same rows. The Loop store implements it in the
//! `moatless-loops` crate, beside the tables it reads.

use async_trait::async_trait;

use crate::models::SubscriptionMatch;
use moatless_types::store::StoreError;

/// The Subscriptions an inbound provider event is matched against.
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
