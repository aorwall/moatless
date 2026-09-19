//! How long an Adapter's inbound events are held before they are combined.
//!
//! Three authorities, narrowest first: the Loop's own
//! `loop_configs.message_buffer_seconds`, the deployment's environment, then the
//! built-in default per [`AdapterKind`]. All three belong to a Loop, so a
//! subject routed without one is not buffered at all.

use std::time::Duration;

use moatless_types::adapter::AdapterKind;
use moatless_types::loops::SubscriptionMatch;

/// Global default, overriding [`builtin_window`] for every Adapter kind.
const GLOBAL_ENV: &str = "ADAPTER_MESSAGE_BUFFER_SECONDS";

/// Per-kind default: `ADAPTER_MESSAGE_BUFFER_SECONDS_GITHUB_PR`,
/// `..._SLACK`, and so on. Outranks [`GLOBAL_ENV`].
const KIND_ENV_PREFIX: &str = "ADAPTER_MESSAGE_BUFFER_SECONDS_";

/// Upper bound on any configured window. A Loop that asks for more is clamped
/// rather than refused, so a typo cannot park a review out of reach for a day.
const MAX_WINDOW: Duration = Duration::from_hours(1);

/// Built-in window for `adapter`, before the environment and the Loop.
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

/// The window an event routed under `subscription` waits out.
///
/// A delivery-only Subscription is a connection with no Loop, so it carries no
/// window and no place to configure one: those events are never buffered,
/// whatever the environment and the built-in say for their Adapter.
pub(crate) fn buffer_window(subscription: &SubscriptionMatch) -> Duration {
    if subscription.is_delivery_only() {
        return Duration::ZERO;
    }
    configured_window(
        subscription.message_buffer_seconds,
        subscription.adapter_kind,
    )
}

/// The window for an event on `adapter`, under `loop_override`.
///
/// `Duration::ZERO` means "deliver this event on its own", which is what every
/// caller treats as buffering being off.
fn configured_window(loop_override: Option<i32>, adapter: AdapterKind) -> Duration {
    let seconds = loop_override.or_else(|| env_default(adapter)).map_or_else(
        || builtin_window(adapter),
        |seconds| Duration::from_secs(u64::from(seconds.max(0).unsigned_abs())),
    );
    seconds.min(MAX_WINDOW)
}

/// The environment's default for `adapter`.
///
/// `None` is "the environment says nothing", so the built-in applies; `Some(0)`
/// is an operator explicitly turning buffering off for that kind.
fn env_default(adapter: AdapterKind) -> Option<i32> {
    let key = format!("{KIND_ENV_PREFIX}{}", adapter.as_str().to_uppercase());
    read_seconds(&key).or_else(|| read_seconds(GLOBAL_ENV))
}

fn read_seconds(key: &str) -> Option<i32> {
    let raw = std::env::var(key).ok()?;
    match raw.trim().parse::<i32>() {
        Ok(seconds) if seconds >= 0 => Some(seconds),
        _ => {
            tracing::warn!(key, value = %raw, "Ignoring unparseable inbound buffer window");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use moatless_adapter_model::loops::delivery_only_subscription;
    use moatless_types::access_control::Scope;
    use moatless_types::adapter::AdapterConnection;

    use super::*;

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

    /// A Loop's Subscription on the same connection: the id is the only part
    /// [`buffer_window`] reads apart from the window itself.
    fn loop_subscription(window: Option<i32>) -> SubscriptionMatch {
        let mut subscription =
            delivery_only_subscription(&connection(), AdapterKind::GithubPr, "acme/widget");
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

        assert_eq!(buffer_window(&delivery_only), Duration::ZERO);
    }

    /// A Loop takes the configured window, so `github_pr` keeps buffering
    /// without anyone opting in. Compared against `configured_window` rather
    /// than a minute, so an environment that sets a default does not fail it.
    #[test]
    fn a_loop_takes_the_window_its_configuration_gives_it() {
        assert_eq!(
            buffer_window(&loop_subscription(None)),
            configured_window(None, AdapterKind::GithubPr)
        );
        assert_eq!(
            buffer_window(&loop_subscription(Some(5))),
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
            configured_window(Some(5), AdapterKind::GithubPr),
            Duration::from_secs(5)
        );
        assert_eq!(
            configured_window(Some(0), AdapterKind::GithubPr),
            Duration::ZERO
        );
        assert_eq!(
            configured_window(Some(30), AdapterKind::Slack),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn a_window_is_clamped_rather_than_refused() {
        assert_eq!(
            configured_window(Some(i32::MAX), AdapterKind::Slack),
            MAX_WINDOW
        );
        assert_eq!(
            configured_window(Some(-7), AdapterKind::Slack),
            Duration::ZERO
        );
    }

    /// The per-kind key is derived from the kind string, so `github_pr` must
    /// reach `ADAPTER_MESSAGE_BUFFER_SECONDS_GITHUB_PR` and not some hand-kept
    /// second spelling of the same name.
    #[test]
    fn the_per_kind_env_key_is_the_kind_string_upcased() {
        assert_eq!(
            format!(
                "{KIND_ENV_PREFIX}{}",
                AdapterKind::GithubPr.as_str().to_uppercase()
            ),
            "ADAPTER_MESSAGE_BUFFER_SECONDS_GITHUB_PR"
        );
    }
}
