//! Slack access control — channel and user policy enforcement.

use crate::channel::models::{AccessDecision, ChannelAccessPolicy};

use super::config::SlackProviderConfig;

/// Enforce Slack access policy (synchronous, no DB/API needed).
///
/// Unlike Telegram, Slack does not support `Pairing` — it is treated as `Allowlist`.
pub fn enforce_slack_access(
    config: &SlackProviderConfig,
    channel_id: &str,
    user_id: &str,
) -> AccessDecision {
    match config.access_policy {
        ChannelAccessPolicy::Disabled => AccessDecision::Blocked,
        ChannelAccessPolicy::Open => AccessDecision::Allowed,
        ChannelAccessPolicy::Allowlist | ChannelAccessPolicy::Pairing => {
            // Check channel allowlist (if non-empty)
            if !config.channel_allowlist.is_empty()
                && !config.channel_allowlist.iter().any(|c| c == channel_id)
            {
                return AccessDecision::Blocked;
            }
            // Check user allowlist (if non-empty)
            if !config.user_allowlist.is_empty()
                && !config.user_allowlist.iter().any(|u| u == user_id)
            {
                return AccessDecision::Blocked;
            }
            AccessDecision::Allowed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(
        policy: ChannelAccessPolicy,
        channels: Vec<&str>,
        users: Vec<&str>,
    ) -> SlackProviderConfig {
        SlackProviderConfig {
            access_policy: policy,
            channel_allowlist: channels.into_iter().map(String::from).collect(),
            user_allowlist: users.into_iter().map(String::from).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn open_allows_any() {
        let cfg = make_config(ChannelAccessPolicy::Open, vec![], vec![]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn disabled_blocks_all() {
        let cfg = make_config(ChannelAccessPolicy::Disabled, vec![], vec![]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn allowlist_channel_match() {
        let cfg = make_config(ChannelAccessPolicy::Allowlist, vec!["C123"], vec![]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn allowlist_channel_mismatch() {
        let cfg = make_config(ChannelAccessPolicy::Allowlist, vec!["C999"], vec![]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn allowlist_user_match() {
        let cfg = make_config(ChannelAccessPolicy::Allowlist, vec![], vec!["U456"]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn allowlist_user_mismatch() {
        let cfg = make_config(ChannelAccessPolicy::Allowlist, vec![], vec!["U999"]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn allowlist_both_must_match() {
        let cfg = make_config(ChannelAccessPolicy::Allowlist, vec!["C123"], vec!["U456"]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Allowed
        ));
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U999"),
            AccessDecision::Blocked
        ));
        assert!(matches!(
            enforce_slack_access(&cfg, "C999", "U456"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn allowlist_empty_lists_allows_all() {
        // With allowlist policy but no lists specified, all pass
        let cfg = make_config(ChannelAccessPolicy::Allowlist, vec![], vec![]);
        assert!(matches!(
            enforce_slack_access(&cfg, "C123", "U456"),
            AccessDecision::Allowed
        ));
    }
}
