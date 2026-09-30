//! How long an Adapter's inbound events are held before they are combined.
//!
//! Three authorities, narrowest first: the Loop's own
//! `loop_configs.message_buffer_seconds`, the deployment's
//! [`MessageBufferConfig`](moatless_types::adapter::MessageBufferConfig), then the built-in default per [`AdapterKind`]. All
//! three belong to a Loop, so a subject routed without one is not buffered at
//! all. [`buffer_window`] owns which Adapter the latter two are keyed on.

use std::time::Duration;

use moatless_types::adapter::AdapterKind;
use moatless_types::adapter::MessageBufferConfig;
use moatless_types::loops::SubscriptionMatch;

/// Upper bound on any configured window. A Loop that asks for more is clamped
/// rather than refused, so a typo cannot park a review out of reach for a day.
const MAX_WINDOW: Duration = Duration::from_hours(1);

/// Built-in window for `adapter`, before the deployment and the Loop.
///
/// Only GitHub PR buffers by default: a single review posts one webhook per
/// comment within a second or two, and each one used to become its own agent
/// turn on the same review. Chat adapters already carry one event per human
/// keystroke-batch, so holding them only adds latency.
fn builtin_window(adapter: AdapterKind) -> Duration {
    match adapter {
        AdapterKind::GithubPr => Duration::from_mins(1),
        _ => Duration::ZERO,
    }
}

/// The window an event on `event_kind` waits out under `subscription`.
///
/// `event_kind` is the event's own Adapter, deliberately not
/// `subscription.adapter_kind`, which holds the connection's kind: a GitHub PR
/// Loop reads `github` there and never reaches the `github_pr` default. A
/// delivery-only Subscription has no Loop and so no window: never buffered.
pub(crate) fn buffer_window(
    subscription: &SubscriptionMatch,
    event_kind: AdapterKind,
    deployment: &MessageBufferConfig,
) -> Duration {
    if subscription.is_delivery_only() {
        return Duration::ZERO;
    }
    configured_window(subscription.message_buffer_seconds, event_kind, deployment)
}

/// The window for an event on `adapter`, under `loop_override`.
///
/// `Duration::ZERO` means "deliver this event on its own", which is what every
/// caller treats as buffering being off.
fn configured_window(
    loop_override: Option<i32>,
    adapter: AdapterKind,
    deployment: &MessageBufferConfig,
) -> Duration {
    let seconds = loop_override
        .or_else(|| deployment_default(adapter, deployment))
        .map_or_else(
            || builtin_window(adapter),
            |seconds| Duration::from_secs(u64::from(seconds.max(0).unsigned_abs())),
        );
    seconds.min(MAX_WINDOW)
}

/// The deployment's default for `adapter`.
///
/// `None` is "the deployment says nothing", so the built-in applies; `Some(0)`
/// is an operator explicitly turning buffering off for that kind.
fn deployment_default(adapter: AdapterKind, deployment: &MessageBufferConfig) -> Option<i32> {
    deployment
        .per_kind_seconds
        .get(&adapter.as_str().to_uppercase())
        .copied()
        .or(deployment.global_seconds)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_adapter_model::loops::delivery_only_subscription;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterConnection;

    use super::*;

    fn none() -> MessageBufferConfig {
        MessageBufferConfig::default()
    }

    fn connection() -> AdapterConnection {
        let now = Utc::now();
        AdapterConnection {
            id: "pc_org".to_string(),
            adapter_kind: AdapterKind::Github,
            connection_kind: "org_webhook".to_string(),
            scope_value: "acme".to_string(),
            credential_ref: None,
            webhook_secret: None,
            transport_config: serde_json::json!({}),
            created_by: "u_installer".to_string(),
            scope: Scope::User,
            owner_user_id: Some("u_owner".to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    /// A Loop's Subscription as `subscriptions` stores one: `adapter_kind` is
    /// the connection's, `github`, even where every event it routes is a
    /// `github_pr` one.
    fn loop_subscription(window: Option<i32>) -> SubscriptionMatch {
        let mut subscription =
            delivery_only_subscription(&connection(), AdapterKind::Github, "acme/widget");
        subscription.id = "lc_review".to_string();
        subscription.message_buffer_seconds = window;
        subscription
    }

    /// The repo has no Loop, so nothing holds its events and nothing can
    /// configure a hold: the built-in minute for `github_pr` does not apply.
    #[test]
    fn a_subject_with_no_loop_is_not_buffered() {
        let delivery_only =
            delivery_only_subscription(&connection(), AdapterKind::GithubPr, "acme/widget");

        assert_eq!(
            buffer_window(&delivery_only, AdapterKind::GithubPr, &none()),
            Duration::ZERO
        );
    }

    /// The window follows the event's Adapter, so a `github_pr` event buffers
    /// under a Loop whose Subscription says `github`.
    #[test]
    fn a_pr_event_takes_the_pr_window_under_a_github_subscription() {
        assert_eq!(
            buffer_window(&loop_subscription(None), AdapterKind::GithubPr, &none()),
            Duration::from_mins(1)
        );
        assert_eq!(
            buffer_window(&loop_subscription(Some(5)), AdapterKind::GithubPr, &none()),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn github_pr_buffers_for_a_minute_and_other_adapters_do_not() {
        assert_eq!(
            builtin_window(AdapterKind::GithubPr),
            Duration::from_mins(1)
        );
        assert_eq!(builtin_window(AdapterKind::Slack), Duration::ZERO);
        assert_eq!(builtin_window(AdapterKind::Linear), Duration::ZERO);
    }

    #[test]
    fn a_loop_override_outranks_the_builtin_in_both_directions() {
        assert_eq!(
            configured_window(Some(5), AdapterKind::GithubPr, &none()),
            Duration::from_secs(5)
        );
        assert_eq!(
            configured_window(Some(0), AdapterKind::GithubPr, &none()),
            Duration::ZERO
        );
        assert_eq!(
            configured_window(Some(30), AdapterKind::Slack, &none()),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn a_window_is_clamped_rather_than_refused() {
        assert_eq!(
            configured_window(Some(i32::MAX), AdapterKind::Slack, &none()),
            MAX_WINDOW
        );
        assert_eq!(
            configured_window(Some(-7), AdapterKind::Slack, &none()),
            Duration::ZERO
        );
    }

    /// The per-kind default is keyed by the kind string upcased, so `github_pr`
    /// reads `ADAPTER_MESSAGE_BUFFER_SECONDS_GITHUB_PR`, and it outranks the
    /// global default.
    #[test]
    fn the_per_kind_default_outranks_the_global_one() {
        let deployment = MessageBufferConfig {
            global_seconds: Some(20),
            per_kind_seconds: [("GITHUB_PR".to_string(), 7)].into(),
        };
        assert_eq!(
            configured_window(None, AdapterKind::GithubPr, &deployment),
            Duration::from_secs(7)
        );
        assert_eq!(
            configured_window(None, AdapterKind::Slack, &deployment),
            Duration::from_secs(20)
        );
    }
}
