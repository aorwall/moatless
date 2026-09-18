//! The policy every consumer of a connection shares, over an in-memory store.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use moatless_types::access_control::{Scope, ScopeContext};
use moatless_types::adapter::AdapterKind;
use moatless_types::loops::{RoutingMode, SubscriptionMatch};
use moatless_types::store::StoreError;

use super::{
    ConnectionInstaller, ConnectionLookup, ConnectionReader, connection_for_auth,
    resolve_chat_provider_event, resolve_connection,
};
use crate::loops::LoopReader;
use crate::test_support::{InMemoryConnections, connection};

fn viewer() -> ScopeContext {
    ScopeContext {
        user_id: "u_viewer".into(),
        is_admin: false,
    }
}

fn subscription(id: &str, connection_id: &str, source_matcher: &str) -> SubscriptionMatch {
    let now = Utc::now();
    SubscriptionMatch {
        id: id.into(),
        name: "loop".into(),
        adapter_kind: AdapterKind::Slack,
        connection_id: connection_id.into(),
        source_matcher: source_matcher.into(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        workspace_id: "ws_1".into(),
        prompt_template: None,
        task_name: None,
        tag_ids: vec![],
        skills: vec![],
        run_as_user_id: Some("u_test".into()),
        agent_type: "claude-code".into(),
        model: None,
        inactive_ttl_days: None,
        node_selector: None,
        stop_when_done: None,
        message_buffer_seconds: None,
        allow_direct_send: true,
        adapter_config: None,
        ongoing_task_id: None,
        close_task_on_close: true,
        notify_on_success: false,
        active: true,
        created_by: "system".into(),
        scope: Scope::Global,
        owner_user_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// A `LoopReader` answering the two reads the router makes with whatever the
/// case under test seeded, and nothing else.
struct Subscriptions {
    for_source: Option<SubscriptionMatch>,
    wildcard: Option<SubscriptionMatch>,
}

#[async_trait]
impl LoopReader for Subscriptions {
    async fn subscription(&self, _: &str) -> Result<Option<SubscriptionMatch>, StoreError> {
        Ok(None)
    }
    async fn runnable_subscription(
        &self,
        _: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError> {
        Ok(None)
    }
    async fn subscription_for_source(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError> {
        Ok(self.for_source.clone())
    }
    async fn subscriptions_for_source(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<SubscriptionMatch>, StoreError> {
        Ok(vec![])
    }
    async fn wildcard_subscription(
        &self,
        _: &str,
    ) -> Result<Option<SubscriptionMatch>, StoreError> {
        Ok(self.wildcard.clone())
    }
}

#[tokio::test]
async fn a_named_connection_of_another_adapter_is_refused() {
    let store = InMemoryConnections::new(vec![connection("pc_1", AdapterKind::Slack, "T1")]);
    let err = resolve_connection(&store, &viewer(), AdapterKind::Linear, Some("pc_1"))
        .await
        .expect_err("adapter kind mismatch");
    assert_eq!(
        err.to_string(),
        "Invalid request: Connection pc_1 is for adapter slack, not linear"
    );
}

#[tokio::test]
async fn the_single_accessible_connection_needs_no_id() {
    let store = InMemoryConnections::new(vec![
        connection("pc_1", AdapterKind::Slack, "T1"),
        connection("pc_2", AdapterKind::Linear, "org"),
    ]);
    let resolved = resolve_connection(&store, &viewer(), AdapterKind::Slack, None)
        .await
        .expect("the one slack connection");
    assert_eq!(resolved.id, "pc_1");
}

#[tokio::test]
async fn no_connection_of_that_adapter_names_the_command_that_creates_one() {
    let store = InMemoryConnections::default();
    let err = resolve_connection(&store, &viewer(), AdapterKind::Slack, None)
        .await
        .expect_err("nothing configured");
    assert_eq!(
        err.to_string(),
        "Invalid request: No slack connection configured. \
         Create one with `moat adapters connections create --adapter slack ...` first."
    );
}

#[tokio::test]
async fn several_accessible_connections_list_the_ids_to_choose_from() {
    let store = InMemoryConnections::new(vec![
        connection("pc_1", AdapterKind::Slack, "T1"),
        connection("pc_2", AdapterKind::Slack, "T2"),
        connection("pc_3", AdapterKind::Slack, "T3"),
    ]);
    let err = resolve_connection(&store, &viewer(), AdapterKind::Slack, None)
        .await
        .expect_err("ambiguous");
    assert_eq!(
        err.to_string(),
        "Invalid request: Multiple slack connections accessible: [pc_1, pc_2, pc_3]. \
         Pass --connection <id> to choose."
    );
}

#[tokio::test]
async fn a_connection_the_viewer_cannot_see_is_not_found() {
    let store = InMemoryConnections::default();
    let err = connection_for_auth(&store, &viewer(), "pc_1")
        .await
        .expect_err("not visible");
    assert_eq!(
        err.to_string(),
        "Not found: Provider connection pc_1 not found"
    );
}

/// Routing a tenant's event to whichever connection happens to be installed
/// would hand its messages to another tenant's workspace.
#[tokio::test]
async fn an_unknown_scope_value_does_not_fall_back_to_another_connection() {
    let store = InMemoryConnections::new(vec![connection("pc_1", AdapterKind::Slack, "default")]);
    let loops = Subscriptions {
        for_source: Some(subscription("sub_1", "pc_1", "C1")),
        wildcard: None,
    };
    let matched = resolve_chat_provider_event(&store, &loops, "slack", "T_other", "C1")
        .await
        .expect("routing");
    assert!(matched.is_none());
}

#[tokio::test]
async fn a_subscription_on_another_connection_does_not_match() {
    let store = InMemoryConnections::new(vec![connection("pc_1", AdapterKind::Slack, "T1")]);
    let loops = Subscriptions {
        for_source: Some(subscription("sub_1", "pc_other", "C1")),
        wildcard: None,
    };
    let matched = resolve_chat_provider_event(&store, &loops, "slack", "T1", "C1")
        .await
        .expect("routing");
    assert!(matched.is_none());
}

#[tokio::test]
async fn an_unmatched_source_falls_back_to_the_adapters_wildcard() {
    let store = InMemoryConnections::new(vec![connection("pc_1", AdapterKind::Slack, "T1")]);
    let loops = Subscriptions {
        for_source: None,
        wildcard: Some(subscription("sub_wild", "pc_1", "*")),
    };
    let (connection, subscription) =
        resolve_chat_provider_event(&store, &loops, "slack", "T1", "C9")
            .await
            .expect("routing")
            .expect("the wildcard subscription");
    assert_eq!(connection.id, "pc_1");
    assert_eq!(subscription.id, "sub_wild");
}

#[tokio::test]
async fn an_install_replaces_the_connection_for_the_same_external_account() {
    let store = Arc::new(InMemoryConnections::new(vec![connection(
        "pc_old",
        AdapterKind::Linear,
        "org",
    )]));
    let installer: Arc<dyn ConnectionInstaller> = store.clone();
    installer
        .install(&connection("pc_new", AdapterKind::Linear, "org"))
        .await
        .expect("install");
    let lookup: Arc<dyn ConnectionLookup> = store.clone();
    let found = lookup
        .by_external_account("linear", "org")
        .await
        .expect("lookup")
        .expect("the installed connection");
    assert_eq!(found.id, "pc_new");
    let reader: Arc<dyn ConnectionReader> = store;
    assert_eq!(
        reader
            .list(&viewer(), Some(AdapterKind::Linear))
            .await
            .expect("list")
            .len(),
        1
    );
}
