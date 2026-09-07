#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration fixtures fail on invalid setup"
)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use chrono::Utc;
use moatless_backend::compose::compose;
use moatless_identity::auth::{
    api_key,
    password::{
        dao::{PgUserCredentialDao, UserCredentialDao},
        hash,
    },
    users::{PgUserDao, User, UserDao},
};
use moatless_types::{auth::AuthProvider, user::UserRole};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "initial-test-password";

fn router(pool: &PgPool) -> Router {
    let mut config = crate::common::test_config();
    config.auth.provider = AuthProvider::Local;
    compose(pool.clone(), &config)
        .build_authentication(Arc::new(config))
        .unwrap()
        .routes()
        .0
}

async fn seed_user(pool: &PgPool, login: &str, role: UserRole) -> Uuid {
    let id = Uuid::new_v4();
    let now = Utc::now();
    PgUserDao(pool.clone())
        .upsert_user(&User {
            id,
            login: login.into(),
            email: Some(format!("{login}@example.com")),
            name: None,
            avatar_url: None,
            provider: "local".into(),
            provider_user_id: None,
            role,
            created_at: now,
            last_login_at: now,
            settings: json!({}),
            is_bot: false,
        })
        .await
        .unwrap();
    let encoded = hash::hash_password(PASSWORD.into()).await.unwrap();
    PgUserCredentialDao(pool.clone())
        .upsert(&id, login, &encoded, false)
        .await
        .unwrap();
    id
}

async fn request(
    router: &Router,
    method: &str,
    path: &str,
    body: Value,
    cookie: Option<&str>,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-forwarded-for", "192.0.2.1");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    router
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn login(router: &Router, name: &str, password: &str) -> Response {
    request(
        router,
        "POST",
        "/api/v1/auth/password/login",
        json!({"email": format!("{name}@example.com"), "password": password}),
        None,
    )
    .await
}

fn session_cookie(response: &Response) -> String {
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ")
}

async fn active_sessions(pool: &PgPool, user: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn failed_login_is_indistinguishable_and_throttling_precedes_password_work(pool: PgPool) {
    let user = seed_user(&pool, "alice", UserRole::User).await;
    let app = router(&pool);
    let unknown = login(&app, "missing", PASSWORD).await;
    let wrong = login(&app, "alice", "incorrect-password").await;
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        axum::body::to_bytes(unknown.into_body(), 4096)
            .await
            .unwrap(),
        axum::body::to_bytes(wrong.into_body(), 4096).await.unwrap()
    );
    let throttle = moatless_identity::auth::password::throttle::LoginThrottle::new(pool.clone());
    for _ in 0..10 {
        throttle
            .record_failure("password_email", "alice@example.com")
            .await
            .unwrap();
    }
    assert_eq!(
        login(&app, "alice", PASSWORD).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(active_sessions(&pool, user).await, 0);
    assert_eq!(
        PgUserCredentialDao(pool.clone())
            .find_by_user_id(&user)
            .await
            .unwrap()
            .unwrap()
            .failed_attempts,
        1
    );
}

#[sqlx::test]
async fn password_change_requires_cookie_and_current_password_then_revokes_every_session(
    pool: PgPool,
) {
    let user = seed_user(&pool, "alice", UserRole::User).await;
    let app = router(&pool);
    let first = session_cookie(&login(&app, "alice", PASSWORD).await);
    let _second = session_cookie(&login(&app, "alice", PASSWORD).await);
    assert_eq!(active_sessions(&pool, user).await, 2);
    let change = json!({"currentPassword": PASSWORD, "newPassword": "replacement-test-password"});
    let keys = moatless_identity::auth::api_keys::PgApiKeyDao(pool.clone());
    let key = api_key::generate_key(&keys, &user.to_string(), "test", None, None, None)
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/auth/password")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", key.key))
                .body(Body::from(change.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        request(
            &app,
            "PUT",
            "/api/v1/auth/password",
            json!({"currentPassword": "wrong", "newPassword": "replacement-test-password"}),
            Some(&first)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(active_sessions(&pool, user).await, 2);
    assert_eq!(
        request(&app, "PUT", "/api/v1/auth/password", change, Some(&first))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(active_sessions(&pool, user).await, 0);
    assert_eq!(
        login(&app, "alice", "replacement-test-password")
            .await
            .status(),
        StatusCode::OK
    );
}

#[sqlx::test]
async fn admin_reset_revokes_target_sessions_and_preserves_must_change(pool: PgPool) {
    let user = seed_user(&pool, "alice", UserRole::User).await;
    seed_user(&pool, "admin", UserRole::Admin).await;
    let app = router(&pool);
    let user_cookie = session_cookie(&login(&app, "alice", PASSWORD).await);
    let admin_cookie = session_cookie(&login(&app, "admin", PASSWORD).await);
    let reset = json!({"newPassword": "administrator-reset-password", "mustChangePassword": true});
    assert_eq!(
        request(
            &app,
            "PUT",
            "/api/v1/users/alice/password",
            reset.clone(),
            Some(&user_cookie)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(active_sessions(&pool, user).await, 1);
    assert_eq!(
        request(
            &app,
            "PUT",
            "/api/v1/users/alice/password",
            reset,
            Some(&admin_cookie)
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(active_sessions(&pool, user).await, 0);
    let credential = PgUserCredentialDao(pool.clone())
        .find_by_user_id(&user)
        .await
        .unwrap()
        .unwrap();
    assert!(credential.must_change);
    assert!(
        hash::verify_password(
            "administrator-reset-password".into(),
            credential.password_hash
        )
        .await
        .unwrap()
    );
}

#[sqlx::test]
async fn racing_account_creation_commits_one_complete_account(pool: PgPool) {
    seed_user(&pool, "admin", UserRole::Admin).await;
    let app = router(&pool);
    let cookie = session_cookie(&login(&app, "admin", PASSWORD).await);
    let input = json!({"isBot": false, "login": "new-user", "email": "new@example.com", "password": PASSWORD});
    let (first, second) = tokio::join!(
        request(&app, "POST", "/api/v1/users", input.clone(), Some(&cookie)),
        request(&app, "POST", "/api/v1/users", input, Some(&cookie))
    );
    let mut statuses = [first.status().as_u16(), second.status().as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 409]);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users u JOIN user_credentials c ON c.user_id = u.id JOIN user_provider_ids p ON p.user_id = u.id WHERE u.email_normalized = 'new@example.com'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn oauth_rejects_missing_and_mismatched_state_before_provider_exchange(pool: PgPool) {
    let app = router(&pool);
    for cookie in [None, Some("oauth_state=different")] {
        let response = request(
            &app,
            "GET",
            "/api/v1/auth/github-app/callback?code=unused&state=expected",
            Value::Null,
            cookie,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
