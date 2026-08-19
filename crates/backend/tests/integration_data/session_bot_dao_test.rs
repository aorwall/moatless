#![cfg(feature = "db-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Regression: a bot (service-account) user's issued session must validate.
//!
//! The token/api-key exchange endpoints mint sessions for bot users so their
//! sandbox `moat` CLI can call the API. `find_active_by_token_hash` must not
//! silently drop those — filtering `NOT u.is_bot` here wedged a bot-owned
//! task's `moat` in a permanent 401 once its `X-User-Token` JWT expired and it
//! fell back to the session cookie.

use chrono::{Duration, Utc};
use moatless_backend::auth::sessions::{NewAuthSession, PgSessionDao, SessionDao};
use moatless_backend::auth::users::{PgUserDao, User, UserDao};
use moatless_types::user::UserRole;
use sqlx::PgPool;
use uuid::Uuid;

fn make_user(id: Uuid, login: &str, is_bot: bool) -> User {
    let now = Utc::now();
    User {
        id,
        login: login.to_string(),
        email: Some(format!("{login}@example.com")),
        name: Some(login.to_string()),
        avatar_url: None,
        provider: "github_app".to_string(),
        provider_user_id: Some(format!("bot:{login}")),
        role: UserRole::User,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot,
    }
}

async fn seed_session(sessions: &PgSessionDao, user_id: Uuid, token_hash: &[u8]) {
    let now = Utc::now();
    sessions
        .insert_session(&NewAuthSession {
            id: Uuid::new_v4(),
            user_id,
            token_hash: token_hash.to_vec(),
            csrf_token_hash: b"csrf-hash".to_vec(),
            authenticated_via: "token".to_string(),
            group_id: None,
            created_at: now,
            last_seen_at: now,
            idle_expires_at: now + Duration::hours(1),
            absolute_expires_at: now + Duration::days(1),
        })
        .await
        .unwrap();
}

/// The session a bot user was issued resolves — same as a human user's would.
#[sqlx::test]
async fn bot_user_session_validates(pool: PgPool) {
    let users = PgUserDao(pool.clone());
    let sessions = PgSessionDao(pool.clone());

    let bot_id = Uuid::new_v4();
    users.upsert_user(&make_user(bot_id, "alerts-bot", true)).await.unwrap();
    let bot_hash = b"bot-session-token-hash".to_vec();
    seed_session(&sessions, bot_id, &bot_hash).await;

    let human_id = Uuid::new_v4();
    users.upsert_user(&make_user(human_id, "octocat", false)).await.unwrap();
    let human_hash = b"human-session-token-hash".to_vec();
    seed_session(&sessions, human_id, &human_hash).await;

    let now = Utc::now();
    let bot = sessions.find_active_by_token_hash(&bot_hash, now).await.unwrap();
    assert_eq!(
        bot.expect("bot session must validate").user_id,
        bot_id,
        "a bot's issued session is dropped by find_active_by_token_hash"
    );

    let human = sessions.find_active_by_token_hash(&human_hash, now).await.unwrap();
    assert_eq!(human.expect("human session must validate").user_id, human_id);
}
