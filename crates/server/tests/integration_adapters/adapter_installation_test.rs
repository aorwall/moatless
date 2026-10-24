#![cfg(feature = "db-tests")]
//! Contract for the consolidated external Adapter installation seam.
//!
//! One production installation must register every Adapter's capabilities,
//! every durable-delivery kind, and every concrete handle required by
//! Adapter-specific handlers. These assertions cross the same public seam
//! startup uses rather than reconstructing its writes independently.

use std::sync::Arc;

use crate::common;
use axum::extract::FromRef;

use moatless_backend::adapters::providers::github_pr::adapter::GithubPrAdapter;
use moatless_backend::adapters::providers::linear::adapter::LinearAdapter;
use moatless_backend::adapters::providers::slack::adapter::SlackAdapter;
use moatless_backend::adapters::providers::telegram::adapter::TelegramAdapter;
use moatless_types::adapter::AdapterKind;
use sqlx::PgPool;

use common::{test_app_state_with_adapters, test_config};

#[sqlx::test]
async fn installation_registers_capabilities_deliveries_and_handler_access(pool: PgPool) {
    let config = test_config();
    let state = test_app_state_with_adapters(pool, config);
    let installations = match state.adapter_installations.as_ref() {
        Some(installations) => installations,
        None => panic!("database-backed state must install external Adapters"),
    };
    let adapters = installations.adapters();

    for kind in [
        AdapterKind::Slack,
        AdapterKind::Webhook,
        AdapterKind::Telegram,
        AdapterKind::GithubPr,
        AdapterKind::Linear,
    ] {
        assert!(adapters.get(kind).is_some(), "registry missing {kind:?}");
    }

    assert!(Option::<Arc<SlackAdapter>>::from_ref(&state).is_some());
    assert!(Option::<Arc<TelegramAdapter>>::from_ref(&state).is_some());
    assert!(Option::<Arc<GithubPrAdapter>>::from_ref(&state).is_some());
    assert!(Option::<Arc<LinearAdapter>>::from_ref(&state).is_some());

    let mut kinds = installations.delivery_handlers(&state).kinds();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        [
            "github.check_run",
            "github.check_run_trigger",
            "github.close",
            "github.merge_conflict",
            "github.post_merge",
            "github.pr_merged",
            "github.pr_message",
            "linear.comment",
            "linear.issue_labeled",
            "linear.issue_update",
            "linear.notification",
            "linear.session_prompt",
            "linear.session_provision",
            "slack.inbound_message",
            "telegram.inbound_message",
            "webhook.inbound_message",
        ]
    );
}
