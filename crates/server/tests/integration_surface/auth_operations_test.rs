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
use moatless_backend::compose::compose;
use moatless_identity::auth::password::hash;
use moatless_types::{auth::AuthProvider, user::UserRole};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "initial-test-password";

fn router(pool: &PgPool) -> Router {
    let mut config = crate::common::test_config();
    config.auth.provider = AuthProvider::Local;
    compose(pool.clone(), &config, None)
        .build_authentication(Arc::new(config))
        .unwrap()
        .routes()
        .0
}

async fn seed_user(pool: &PgPool, login: &str, role: UserRole) -> Uuid {
    moatless_identity::compose::seed_local_account(pool.clone(), login, role, PASSWORD).await
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

/// Add a second row carrying `login`, an hour newer than the one already there.
///
/// A login is unique to a provider, not to the table: signing in to a second
/// provider under the same handle leaves two rows.
async fn seed_newer_duplicate(pool: &PgPool, login: &str, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, login, email, provider, role, created_at, last_login_at, is_bot, status)
         VALUES ($1, $2, $3, 'github_app', 'user', now() + interval '1 hour', now(), false, 'active')",
    )
    .bind(id)
    .bind(login)
    .bind(email)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn user_status(pool: &PgPool, user: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn body_json(response: Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
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
        moatless_identity::compose::local_credential(pool.clone(), &user)
            .await
            .unwrap()
            .failed_attempts,
        1
    );
}

#[sqlx::test]
async fn owner_built_sessions_are_shared_by_login_middleware_and_logout(pool: PgPool) {
    let user = seed_user(&pool, "session-owner", UserRole::User).await;
    let mut config = crate::common::test_config();
    config.auth.provider = AuthProvider::Local;
    let origin = config.auth.frontend_url.clone();
    let auth = compose(pool.clone(), &config, None)
        .build_authentication(Arc::new(config))
        .unwrap();
    let app = auth
        .routes()
        .0
        .route(
            "/session-probe",
            axum::routing::get(|| async { StatusCode::OK }),
        )
        .layer(axum::middleware::from_fn(
            moatless_identity::auth::middleware::auth_middleware,
        ))
        .layer(axum::Extension(auth.layer()));

    let cookie = session_cookie(&login(&app, "session-owner", PASSWORD).await);
    assert_eq!(active_sessions(&pool, user).await, 1);
    assert_eq!(
        request(&app, "GET", "/session-probe", Value::Null, Some(&cookie))
            .await
            .status(),
        StatusCode::OK
    );
    let csrf = cookie
        .split("; ")
        .find_map(|part| part.strip_prefix("moatless_csrf="))
        .unwrap();
    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header("cookie", &cookie)
                .header("x-csrf-token", csrf)
                .header("origin", origin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::OK);
    assert_eq!(active_sessions(&pool, user).await, 0);
    assert_eq!(
        request(&app, "GET", "/session-probe", Value::Null, Some(&cookie))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
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
    let key =
        moatless_identity::compose::issue_api_key(pool.clone(), &user.to_string(), "test").await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/auth/password")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {key}"))
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
    let credential = moatless_identity::compose::local_credential(pool.clone(), &user)
        .await
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

/// `PATCH /api/v1/users/{login}` answers with the oldest row carrying a login,
/// so a superseded duplicate is unreachable there. The id-keyed admin route is
/// the handle on every other row.
#[sqlx::test]
async fn id_keyed_update_reaches_the_duplicate_a_shared_login_hides(pool: PgPool) {
    let original = seed_user(&pool, "alice", UserRole::User).await;
    let duplicate = seed_newer_duplicate(&pool, "alice", "alice@github.example.com").await;
    seed_user(&pool, "admin", UserRole::Admin).await;
    let app = router(&pool);
    let cookie = session_cookie(&login(&app, "admin", PASSWORD).await);

    let by_login = request(
        &app,
        "PATCH",
        "/api/v1/users/alice",
        json!({"status": "disabled"}),
        Some(&cookie),
    )
    .await;
    assert_eq!(by_login.status(), StatusCode::OK);
    assert_eq!(body_json(by_login).await["id"], original.to_string());
    assert_eq!(user_status(&pool, duplicate).await, "active");

    let by_id = request(
        &app,
        "PATCH",
        &format!("/api/v1/admin/users/{duplicate}"),
        json!({"status": "disabled"}),
        Some(&cookie),
    )
    .await;
    assert_eq!(by_id.status(), StatusCode::OK);
    assert_eq!(body_json(by_id).await["id"], duplicate.to_string());
    assert_eq!(user_status(&pool, duplicate).await, "disabled");
}

/// The id-keyed route is admin-only, and it keeps the guard that stops an admin
/// disabling the account they are signed in as.
#[sqlx::test]
async fn id_keyed_update_refuses_a_non_admin_and_a_self_disable(pool: PgPool) {
    let alice = seed_user(&pool, "alice", UserRole::User).await;
    let admin = seed_user(&pool, "admin", UserRole::Admin).await;
    let app = router(&pool);
    let user_cookie = session_cookie(&login(&app, "alice", PASSWORD).await);
    let admin_cookie = session_cookie(&login(&app, "admin", PASSWORD).await);

    // A non-admin is refused even on the row they own.
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("/api/v1/admin/users/{alice}"),
            json!({"name": "self-service"}),
            Some(&user_cookie)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("/api/v1/admin/users/{admin}"),
            json!({"status": "disabled"}),
            Some(&admin_cookie)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(user_status(&pool, admin).await, "active");
    assert_eq!(user_status(&pool, alice).await, "active");
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
