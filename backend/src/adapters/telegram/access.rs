//! Telegram access control — DM and group policies.

use tracing::{debug, info};

use crate::adapters::error::AdapterError;
use crate::adapters::models::{AccessDecision, LoopContext, SourceAccessPolicy};
use crate::db::DbPool;

use super::api::TelegramApi;
use super::config::TelegramAdapterConfig;
use super::pairing;

/// Context about the Telegram sender for DM access enforcement.
pub struct DmAccessContext<'a> {
    pub tg_config: &'a TelegramAdapterConfig,
    pub from_id: i64,
    pub from_username: Option<&'a str>,
    pub from_first_name: Option<&'a str>,
    pub from_last_name: Option<&'a str>,
    pub config: &'a LoopContext,
    pub api: &'a TelegramApi,
    pub chat_id: i64,
    pub db: &'a DbPool,
}

/// Enforce DM access policy. May send a pairing message to the user.
pub async fn enforce_dm_access(ctx: &DmAccessContext<'_>) -> Result<AccessDecision, AdapterError> {
    let from_id_str = ctx.from_id.to_string();

    let from_id = ctx.from_id;

    match ctx.tg_config.dm_access_policy {
        SourceAccessPolicy::Disabled => {
            debug!(from_id, "DM blocked: dm_access_policy=disabled");
            Ok(AccessDecision::Blocked)
        }
        SourceAccessPolicy::Open => Ok(AccessDecision::Allowed),
        SourceAccessPolicy::Allowlist => {
            if ctx.tg_config.dm_allowlist.contains(&from_id_str) {
                Ok(AccessDecision::Allowed)
            } else {
                debug!(from_id, "DM blocked: user not in dm_allowlist");
                Ok(AccessDecision::Blocked)
            }
        }
        SourceAccessPolicy::Pairing => {
            // Check allowlist first — already-approved users pass through
            if ctx.tg_config.dm_allowlist.contains(&from_id_str) {
                return Ok(AccessDecision::Allowed);
            }

            // Build meta for the pairing request
            let meta = serde_json::json!({
                "username": ctx.from_username,
                "first_name": ctx.from_first_name,
                "last_name": ctx.from_last_name,
            });

            let req =
                pairing::upsert_pairing_request(ctx.db, &ctx.config.id, &from_id_str, Some(meta))
                    .await?;

            // Send pairing message
            let message = format!(
                "Your Telegram user ID: `{from_id}`\n\n\
                 Pairing code: `{code}`\n\n\
                 Ask your admin to approve this code.",
                code = req.code,
            );

            let chat_id_str = ctx.chat_id.to_string();
            if let Err(e) = ctx
                .api
                .send_message(&chat_id_str, &message, None, None)
                .await
            {
                // Best-effort — log but don't fail the whole flow
                info!(from_id, error = %e, "Failed to send pairing message");
            }

            info!(
                from_id,
                code = %req.code,
                trigger_id = %ctx.config.id,
                "Pairing code sent to Telegram user"
            );

            Ok(AccessDecision::PairingSent)
        }
    }
}

/// Enforce group access policy (synchronous, no DB/API needed).
pub fn enforce_group_access(
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
            enforce_group_access(&cfg, -100123, "hello"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn group_disabled_blocks() {
        let cfg = make_group_config(SourceAccessPolicy::Disabled, vec![], false);
        assert!(matches!(
            enforce_group_access(&cfg, -100123, "hello"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn group_allowlist_allows_listed() {
        let cfg = make_group_config(SourceAccessPolicy::Allowlist, vec!["-100123"], false);
        assert!(matches!(
            enforce_group_access(&cfg, -100123, "hello"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn group_allowlist_blocks_unlisted() {
        let cfg = make_group_config(SourceAccessPolicy::Allowlist, vec!["-100999"], false);
        assert!(matches!(
            enforce_group_access(&cfg, -100123, "hello"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn group_require_mention_blocks_without_mention() {
        let cfg = make_group_config(SourceAccessPolicy::Open, vec![], true);
        assert!(matches!(
            enforce_group_access(&cfg, -100123, "hello world"),
            AccessDecision::Blocked
        ));
    }

    #[test]
    fn group_require_mention_allows_with_mention() {
        let cfg = make_group_config(SourceAccessPolicy::Open, vec![], true);
        assert!(matches!(
            enforce_group_access(&cfg, -100123, "hello @testbot fix this"),
            AccessDecision::Allowed
        ));
    }

    #[test]
    fn group_require_mention_allowlist_both_required() {
        let cfg = make_group_config(SourceAccessPolicy::Allowlist, vec!["-100123"], true);
        // In allowlist but no mention → blocked
        assert!(matches!(
            enforce_group_access(&cfg, -100123, "hello"),
            AccessDecision::Blocked
        ));
        // In allowlist and has mention → allowed
        assert!(matches!(
            enforce_group_access(&cfg, -100123, "@testbot hello"),
            AccessDecision::Allowed
        ));
    }

    // -- DM policy tests (synchronous checks only) --

    #[test]
    fn dm_open_allows() {
        let cfg = make_config(SourceAccessPolicy::Open, vec![]);
        assert_eq!(cfg.dm_access_policy, SourceAccessPolicy::Open);
    }

    #[test]
    fn dm_disabled_policy() {
        let cfg = make_config(SourceAccessPolicy::Disabled, vec![]);
        assert_eq!(cfg.dm_access_policy, SourceAccessPolicy::Disabled);
    }

    #[test]
    fn dm_allowlist_contains_check() {
        let cfg = make_config(SourceAccessPolicy::Allowlist, vec!["123", "456"]);
        assert!(cfg.dm_allowlist.contains(&"123".to_string()));
        assert!(!cfg.dm_allowlist.contains(&"789".to_string()));
    }

    #[test]
    fn dm_pairing_checks_allowlist_first() {
        let cfg = make_config(SourceAccessPolicy::Pairing, vec!["123"]);
        // User 123 is in allowlist → should be allowed even with pairing policy
        assert!(cfg.dm_allowlist.contains(&"123".to_string()));
    }
}
