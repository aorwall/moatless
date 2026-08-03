//! Slack identity resolution, by use case.
//!
//! Three callers, one place: the bot's own identity at construction, the
//! inbound sender during normalization, and the `AdapterUserIdentity` lookup
//! the shared reply paths use. Every `users` / `user_provider_ids` query the
//! Slack Adapter makes lives here.

use tracing::warn;
use uuid::Uuid;

use crate::adapters::error::AdapterError;
use crate::adapters::AdapterUserIdentity;
use crate::db::DbPool;

use super::adapter::SlackAdapter;
use super::api::{SlackApi, SlackUserInfo};

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
    db: &DbPool,
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
    if let Some(user_id) = user_by_slack_id(db, slack_user_id).await? {
        return Ok(SenderIdentity::Mapped { user_id, handle });
    }
    let mapped = match email {
        Some(ref email) => user_by_email(db, slack_user_id, email).await?,
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

/// Look up a local system user by their Slack user ID via `user_provider_ids`.
pub(super) async fn user_by_slack_id(
    db: &DbPool,
    slack_user_id: &str,
) -> Result<Option<Uuid>, AdapterError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r#"SELECT u.id FROM users u
               JOIN user_provider_ids p ON u.id = p.user_id
               WHERE p.adapter_kind = 'slack' AND p.provider_user_id = $1
               LIMIT 1"#,
    )
    .bind(slack_user_id)
    .fetch_optional(db)
    .await
    .map_err(AdapterError::Db)?;

    if let Some((id,)) = row {
        tracing::debug!(
            slack_user = %slack_user_id,
            user_id = %id,
            "Resolved Slack user via provider_id mapping"
        );
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

/// Look up a local system user by email. Shared by the inbound sender use case
/// and by the `AdapterUserIdentity` impl below.
pub(super) async fn user_by_email(
    db: &DbPool,
    identity: &str,
    email: &str,
) -> Result<Option<Uuid>, AdapterError> {
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, login FROM users WHERE email = $1 LIMIT 1")
            .bind(email)
            .fetch_optional(db)
            .await
            .map_err(AdapterError::Db)?;

    if let Some((id, login)) = row {
        tracing::info!(slack_user = %identity, email = %email, user_id = %id, login = %login, "Resolved Slack user to system user");
        Ok(Some(id))
    } else {
        warn!(slack_user = %identity, email = %email, "No system user found for Slack user email");
        Ok(None)
    }
}

/// Cache the Slack user ID for future provider-ID lookups. Best-effort: a
/// failure is logged and the delivery continues.
pub(super) async fn cache_provider_id(db: &DbPool, user_id: Uuid, slack_user_id: &str) {
    match sqlx::query(
        r#"INSERT INTO user_provider_ids (user_id, provider, provider_user_id)
                   VALUES ($1, 'slack', $2)
                   ON CONFLICT (user_id, provider) DO NOTHING"#,
    )
    .bind(user_id)
    .bind(slack_user_id)
    .execute(db)
    .await
    {
        Ok(result) if result.rows_affected() > 0 => {
            tracing::info!(
                user_id = %user_id,
                slack_user = %slack_user_id,
                "Cached Slack user ID in user_provider_ids"
            );
        }
        Ok(_) => {}
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
#[cfg(test)]
pub(super) async fn bot_user_id(api: &SlackApi, configured: Option<&str>) -> Option<String> {
    if let Some(id) = configured {
        let id = id.trim();
        if !id.is_empty() {
            tracing::info!(bot_user_id = %id, "Slack bot identity from env var");
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
                "Could not resolve Slack bot identity — set SLACK_BOT_USER_ID explicitly; \
                 agent system prompt will fall back to an unknown-identity message"
            );
            None
        }
    }
}

#[async_trait::async_trait]
impl AdapterUserIdentity for SlackAdapter {
    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError> {
        let app = self.app(crate::adapter_apps::DEFAULT_APP_KEY).await?;
        let Some(info) = app.api.get_user_info(identity).await? else {
            warn!(slack_user = %identity, "Slack users.info returned no user");
            return Ok(None);
        };

        // Bot / app accounts share emails across deployments — never resolve them by email.
        if info.is_bot || info.is_app_user {
            tracing::info!(
                slack_user = %identity,
                is_bot = info.is_bot,
                is_app_user = info.is_app_user,
                handle = %info.handle.as_deref().unwrap_or(""),
                "Slack user is a bot/app account — skipping email-based user mapping"
            );
            return Ok(None);
        }

        let Some(email) = info.email else {
            warn!(slack_user = %identity, "Slack API returned no email for user");
            return Ok(None);
        };

        // The trait speaks `String` for every provider, so the id is rendered
        // here rather than carried as one.
        Ok(user_by_email(db, identity, &email)
            .await?
            .map(|id| id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::adapters::test_support::ProviderStub;
    use crate::adapters::AdapterUserIdentity;
    use crate::db::DbPool;

    use super::super::adapter::SlackAdapter;
    use super::super::api::SlackApi;
    use super::{bot_user_id, resolve_inbound_sender, SenderIdentity};

    fn unreachable_pool() -> DbPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_millis(10))
            .connect_lazy("postgres://fake:fake@127.0.0.1:1/fake")
            .unwrap()
    }

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
            resolve_inbound_sender(&api(&stub), &unreachable_pool(), "U1BOT", false, false)
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

        let identity =
            resolve_inbound_sender(&api(&stub), &unreachable_pool(), "U1BOT", true, false)
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

        let identity = resolve_inbound_sender(&api(&stub), &unreachable_pool(), "U1", true, true)
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

        let identity = resolve_inbound_sender(&api(&stub), &unreachable_pool(), "U1", true, true)
            .await
            .expect("an absent user is not an error");

        let SenderIdentity::Unmapped { email, handle, .. } = identity else {
            panic!("expected an unmapped sender");
        };
        assert_eq!(email, None);
        assert_eq!(handle, None, "no user info, so no handle to record");
    }

    #[tokio::test]
    async fn a_bot_account_is_never_resolved_by_the_identity_capability() {
        let stub = stub(bot("soapbot")).await;
        let adapter = SlackAdapter::with_api(api(&stub), "secret", None);

        assert_eq!(
            adapter
                .resolve_user("U1BOT", &unreachable_pool())
                .await
                .expect("a bot is dropped before the email query"),
            None
        );
    }

    #[tokio::test]
    async fn a_user_without_an_email_is_never_resolved_by_the_identity_capability() {
        let stub = stub(serde_json::json!({
            "ok": true,
            "user": { "id": "U1", "name": "alice", "is_bot": false, "profile": {} }
        }))
        .await;
        let adapter = SlackAdapter::with_api(api(&stub), "secret", None);

        assert_eq!(
            adapter
                .resolve_user("U1", &unreachable_pool())
                .await
                .expect("no email means no query"),
            None
        );
    }

    #[tokio::test]
    async fn an_absent_user_is_never_resolved_by_the_identity_capability() {
        let stub = stub(serde_json::json!({"ok": true})).await;
        let adapter = SlackAdapter::with_api(api(&stub), "secret", None);

        assert_eq!(
            adapter
                .resolve_user("U1", &unreachable_pool())
                .await
                .expect("no user means no query"),
            None
        );
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
        let api =
            SlackApi::with_base_url("xoxb-test", crate::adapters::test_support::UNREACHABLE_BASE);
        assert_eq!(bot_user_id(&api, None).await, None);
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use sqlx::PgPool;

    use super::{cache_provider_id, user_by_email, user_by_slack_id};

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
        assert_eq!(user_by_slack_id(&pool, "U1").await.unwrap(), None);
        assert_eq!(
            user_by_email(&pool, "U1", "nobody@example.com")
                .await
                .unwrap(),
            None
        );
    }

    /// Both queries read `users.id` as the `uuid` the column is, so a sender
    /// that is mapped resolves instead of failing to decode.
    #[sqlx::test]
    async fn a_found_row_resolves_to_the_user_id(pool: PgPool) {
        let user_id = seed_user(&pool, "alice@example.com", Some("U1")).await;

        assert_eq!(user_by_slack_id(&pool, "U1").await.unwrap(), Some(user_id));
        assert_eq!(
            user_by_email(&pool, "U1", "alice@example.com")
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

        cache_provider_id(&pool, user_id, "U1").await;

        let cached: Option<(uuid::Uuid, String)> = sqlx::query_as(
            "SELECT user_id, provider_user_id FROM user_provider_ids WHERE provider = 'slack'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(cached, Some((user_id, "U1".to_string())));
        assert_eq!(user_by_slack_id(&pool, "U1").await.unwrap(), Some(user_id));
    }

    /// Best-effort, and it stays that way: a second write of the same pair is a
    /// no-op rather than a failed delivery.
    #[sqlx::test]
    async fn a_repeated_cache_write_is_a_no_op(pool: PgPool) {
        let user_id = seed_user(&pool, "alice@example.com", None).await;

        cache_provider_id(&pool, user_id, "U1").await;
        cache_provider_id(&pool, user_id, "U1").await;

        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM user_provider_ids WHERE provider = 'slack'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 1);
    }
}
