//! Telegram access control — DM and group policies.

use tracing::debug;

use moatless_loops::adapters::models::{AccessDecision, SourceAccessPolicy};

use super::config::TelegramAdapterConfig;

/// Who sent the private message, as the DM policy needs to see them.
pub(super) struct DmSender<'a> {
    pub(super) tg_config: &'a TelegramAdapterConfig,
    pub(super) from_id: i64,
    pub(super) from_username: Option<&'a str>,
    pub(super) from_first_name: Option<&'a str>,
    pub(super) from_last_name: Option<&'a str>,
}

/// What the DM policy decided.
///
/// [`AccessDecision`] cannot express the third case, because raising a pairing
/// request is work rather than a verdict — and this function no longer does
/// work. The caller turns it into a
/// [`super::webhook::Effect::RequestPairing`].
pub(super) enum DmDecision {
    Allowed,
    Blocked,
    /// An unknown sender under a `Pairing` policy. `meta` is what the pairing
    /// request records about them.
    RequestPairing {
        meta: serde_json::Value,
    },
}

/// Enforce DM access policy.
pub(super) fn enforce_dm_access(sender: &DmSender<'_>) -> DmDecision {
    let from_id = sender.from_id;
    let from_id_str = from_id.to_string();

    match sender.tg_config.dm_access_policy {
        SourceAccessPolicy::Disabled => {
            debug!(from_id, "DM blocked: dm_access_policy=disabled");
            DmDecision::Blocked
        }
        SourceAccessPolicy::Open => DmDecision::Allowed,
        SourceAccessPolicy::Allowlist => {
            if sender.tg_config.dm_allowlist.contains(&from_id_str) {
                DmDecision::Allowed
            } else {
                debug!(from_id, "DM blocked: user not in dm_allowlist");
                DmDecision::Blocked
            }
        }
        SourceAccessPolicy::Pairing => {
            // Check allowlist first — already-approved users pass through
            if sender.tg_config.dm_allowlist.contains(&from_id_str) {
                return DmDecision::Allowed;
            }

            DmDecision::RequestPairing {
                meta: serde_json::json!({
                    "username": sender.from_username,
                    "first_name": sender.from_first_name,
                    "last_name": sender.from_last_name,
                }),
            }
        }
    }
}

/// Enforce group access policy (synchronous, no DB/API needed).
pub(super) fn enforce_group_access(
    tg_config: &TelegramAdapterConfig,
    chat_id: i64,
    text: &str,
) -> AccessDecision {
    match tg_config.group_access_policy {
        SourceAccessPolicy::Disabled => {
            debug!(
                chat_id,
                "Group message blocked: group_access_policy=disabled"
            );
            return AccessDecision::Blocked;
        }
        SourceAccessPolicy::Allowlist | SourceAccessPolicy::Pairing => {
            // Pairing is not meaningful for groups; treat as Allowlist
            let chat_id_str = chat_id.to_string();
            if !tg_config.group_allowlist.contains(&chat_id_str) {
                debug!(
                    chat_id,
                    "Group message blocked: chat not in group_allowlist"
                );
                return AccessDecision::Blocked;
            }
        }
        SourceAccessPolicy::Open => {}
    }

    // Check mention requirement
    if tg_config.require_mention {
        if let Some(ref bot_username) = tg_config.bot_username {
            let mention = if bot_username.starts_with('@') {
                bot_username.clone()
            } else {
                format!("@{bot_username}")
            };
            if !text.contains(&mention) {
                debug!(
                    chat_id,
                    "Group message blocked: require_mention and no bot mention"
                );
                return AccessDecision::Blocked;
            }
        }
    }

    AccessDecision::Allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(
        dm_policy: SourceAccessPolicy,
        dm_allowlist: Vec<&str>,
    ) -> TelegramAdapterConfig {
        TelegramAdapterConfig {
            bot_username: Some("testbot".into()),
            dm_access_policy: dm_policy,
            dm_allowlist: dm_allowlist.into_iter().map(String::from).collect(),
            ..Default::default()
        }
    }

    fn make_group_config(
        group_policy: SourceAccessPolicy,
        group_allowlist: Vec<&str>,
        require_mention: bool,
    ) -> TelegramAdapterConfig {
        TelegramAdapterConfig {
            bot_username: Some("testbot".into()),
            group_access_policy: group_policy,
            group_allowlist: group_allowlist.into_iter().map(String::from).collect(),
            require_mention,
            ..Default::default()
        }
    }

    // -- Group policy tests (synchronous, no DB) --

    #[test]
    fn group_open_allows() {
        let cfg = make_group_config(SourceAccessPolicy::Open, vec![], false);
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "hello"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn group_disabled_blocks() {
        let cfg = make_group_config(SourceAccessPolicy::Disabled, vec![], false);
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "hello"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn group_allowlist_allows_listed() {
        let cfg = make_group_config(SourceAccessPolicy::Allowlist, vec!["-100123"], false);
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "hello"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn group_allowlist_blocks_unlisted() {
        let cfg = make_group_config(SourceAccessPolicy::Allowlist, vec!["-100999"], false);
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "hello"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn group_require_mention_blocks_without_mention() {
        let cfg = make_group_config(SourceAccessPolicy::Open, vec![], true);
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "hello world"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn group_require_mention_allows_with_mention() {
        let cfg = make_group_config(SourceAccessPolicy::Open, vec![], true);
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "hello @testbot fix this"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn group_require_mention_allowlist_both_required() {
        let cfg = make_group_config(SourceAccessPolicy::Allowlist, vec!["-100123"], true);
        // In allowlist but no mention → blocked
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "hello"),
            AccessDecision::Blocked
        ));
        // In allowlist and has mention → allowed
        assert!(matches!(
            enforce_group_access(&cfg, -100_123, "@testbot hello"),
            AccessDecision::Allowed
        ));
    }

    // -- DM policy tests --
    //
    // These call the policy now. Before the decision was split from the pairing
    // write they could only assert the config they were given back, because
    // reaching the function meant a database and a Bot API client.

    fn decide(cfg: &TelegramAdapterConfig, from_id: i64) -> DmDecision {
        enforce_dm_access(&DmSender {
            tg_config: cfg,
            from_id,
            from_username: Some("ada"),
            from_first_name: Some("Ada"),
            from_last_name: None,
        })
    }

    #[test]
    fn dm_open_allows() {
        let cfg = make_config(SourceAccessPolicy::Open, vec![]);
        assert!(matches!(decide(&cfg, 123), DmDecision::Allowed));
    }

    #[test]
    fn dm_disabled_policy() {
        let cfg = make_config(SourceAccessPolicy::Disabled, vec![]);
        assert!(matches!(decide(&cfg, 123), DmDecision::Blocked));
    }

    #[test]
    fn dm_allowlist_contains_check() {
        let cfg = make_config(SourceAccessPolicy::Allowlist, vec!["123", "456"]);
        assert!(matches!(decide(&cfg, 123), DmDecision::Allowed));
        assert!(matches!(decide(&cfg, 789), DmDecision::Blocked));
    }

    #[test]
    fn dm_pairing_checks_allowlist_first() {
        let cfg = make_config(SourceAccessPolicy::Pairing, vec!["123"]);
        // User 123 is in allowlist → should be allowed even with pairing policy
        assert!(matches!(decide(&cfg, 123), DmDecision::Allowed));

        // Anyone else gets a pairing request carrying what is known about them.
        let DmDecision::RequestPairing { meta } = decide(&cfg, 789) else {
            panic!("an unlisted sender under a pairing policy is asked to pair");
        };
        assert_eq!(meta["username"], "ada");
        assert_eq!(meta["first_name"], "Ada");
        assert_eq!(meta["last_name"], serde_json::Value::Null);
    }
}
