//! What one Telegram update normalizes to, pinned before anything moves.
//!
//! Everything here is asserted through [`normalize`] below — the one place that
//! names the function under test. When the breakup moved that function out of
//! `adapter.rs` and turned its three Sender effects into returned values, every
//! assertion in this file stayed byte-identical and only the helper's body
//! changed. That is the whole point of writing them at one call site.
//!
//! Two properties are asserted as *counts* rather than outcomes, because they
//! are what the move is at risk of changing: how many Bot API calls a path
//! makes, and how many rows it writes. Both go through
//! [`super::test_support::bot_api_stub`], so no test reaches api.telegram.org.

use axum::http::StatusCode;

use moatless_loops::adapters::error::AdapterError;
use moatless_loops::adapters::models::{InboundMessage, LoopContext};
use moatless_loops::adapters::test_support::stub_provider;
use moatless_core::db::DbPool;

use super::adapter::TelegramAdapter;
use super::api::TelegramApi;
use super::config::TelegramAdapterConfig;
use super::test_support::{bot_api_stub, test_loop_context, BotApiStub};
use super::webhook::{Normalization, NormalizeContext};

/// The outcome of normalizing one update, in the vocabulary the assertions use.
#[derive(Debug)]
#[cfg_attr(
    not(feature = "db-tests"),
    allow(
        dead_code,
        reason = "only a routable message carries one, and every routable case needs a database"
    )
)]
enum Normalized {
    Message(Box<InboundMessage>),
    Ignored,
}

impl Normalized {
    /// The message, or a panic naming what came back instead.
    #[cfg_attr(
        not(feature = "db-tests"),
        allow(dead_code, reason = "every routable case needs a database")
    )]
    fn message(self) -> InboundMessage {
        match self {
            Self::Message(message) => *message,
            Self::Ignored => panic!("expected a routable message, got Ignored"),
        }
    }

    fn is_ignored(&self) -> bool {
        matches!(self, Self::Ignored)
    }
}

/// Normalize one update, applying whatever effects it decided on.
///
/// **The single adaptation point of this file.** Every assertion below is
/// written against `Normalized` and the stub's call log, so moving
/// normalization into `webhook::normalize` — where the effects become returned
/// values applied by the caller — changes this function and nothing else.
async fn normalize(
    provider: &TelegramAdapter,
    update: &serde_json::Value,
    config: &LoopContext,
    is_private_chat: bool,
    db: &DbPool,
) -> Result<Normalized, AdapterError> {
    let tg_config = TelegramAdapterConfig::from_config(config);
    let normalized = super::webhook::normalize(
        update,
        &NormalizeContext {
            config,
            tg_config: &tg_config,
            db,
            is_private_chat,
        },
    )
    .await?;

    // Normalization no longer performs its effects, so the helper does what
    // ingress does — otherwise the Bot API and pairing-row counts below would
    // be measuring the move rather than the behaviour.
    let (outcome, effects) = match normalized {
        Normalization::Message { message, effects } => (Normalized::Message(message), effects),
        Normalization::Ignored { effects, .. } => (Normalized::Ignored, effects),
    };
    super::ingress::apply_effects(provider.api(), db, &config.id, effects).await?;
    Ok(outcome)
}

// =========================================================================
// Fixtures
// =========================================================================

/// A Subscription whose source matcher is one chat, with `@testbot` configured
/// and both access policies open.
fn config(source_id: &str) -> LoopContext {
    LoopContext {
        adapter_config: Some(serde_json::json!({ "bot_username": "@testbot" })),
        ..test_loop_context(source_id)
    }
}

fn with_adapter_config(mut config: LoopContext, adapter_config: serde_json::Value) -> LoopContext {
    config.adapter_config = Some(adapter_config);
    config
}

/// A private-chat text message from user 999 in chat 111.
fn private_message(text: &str) -> serde_json::Value {
    serde_json::json!({
        "update_id": 5000,
        "message": {
            "message_id": 42,
            "from": { "id": 999, "first_name": "Ada", "username": "ada" },
            "chat": { "id": 111, "type": "private" },
            "text": text,
        }
    })
}

/// A group text message from user 999 in chat -100123.
fn group_message(text: &str) -> serde_json::Value {
    serde_json::json!({
        "update_id": 5001,
        "message": {
            "message_id": 43,
            "from": { "id": 999, "first_name": "Ada" },
            "chat": { "id": -100_123, "type": "supergroup" },
            "text": text,
        }
    })
}

/// A provider with a recording Bot API stub behind it.
async fn provider_with_stub() -> (TelegramAdapter, BotApiStub) {
    let stub = bot_api_stub(serde_json::json!({ "message_id": 7 })).await;
    let provider = TelegramAdapter::with_api(TelegramApi::with_base_url(stub.base_url()));
    (provider, stub)
}

/// A pool that resolves nothing, for the paths that return before the sender
/// lookup. A path that reaches the lookup with this pool fails loudly rather
/// than quietly resolving to the fallback user.
fn unreachable_pool() -> DbPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://fake:fake@127.0.0.1:1/fake")
        .expect("lazy pool")
}

// =========================================================================
// Updates that carry nothing to route
// =========================================================================

#[tokio::test]
async fn an_update_with_no_message_is_ignored() {
    let (provider, stub) = provider_with_stub().await;
    let update = serde_json::json!({
        "update_id": 12348,
        "inline_query": { "id": "abc", "query": "test" }
    });

    let outcome = normalize(
        &provider,
        &update,
        &config("111"),
        false,
        &unreachable_pool(),
    )
    .await
    .expect("an unsupported update is ignored, not an error");

    assert!(outcome.is_ignored());
    assert!(
        stub.methods().is_empty(),
        "no Bot API call: {:?}",
        stub.methods()
    );
}

#[tokio::test]
async fn a_channel_post_without_a_sender_is_ignored() {
    let (provider, stub) = provider_with_stub().await;
    let update = serde_json::json!({
        "update_id": 12347,
        "channel_post": {
            "message_id": 45,
            "chat": { "id": -100_123, "type": "channel" },
            "text": "channel message"
        }
    });

    let outcome = normalize(
        &provider,
        &update,
        &config("-100123"),
        false,
        &unreachable_pool(),
    )
    .await
    .expect("a senderless post is ignored, not an error");

    assert!(outcome.is_ignored());
    assert!(stub.methods().is_empty());
}

#[tokio::test]
async fn a_message_with_no_text_is_ignored() {
    let (provider, stub) = provider_with_stub().await;
    let update = serde_json::json!({
        "update_id": 12346,
        "message": {
            "message_id": 43,
            "from": { "id": 999, "first_name": "Test" },
            "chat": { "id": 111, "type": "private" },
            "sticker": { "file_id": "abc" }
        }
    });

    let outcome = normalize(
        &provider,
        &update,
        &config("111"),
        false,
        &unreachable_pool(),
    )
    .await
    .expect("a sticker is ignored, not an error");

    assert!(outcome.is_ignored());
    assert!(stub.methods().is_empty());
}

#[tokio::test]
async fn a_message_that_is_only_a_bot_mention_is_ignored() {
    let (provider, stub) = provider_with_stub().await;

    let outcome = normalize(
        &provider,
        &group_message("@testbot"),
        &config("-100123"),
        false,
        &unreachable_pool(),
    )
    .await
    .expect("an empty message after mention stripping is ignored");

    assert!(outcome.is_ignored());
    assert!(stub.methods().is_empty());
}

// =========================================================================
// Envelopes no re-delivery can repair
// =========================================================================

#[tokio::test]
async fn each_missing_envelope_field_is_named_in_the_rejection() {
    let (provider, _stub) = provider_with_stub().await;

    for (update, expected) in [
        (
            serde_json::json!({
                "message": { "message_id": 1, "chat": { "id": 1 }, "text": "hi" }
            }),
            "Invalid request: Missing update_id",
        ),
        (
            serde_json::json!({
                "update_id": 1,
                "message": { "message_id": 1, "text": "hi" }
            }),
            "Invalid request: Missing chat.id",
        ),
        (
            serde_json::json!({
                "update_id": 1,
                "message": { "chat": { "id": 1 }, "text": "hi" }
            }),
            "Invalid request: Missing message_id",
        ),
    ] {
        let error = normalize(
            &provider,
            &update,
            &config("111"),
            false,
            &unreachable_pool(),
        )
        .await
        .expect_err("a malformed envelope is rejected");
        assert_eq!(error.to_string(), expected);
    }
}

// =========================================================================
// Commands answered inline
// =========================================================================

#[tokio::test]
async fn start_is_answered_with_one_message_and_the_update_is_still_ignored() {
    for text in ["/start", "/start@testbot", "  /start  "] {
        let (provider, stub) = provider_with_stub().await;

        let outcome = normalize(
            &provider,
            &private_message(text),
            &config("111"),
            true,
            &unreachable_pool(),
        )
        .await
        .expect("a command is answered, not an error");

        assert!(outcome.is_ignored(), "{text} must not route");
        assert_eq!(stub.methods(), vec!["sendMessage".to_string()], "{text}");
        let call = &stub.calls()[0];
        assert_eq!(call.body["chat_id"], "111");
        assert_eq!(
            call.body["text"],
            "Welcome! Send me a message to get started."
        );
    }
}

#[tokio::test]
async fn help_is_answered_with_one_message_and_the_update_is_still_ignored() {
    for text in ["/help", "/help@testbot"] {
        let (provider, stub) = provider_with_stub().await;

        let outcome = normalize(
            &provider,
            &private_message(text),
            &config("111"),
            true,
            &unreachable_pool(),
        )
        .await
        .expect("a command is answered, not an error");

        assert!(outcome.is_ignored(), "{text} must not route");
        assert_eq!(stub.methods(), vec!["sendMessage".to_string()], "{text}");
        assert_eq!(
            stub.calls()[0].body["text"],
            "Send any message and I'll create a task for it."
        );
    }
}

/// The answer is best-effort: a Bot API that refuses it does not turn the
/// update into a failed delivery.
#[tokio::test]
async fn a_refused_command_answer_still_ignores_the_update() {
    let base = stub_provider(StatusCode::INTERNAL_SERVER_ERROR, "nope").await;
    let provider = TelegramAdapter::with_api(TelegramApi::with_base_url(base));

    let outcome = normalize(
        &provider,
        &private_message("/start"),
        &config("111"),
        true,
        &unreachable_pool(),
    )
    .await
    .expect("a failed answer is swallowed");

    assert!(outcome.is_ignored());
}

// =========================================================================
// Access policy
// =========================================================================

#[tokio::test]
async fn a_blocking_access_decision_ignores_the_update_without_a_bot_call() {
    for (policy_config, is_private) in [
        (serde_json::json!({ "dm_access_policy": "disabled" }), true),
        (
            serde_json::json!({ "dm_access_policy": "allowlist", "dm_allowlist": ["1"] }),
            true,
        ),
        (
            serde_json::json!({ "group_access_policy": "disabled" }),
            false,
        ),
        (
            serde_json::json!({ "group_access_policy": "allowlist", "group_allowlist": ["-1"] }),
            false,
        ),
        (
            serde_json::json!({ "bot_username": "@testbot", "require_mention": true }),
            false,
        ),
    ] {
        let (provider, stub) = provider_with_stub().await;
        let update = if is_private {
            private_message("do the thing")
        } else {
            group_message("do the thing")
        };

        let outcome = normalize(
            &provider,
            &update,
            &with_adapter_config(config("111"), policy_config.clone()),
            is_private,
            &unreachable_pool(),
        )
        .await
        .expect("a denied message is ignored, not an error");

        assert!(outcome.is_ignored(), "{policy_config}");
        assert!(stub.methods().is_empty(), "{policy_config}");
    }
}

/// `Pairing` is not meaningful for a group, and `enforce_group_access` treats
/// it as an allowlist rather than sending a code into the room.
#[tokio::test]
async fn a_group_never_gets_a_pairing_code() {
    let (provider, stub) = provider_with_stub().await;

    let outcome = normalize(
        &provider,
        &group_message("do the thing"),
        &with_adapter_config(
            config("-100123"),
            serde_json::json!({ "group_access_policy": "pairing" }),
        ),
        false,
        &unreachable_pool(),
    )
    .await
    .expect("an unlisted group is ignored");

    assert!(outcome.is_ignored());
    assert!(stub.methods().is_empty());
}

// =========================================================================
// Routable messages — these reach the sender lookup, so they need a database
// =========================================================================

#[cfg(feature = "db-tests")]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions; the lints only fire when the db-tests feature is enabled"
)]
mod db_tests {
    use sqlx::PgPool;

    use moatless_loops::adapters::models::Audience;
    use moatless_loops::adapters::AdapterKind;

    use super::super::test_support::OWNER;
    use super::*;

    /// Map a Telegram user id to a system user, the way the OAuth link does.
    async fn map_sender(pool: &PgPool, telegram_user_id: &str, user_id: &str) {
        sqlx::query(
            "INSERT INTO provider_tokens \
             (user_id, provider, provider_user_id, access_token, scope, owner_user_id) \
             VALUES ($1, 'telegram', $2, 'token', 'user', $1)",
        )
        .bind(user_id)
        .bind(telegram_user_id)
        .execute(pool)
        .await
        .expect("map Telegram sender");
    }

    /// A private chat threads on the chat, so every message from one user lands
    /// on one task.
    #[sqlx::test]
    async fn a_private_chat_threads_every_message_onto_the_chat(pool: PgPool) {
        let (provider, stub) = provider_with_stub().await;

        let message = normalize(
            &provider,
            &private_message("do the thing"),
            &config("111"),
            true,
            &pool,
        )
        .await
        .unwrap()
        .message();

        assert_eq!(message.adapter_kind, AdapterKind::Telegram);
        assert_eq!(message.source_id, "111");
        assert_eq!(message.thread_id.as_deref(), Some("111"));
        assert_eq!(message.message_thread_id, "111");
        assert_eq!(message.text, "do the thing");
        assert_eq!(message.user_id, OWNER, "an unmapped sender falls back");
        assert_eq!(
            message.author.as_deref(),
            Some("@ada"),
            "the sender who could not be resolved is still named in Telegram's terms"
        );
        assert_eq!(message.external_event_id.as_deref(), Some("tg:m:42"));
        assert_eq!(message.audience, Audience::Everyone);
        assert_eq!(message.adapter_context, serde_json::json!({}));
        assert!(message.file_ids.is_empty());
        assert_eq!(
            message.thread_properties,
            serde_json::json!({
                "chat_id": "111",
                "message_id": 42,
                "forum_topic_id": serde_json::Value::Null,
            })
        );
        assert!(stub.methods().is_empty(), "normalization posts nothing");
    }

    /// The wildcard rule: a private chat matched by a `*` Subscription routes
    /// under `*`, so the router finds that same wildcard config again.
    #[sqlx::test]
    async fn a_private_chat_under_a_wildcard_subscription_keeps_the_wildcard_source(pool: PgPool) {
        let (provider, _stub) = provider_with_stub().await;

        let message = normalize(&provider, &private_message("hi"), &config("*"), true, &pool)
            .await
            .unwrap()
            .message();

        assert_eq!(message.source_id, "*");
        assert_eq!(message.thread_id.as_deref(), Some("111"));
        assert_eq!(message.message_thread_id, "111");
    }

    /// A forum topic is its own subject, keyed `chat:topic`.
    #[sqlx::test]
    async fn a_forum_topic_is_its_own_thread(pool: PgPool) {
        let (provider, _stub) = provider_with_stub().await;
        let update = serde_json::json!({
            "update_id": 5002,
            "message": {
                "message_id": 44,
                "message_thread_id": 77,
                "from": { "id": 999, "first_name": "Ada" },
                "chat": { "id": -100_123, "type": "supergroup", "is_forum": true },
                "text": "in the topic",
            }
        });

        let message = normalize(&provider, &update, &config("-100123"), false, &pool)
            .await
            .unwrap()
            .message();

        assert_eq!(message.source_id, "-100123");
        assert_eq!(message.thread_id.as_deref(), Some("-100123:77"));
        assert_eq!(message.message_thread_id, "-100123:77");
        assert_eq!(
            message.author.as_deref(),
            Some("Ada"),
            "no @username on this account, so the display name stands in"
        );
        assert_eq!(
            message.thread_properties,
            serde_json::json!({
                "chat_id": "-100123",
                "message_id": 44,
                "forum_topic_id": 77,
            })
        );
    }

    /// `message_thread_id` without `chat.is_forum` is not a topic — the group
    /// falls back to reply threading.
    #[sqlx::test]
    async fn a_thread_id_without_a_forum_flag_is_not_a_topic(pool: PgPool) {
        let (provider, _stub) = provider_with_stub().await;
        let update = serde_json::json!({
            "update_id": 5003,
            "message": {
                "message_id": 44,
                "message_thread_id": 77,
                "from": { "id": 999, "first_name": "Ada" },
                "chat": { "id": -100_123, "type": "supergroup" },
                "text": "not in a topic",
            }
        });

        let message = normalize(&provider, &update, &config("-100123"), false, &pool)
            .await
            .unwrap()
            .message();

        assert_eq!(message.thread_id, None);
        assert_eq!(message.message_thread_id, "44");
    }

    /// In a non-forum group, a reply continues the replied-to message's thread
    /// and a fresh message starts its own.
    #[sqlx::test]
    async fn a_group_reply_continues_the_replied_message_thread(pool: PgPool) {
        let (provider, _stub) = provider_with_stub().await;
        let update = serde_json::json!({
            "update_id": 5004,
            "message": {
                "message_id": 90,
                "reply_to_message": { "message_id": 55 },
                "from": { "id": 999, "first_name": "Ada" },
                "chat": { "id": -100_123, "type": "supergroup" },
                "text": "and another thing",
            }
        });

        let message = normalize(&provider, &update, &config("-100123"), false, &pool)
            .await
            .unwrap()
            .message();

        assert_eq!(message.thread_id.as_deref(), Some("55"));
        assert_eq!(message.message_thread_id, "55");
        assert_eq!(message.external_event_id.as_deref(), Some("tg:m:90"));

        let fresh = normalize(
            &provider,
            &group_message("something new"),
            &config("-100123"),
            false,
            &pool,
        )
        .await
        .unwrap()
        .message();
        assert_eq!(fresh.thread_id, None);
        assert_eq!(fresh.message_thread_id, "43");
    }

    /// A mapped sender routes as their own user; the mention is stripped from
    /// what the task reads.
    #[sqlx::test]
    async fn a_mapped_sender_routes_as_their_own_user(pool: PgPool) {
        map_sender(&pool, "999", "user-ada").await;
        let (provider, _stub) = provider_with_stub().await;

        let message = normalize(
            &provider,
            &group_message("@testbot fix the bug"),
            &config("-100123"),
            false,
            &pool,
        )
        .await
        .unwrap()
        .message();

        assert_eq!(message.user_id, "user-ada");
        assert_eq!(message.text, "fix the bug");
    }

    /// A `Pairing` DM policy writes exactly one request row, sends exactly one
    /// message, and still ignores the update.
    #[sqlx::test]
    async fn a_pairing_policy_upserts_one_request_and_sends_one_message(pool: PgPool) {
        seed_trigger(&pool, "tt_test").await;
        let (provider, stub) = provider_with_stub().await;
        let subscription = with_adapter_config(
            config("111"),
            serde_json::json!({ "dm_access_policy": "pairing" }),
        );

        let outcome = normalize(
            &provider,
            &private_message("let me in"),
            &subscription,
            true,
            &pool,
        )
        .await
        .unwrap();

        assert!(outcome.is_ignored());
        assert_eq!(stub.methods(), vec!["sendMessage".to_string()]);

        let (code, status, meta): (String, String, serde_json::Value) = sqlx::query_as(
            "SELECT code, status, meta FROM integration_pairing_requests \
             WHERE loop_id = 'tt_test' AND provider_user_id = '999'",
        )
        .fetch_one(&pool)
        .await
        .expect("exactly one pairing request");

        assert_eq!(status, "pending");
        assert_eq!(meta["username"], "ada");
        assert_eq!(meta["first_name"], "Ada");
        let sent = stub.calls()[0].body["text"].as_str().unwrap().to_string();
        assert!(sent.contains("Your Telegram user ID: `999`"), "{sent}");
        assert!(sent.contains(&format!("Pairing code: `{code}`")), "{sent}");

        // A second update reuses the pending row rather than writing a second.
        normalize(
            &provider,
            &private_message("still waiting"),
            &subscription,
            true,
            &pool,
        )
        .await
        .unwrap();
        let rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM integration_pairing_requests WHERE loop_id = 'tt_test'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 1, "the pending request is upserted, not duplicated");
    }

    /// An already-approved sender passes a `Pairing` policy without a code.
    #[sqlx::test]
    async fn an_allowlisted_sender_skips_pairing(pool: PgPool) {
        seed_trigger(&pool, "tt_test").await;
        let (provider, stub) = provider_with_stub().await;

        let message = normalize(
            &provider,
            &private_message("do the thing"),
            &with_adapter_config(
                config("111"),
                serde_json::json!({ "dm_access_policy": "pairing", "dm_allowlist": ["999"] }),
            ),
            true,
            &pool,
        )
        .await
        .unwrap()
        .message();

        assert_eq!(message.text, "do the thing");
        assert!(stub.methods().is_empty());
    }

    /// The `loops` row `integration_pairing_requests.loop_id` references.
    async fn seed_trigger(pool: &PgPool, id: &str) {
        sqlx::query(
            "INSERT INTO loops (id, name, kind, active, scope, created_by) \
             VALUES ($1, 'telegram', 'adapter_event', true, 'global', 'system')",
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("seed trigger");
    }
}
