//! The seam the Loop domain publishes to the Adapters.
//!
//! An Adapter reads Subscriptions to decide if an inbound event is its work —
//! never writing one, never reading a Schedule, a receipt, or a Loop's config
//! rows. Only the five reads below cross, not the whole Loop store.
//!
//! [`SubscriptionMatch`] lives in `moatless_types::loops`; [`LoopReader`] is
//! implemented in `moatless-loops`. [`subscription_context`] projects a
//! Subscription into the [`LoopContext`] shape older adapter methods accept.

use async_trait::async_trait;
use chrono::Utc;

use moatless_types::adapter::{AdapterConnection, AdapterKind};
use moatless_types::loops::{LoopContext, RoutingMode, SubscriptionMatch};
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

/// Build the config-only Subscription used when a connection has no matching
/// Loop.
///
/// Never persisted or used to authorize Task creation; it exists only to keep
/// normalization, metrics, and message labelling consistent for events
/// delivered to existing Bindings.
pub fn delivery_only_subscription(
    connection: &AdapterConnection,
    adapter_kind: AdapterKind,
    source_matcher: &str,
) -> SubscriptionMatch {
    let now = Utc::now();
    SubscriptionMatch {
        id: SubscriptionMatch::delivery_only_id(&connection.id),
        // Not a real Loop, so it has no configured name — say so rather than
        // borrowing one that would read as a Loop in any UI that shows it.
        name: "delivery-only".to_string(),
        adapter_kind,
        connection_id: connection.id.clone(),
        source_matcher: source_matcher.to_string(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        workspace_id: String::new(),
        prompt_template: None,
        task_name: None,
        tag_ids: Vec::new(),
        memory_preload: Vec::new(),
        skills: Vec::new(),
        // Never `None`: an org connection owned by nobody would otherwise
        // panic every reader of `SubscriptionMatch::effective_user_id`, which
        // is where an unmapped sender lands. Whoever connected it, then.
        run_as_user_id: Some(
            connection
                .owner_user_id
                .clone()
                .unwrap_or_else(|| connection.created_by.clone()),
        ),
        agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
        model: None,
        close_after_settled_days: None,
        settle_after_hours: None,
        node_selector: None,
        stop_when_done: None,
        message_buffer_seconds: None,
        allow_direct_send: false,
        adapter_config: None,
        ongoing_task_id: None,
        close_task_on_close: false,
        notify_on_success: false,
        active: true,
        created_by: connection.created_by.clone(),
        scope: connection.scope,
        owner_user_id: connection.owner_user_id.clone(),
        created_at: now,
        updated_at: now,
    }
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
        memory_preload: subscription.memory_preload.clone(),
        skills: subscription.skills.clone(),
        run_as_user_id: subscription.run_as_user_id.clone(),
        agent_profile: subscription.agent_profile,
        model: subscription.model.clone(),
        close_after_settled_days: subscription.close_after_settled_days,
        settle_after_hours: subscription.settle_after_hours,
        node_selector: subscription.node_selector.clone(),
        stop_when_done: subscription.stop_when_done.unwrap_or(false),
        created_by: subscription.created_by.clone(),
        created_at: subscription.created_at,
        updated_at: subscription.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_types::access_control::Scope;

    use super::*;

    fn connection() -> AdapterConnection {
        let now = Utc::now();
        AdapterConnection {
            id: "pc_org".to_string(),
            adapter_kind: AdapterKind::Github,
            connection_kind: "org_webhook".to_string(),
            scope_value: "acme".to_string(),
            credential_ref: None,
            webhook_secret: Some("s3cr3t".to_string()),
            transport_config: serde_json::json!({}),
            created_by: "u_installer".to_string(),
            scope: Scope::User,
            owner_user_id: Some("u_owner".to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    /// Every writer of a Subscription id — the buffer above all — has to tell
    /// this one apart from a Loop's, since no row answers to it.
    #[test]
    fn a_connection_derived_subscription_says_it_is_delivery_only() {
        let subscription =
            delivery_only_subscription(&connection(), AdapterKind::GithubPr, "acme/widget");

        assert!(subscription.is_delivery_only());
        assert_eq!(subscription.connection_id, "pc_org");
    }
}
