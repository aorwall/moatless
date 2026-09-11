//! Slack identity resolution.
//!
//! Resolving the inbound sender during normalization, and naming a Slack user
//! for a reader. `users` and `user_provider_ids` belong to `identity`;
//! everything here goes through `ProviderIdentityLink`.

use tracing::warn;
use uuid::Uuid;

use moatless_adapter_model::error::AdapterError;
use moatless_ports::identity::ProviderIdentityLink;

use super::api::{SlackApi, SlackUserInfo};

/// `user_provider_ids.provider` for a Slack account.
const PROVIDER: &str = super::delivery::ADAPTER;

/// Whether a token is a Slack user id rather than a Moatless login.
///
/// What lets an outbound `<@…>` carry either: an id the Adapter can already
/// address is passed through, so a Slack user Moatless has never seen stays
/// reachable. Slack ids are uppercase and start `U` or `W`; a Moatless login is
/// lowercase, so the two do not overlap in practice.
pub(super) fn is_slack_user_id(token: &str) -> bool {
    token.len() >= 8
        && token.starts_with(['U', 'W'])
        && token[1..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Name one Slack user for a reader: the Moatless login they are linked to,
/// else the handle Slack knows them by, else nothing.
pub(super) async fn name_of(
    api: &SlackApi,
    users: &dyn ProviderIdentityLink,
    slack_user_id: &str,
) -> Option<String> {
    match users.user_at(PROVIDER, slack_user_id).await {
        Ok(Some(user)) => return Some(user.login),
        Ok(None) => {}
        Err(e) => {
            warn!(slack_user = %slack_user_id, error = %e, "Slack user lookup failed");
            return None;
        }
    }
    api.get_user_info(slack_user_id)
        .await
        .ok()
        .flatten()
        .and_then(|info| info.handle)
}

/// What one inbound Slack sender resolved to.
pub(super) enum SenderIdentity {
    /// A bot or app account with `allow_bot_messages` off. Their messages do
    /// not drive task flow, and they share emails across deployments, so
    /// resolving them by email is unsafe.
    Rejected { handle: Option<String> },
    /// Mapped to a system user, by provider id or by email. Carried as the
    /// `uuid` the column is; the string form is built once, where the shared
    /// `InboundMessage` demands one.
    ///
    /// `handle` rides along on both resolved variants because it says who wrote
    /// the message in Slack's own terms, which the Moatless user it resolved to
    /// does not: an unmapped thread reply is attributed to the trigger owner.
    Mapped {
        user_id: Uuid,
        handle: Option<String>,
    },
    /// No mapping. `is_bot` is the sender's final bot verdict — a foreign bot
    /// allowed through is attributed to the trigger owner rather than notified
    /// — and `email` is the address the unmapped-sender notice quotes.
    Unmapped {
        is_bot: bool,
        email: Option<String>,
        handle: Option<String>,
    },
}

/// Resolve the sender of one inbound Slack event.
///
/// Spends exactly one `users.info`, then tries the provider-id mapping before
/// the email one. `already_bot` carries the verdict the `bot_id` field on the
/// event already produced: a sender known to be a bot is never resolved by
/// email, whichever of the two checks found it.
pub(super) async fn resolve_inbound_sender(
    api: &SlackApi,
    users: &dyn ProviderIdentityLink,
    slack_user_id: &str,
    allow_bot_messages: bool,
    already_bot: bool,
) -> Result<SenderIdentity, AdapterError> {
    // Fetch user info once (bot flags + email). A failed or empty lookup is
    // not fatal: the sender simply stays unresolved.
    let info: Option<SlackUserInfo> = api.get_user_info(slack_user_id).await.ok().flatten();

    // Drop bot/app-account senders: this catches external deployment bots that
    // slip past the `bot_id` field filter. When `allow_bot_messages` is on we
    // skip the drop AND treat the sender as a bot for the rest of the
    // pipeline, so it is never resolved by email or cached.
    let mut is_bot = already_bot;
    if let Some(ref info) = info {
        if info.is_bot || info.is_app_user {
            if !allow_bot_messages {
                return Ok(SenderIdentity::Rejected {
                    handle: info.handle.clone(),
                });
            }
            is_bot = true;
        }
    }

    let handle = info.as_ref().and_then(|info| info.handle.clone());
    let email = info.and_then(|info| info.email);
    if is_bot {
        return Ok(SenderIdentity::Unmapped {
            is_bot,
            email,
            handle,
        });
    }

    // Provider ID first (fast, and it needs no Slack call), then email.
    if let Some(user_id) = user_by_slack_id(users, slack_user_id).await? {
        return Ok(SenderIdentity::Mapped { user_id, handle });
    }
    let mapped = match email {
        Some(ref email) => user_by_email(users, slack_user_id, email).await?,
        None => None,
    };
    Ok(match mapped {
        Some(user_id) => SenderIdentity::Mapped { user_id, handle },
        None => SenderIdentity::Unmapped {
            is_bot,
            email,
            handle,
        },
    })
}

/// Look up a local system user by their Slack user ID.
pub(super) async fn user_by_slack_id(
    users: &dyn ProviderIdentityLink,
    slack_user_id: &str,
) -> Result<Option<Uuid>, AdapterError> {
    let user = users.user_at(PROVIDER, slack_user_id).await?;

    if let Some(ref user) = user {
        tracing::debug!(
            slack_user = %slack_user_id,
            user_id = %user.id,
            "Resolved Slack user via provider_id mapping"
        );
    }
    Ok(user.map(|user| user.id))
}

/// Look up a local system user by email, for inbound sender normalization.
pub(super) async fn user_by_email(
    users: &dyn ProviderIdentityLink,
    identity: &str,
    email: &str,
) -> Result<Option<Uuid>, AdapterError> {
    let user = users.user_by_email(email).await?;

    if let Some(user) = user {
        tracing::info!(slack_user = %identity, email = %email, user_id = %user.id, login = %user.login, "Resolved Slack user to system user");
        Ok(Some(user.id))
    } else {
        warn!(slack_user = %identity, email = %email, "No system user found for Slack user email");
        Ok(None)
    }
}

/// Cache the Slack user ID for future provider-ID lookups. Best-effort: a
/// failure is logged and the delivery continues.
pub(super) async fn cache_provider_id(
    users: &dyn ProviderIdentityLink,
    user_id: Uuid,
    slack_user_id: &str,
) {
    match users.link(&user_id, PROVIDER, slack_user_id).await {
        Ok(true) => {
            tracing::info!(
                user_id = %user_id,
                slack_user = %slack_user_id,
                "Cached Slack user ID in user_provider_ids"
            );
        }
        Ok(false) => {}
        Err(e) => {
            warn!(
                error = %e,
                user_id = %user_id,
                slack_user = %slack_user_id,
                "Failed to cache Slack provider ID in user_provider_ids"
            );
        }
    }
}

/// Prefer the explicitly configured value; when absent, ask `auth.test` so the
/// agent's system prompt always has the bot's identity available.
///
/// `auth.test` answers for whichever token calls it, and every outbound post
/// uses that same bot token, so the id it returns is the one the agent's own
/// messages carry in the thread. Do not derive this from a Loop's
/// `run_as_user_id`: that is a Moatless identity, and Slack never attributes a
/// post to it.
pub(super) async fn bot_user_id(api: &SlackApi, configured: Option<&str>) -> Option<String> {
    if let Some(id) = configured {
        let id = id.trim();
        if !id.is_empty() {
            tracing::info!(bot_user_id = %id, "Slack bot identity from configured app setting");
            return Some(id.to_string());
        }
    }

    match api.auth_test().await {
        Ok(info) => {
            tracing::info!(
                bot_user_id = %info.user_id,
                handle = %info.user.as_deref().unwrap_or(""),
                team = %info.team.as_deref().unwrap_or(""),
                "Slack bot identity resolved via auth.test"
            );
            Some(info.user_id)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Could not resolve Slack bot identity — set the app's `bot_user_id` explicitly; \
                 agent system prompt will fall back to an unknown-identity message"
            );
            None
        }
    }
}

/// An identity port nothing stands behind: reaching it at all panics, which is
/// how "this path resolves no sender" is asserted for the reasons raised before
/// the sender lookup.
#[cfg(test)]
pub(crate) struct UnreachableUsers;

#[cfg(test)]
#[async_trait::async_trait]
impl ProviderIdentityLink for UnreachableUsers {
    async fn provider_id_for_login(
        &self,
        _provider: &str,
        _login: &str,
    ) -> Result<Option<String>, moatless_types::store::StoreError> {
        panic!("this path must resolve no sender")
    }

    async fn user_with_login(
        &self,
        _provider: &str,
        _login: &str,
    ) -> Result<Option<String>, moatless_types::store::StoreError> {
        panic!("this path must resolve no sender")
    }

    async fn user_at(
        &self,
        _provider: &str,
        _provider_user_id: &str,
    ) -> Result<Option<moatless_ports::identity::LinkedUser>, moatless_types::store::StoreError>
    {
        panic!("this path must resolve no sender")
    }

    async fn user_by_email(
        &self,
        _email: &str,
    ) -> Result<Option<moatless_ports::identity::LinkedUser>, moatless_types::store::StoreError>
    {
        panic!("this path must resolve no sender")
    }

    async fn user_who_connected(
        &self,
        _provider: &str,
        _provider_user_id: &str,
    ) -> Result<Option<String>, moatless_types::store::StoreError> {
        panic!("this path must resolve no sender")
    }

    async fn link(
        &self,
        _user_id: &Uuid,
        _provider: &str,
        _provider_user_id: &str,
    ) -> Result<bool, moatless_types::store::StoreError> {
        panic!("this path must link no sender")
    }
}

#[cfg(test)]
mod tests {
    use moatless_adapter_model::test_support::ProviderStub;

    use super::{SenderIdentity, UnreachableUsers, bot_user_id, resolve_inbound_sender};
    use crate::api::SlackApi;

    async fn stub(user: serde_json::Value) -> ProviderStub {
        ProviderStub::start(move |path| match path {
            "/users.info" => user.clone(),
            "/auth.test" => serde_json::json!({
                "ok": true, "user_id": "U0BOT", "user": "soapbot", "team": "acme",
                "team_id": "T1",
            }),
            other => serde_json::json!({"ok": false, "error": format!("unexpected {other}")}),
        })
        .await
    }

    fn api(stub: &ProviderStub) -> SlackApi {
        SlackApi::with_base_url("xoxb-test", stub.base_url())
    }

    fn bot(handle: &str) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "user": {
                "id": "U1BOT", "name": handle, "is_bot": true, "is_app_user": false,
                "profile": { "email": "bot@example.com" },
            }
        })
    }

    #[tokio::test]
    async fn a_bot_account_is_rejected_before_any_query_runs() {
        let stub = stub(bot("soapbot")).await;

        let identity =
            resolve_inbound_sender(&api(&stub), &UnreachableUsers, "U1BOT", false, false)
                .await
                .expect("the bot drop needs no database");

        let SenderIdentity::Rejected { handle } = identity else {
            panic!("expected a rejected bot");
        };
        assert_eq!(handle.as_deref(), Some("soapbot"));
        assert_eq!(stub.paths(), vec!["/users.info".to_string()]);
    }

    #[tokio::test]
    async fn an_allowed_bot_is_never_resolved_by_email() {
        let stub = stub(bot("ci")).await;

        let identity = resolve_inbound_sender(&api(&stub), &UnreachableUsers, "U1BOT", true, false)
            .await
            .expect("an allowed bot skips every query");

        let SenderIdentity::Unmapped {
            is_bot,
            email,
            handle,
        } = identity
        else {
            panic!("expected an unmapped bot");
        };
        assert!(is_bot);
        assert_eq!(
            email.as_deref(),
            Some("bot@example.com"),
            "the email is carried for the notice, not for resolution"
        );
        assert_eq!(
            handle.as_deref(),
            Some("ci"),
            "who wrote it in Slack's terms is recorded even with nobody to resolve it to"
        );
    }

    /// A sender the `bot_id` field already flagged skips both queries too,
    /// even though `users.info` reports a plain human account.
    #[tokio::test]
    async fn a_sender_the_event_already_flagged_skips_the_mapping() {
        let stub = stub(serde_json::json!({
            "ok": true,
            "user": {
                "id": "U1", "name": "alice", "is_bot": false, "is_app_user": false,
                "profile": { "email": "alice@example.com" },
            }
        }))
        .await;

        let identity = resolve_inbound_sender(&api(&stub), &UnreachableUsers, "U1", true, true)
            .await
            .expect("an already-flagged bot skips every query");

        assert!(matches!(
            identity,
            SenderIdentity::Unmapped { is_bot: true, .. }
        ));
    }

    #[tokio::test]
    async fn a_missing_user_leaves_the_sender_unmapped_with_no_email() {
        let stub = stub(serde_json::json!({"ok": false, "error": "user_not_found"})).await;

        let identity = resolve_inbound_sender(&api(&stub), &UnreachableUsers, "U1", true, true)
            .await
            .expect("an absent user is not an error");

        let SenderIdentity::Unmapped { email, handle, .. } = identity else {
            panic!("expected an unmapped sender");
        };
        assert_eq!(email, None);
        assert_eq!(handle, None, "no user info, so no handle to record");
    }

    #[tokio::test]
    async fn a_configured_bot_id_wins_without_asking_slack() {
        let stub = stub(serde_json::json!({})).await;

        assert_eq!(
            bot_user_id(&api(&stub), Some("  U0CONFIGURED  ")).await,
            Some("U0CONFIGURED".to_string())
        );
        assert!(stub.paths().is_empty(), "no auth.test for a configured id");
    }

    #[tokio::test]
    async fn an_absent_bot_id_is_asked_of_auth_test() {
        let stub = stub(serde_json::json!({})).await;

        assert_eq!(
            bot_user_id(&api(&stub), None).await,
            Some("U0BOT".to_string())
        );
        assert_eq!(
            bot_user_id(&api(&stub), Some("   ")).await,
            Some("U0BOT".to_string()),
            "a blank configured value is no value"
        );
        assert_eq!(stub.paths().len(), 2);
    }

    #[tokio::test]
    async fn an_unreachable_slack_leaves_the_bot_unidentified() {
        let api = SlackApi::with_base_url(
            "xoxb-test",
            moatless_adapter_model::test_support::UNREACHABLE_BASE,
        );
        assert_eq!(bot_user_id(&api, None).await, None);
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use std::sync::Arc;

    use moatless_identity::ports::UserDirectory;
    use sqlx::PgPool;

    use super::{cache_provider_id, user_by_email, user_by_slack_id};

    /// The identity seam over a real pool — what the composition root hands the
    /// Slack ingress.
    fn users(pool: &PgPool) -> Arc<UserDirectory> {
        moatless_identity::compose::user_directory(pool.clone())
    }

    async fn seed_user(pool: &PgPool, email: &str, slack_user_id: Option<&str>) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, login, email, provider, created_at, last_login_at)
             VALUES ($1, 'alice', $2, 'local', now(), now())",
        )
        .bind(id)
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
        if let Some(slack_user_id) = slack_user_id {
            sqlx::query(
                "INSERT INTO user_provider_ids (user_id, provider, provider_user_id)
                 VALUES ($1, 'slack', $2)",
            )
            .bind(id)
            .bind(slack_user_id)
            .execute(pool)
            .await
            .unwrap();
        }
        id
    }

    #[sqlx::test]
    async fn an_unknown_slack_id_and_an_unknown_email_resolve_to_nothing(pool: PgPool) {
        assert_eq!(
            user_by_slack_id(users(&pool).as_ref(), "U1").await.unwrap(),
            None
        );
        assert_eq!(
            user_by_email(users(&pool).as_ref(), "U1", "nobody@example.com")
                .await
                .unwrap(),
            None
        );
    }

    /// Both lookups read `users.id` as the `uuid` the column is, so a sender
    /// that is mapped resolves instead of failing to decode.
    #[sqlx::test]
    async fn a_found_row_resolves_to_the_user_id(pool: PgPool) {
        let user_id = seed_user(&pool, "alice@example.com", Some("U1")).await;

        assert_eq!(
            user_by_slack_id(users(&pool).as_ref(), "U1").await.unwrap(),
            Some(user_id)
        );
        assert_eq!(
            user_by_email(users(&pool).as_ref(), "U1", "alice@example.com")
                .await
                .unwrap(),
            Some(user_id)
        );
    }

    /// The cache write binds the same `uuid`, so the row it has always claimed
    /// to write actually appears — and the next delivery from this sender is
    /// answered by the provider-id lookup with no email query behind it.
    #[sqlx::test]
    async fn the_cache_write_lands_a_row(pool: PgPool) {
        let user_id = seed_user(&pool, "alice@example.com", None).await;

        cache_provider_id(users(&pool).as_ref(), user_id, "U1").await;

        let cached: Option<(uuid::Uuid, String)> = sqlx::query_as(
            "SELECT user_id, provider_user_id FROM user_provider_ids WHERE provider = 'slack'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(cached, Some((user_id, "U1".to_string())));
        assert_eq!(
            user_by_slack_id(users(&pool).as_ref(), "U1").await.unwrap(),
            Some(user_id)
        );
    }

    /// Best-effort, and it stays that way: a second write of the same pair is a
    /// no-op rather than a failed delivery.
    #[sqlx::test]
    async fn a_repeated_cache_write_is_a_no_op(pool: PgPool) {
        let user_id = seed_user(&pool, "alice@example.com", None).await;

        cache_provider_id(users(&pool).as_ref(), user_id, "U1").await;
        cache_provider_id(users(&pool).as_ref(), user_id, "U1").await;

        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM user_provider_ids WHERE provider = 'slack'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 1);
    }
}
