#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for ING-3: GitHub and Linear webhook work crossing the
//! durable adapter-work handoff before the provider is acknowledged.
//!
//! These exercise the real registered handlers against a real pool, because the
//! properties under test are exactly the ones a unit test cannot see: that a
//! record admitted by a process which then disappears is executed by the next
//! one, that a redelivery produces no second unit of work, and that a failure
//! no retry can fix stops in a state an operator can find.
//!
//! Payloads are written as literal JSON rather than through the (crate-private)
//! payload structs on purpose: that is the shape a record left behind by an
//! earlier build actually has, and it is what a handler must still be able to
//! read.

mod common;

use std::sync::Arc;
use std::time::Duration;

use moatless_vibe_backend::adapters::github_pr;
use moatless_vibe_backend::adapters::linear;
use moatless_vibe_backend::adapters::work::{
    spawn_adapter_work_worker, AdapterWorkHandlers, AdapterWorkRequest, Admission,
};
use moatless_vibe_backend::state::AppState;
use sqlx::PgPool;
use sqlx::Row;
use tokio_util::sync::CancellationToken;

use common::test_app_state;

const CONNECTION_ID: &str = "conn-github-1";
const LINEAR_CONNECTION_ID: &str = "conn-linear-1";

/// One `integration_connections` row — the only fixture every handler needs to
/// get past its "does this tenant still exist?" guard.
async fn seed_connection(pool: &PgPool, id: &str, adapter_kind: &str, account: &str) {
    sqlx::query(
        r#"INSERT INTO integration_connections
               (id, adapter_kind, connection_kind, external_account_id, created_by, scope)
           VALUES ($1, $2, 'org', $3, 'user-1', 'user')
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(adapter_kind)
    .bind(account)
    .execute(pool)
    .await
    .expect("seed integration connection");
}

fn github_context(connection_id: &str) -> serde_json::Value {
    serde_json::json!({
        "connection_id": connection_id,
        // No trigger: the repo has a connection but no task trigger, which is
        // the delivery-only shape every handler has to tolerate.
        "trigger_id": serde_json::Value::Null,
        "repo_full_name": "acme/widgets",
    })
}

/// One request per kind this ticket admits. Every one of them has to survive
/// the crash test, so they are enumerated once and reused.
fn every_kind(connection_id: &str, linear_connection_id: &str) -> Vec<AdapterWorkRequest> {
    let mut pr_message = github_context(connection_id);
    pr_message["message"] = serde_json::json!({
        "provider": "github_pr",
        "source_id": "acme/widgets",
        "thread_id": "42",
        "message_thread_id": "42",
        "user_id": "user-1",
        "text": "a comment on the PR",
        "file_ids": [],
        "thread_properties": {"owner": "acme", "repo": "widgets", "pr_number": 42},
        "provider_context": {},
        // Deliberately not a `pr:opened:` id: this asserts recovery, not task
        // creation, and a created task would drag a repository fixture in.
        "external_event_id": "github:ic:9001",
        "source_task_id": serde_json::Value::Null,
    });

    let mut close = github_context(connection_id);
    close["close"] = serde_json::json!({
        "pr_number": 42,
        "merged": true,
        "title": "Add the widget",
        "html_url": "https://github.com/acme/widgets/pull/42",
        "sender_login": "octocat",
    });

    let mut post_merge = github_context(connection_id);
    post_merge["owner"] = serde_json::json!("acme");
    post_merge["repo"] = serde_json::json!("widgets");
    post_merge["sender_login"] = serde_json::json!("octocat");
    post_merge["run"] = serde_json::json!({
        "run_id": 7,
        "name": "CI",
        "conclusion": "success",
        "head_sha": "deadbeef",
        "head_branch": "main",
        "html_url": "https://github.com/acme/widgets/actions/runs/7",
    });

    let mut merge_conflict = github_context(connection_id);
    merge_conflict["owner"] = serde_json::json!("acme");
    merge_conflict["repo"] = serde_json::json!("widgets");
    merge_conflict["pr_number"] = serde_json::json!(42);
    merge_conflict["sender_login"] = serde_json::json!("octocat");

    vec![
        AdapterWorkRequest {
            provider: github_pr::work::PROVIDER,
            kind: github_pr::work::kind::PR_MESSAGE,
            event_id: "delivery-1".to_string(),
            payload: pr_message,
        },
        AdapterWorkRequest {
            provider: github_pr::work::PROVIDER,
            kind: github_pr::work::kind::CLOSE,
            event_id: "delivery-2".to_string(),
            payload: close,
        },
        AdapterWorkRequest {
            provider: github_pr::work::PROVIDER,
            kind: github_pr::work::kind::POST_MERGE,
            event_id: "delivery-3".to_string(),
            payload: post_merge,
        },
        AdapterWorkRequest {
            provider: github_pr::work::PROVIDER,
            kind: github_pr::work::kind::MERGE_CONFLICT,
            event_id: "delivery-4".to_string(),
            payload: merge_conflict,
        },
        AdapterWorkRequest {
            provider: linear::work::PROVIDER,
            kind: linear::work::SESSION_PROVISION,
            // Exactly the shape `handlers::dedup_key` produces.
            event_id: "AgentSessionEvent:created:session-A".to_string(),
            payload: serde_json::json!({
                "connection_id": linear_connection_id,
                "session": {
                    "id": "session-A",
                    "issue": {
                        "id": "issue-1",
                        "identifier": "ENG-7",
                        "title": "Fix the thing",
                        "teamId": "team-1",
                    },
                },
            }),
        },
    ]
}

/// The registry the running process builds, assembled the same way `main` does.
fn registered_handlers(state: &AppState) -> Arc<AdapterWorkHandlers> {
    let mut handlers = AdapterWorkHandlers::new();
    github_pr::work::register(&mut handlers, state);
    linear::work::register(&mut handlers, state);
    Arc::new(handlers)
}

/// Run the worker until `done` holds or `budget` elapses, then cancel it — the
/// same start/cancel path `main` uses.
async fn run_worker_until(
    state: &AppState,
    budget: Duration,
    done: impl AsyncFn() -> bool,
) -> bool {
    let cancel = CancellationToken::new();
    let handle = spawn_adapter_work_worker(
        state.adapter_work.clone(),
        registered_handlers(state),
        cancel.clone(),
    );
    let deadline = tokio::time::Instant::now() + budget;
    let mut finished = false;
    while tokio::time::Instant::now() < deadline {
        if done().await {
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    cancel.cancel();
    handle.await.expect("worker exits cleanly on cancellation");
    finished
}

async fn status_of(
    pool: &PgPool,
    kind: &str,
    event_id: &str,
) -> Option<(String, i32, Option<String>)> {
    sqlx::query(
        "SELECT status, attempts, last_error FROM adapter_work WHERE kind = $1 AND event_id = $2",
    )
    .bind(kind)
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .expect("read adapter_work")
    .map(|row| {
        (
            row.get::<String, _>("status"),
            row.get::<i32, _>("attempts"),
            row.get::<Option<String>, _>("last_error"),
        )
    })
}

async fn count_records(pool: &PgPool, kind: &str, event_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM adapter_work WHERE kind = $1 AND event_id = $2")
        .bind(kind)
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("count adapter_work")
}

/// Verification: crash/restart recovery for every admitted work kind.
///
/// The admitting state is dropped without ever running a worker — from the
/// queue's side that is indistinguishable from a process killed after it
/// acknowledged the provider. A different process then has to finish all five.
#[sqlx::test]
async fn every_admitted_kind_is_executed_by_the_process_that_comes_after_the_crash(pool: PgPool) {
    seed_connection(&pool, CONNECTION_ID, "github", "acme").await;
    seed_connection(&pool, LINEAR_CONNECTION_ID, "linear", "org-1").await;

    let requests = every_kind(CONNECTION_ID, LINEAR_CONNECTION_ID);
    let kinds: Vec<(&'static str, String)> = requests
        .iter()
        .map(|r| (r.kind, r.event_id.clone()))
        .collect();

    {
        let admitting = test_app_state(pool.clone());
        for request in requests {
            let kind = request.kind;
            assert_eq!(
                admitting
                    .adapter_work
                    .admit(request)
                    .await
                    .expect("admitted"),
                Admission::Admitted,
                "{kind} must be durably owned before the provider is acknowledged"
            );
        }
        // The process that acknowledged the provider is gone. Nothing it held
        // in memory survives; only the rows do.
    }

    let recovering = test_app_state(pool.clone());
    let pool_for_check = pool.clone();
    let kinds_for_check = kinds.clone();
    let finished = run_worker_until(&recovering, Duration::from_secs(30), async || {
        for (kind, event_id) in &kinds_for_check {
            match status_of(&pool_for_check, kind, event_id).await {
                Some((status, ..)) if status == "succeeded" || status == "failed" => {}
                _ => return false,
            }
        }
        true
    })
    .await;

    assert!(finished, "every admitted kind reached a terminal state");
    for (kind, event_id) in &kinds {
        let (status, attempts, last_error) = status_of(&pool, kind, event_id)
            .await
            .expect("the record is still there");
        assert_eq!(
            status, "succeeded",
            "{kind} did not complete after the restart: {last_error:?}"
        );
        assert_eq!(attempts, 1, "{kind} completed on its first attempt");
    }
}

/// Verification: duplicate GitHub delivery.
///
/// A redelivery reuses `X-GitHub-Delivery`, so it must not become a second unit
/// of work — while a *different* kind from the same delivery must still be its
/// own record, or one permanent failure would poison the others' retries.
#[sqlx::test]
async fn a_redelivered_github_webhook_produces_no_second_unit_of_work(pool: PgPool) {
    seed_connection(&pool, CONNECTION_ID, "github", "acme").await;
    let state = test_app_state(pool.clone());

    let mut message = github_context(CONNECTION_ID);
    message["message"] = serde_json::json!({
        "provider": "github_pr",
        "source_id": "acme/widgets",
        "thread_id": "42",
        "message_thread_id": "42",
        "user_id": "user-1",
        "text": "a comment on the PR",
        "file_ids": [],
        "thread_properties": {"owner": "acme", "repo": "widgets", "pr_number": 42},
        "provider_context": {},
        "external_event_id": "github:ic:9001",
        "source_task_id": serde_json::Value::Null,
    });
    let request = || AdapterWorkRequest {
        provider: github_pr::work::PROVIDER,
        kind: github_pr::work::kind::PR_MESSAGE,
        event_id: "delivery-dup".to_string(),
        payload: message.clone(),
    };

    assert_eq!(
        state.adapter_work.admit(request()).await.expect("first"),
        Admission::Admitted
    );
    assert_eq!(
        state
            .adapter_work
            .admit(request())
            .await
            .expect("redelivery"),
        Admission::Duplicate,
        "GitHub's own delivery id is the idempotency key"
    );
    assert_eq!(
        count_records(&pool, github_pr::work::kind::PR_MESSAGE, "delivery-dup").await,
        1
    );

    // The same delivery also triggers a merge-conflict check. That is a
    // different kind, so it is its own record under the same event id.
    let mut conflict = github_context(CONNECTION_ID);
    conflict["owner"] = serde_json::json!("acme");
    conflict["repo"] = serde_json::json!("widgets");
    conflict["pr_number"] = serde_json::json!(42);
    conflict["sender_login"] = serde_json::json!("octocat");
    assert_eq!(
        state
            .adapter_work
            .admit(AdapterWorkRequest {
                provider: github_pr::work::PROVIDER,
                kind: github_pr::work::kind::MERGE_CONFLICT,
                event_id: "delivery-dup".to_string(),
                payload: conflict,
            })
            .await
            .expect("conflict check"),
        Admission::Admitted,
        "one delivery's distinct work kinds must not dedup against each other"
    );

    // And the duplicate really produced no second effect: the work runs once.
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, github_pr::work::kind::PR_MESSAGE, "delivery-dup").await,
            Some((ref status, ..)) if status == "succeeded" || status == "failed"
        )
    })
    .await;

    let (status, attempts, last_error) =
        status_of(&pool, github_pr::work::kind::PR_MESSAGE, "delivery-dup")
            .await
            .expect("record");
    assert_eq!(status, "succeeded", "{last_error:?}");
    assert_eq!(attempts, 1, "the redelivery cost no extra execution");

    // A late redelivery still dedups against the finished record.
    assert_eq!(
        state
            .adapter_work
            .admit(request())
            .await
            .expect("late redelivery"),
        Admission::Duplicate
    );
}

/// Verification: duplicate Linear delivery.
///
/// The key is the dispatcher's existing dedup identity, so two distinct
/// sessions sharing Linear's stable `webhookId` stay distinct while a genuine
/// retry of one session collapses.
#[sqlx::test]
async fn a_redelivered_linear_session_produces_no_second_unit_of_work(pool: PgPool) {
    seed_connection(&pool, LINEAR_CONNECTION_ID, "linear", "org-1").await;
    let state = test_app_state(pool.clone());

    let session_request = |session_id: &str| AdapterWorkRequest {
        provider: linear::work::PROVIDER,
        kind: linear::work::SESSION_PROVISION,
        event_id: format!("AgentSessionEvent:created:{session_id}"),
        payload: serde_json::json!({
            "connection_id": LINEAR_CONNECTION_ID,
            "session": {
                "id": session_id,
                "issue": {"id": "issue-1", "identifier": "ENG-7", "teamId": "team-1"},
            },
        }),
    };

    assert_eq!(
        state
            .adapter_work
            .admit(session_request("session-A"))
            .await
            .expect("first"),
        Admission::Admitted
    );
    assert_eq!(
        state
            .adapter_work
            .admit(session_request("session-A"))
            .await
            .expect("retry"),
        Admission::Duplicate,
        "a Linear retry of one session is one unit of work"
    );
    assert_eq!(
        state
            .adapter_work
            .admit(session_request("session-B"))
            .await
            .expect("second session"),
        Admission::Admitted,
        "two sessions sharing a webhookId must not collapse into one"
    );

    assert_eq!(
        count_records(
            &pool,
            linear::work::SESSION_PROVISION,
            "AgentSessionEvent:created:session-A"
        )
        .await,
        1
    );
}

/// Verification: terminal handling of a permanent failure.
///
/// A record whose tenant no longer exists can never succeed. It must stop
/// immediately — not burn eight attempts — and it must say why, because an
/// abandoned webhook that looks like one that never arrived is the failure this
/// whole seam exists to make visible.
#[sqlx::test]
async fn work_that_can_never_succeed_stops_at_a_terminal_state(pool: PgPool) {
    // Deliberately no connection row: the tenant was deleted between the
    // acknowledgment and the execution.
    let state = test_app_state(pool.clone());

    let mut orphaned = github_context("conn-that-was-deleted");
    orphaned["owner"] = serde_json::json!("acme");
    orphaned["repo"] = serde_json::json!("widgets");
    orphaned["pr_number"] = serde_json::json!(42);
    orphaned["sender_login"] = serde_json::json!("octocat");
    state
        .adapter_work
        .admit(AdapterWorkRequest {
            provider: github_pr::work::PROVIDER,
            kind: github_pr::work::kind::MERGE_CONFLICT,
            event_id: "delivery-orphan".to_string(),
            payload: orphaned,
        })
        .await
        .expect("admitted");

    // A payload an older build wrote and this one cannot read is equally
    // hopeless, and must be terminal rather than retried forever.
    state
        .adapter_work
        .admit(AdapterWorkRequest {
            provider: linear::work::PROVIDER,
            kind: linear::work::SESSION_PROVISION,
            event_id: "AgentSessionEvent:created:unreadable".to_string(),
            payload: serde_json::json!({"connection_id": LINEAR_CONNECTION_ID}),
        })
        .await
        .expect("admitted");

    let cases = [
        (github_pr::work::kind::MERGE_CONFLICT, "delivery-orphan"),
        (
            linear::work::SESSION_PROVISION,
            "AgentSessionEvent:created:unreadable",
        ),
    ];
    let pool_for_check = pool.clone();
    let finished = run_worker_until(&state, Duration::from_secs(30), async || {
        for (kind, event_id) in cases {
            match status_of(&pool_for_check, kind, event_id).await {
                Some((status, ..)) if status == "failed" => {}
                _ => return false,
            }
        }
        true
    })
    .await;
    assert!(finished, "both records reached a terminal state");

    let (status, attempts, last_error) = status_of(
        &pool,
        github_pr::work::kind::MERGE_CONFLICT,
        "delivery-orphan",
    )
    .await
    .expect("record");
    assert_eq!(status, "failed");
    assert_eq!(
        attempts, 1,
        "a permanent failure spends no budget on retrying"
    );
    let last_error = last_error.expect("a terminal record says why it was abandoned");
    assert!(
        last_error.contains("conn-that-was-deleted"),
        "the cause names what was missing: {last_error}"
    );

    let (status, attempts, last_error) = status_of(
        &pool,
        linear::work::SESSION_PROVISION,
        "AgentSessionEvent:created:unreadable",
    )
    .await
    .expect("record");
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1);
    assert!(
        last_error
            .unwrap_or_default()
            .contains("payload does not match its kind"),
        "an unreadable payload is terminal, not a retry loop"
    );
}
