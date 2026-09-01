#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! `DELETE /api/v1/adapters/connections/{id}`.
//!
//! The route exists so a test run can remove the connections it created; before
//! it, a shared environment accumulated one per trigger fixture forever. What
//! matters is that it removes only what the caller controls, and that a
//! connection they cannot control is refused the same way as one that is not
//! there — a delete must not confirm existence the read would have hidden.

use moatless_types::adapter::AdapterKind;
use crate::common;

use chrono::Utc;
use tower::ServiceExt;

use moatless_types::adapter::AdapterConnection;
use moatless_backend::adapters::routes::adapter_router;
use moatless_types::access_control::Scope;
use moatless_backend::state::AppState;

use common::{authed_request, test_app_state, user_scope, USER_1, USER_2};

/// Write an `adapter_connections` row owned by `user_id`.
async fn seed_connection(state: &AppState, id: &str, user_id: &str) -> String {
    let now = Utc::now();
    let row = AdapterConnection {
        id: id.to_string(),
        adapter_kind: AdapterKind::Webhook,
        connection_kind: "webhook".to_string(),
        scope_value: format!("acct-{id}"),
        credential_ref: None,
        webhook_secret: None,
        transport_config: serde_json::json!({}),
        created_by: user_id.to_string(),
        scope: Scope::User,
        owner_user_id: Some(user_id.to_string()),
        created_at: now,
        updated_at: now,
    };
    state
        .adapter_connections
        .upsert(&row, &user_scope(user_id))
        .await
        .expect("upsert adapter connection")
        .id
}

#[sqlx::test]
async fn owner_deletes_their_connection_once(pool: sqlx::PgPool) {
    let state = test_app_state(pool);
    let id = seed_connection(&state, "pc_delete_me", USER_1).await;
    let (router, _) = adapter_router(state.clone());

    let response = router
        .clone()
        .oneshot(authed_request(
            "DELETE",
            &format!("/api/v1/adapters/connections/{id}"),
            USER_1,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 204);

    assert!(
        state
            .adapter_connections
            .find_by_id(&id, &user_scope(USER_1))
            .await
            .unwrap()
            .is_none(),
        "the connection is still there after a 204"
    );

    // Deleting it again is a delete of something that is not there.
    let response = router
        .oneshot(authed_request(
            "DELETE",
            &format!("/api/v1/adapters/connections/{id}"),
            USER_1,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[sqlx::test]
async fn another_users_connection_is_not_deletable(pool: sqlx::PgPool) {
    let state = test_app_state(pool);
    let id = seed_connection(&state, "pc_not_yours", USER_1).await;
    let (router, _) = adapter_router(state.clone());

    let response = router
        .oneshot(authed_request(
            "DELETE",
            &format!("/api/v1/adapters/connections/{id}"),
            USER_2,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        404,
        "a connection somebody else owns must read as absent, not as forbidden"
    );

    assert!(
        state
            .adapter_connections
            .find_by_id(&id, &user_scope(USER_1))
            .await
            .unwrap()
            .is_some(),
        "the owner's connection was removed by somebody who does not control it"
    );
}
