#![cfg(feature = "db-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Internal-uuid identity: provider-id resolution and the native-uuid foreign
//! keys. `#[sqlx::test]` applies every migration (0090 included) to a throwaway
//! database, so these run against the real uuid-typed `users.id`.

use chrono::Utc;
use moatless_vibe_backend::auth::models::UserRole;
use moatless_vibe_backend::auth::password::dao::{
    PgUserCredentialDao, UserCredentialDao,
};
use moatless_vibe_backend::auth::users::{PgUserDao, User, UserDao};
use moatless_vibe_backend::team::dao::{PgTeamDao, TeamDao};
use sqlx::PgPool;
use uuid::Uuid;

fn make_user(id: Uuid, login: &str, provider: &str, provider_user_id: &str) -> User {
    let now = Utc::now();
    User {
        id,
        login: login.to_string(),
        email: None,
        name: None,
        avatar_url: None,
        provider: provider.to_string(),
        provider_user_id: Some(provider_user_id.to_string()),
        role: UserRole::User,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot: false,
    }
}

/// Login resolves by `(provider, provider_user_id)`; legacy references resolve
/// by `provider_user_id` alone. `github`/`github_app` collapse to one key.
#[sqlx::test]
async fn resolves_users_by_provider_and_legacy_id(pool: PgPool) {
    let repo = PgUserDao(pool.clone());

    // GitHub OAuth user: internal uuid + preserved numeric provider id.
    let gh = Uuid::new_v4();
    repo.upsert_user(&make_user(gh, "octocat", "github", "609489"))
        .await
        .unwrap();
    repo.upsert_provider_id(&gh, "github", "609489")
        .await
        .unwrap();

    // Auth0 user.
    let a0 = Uuid::new_v4();
    repo.upsert_user(&make_user(a0, "alice", "auth0", "auth0|abc"))
        .await
        .unwrap();
    repo.upsert_provider_id(&a0, "auth0", "auth0|abc")
        .await
        .unwrap();

    // Local user.
    let loc = Uuid::new_v4();
    repo.upsert_user(&make_user(loc, "bob", "local", "local:bob"))
        .await
        .unwrap();
    repo.upsert_provider_id(&loc, "local", "local:bob")
        .await
        .unwrap();

    // Bot: `github_app` collapses to the `github` lookup key at the call site.
    let bot = Uuid::new_v4();
    repo.upsert_user(&make_user(
        bot,
        "reviewer[bot]",
        "github_app",
        "bot:reviewer",
    ))
    .await
    .unwrap();
    repo.upsert_provider_id(&bot, "github", "bot:reviewer")
        .await
        .unwrap();

    // Login path: (provider, provider_user_id) -> internal uuid.
    assert_eq!(
        repo.find_user_by_provider_id("github", "609489")
            .await
            .unwrap()
            .unwrap()
            .id,
        gh
    );
    assert_eq!(
        repo.find_user_by_provider_id("auth0", "auth0|abc")
            .await
            .unwrap()
            .unwrap()
            .id,
        a0
    );
    assert_eq!(
        repo.find_user_by_provider_id("github", "bot:reviewer")
            .await
            .unwrap()
            .unwrap()
            .id,
        bot
    );
    assert!(repo
        .find_user_by_provider_id("github", "does-not-exist")
        .await
        .unwrap()
        .is_none());
    // A github user is not reachable under the auth0 provider key.
    assert!(repo
        .find_user_by_provider_id("auth0", "609489")
        .await
        .unwrap()
        .is_none());

    // Legacy path: provider_user_id alone, regardless of provider.
    assert_eq!(
        repo.find_user_by_legacy_id("609489")
            .await
            .unwrap()
            .unwrap()
            .id,
        gh
    );
    assert_eq!(
        repo.find_user_by_legacy_id("auth0|abc")
            .await
            .unwrap()
            .unwrap()
            .id,
        a0
    );
    assert_eq!(
        repo.find_user_by_legacy_id("local:bob")
            .await
            .unwrap()
            .unwrap()
            .id,
        loc
    );
    assert_eq!(
        repo.find_user_by_legacy_id("bot:reviewer")
            .await
            .unwrap()
            .unwrap()
            .id,
        bot
    );
    assert!(repo.find_user_by_legacy_id("nope").await.unwrap().is_none());

    // The provider id is preserved on the user row.
    assert_eq!(
        repo.get_user(&gh)
            .await
            .unwrap()
            .unwrap()
            .provider_user_id
            .as_deref(),
        Some("609489")
    );
}

/// The FK columns (`team_members.user_id`, `user_credentials.user_id`) are
/// native `uuid` referencing `users.id`; rows resolve through the DAOs and the
/// FK rejects an unknown user.
#[sqlx::test]
async fn native_uuid_foreign_keys_resolve_and_enforce(pool: PgPool) {
    let users = PgUserDao(pool.clone());
    let teams = PgTeamDao(pool.clone());
    let creds = PgUserCredentialDao(pool.clone());

    let uid = Uuid::new_v4();
    let uid_s = uid.to_string();
    users
        .upsert_user(&make_user(uid, "carol", "github", "12345"))
        .await
        .unwrap();

    // Personal team; membership rides the native-uuid FK to users.
    sqlx::query(
        "INSERT INTO teams (id, name, slug) VALUES ($1, $2, $1) ON CONFLICT (id) DO NOTHING",
    )
    .bind(format!("personal-{uid_s}"))
    .bind(format!("{uid_s} Personal"))
    .execute(&pool)
    .await
    .unwrap();
    teams
        .add_member(&format!("personal-{uid_s}"), &uid, "owner")
        .await
        .unwrap();

    assert!(teams
        .is_member(&format!("personal-{uid_s}"), &uid)
        .await
        .unwrap());
    assert_eq!(
        teams.find_team_ids_for_user(&uid).await.unwrap(),
        vec![format!("personal-{uid_s}")]
    );

    // user_credentials.user_id is also a native-uuid FK to users.
    creds
        .upsert(&uid, "carol", "$argon2id$hash", false)
        .await
        .unwrap();
    assert_eq!(
        creds.find_by_user_id(&uid).await.unwrap().unwrap().user_id,
        uid
    );

    // The FK is load-bearing: a membership for an unknown user is rejected.
    let orphan = Uuid::new_v4();
    let err =
        sqlx::query("INSERT INTO team_members (team_id, user_id, role) VALUES ($1, $2, 'member')")
            .bind(format!("personal-{uid_s}"))
            .bind(orphan)
            .execute(&pool)
            .await;
    assert!(
        err.is_err(),
        "team_members.user_id FK must reject an unknown user"
    );
}
