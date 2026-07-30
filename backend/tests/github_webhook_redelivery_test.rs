#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! The GitHub Adapter has no in-process delivery cache; durable admission is
//! the only thing that collapses a redelivery.
//!
//! Both properties here are ones the deleted cache used to hide. A "replica" is
//! its own `AppState` + `GithubPrAdapter` over the shared pool — which is what
//! a second backend process is, and what the per-process cache could never see.
//!
//! - A redelivery landing on a *different* replica must do no second effect.
//!   The cache made this untestable: it answered `200 duplicate` only on the
//!   replica that saw the first delivery.
//! - The work the ingress path does *before* admitting now re-runs on every
//!   redelivery. It has to be harmless, and that is asserted rather than
//!   assumed: the PR status and review decision writes derive their value from
//!   the delivery payload, so a replay writes the same value again.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderMap;
use moatless_vibe_backend::adapters::delivery::{
    spawn_adapter_delivery_worker, AdapterDeliveryHandlers,
};
use moatless_vibe_backend::adapters::github_pr::adapter::{
    resolve_and_process_github_webhook, GithubPrAdapter, GithubWebhookOutcome,
};
use moatless_vibe_backend::adapters::github_pr::delivery::kind;
use moatless_vibe_backend::auth::github::registry::GitHubAppRegistry;
use moatless_vibe_backend::auth::provider_token_dao::PgProviderTokenDao;
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::state::AppState;
use moatless_vibe_backend::task::dao as task_dao;
use sqlx::{PgPool, Row};
use tokio_util::sync::CancellationToken;

use common::{make_repository_row, make_task_row, seed_team, test_app_state, test_config};

const CONNECTION_ID: &str = "conn-github-redelivery";
const OWNER: &str = "acme";
const REPO_FULL_NAME: &str = "acme/widgets";
const SUBJECT_KEY: &str = "acme/widgets#42";
/// The task's owner. Deliberately not the connection's user: an event whose
/// sender resolves to the task owner is filtered as a self-echo, which would
/// hide the delivery this test is counting.
const TASK_USER: &str = "user-task-owner";
const CONNECTION_USER: &str = "user-connection-owner";

// =========================================================================
// Fixtures
// =========================================================================

/// One backend process: its own state and its own adapter instance, sharing
/// only the database. Two of these is the whole point of these tests.
fn replica(pool: &PgPool) -> (AppState, GithubPrAdapter) {
    let state = test_app_state(pool.clone());
    let adapter = GithubPrAdapter::new(
        None,
        None,
        "https://app.example.com".to_string(),
        pool.clone(),
        Arc::new(PgProviderTokenDao(pool.clone())),
        None,
        None,
        Arc::new(GitHubAppRegistry::new(
            moatless_vibe_backend::auth::github::service::GitHubAppService::from_config(
                &test_config(),
            )
            .map(Arc::new),
            None,
            None,
        )),
    );
    (state, adapter)
}

async fn seed_fixtures(pool: &PgPool, task_id: &str) {
    seed_team(pool, &format!("personal-{TASK_USER}")).await;
    repository_dao::insert(pool, &make_repository_row("repo-1", TASK_USER))
        .await
        .expect("seed repository");
    task_dao::insert_task(pool, &make_task_row(task_id, TASK_USER))
        .await
        .expect("seed task");

    sqlx::query(
        r#"INSERT INTO integration_connections
               (id, adapter_kind, connection_kind, external_account_id, created_by,
                scope, owner_user_id)
           VALUES ($1, 'github', 'org', $2, $3, 'user', $3)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(CONNECTION_ID)
    .bind(OWNER)
    .bind(CONNECTION_USER)
    .execute(pool)
    .await
    .expect("seed integration connection");

    // The PR the events name, already connected to the task — so the ingress
    // path has a binding to read and a status column to write.
    sqlx::query(
        r#"INSERT INTO task_bindings
               (id, task_id, adapter_kind, subject_kind, subject_key, connection_id,
                destination_config, origin)
           VALUES ('binding-1', $1, 'github', 'github_pr', $2, $3, $4, 'connected_thread')
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(task_id)
    .bind(SUBJECT_KEY)
    .bind(CONNECTION_ID)
    .bind(serde_json::json!({"owner": OWNER, "repo": "widgets", "pr_number": 42}))
    .execute(pool)
    .await
    .expect("seed task binding");
}

// =========================================================================
// Delivery
// =========================================================================

fn headers(event_type: &str, delivery_id: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-github-event", event_type.parse().unwrap());
    headers.insert("x-github-delivery", delivery_id.parse().unwrap());
    headers
}

/// Deliver one webhook to one replica, exactly as the HTTP handler does.
async fn deliver(
    state: &AppState,
    adapter: &GithubPrAdapter,
    event_type: &str,
    delivery_id: &str,
    payload: &serde_json::Value,
) -> GithubWebhookOutcome {
    let body = serde_json::to_vec(payload).unwrap();
    resolve_and_process_github_webhook(state, adapter, &headers(event_type, delivery_id), &body)
        .await
        .expect("webhook accepted")
}

fn issue_comment_payload(comment_id: u64, sender: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "created",
        "repository": {
            "full_name": REPO_FULL_NAME,
            "owner": { "login": OWNER },
        },
        "sender": { "login": sender },
        "issue": {
            "number": 42,
            "pull_request": { "url": "https://api.github.com/repos/acme/widgets/pulls/42" },
        },
        "comment": {
            "id": comment_id,
            "body": "please take another look",
            "user": { "login": sender },
            "html_url": "https://github.com/acme/widgets/pull/42#issuecomment-1",
        },
    })
}

fn pull_request_payload(action: &str, state: &str, merged: bool) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "repository": {
            "full_name": REPO_FULL_NAME,
            "owner": { "login": OWNER },
        },
        "sender": { "login": "octocat" },
        "pull_request": {
            "number": 42,
            "title": "Add widgets",
            "body": "the description",
            "state": state,
            "merged": merged,
            "draft": false,
            "html_url": "https://github.com/acme/widgets/pull/42",
            "user": { "login": "octocat" },
            "head": { "ref": "feature", "sha": "abc123" },
            "base": { "ref": "main", "sha": "def456" },
        },
    })
}

fn review_payload(review_id: u64, review_state: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "submitted",
        "repository": {
            "full_name": REPO_FULL_NAME,
            "owner": { "login": OWNER },
        },
        "sender": { "login": "octocat" },
        "pull_request": { "number": 42, "title": "Add widgets" },
        "review": {
            "id": review_id,
            "state": review_state,
            "body": "looks good",
            "user": { "login": "octocat" },
            "html_url": "https://github.com/acme/widgets/pull/42#pullrequestreview-1",
        },
    })
}

// =========================================================================
// Reads
// =========================================================================

async fn count_records(pool: &PgPool, kind: &str, event_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery WHERE kind = $1 AND event_id = $2")
        .bind(kind)
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("count adapter_delivery")
}

async fn status_of(pool: &PgPool, kind: &str, event_id: &str) -> Option<(String, i32)> {
    sqlx::query("SELECT status, attempts FROM adapter_delivery WHERE kind = $1 AND event_id = $2")
        .bind(kind)
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .expect("read adapter_delivery")
        .map(|row| (row.get("status"), row.get("attempts")))
}

/// The value the ingress path derived from the delivery payload and wrote onto
/// the binding — everything except the refresh timestamp, which moves on every
/// write by design.
type PersistedPrState = (
    Option<String>,
    Option<String>,
    Option<serde_json::Value>,
    Option<String>,
);

async fn persisted_pr_state(pool: &PgPool) -> PersistedPrState {
    let row = sqlx::query(
        r#"SELECT external_status, external_state_kind, external_status_raw,
                  external_review_decision
             FROM task_bindings WHERE id = 'binding-1'"#,
    )
    .fetch_one(pool)
    .await
    .expect("read binding");
    (
        row.get("external_status"),
        row.get("external_state_kind"),
        row.get("external_status_raw"),
        row.get("external_review_decision"),
    )
}

/// Wait for a spawned best-effort write to land. The ingress path persists PR
/// status and review decisions off the response path, so there is no handle to
/// await — polling is the only way to observe them.
async fn wait_for(pool: &PgPool, ready: impl Fn(&PersistedPrState) -> bool) -> PersistedPrState {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = persisted_pr_state(pool).await;
        if ready(&state) {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "spawned binding write never landed: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn clear_persisted_pr_state(pool: &PgPool) {
    sqlx::query(
        r#"UPDATE task_bindings
              SET external_status = NULL, external_state_kind = NULL,
                  external_status_raw = NULL, external_review_decision = NULL
            WHERE id = 'binding-1'"#,
    )
    .execute(pool)
    .await
    .expect("clear binding state");
}

async fn message_count(pool: &PgPool, task_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM task_messages WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .expect("count task messages")
}

/// Run the worker until `done` holds or the budget elapses, then cancel it.
async fn run_worker_until(state: &AppState, budget: Duration, done: impl AsyncFn() -> bool) {
    let mut handlers = AdapterDeliveryHandlers::new();
    moatless_vibe_backend::adapters::github_pr::delivery::register(&mut handlers, state);
    let cancel = CancellationToken::new();
    let handle = spawn_adapter_delivery_worker(
        state.adapter_delivery.clone(),
        Arc::new(handlers),
        cancel.clone(),
    );
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        if done().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    cancel.cancel();
    handle.await.expect("worker exits cleanly on cancellation");
}

// =========================================================================
// Tests
// =========================================================================

/// Verification: a redelivery on a different replica does no second effect.
///
/// The first delivery is accepted by one replica; the redelivery — same body,
/// same `X-GitHub-Delivery` — is accepted by a second replica that has never
/// seen it. Nothing in that process could recognise it, and nothing needs to:
/// the durable record is the recognition.
#[sqlx::test]
async fn a_redelivery_on_a_different_replica_does_no_second_effect(pool: PgPool) {
    let task_id = "task-gh-redelivery";
    seed_fixtures(&pool, task_id).await;

    let payload = issue_comment_payload(9001, "octocat");
    let (state_a, adapter_a) = replica(&pool);
    let (state_b, adapter_b) = replica(&pool);

    let first = deliver(
        &state_a,
        &adapter_a,
        "issue_comment",
        "delivery-1",
        &payload,
    )
    .await;
    let redelivery = deliver(
        &state_b,
        &adapter_b,
        "issue_comment",
        "delivery-1",
        &payload,
    )
    .await;

    // Neither is dismissed at the door. The second one is processed in full and
    // simply finds the work already admitted.
    assert!(
        matches!(first, GithubWebhookOutcome::Processed { .. }),
        "{first:?}"
    );
    assert!(
        matches!(redelivery, GithubWebhookOutcome::Processed { .. }),
        "the replica that never saw the first delivery still processes this one: {redelivery:?}"
    );

    assert_eq!(
        count_records(&pool, kind::PR_MESSAGE, "delivery-1").await,
        1,
        "two replicas, one delivery id, one unit of work"
    );
    // The same delivery also derives a merge-conflict check. Different kind,
    // so it is its own record — and it, too, is admitted exactly once.
    assert_eq!(
        count_records(&pool, kind::MERGE_CONFLICT, "delivery-1").await,
        1
    );

    // And the effect really happened once: the work executes on its first
    // attempt and the bound task hears the comment a single time.
    let pool_for_check = pool.clone();
    run_worker_until(&state_a, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, kind::PR_MESSAGE, "delivery-1").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;

    let (status, attempts) = status_of(&pool, kind::PR_MESSAGE, "delivery-1")
        .await
        .expect("record");
    assert_eq!(status, "succeeded");
    assert_eq!(attempts, 1, "the redelivery cost no second execution");
    assert_eq!(
        message_count(&pool, task_id).await,
        1,
        "one comment, one message on the task"
    );
}

/// Verification: the work that runs before admission is safe to repeat.
///
/// This is what the deleted cache was covering, and the reason deleting it is a
/// performance question rather than a correctness one. Both writes derive their
/// value from the delivery payload, so the replay recomputes the same value.
/// The columns are cleared between the two deliveries precisely so the second
/// write has to be observed doing the work rather than merely not undoing it.
///
/// The PR-status half deliberately redelivers to the *same* replica — the one
/// case the deleted gate used to answer without running anything, so this half
/// fails against the cached implementation rather than merely passing on both.
#[sqlx::test]
async fn replaying_the_pre_admission_writes_lands_on_the_same_value(pool: PgPool) {
    let task_id = "task-gh-replay";
    seed_fixtures(&pool, task_id).await;

    let (state_a, adapter_a) = replica(&pool);
    let (state_b, adapter_b) = replica(&pool);

    // 1. PR status, derived from `pull_request.{state,merged,draft}`.
    let opened = pull_request_payload("opened", "open", false);
    deliver(&state_a, &adapter_a, "pull_request", "delivery-pr", &opened).await;
    let after_first = wait_for(&pool, |s| s.0.is_some()).await;
    assert_eq!(after_first.0.as_deref(), Some("open"));

    clear_persisted_pr_state(&pool).await;
    let redelivery = deliver(&state_a, &adapter_a, "pull_request", "delivery-pr", &opened).await;
    assert!(
        matches!(redelivery, GithubWebhookOutcome::Processed { .. }),
        "a redelivery is processed in full, not dismissed at the door: {redelivery:?}"
    );
    let after_redelivery = wait_for(&pool, |s| s.0.is_some()).await;
    assert_eq!(
        after_redelivery, after_first,
        "the same payload derives the same PR status on a replay"
    );

    // The same value again, this time from a replica that never saw the first.
    clear_persisted_pr_state(&pool).await;
    deliver(&state_b, &adapter_b, "pull_request", "delivery-pr", &opened).await;
    let after_other_replica = wait_for(&pool, |s| s.0.is_some()).await;
    assert_eq!(after_other_replica, after_first);

    // 2. Review decision, derived from `review.state`.
    let approved = review_payload(555, "approved");
    deliver(
        &state_a,
        &adapter_a,
        "pull_request_review",
        "delivery-review",
        &approved,
    )
    .await;
    let after_first = wait_for(&pool, |s| s.3.is_some()).await;
    assert_eq!(after_first.3.as_deref(), Some("approved"));

    clear_persisted_pr_state(&pool).await;
    deliver(
        &state_b,
        &adapter_b,
        "pull_request_review",
        "delivery-review",
        &approved,
    )
    .await;
    let after_redelivery = wait_for(&pool, |s| s.3.is_some()).await;
    assert_eq!(
        after_redelivery.3, after_first.3,
        "the same payload derives the same review decision on any replica"
    );

    // A non-verdict review never clobbers the recorded approval, so replaying
    // one is harmless too.
    deliver(
        &state_b,
        &adapter_b,
        "pull_request_review",
        "delivery-review-commented",
        &review_payload(556, "commented"),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        persisted_pr_state(&pool).await.3.as_deref(),
        Some("approved"),
        "a `commented` review is skipped, not persisted over the verdict"
    );
}
