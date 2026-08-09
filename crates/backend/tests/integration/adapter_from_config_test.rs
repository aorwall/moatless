#![cfg(feature = "db-tests")]
//! `from_config` must place every adapter it builds in both the trait-object
//! registry and the concrete-handle set — and the two must not disagree.
//!
//! Before Step 7 a single typed registrar did both writes in one call, and a
//! unit test guarded the typed handles the registry then held. The handles now
//! live on the composition root, filled by a second write in `from_config` that
//! is separate from the trait-object registration. This pins that a future edit
//! cannot register one without the other — register the trait object and forget
//! the handle, or the reverse.

use crate::common;

use moatless_backend::adapters::AdapterKind;
use moatless_backend::compose::compose;
use sqlx::PgPool;

use common::test_config;

#[sqlx::test]
async fn from_config_populates_both_the_registry_and_the_handles(pool: PgPool) {
    let config = test_config();
    let ports = compose(pool.clone(), &config);
    let (adapters, handles) = moatless_backend::compose::from_config(
        &config,
        ports.pool.clone(),
        ports.provider_tokens.clone(),
        ports.provider_token_auth.clone(),
        ports.github_apps.clone(),
        ports.github_app_service.clone(),
        ports.secrets.clone(),
    )
    .await;

    // Every concrete handle the root hands to a provider's own handlers.
    assert!(handles.github_pr.is_some(), "github_pr handle missing");
    assert!(handles.linear.is_some(), "linear handle missing");
    assert!(handles.slack.is_some(), "slack handle missing");
    assert!(handles.telegram.is_some(), "telegram handle missing");

    // The same providers as trait objects in the registry the delivery
    // machinery dispatches through — plus Webhook, which has no concrete handle
    // and so appears here alone.
    assert!(
        adapters.get(AdapterKind::GithubPr).is_some(),
        "registry missing github_pr"
    );
    assert!(
        adapters.get(AdapterKind::Linear).is_some(),
        "registry missing linear"
    );
    assert!(
        adapters.get(AdapterKind::Slack).is_some(),
        "registry missing slack"
    );
    assert!(
        adapters.get(AdapterKind::Telegram).is_some(),
        "registry missing telegram"
    );
    assert!(
        adapters.get(AdapterKind::Webhook).is_some(),
        "registry missing webhook"
    );
}
