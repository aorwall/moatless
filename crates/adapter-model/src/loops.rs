//! The seam the Loop domain publishes to the Adapters.
//!
//! An Adapter reads Subscriptions to decide whether an inbound event is work it
//! should do — never writing one, and never reading a Schedule, a receipt, or a
//! Loop's config rows. What crosses is the five reads below, not the Loop store
//! and its twenty-six functions.
//!
//! [`SubscriptionMatch`], the value every read returns, lives in
//! `moatless_types::loops`; the Loop store implements the reader in
//! `moatless-loops`, beside the tables it reads.
//!
//! [`subscription_context`] projects a Subscription into the config-shaped
//! [`LoopContext`] the older adapter trait methods still accept.

use async_trait::async_trait;

use moatless_types::loops::{LoopContext, SubscriptionMatch};
use moatless_types::store::StoreError;

/// The Subscriptions an inbound provider event is matched against.
#[async_trait]
pub trait LoopReader: Send + Sync {
    /// The Subscription with this id, whatever its state. A Binding whose
    /// Subscription was deleted keeps working, so `None` means "no Subscription
    /// context", never "stop".
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

/// Build a synthetic `LoopContext` from a subscription view for adapter trait
/// methods that still accept a config-shaped value.
pub fn subscription_context(subscription: &SubscriptionMatch) -> LoopContext {
    LoopContext {
        id: subscription.id.clone(),
        adapter_kind: subscription.adapter_kind.as_str().to_string(),
        source_id: subscription.source_matcher.clone(),
        source_name: subscription.source_name.clone(),
        workspace_id: subscription.workspace_id.clone(),
        routing_mode: subscription.routing_mode,
        prompt_template: subscription.prompt_template.clone(),
        active: subscription.active,
        allow_direct_send: subscription.allow_direct_send,
        adapter_config: subscription.adapter_config.clone(),
        ongoing_task_id: subscription.ongoing_task_id.clone(),
        scope: subscription.scope,
        task_name: subscription.task_name.clone(),
        tag_ids: subscription.tag_ids.clone(),
        skills: subscription.skills.clone(),
        run_as_user_id: subscription.run_as_user_id.clone(),
        agent_type: subscription.agent_type.clone(),
        model: subscription.model.clone(),
        inactive_ttl_days: subscription.inactive_ttl_days,
        created_by: subscription.created_by.clone(),
        created_at: subscription.created_at,
        updated_at: subscription.updated_at,
    }
}
