use super::{NewAuthSession, PgSessionDao, SessionDao};
use crate::auth::users::PgUserDao;
use crate::auth::users::User;
use crate::auth::users::UserDao;
use chrono::{Duration, Utc};
use moatless_types::user::UserRole;
use sqlx::PgPool;
use uuid::Uuid;
async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    PgUserDao(pool.clone())
        .upsert_user(&make_user(id, &id.to_string(), false))
        .await
        .unwrap();
    id
}

async fn active_count(pool: &PgPool, user_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../core/migrations")]
async fn unknown_expired_and_disabled_account_sessions_do_not_resolve(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let sessions = PgSessionDao(pool.clone());
    let now = Utc::now();
    assert!(
        sessions
            .find_active_by_token_hash(b"unknown", now)
            .await
            .unwrap()
            .is_none()
    );

    seed_session(&sessions, user_id, b"idle").await;
    sqlx::query("UPDATE auth_sessions SET idle_expires_at = $1 WHERE token_hash = $2")
        .bind(now)
        .bind(b"idle".as_slice())
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sessions
            .find_active_by_token_hash(b"idle", now)
            .await
            .unwrap()
            .is_none()
    );

    seed_session(&sessions, user_id, b"absolute").await;
    sqlx::query("UPDATE auth_sessions SET idle_expires_at = $1, absolute_expires_at = $1 WHERE token_hash = $2")
        .bind(now).bind(b"absolute".as_slice()).execute(&pool).await.unwrap();
    assert!(
        sessions
            .find_active_by_token_hash(b"absolute", now)
            .await
            .unwrap()
            .is_none()
    );

    seed_session(&sessions, user_id, b"disabled").await;
    sqlx::query("UPDATE users SET status = 'disabled' WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sessions
            .find_active_by_token_hash(b"disabled", now)
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrations = "../core/migrations")]
async fn revocation_audits_only_newly_revoked_sessions(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let other_user_id = seed_user(&pool).await;
    let sessions = PgSessionDao(pool.clone());
    for token in [b"first".as_slice(), b"second", b"third"] {
        seed_session(&sessions, user_id, token).await;
    }
    seed_session(&sessions, other_user_id, b"other-user").await;
    assert!(
        sessions
            .revoke_by_token_hash(b"first", Utc::now())
            .await
            .unwrap()
    );
    assert!(
        !sessions
            .revoke_by_token_hash(b"first", Utc::now())
            .await
            .unwrap()
    );
    assert!(
        !sessions
            .revoke_by_token_hash(b"unknown", Utc::now())
            .await
            .unwrap()
    );
    assert!(
        sessions
            .find_active_by_token_hash(b"first", Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sessions
            .delete_sessions_for_user(&user_id.to_string())
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sessions
            .delete_sessions_for_user(&user_id.to_string())
            .await
            .unwrap(),
        0
    );
    assert_eq!(active_count(&pool, user_id).await, 0);
    assert_eq!(active_count(&pool, other_user_id).await, 1);
    let events: Vec<String> =
        sqlx::query_scalar("SELECT event FROM auth_audit_events WHERE user_id = $1 ORDER BY event")
            .bind(user_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        events,
        [
            "session_created",
            "session_created",
            "session_created",
            "session_revoked",
            "session_revoked_all",
            "session_revoked_all"
        ]
    );
    assert!(matches!(
        sessions.delete_sessions_for_user("not-a-uuid").await,
        Err(sqlx::Error::Protocol(_))
    ));
}

#[sqlx::test(migrations = "../core/migrations")]
async fn audit_insert_failure_rolls_back_session_creation(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let sessions = PgSessionDao(pool.clone());
    sqlx::query("ALTER TABLE auth_audit_events ADD CONSTRAINT reject_creation CHECK (event <> 'session_created')")
        .execute(&pool).await.unwrap();
    assert!(
        sessions
            .insert_session(&new_session(user_id, b"failed"))
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auth_audit_events WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audits, 0);
}

#[sqlx::test(migrations = "../core/migrations")]
async fn audit_insert_failure_rolls_back_single_and_all_session_revocation(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let sessions = PgSessionDao(pool.clone());
    seed_session(&sessions, user_id, b"first").await;
    seed_session(&sessions, user_id, b"second").await;
    sqlx::query("ALTER TABLE auth_audit_events ADD CONSTRAINT reject_revocation CHECK (event = 'session_created')")
        .execute(&pool).await.unwrap();
    assert!(
        sessions
            .revoke_by_token_hash(b"first", Utc::now())
            .await
            .is_err()
    );
    assert_eq!(active_count(&pool, user_id).await, 2);
    assert!(
        sessions
            .delete_sessions_for_user(&user_id.to_string())
            .await
            .is_err()
    );
    assert_eq!(active_count(&pool, user_id).await, 2);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auth_audit_events WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
}

#[sqlx::test(migrations = "../core/migrations")]
async fn touch_caps_expiry_and_does_not_update_revoked_rows(pool: PgPool) {
    let user_id = seed_user(&pool).await;
    let sessions = PgSessionDao(pool.clone());
    seed_session(&sessions, user_id, b"touch").await;
    let before = sessions
        .find_active_by_token_hash(b"touch", Utc::now())
        .await
        .unwrap()
        .unwrap();
    let now = chrono::DateTime::from_timestamp_micros(Utc::now().timestamp_micros()).unwrap();
    sessions
        .touch(
            before.id,
            now,
            before.absolute_expires_at + Duration::days(1),
        )
        .await
        .unwrap();
    let after = sessions
        .find_active_by_token_hash(b"touch", now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.last_seen_at, now);
    assert_eq!(after.idle_expires_at, after.absolute_expires_at);

    sessions.revoke_by_token_hash(b"touch", now).await.unwrap();
    sessions
        .touch(
            before.id,
            now + Duration::minutes(1),
            now + Duration::minutes(2),
        )
        .await
        .unwrap();
    let times: (chrono::DateTime<Utc>, chrono::DateTime<Utc>) =
        sqlx::query_as("SELECT last_seen_at, idle_expires_at FROM auth_sessions WHERE id = $1")
            .bind(before.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(times, (after.last_seen_at, after.idle_expires_at));
}

#[tokio::test]
async fn a_closed_database_preserves_errors_on_every_dao_operation() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/postgres")
        .unwrap();
    pool.close().await;
    let sessions = PgSessionDao(pool);
    let user_id = Uuid::new_v4();
    assert!(matches!(
        sessions
            .insert_session(&new_session(user_id, b"closed"))
            .await,
        Err(sqlx::Error::PoolClosed)
    ));
    assert!(matches!(
        sessions
            .find_active_by_token_hash(b"closed", Utc::now())
            .await,
        Err(sqlx::Error::PoolClosed)
    ));
    assert!(matches!(
        sessions.touch(Uuid::new_v4(), Utc::now(), Utc::now()).await,
        Err(sqlx::Error::PoolClosed)
    ));
    assert!(matches!(
        sessions.revoke_by_token_hash(b"closed", Utc::now()).await,
        Err(sqlx::Error::PoolClosed)
    ));
    assert!(matches!(
        sessions
            .delete_sessions_for_user(&user_id.to_string())
            .await,
        Err(sqlx::Error::PoolClosed)
    ));
}

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

fn new_session(user_id: Uuid, token_hash: &[u8]) -> NewAuthSession {
    let now = Utc::now();
    NewAuthSession {
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
    }
}

async fn seed_session(sessions: &PgSessionDao, user_id: Uuid, token_hash: &[u8]) {
    sessions
        .insert_session(&new_session(user_id, token_hash))
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../core/migrations")]
async fn bot_user_session_validates(pool: PgPool) {
    let users = PgUserDao(pool.clone());
    let sessions = PgSessionDao(pool.clone());

    let bot_id = Uuid::new_v4();
    users
        .upsert_user(&make_user(bot_id, "alerts-bot", true))
        .await
        .unwrap();
    let bot_hash = b"bot-session-token-hash".to_vec();
    seed_session(&sessions, bot_id, &bot_hash).await;

    let human_id = Uuid::new_v4();
    users
        .upsert_user(&make_user(human_id, "octocat", false))
        .await
        .unwrap();
    let human_hash = b"human-session-token-hash".to_vec();
    seed_session(&sessions, human_id, &human_hash).await;

    let now = Utc::now();
    let bot = sessions
        .find_active_by_token_hash(&bot_hash, now)
        .await
        .unwrap();
    assert_eq!(
        bot.expect("bot session must validate").user_id,
        bot_id,
        "a bot's issued session is dropped by find_active_by_token_hash"
    );

    let human = sessions
        .find_active_by_token_hash(&human_hash, now)
        .await
        .unwrap();
    assert_eq!(
        human.expect("human session must validate").user_id,
        human_id
    );
}
