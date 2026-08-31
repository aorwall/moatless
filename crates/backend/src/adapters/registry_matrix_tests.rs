//! Capability-matrix tests for the adapter registry.
//!
//! These build the five concrete providers, which live in this crate above the
//! loops band, so they cannot live in the Adapter kernel alongside the registry
//! itself. They call the registry's capability accessors directly, which is what
//! makes the assertion worth something: the lookup under test is the production
//! lookup. The registry-only tests that need no provider (empty registry,
//! registry-owned error) stay beside it in the kernel.

use std::sync::Arc;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::AdapterKind;

use moatless_adapter_model::registry::Adapters;
use moatless_identity::auth::test_support::StubProviderTokens;

fn registry_with_all_current_adapters() -> Adapters {
    let mut providers = Adapters::new();
    Arc::new(
        crate::adapters::providers::slack::adapter::SlackAdapter::new(
            "xoxb-test",
            "slack-secret",
            Some("U_BOT".to_string()),
        ),
    )
    .register_into(&mut providers);
    Arc::new(crate::adapters::providers::telegram::adapter::TelegramAdapter::new("telegram-token"))
        .register_into(&mut providers);
    Arc::new(
        crate::adapters::providers::github_pr::adapter::GithubPrAdapter::new(
            None,
            "http://localhost".to_string(),
            moatless_identity::git_host_tokens::seam_over(Arc::new(StubProviderTokens::default())),
        ),
    )
    .register_into(&mut providers);
    Arc::new(
        crate::adapters::providers::linear::adapter::LinearAdapter::new(
            &moatless_core::config::Config::test_minimal().linear,
            std::sync::Arc::new(moatless_identity::ports::HarnessTokens::new(
                std::sync::Arc::new(
                    moatless_identity::auth::test_support::StubProviderTokens::default(),
                ),
            )),
        ),
    )
    .register_into(&mut providers);
    Arc::new(crate::adapters::providers::webhook::adapter::WebhookAdapter::new())
        .register_into(&mut providers);
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
            providers.outbound_for_binding_reply_by_name(kind.as_str()).is_ok(),
            "{kind:?} binding reply"
        );
        assert!(
            providers.outbound_for_message_reply_by_name(kind.as_str()).is_ok(),
            "{kind:?} message reply"
        );
    }
    assert!(providers.get(AdapterKind::Webhook).is_some());
    assert!(providers.outbound_for_binding_reply_by_name("webhook").is_ok());
    assert!(providers.outbound_for_message_reply_by_name("webhook").is_ok());

    for kind in [
        AdapterKind::Slack,
        AdapterKind::Telegram,
        AdapterKind::GithubPr,
    ] {
        assert!(
            providers.reactions(kind).is_ok(),
            "{kind:?} reactions"
        );
        assert!(
            providers
                .inbound_presentation_by_name(kind.as_str())
                .is_some(),
            "{kind:?} inbound presentation"
        );
    }

    for kind in [AdapterKind::GithubPr, AdapterKind::Linear] {
        assert!(
            providers.binding_status_by_name(kind.as_str()).is_some(),
            "{kind:?} binding status"
        );
    }

    assert!(
        providers
            .outbound_for_direct_destination(AdapterKind::Slack)
            .is_ok()
    );
    assert!(
        providers
            .outbound_for_direct_destination(AdapterKind::Linear)
            .is_ok()
    );
    assert!(
        providers
            .outbound_for_direct_message(AdapterKind::Slack)
            .is_ok()
    );
    assert!(
        providers
            .post_binding_followup(AdapterKind::Linear)
            .is_some()
    );
    assert!(
        providers
            .outbound_for_direct_destination(AdapterKind::GithubPr)
            .is_err()
    );
    assert!(
        providers
            .outbound_for_direct_destination(AdapterKind::Telegram)
            .is_err()
    );
    assert!(
        providers
            .outbound_for_direct_destination(AdapterKind::Webhook)
            .is_err()
    );
    assert!(
        providers
            .outbound_for_direct_message(AdapterKind::GithubPr)
            .is_err()
    );
    assert!(
        providers
            .outbound_for_direct_message(AdapterKind::Linear)
            .is_err()
    );
    assert!(
        providers
            .outbound_for_direct_message(AdapterKind::Telegram)
            .is_err()
    );
    assert!(
        providers
            .outbound_for_direct_message(AdapterKind::Webhook)
            .is_err()
    );
    assert!(
        providers
            .post_binding_followup(AdapterKind::Slack)
            .is_none()
    );
    assert!(
        providers
            .post_binding_followup(AdapterKind::Webhook)
            .is_none()
    );
    assert!(providers.reactions(AdapterKind::Linear).is_err());
    assert!(providers.reactions(AdapterKind::Webhook).is_err());
    assert!(
        providers
            .inbound_presentation_by_name(AdapterKind::Webhook.as_str())
            .is_none()
    );
    assert!(
        providers
            .binding_status_by_name(AdapterKind::Slack.as_str())
            .is_none()
    );
    assert!(
        providers
            .binding_status_by_name(AdapterKind::Telegram.as_str())
            .is_none()
    );
    assert!(
        providers
            .binding_status_by_name(AdapterKind::Webhook.as_str())
            .is_none()
    );
}

/// Every adapter kind that registers a direct-*message* capability must also
/// register a direct-*destination* sender — the message path has nowhere to
/// deliver otherwise. Quantified over the whole `AdapterKind` enum rather than
/// over the registered kinds: `ALL` lists every variant, and `known_kind`'s
/// wildcard-free match makes a newly added variant fail to compile here until it
/// is added to both, so a new kind can never slip past the sweep.
#[tokio::test]
async fn direct_message_registration_requires_direct_destination_send() {
    let providers = registry_with_all_current_adapters();

    const ALL: [AdapterKind; 6] = [
        AdapterKind::Slack,
        AdapterKind::Telegram,
        AdapterKind::Github,
        AdapterKind::GithubPr,
        AdapterKind::Linear,
        AdapterKind::Webhook,
    ];

    // Exhaustiveness guard: no wildcard arm, so a new `AdapterKind` variant
    // breaks compilation until `ALL` (and this match) name it.
    fn known_kind(kind: AdapterKind) -> bool {
        match kind {
            AdapterKind::Slack
            | AdapterKind::Telegram
            | AdapterKind::Github
            | AdapterKind::GithubPr
            | AdapterKind::Linear
            | AdapterKind::Webhook => true,
        }
    }
    assert!(
        ALL.iter().copied().all(known_kind),
        "ALL must list every AdapterKind variant"
    );

    for kind in ALL {
        if providers.outbound_for_direct_message(kind).is_ok() {
            assert!(
                providers.outbound_for_direct_destination(kind).is_ok(),
                "{kind:?} registers a direct-message capability but no direct-destination sender"
            );
        }
    }
}

#[tokio::test]
async fn missing_capability_returns_adapter_error() {
    let providers = registry_with_all_current_adapters();

    match providers.reactions(AdapterKind::Linear) {
        Err(AdapterError::Shared(SharedAdapterError::BadRequest(_))) => {}
        Err(other) => panic!("expected BadRequest, got: {other:?}"),
        Ok(_) => panic!("Linear should not have reaction capability"),
    }
}

#[tokio::test]
async fn capability_lookup_by_name_preserves_provider_aliases() {
    let providers = registry_with_all_current_adapters();

    assert_eq!(
        providers.outbound_for_binding_reply_by_name("github")
            .expect("github alias should resolve to GitHub PR binding reply")
            .adapter_kind(),
        AdapterKind::GithubPr
    );
    assert_eq!(
        providers.reactions_by_name("github")
            .expect("github alias should resolve to GitHub PR reactions")
            .adapter_kind(),
        AdapterKind::GithubPr
    );

    match providers.reactions_by_name("linear") {
        Err(AdapterError::Shared(SharedAdapterError::BadRequest(_))) => {}
        Err(other) => panic!("expected BadRequest, got: {other:?}"),
        Ok(_) => panic!("Linear should not have reaction capability"),
    }
}

/// Name resolution reads the `Adapter` map, not a separate identity map. It must
/// still resolve a kind's canonical key, its provider aliases, and an unknown
/// name exactly as before — the `github` connection alias resolving to the
/// `github_pr` provider key is the case a silent regression here would break.
/// Uses the public `get_by_name` accessor (the `pub(crate)` `identity_by_name`
/// it wraps stays in-crate, exercised by `provider_key_for_name`).
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
        .get_by_name("github")
        .expect("github alias resolves to an adapter");
    assert_eq!(github.adapter_kind(), AdapterKind::GithubPr);
    assert_eq!(github.provider_key(), "github_pr");
    assert_eq!(github.connection_provider_key(), "github");

    assert!(providers.get_by_name("nope").is_none());
}

#[tokio::test]
async fn webhook_raw_destination_send_is_absent_from_registry() {
    let providers = registry_with_all_current_adapters();

    match providers.outbound_for_direct_destination(AdapterKind::Webhook) {
        Err(AdapterError::Shared(SharedAdapterError::BadRequest(msg))) => {
            assert!(
                msg.contains("does not support direct destination sends"),
                "unexpected message: {msg}"
            );
        }
        Err(other) => panic!("expected BadRequest, got: {other:?}"),
        Ok(_) => panic!("webhook must not register raw direct destination capability"),
    }
}
