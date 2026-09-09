//! `LoopDao`'s implementation of the Loop read seam.
//!
//! The [`LoopReader`] trait is published by the Adapter kernel
//! (`moatless_adapter_model::loops`); this file is the other end of it. An
//! Adapter reads Subscriptions to decide whether an inbound event is work it
//! should do — the five reads below — and never writes one, reads a Schedule,
//! a receipt, or a Loop's config rows, so the Loop store stays private to this
//! crate.

use async_trait::async_trait;
use moatless_types::store::StoreError;

use crate::loops::dao::LoopDao;
use moatless_adapter_model::loops::LoopReader;
use moatless_types::loops::SubscriptionMatch;

/// The Loop tables answer the Subscription reads directly: `LoopDao` is the
/// only store over them, so a wrapper here would be a second surface over one
/// table. Same shape as [`crate::bindings::ports::BindingDirectory`]'s
/// `BindingProbe` implementation.
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
                StoreError::with_source(format!("read the {adapter_kind} wildcard subscription"), e)
            })
    }
}
