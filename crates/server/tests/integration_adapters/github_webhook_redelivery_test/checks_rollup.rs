//! The checks rollup a pull request webhook keeps or clears.

use super::*;

/// A pull request webhook carries no checks, so its write keeps the rollup the
/// last live read stored; only a new head, which has none of the old head's
/// checks, clears it.
#[sqlx::test]
async fn a_pull_request_webhook_keeps_the_checks_rollup_until_a_new_head(pool: PgPool) {
    let (base_url, _requests) = request_counting_github_stub().await;
    seed_fixtures(&pool, "task-checks-rollup").await;
    sqlx::query(
        r#"UPDATE task_bindings
              SET external_status_raw = '{"title": "Old title", "checksState": "failing"}'
            WHERE id = 'binding-1'"#,
    )
    .execute(&pool)
    .await
    .expect("seed the live read's blob");
    let state = stubbed_replica(&pool, &base_url).await;

    let edited = webhook_response(
        &state,
        headers("pull_request", "delivery-checks-edited"),
        serde_json::to_vec(&pull_request_payload("edited", "open", false)).unwrap(),
    )
    .await;
    assert_eq!(edited.status(), StatusCode::OK);
    let (_, _, raw, _) = wait_for(&pool, |state| {
        state
            .2
            .as_ref()
            .is_some_and(|raw| raw["title"] == "Add widgets")
    })
    .await;
    assert_eq!(raw.expect("blob")["checksState"], "failing");

    let pushed = webhook_response(
        &state,
        headers("pull_request", "delivery-checks-synchronize"),
        serde_json::to_vec(&pull_request_payload("synchronize", "open", false)).unwrap(),
    )
    .await;
    assert_eq!(pushed.status(), StatusCode::OK);
    wait_for(&pool, |state| {
        state
            .2
            .as_ref()
            .is_some_and(|raw| raw.get("checksState") == Some(&serde_json::Value::Null))
    })
    .await;
}
