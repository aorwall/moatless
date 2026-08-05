use std::sync::Arc;

use super::super::capabilities::Adapter;
use super::Adapters;
use crate::adapters::error::AdapterError;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::test_support::StubProviderTokens;
use crate::db::DbPool;
use moatless_api_schemas::enums::AdapterKind;
use sqlx::postgres::PgPoolOptions;

struct DummyProvider;

struct IdentityOnlyProvider {
    kind: AdapterKind,
}

impl Adapter for IdentityOnlyProvider {
    fn adapter_kind(&self) -> AdapterKind {
        self.kind
    }
}

impl Adapter for DummyProvider {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Telegram
    }
}

#[test]
fn empty_registry_takes_a_stand_in_adapter() {
    let mut providers = Adapters::new();
    providers.register_trait_object(Arc::new(DummyProvider));

    assert_eq!(providers.registered_types(), vec![AdapterKind::Telegram]);
    assert!(providers.get(AdapterKind::Telegram).is_some());
}

fn lazy_pool() -> DbPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://postgres:postgres@localhost/moatless_test")
        .expect("lazy pool")
}

fn registry_with_all_current_adapters() -> Adapters {
    let mut providers = Adapters::new();
    providers.register_slack(Arc::new(super::super::slack::adapter::SlackAdapter::new(
        "xoxb-test",
        "slack-secret",
        Some("U_BOT".to_string()),
    )));
    providers.register_telegram(Arc::new(
        super::super::telegram::adapter::TelegramAdapter::new("telegram-token"),
    ));
    providers.register_github_pr(Arc::new(
        super::super::github_pr::adapter::GithubPrAdapter::new(
            None,
            None,
            "http://localhost".to_string(),
            Arc::new(StubProviderTokens::default()),
            None,
            None,
            Arc::new(GitHubAppRegistry::default()),
        ),
    ));
    providers.register_linear(Arc::new(super::super::linear::adapter::LinearAdapter::new(
        &crate::config::Config::test_minimal().linear,
        lazy_pool(),
    )));
    providers.register_webhook(Arc::new(
        super::super::webhook::adapter::WebhookAdapter::new(),
    ));
    providers
}

#[tokio::test]
async fn registry_exposes_expected_capability_slots_for_current_adapters() {
    let providers = registry_with_all_current_adapters();

    for kind in [
        AdapterKind::Slack,
        AdapterKind::Telegram,
        AdapterKind::GithubPr,
        AdapterKind::Linear,
    ] {
        assert!(providers.get(kind).is_some(), "{kind:?} identity adapter");
        assert!(
            providers
                .outbound_for_binding_reply_by_name(kind.as_str())
                .is_ok(),
            "{kind:?} binding reply"
        );
        assert!(
            providers
                .outbound_for_message_reply_by_name(kind.as_str())
                .is_ok(),
            "{kind:?} message reply"
        );
    }
    assert!(providers.get(AdapterKind::Webhook).is_some());
    assert!(providers
        .outbound_for_binding_reply_by_name("webhook")
        .is_ok());
    assert!(providers
        .outbound_for_message_reply_by_name("webhook")
        .is_ok());

    for kind in [
        AdapterKind::Slack,
        AdapterKind::Telegram,
        AdapterKind::GithubPr,
    ] {
        assert!(
            providers.destination_builder(kind).is_ok(),
            "{kind:?} destination builder"
        );
        assert!(providers.reactions(kind).is_ok(), "{kind:?} reactions");
        assert!(
            providers.inbound_presentation(kind).is_ok(),
            "{kind:?} inbound presentation"
        );
    }

    for kind in [AdapterKind::GithubPr, AdapterKind::Linear] {
        assert!(
            providers.binding_status(kind).is_some(),
            "{kind:?} binding status"
        );
    }

    assert!(providers.destination_builder(AdapterKind::Linear).is_err());
    assert!(providers.destination_builder(AdapterKind::Webhook).is_err());
    assert!(providers
        .outbound_for_direct_destination(AdapterKind::Slack)
        .is_ok());
    assert!(providers
        .outbound_for_direct_destination(AdapterKind::Linear)
        .is_ok());
    assert!(providers
        .outbound_for_direct_message(AdapterKind::Slack)
        .is_ok());
    assert!(providers.outbound_for_new_thread_by_name("slack").is_ok());
    assert!(providers
        .outbound_for_new_thread_by_name("telegram")
        .is_ok());
    assert!(providers
        .post_binding_followup(AdapterKind::Linear)
        .is_some());
    assert!(providers
        .outbound_for_direct_destination(AdapterKind::GithubPr)
        .is_err());
    assert!(providers
        .outbound_for_direct_destination(AdapterKind::Telegram)
        .is_err());
    assert!(providers
        .outbound_for_direct_destination(AdapterKind::Webhook)
        .is_err());
    assert!(providers
        .outbound_for_direct_message(AdapterKind::GithubPr)
        .is_err());
    assert!(providers
        .outbound_for_direct_message(AdapterKind::Linear)
        .is_err());
    assert!(providers
        .outbound_for_direct_message(AdapterKind::Telegram)
        .is_err());
    assert!(providers
        .outbound_for_direct_message(AdapterKind::Webhook)
        .is_err());
    assert!(providers.outbound_for_new_thread_by_name("github").is_err());
    assert!(providers.outbound_for_new_thread_by_name("linear").is_err());
    assert!(providers
        .outbound_for_new_thread_by_name("webhook")
        .is_err());
    assert!(providers
        .post_binding_followup(AdapterKind::Slack)
        .is_none());
    assert!(providers
        .post_binding_followup(AdapterKind::Webhook)
        .is_none());
    assert!(providers.reactions(AdapterKind::Linear).is_err());
    assert!(providers.reactions(AdapterKind::Webhook).is_err());
    assert!(providers
        .inbound_presentation(AdapterKind::Webhook)
        .is_err());
    assert!(providers.binding_status(AdapterKind::Slack).is_none());
    assert!(providers.binding_status(AdapterKind::Telegram).is_none());
    assert!(providers.binding_status(AdapterKind::Webhook).is_none());
}

#[tokio::test]
async fn direct_message_registration_requires_direct_destination_send() {
    let providers = registry_with_all_current_adapters();

    for kind in providers.direct_messages.keys() {
        assert!(
            providers.direct_destinations.contains_key(kind),
            "{kind:?} direct-message capability needs a direct-destination sender"
        );
    }
}

#[tokio::test]
async fn missing_capability_returns_adapter_error() {
    let providers = registry_with_all_current_adapters();

    match providers.reactions(AdapterKind::Linear) {
        Err(AdapterError::BadRequest(_)) => {}
        Err(other) => panic!("expected BadRequest, got: {other:?}"),
        Ok(_) => panic!("Linear should not have reaction capability"),
    }
}

#[tokio::test]
async fn capability_lookup_by_name_preserves_provider_aliases() {
    let providers = registry_with_all_current_adapters();

    assert_eq!(
        providers
            .outbound_for_binding_reply_by_name("github")
            .expect("github alias should resolve to GitHub PR binding reply")
            .adapter_kind(),
        AdapterKind::GithubPr
    );
    assert_eq!(
        providers
            .reactions_by_name("github")
            .expect("github alias should resolve to GitHub PR reactions")
            .adapter_kind(),
        AdapterKind::GithubPr
    );

    match providers.reactions_by_name("linear") {
        Err(AdapterError::BadRequest(_)) => {}
        Err(other) => panic!("expected BadRequest, got: {other:?}"),
        Ok(_) => panic!("Linear should not have reaction capability"),
    }
}

/// Name resolution reads the `Adapter` map, not a separate identity map. It must
/// still resolve a kind's canonical key, its provider aliases, and an unknown
/// name exactly as before — the `github` connection alias resolving to the
/// `github_pr` provider key is the case a silent regression here would break.
#[tokio::test]
async fn name_resolution_resolves_against_the_adapter_map() {
    let providers = registry_with_all_current_adapters();

    assert_eq!(providers.provider_key_for_name("slack"), Some("slack"));
    assert_eq!(
        providers.provider_key_for_name("github_pr"),
        Some("github_pr")
    );
    // `github` is GitHub PR's connection alias; the provider key it maps to is
    // the adapter target key `github_pr`.
    assert_eq!(providers.provider_key_for_name("github"), Some("github_pr"));
    assert_eq!(providers.provider_key_for_name("nope"), None);

    let github = providers
        .identity_by_name("github")
        .expect("github alias resolves to an adapter");
    assert_eq!(github.adapter_kind(), AdapterKind::GithubPr);
    assert_eq!(github.provider_key(), "github_pr");
    assert_eq!(github.connection_provider_key(), "github");

    assert!(providers.identity_by_name("nope").is_none());
}

#[tokio::test]
async fn missing_outbound_capability_returns_registry_owned_error() {
    let mut providers = Adapters::new();
    let kind = AdapterKind::Slack;
    providers.register_trait_object(Arc::new(IdentityOnlyProvider { kind }));

    match providers.outbound_for_direct_destination(kind) {
        Err(AdapterError::BadRequest(msg)) => {
            assert!(
                msg.contains("does not support direct destination sends"),
                "unexpected message: {msg}"
            );
        }
        Err(other) => panic!("expected BadRequest, got: {other:?}"),
        Ok(_) => panic!("identity-only adapter should not have outbound capability"),
    }
}

#[tokio::test]
async fn webhook_raw_destination_send_is_absent_from_registry() {
    let providers = registry_with_all_current_adapters();

    match providers.outbound_for_direct_destination(AdapterKind::Webhook) {
        Err(AdapterError::BadRequest(msg)) => {
            assert!(
                msg.contains("does not support direct destination sends"),
                "unexpected message: {msg}"
            );
        }
        Err(other) => panic!("expected BadRequest, got: {other:?}"),
        Ok(_) => panic!("webhook must not register raw direct destination capability"),
    }
}
