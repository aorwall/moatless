#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for ING-3: GitHub and Linear webhook work crossing the
//! durable adapter-delivery handoff before the provider is acknowledged.
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

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use moatless_backend::adapters::delivery::spawn_adapter_delivery_worker;
use moatless_adapter_model::delivery::{AdapterDeliveryHandlers, AdapterDeliveryRequest, Admission};
use moatless_backend::adapters::providers::linear;
use moatless_backend::state::AppState;
use sqlx::PgPool;
use sqlx::Row;
use tokio_util::sync::CancellationToken;

use common::{seed_linear_app, test_app_state_with_adapters, test_config};

/// Where a Linear API client would point if one of these handlers made a call.
///
/// Every Linear handler resolves its client from the `adapter_apps` row before
/// it looks for work, so the row has to exist or the record retries forever on
/// "linear app 'default' not configured". None of the records here has a
/// binding to deliver to, so the client is built and never used — and this
/// address, where nothing listens, is what makes that a fast failure rather
/// than a hang if that ever stops being true.
const UNREACHABLE_LINEAR_API: &str = "http://127.0.0.1:1";

const CONNECTION_ID: &str = "conn-github-1";
const LINEAR_CONNECTION_ID: &str = "conn-linear-1";
const GITHUB_ADAPTER: &str = "github";
const GITHUB_PR_MESSAGE: &str = "github.pr_message";
const GITHUB_CLOSE: &str = "github.close";
const GITHUB_POST_MERGE: &str = "github.post_merge";
const GITHUB_MERGE_CONFLICT: &str = "github.merge_conflict";

/// The tenant fixtures every handler needs to get past its "does this still
/// exist?" guards: the `adapter_connections` row, plus — for Linear — the
/// `adapter_apps` row its client is resolved from.
async fn seed_connection(pool: &PgPool, id: &str, adapter_kind: &str, account: &str) {
    sqlx::query(
        // `owner_user_id` is not decoration: a user-scoped connection without
        // one violates `adapter_connections_owner_scope_chk`.
        r#"INSERT INTO adapter_connections
               (id, adapter_kind, connection_kind, external_account_id, created_by, scope,
                owner_user_id)
           VALUES ($1, $2, 'org', $3, 'user-1', 'user', 'user-1')
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(adapter_kind)
    .bind(account)
    .execute(pool)
    .await
    .expect("seed integration connection");

    if adapter_kind == "linear" {
        seed_linear_app(pool, UNREACHABLE_LINEAR_API).await;
    }
}

fn github_context(connection_id: &str) -> serde_json::Value {
    serde_json::json!({
        "connection_id": connection_id,
        // No loop: the repo has a connection but no subscription loop, which is
        // the delivery-only shape every handler has to tolerate.
        "loop_id": serde_json::Value::Null,
        "repo_full_name": "acme/widgets",
    })
}

/// One request per kind this ticket admits. Every one of them has to survive
/// the crash test, so they are enumerated once and reused.
fn every_kind(connection_id: &str, linear_connection_id: &str) -> Vec<AdapterDeliveryRequest> {
    let mut pr_message = github_context(connection_id);
    pr_message["message"] = serde_json::json!({
        "adapter_kind": "github_pr",
        "source_id": "acme/widgets",
        "thread_id": "42",
        "message_thread_id": "42",
        "user_id": "user-1",
        "text": "a comment on the PR",
        "file_ids": [],
        "thread_properties": {"owner": "acme", "repo": "widgets", "pr_number": 42},
        "adapter_context": {},
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
        AdapterDeliveryRequest {
            adapter: GITHUB_ADAPTER,
            kind: GITHUB_PR_MESSAGE,
            event_id: "delivery-1".to_string(),
            payload: pr_message,
        },
        AdapterDeliveryRequest {
            adapter: GITHUB_ADAPTER,
            kind: GITHUB_CLOSE,
            event_id: "delivery-2".to_string(),
            payload: close,
        },
        AdapterDeliveryRequest {
            adapter: GITHUB_ADAPTER,
            kind: GITHUB_POST_MERGE,
            event_id: "delivery-3".to_string(),
            payload: post_merge,
        },
        AdapterDeliveryRequest {
            adapter: GITHUB_ADAPTER,
            kind: GITHUB_MERGE_CONFLICT,
            event_id: "delivery-4".to_string(),
            payload: merge_conflict,
        },
        AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::SESSION_PROVISION,
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
        AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::SESSION_PROMPT,
            event_id: "AgentSessionEvent:prompted:activity-1".to_string(),
            payload: serde_json::json!({
                "connection_id": linear_connection_id,
                "session_id": "session-A",
                // The prompt's downstream identity is built from this, so a
                // record that lost it would double-deliver on the next attempt.
                "activity_id": "activity-1",
                "body": "please look at this again",
            }),
        },
        AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::ISSUE_UPDATE,
            event_id: "Issue:update:issue-1:1700000000000".to_string(),
            payload: serde_json::json!({
                "connection_id": linear_connection_id,
                "issue": {
                    "id": "issue-1",
                    "identifier": "ENG-7",
                    "title": "Fix the thing",
                    "teamId": "team-1",
                    "state": {"id": "s2", "name": "In Progress", "type": "started"},
                },
                "updated_from": {"stateId": "s1"},
                // Linear's own delivery timestamp, half of the forwarded
                // message's identity — carried, never re-read from a clock.
                "delivered_at": 1_700_000_000_000i64,
            }),
        },
        AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::COMMENT,
            event_id: "Comment:create:comment-1".to_string(),
            payload: serde_json::json!({
                "connection_id": linear_connection_id,
                "issue_id": "issue-1",
                "comment": {
                    "id": "comment-1",
                    "body": "have a look at this",
                    "issueId": "issue-1",
                    "user": {"id": "user-9", "name": "Alice"},
                },
            }),
        },
        AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::NOTIFICATION,
            event_id: "AppUserNotification:issueStatusChanged:notif-1".to_string(),
            payload: serde_json::json!({
                "connection_id": linear_connection_id,
                // Which actions are acted on is the worker's decision, so the
                // action rides in the record rather than gating admission.
                "action": "issueStatusChanged",
                "notification": {
                    "id": "notif-1",
                    "type": "issueStatusChanged",
                    "issueId": "issue-1",
                    "issue": {
                        "id": "issue-1",
                        "identifier": "ENG-7",
                        "title": "Fix the thing",
                        "teamId": "team-1",
                        "state": {"id": "s3", "name": "Done", "type": "completed"},
                    },
                },
            }),
        },
    ]
}

/// The registry the running process builds, assembled the same way `main` does.
fn registered_handlers(state: &AppState) -> Arc<AdapterDeliveryHandlers> {
    Arc::new(
        state
            .adapter_installations
            .as_ref()
            .expect("test state installs external Adapters")
            .delivery_handlers(state),
    )
}

/// Run the worker until `done` holds or `budget` elapses, then cancel it — the
/// same start/cancel path `main` uses.
async fn run_worker_until(
    state: &AppState,
    budget: Duration,
    done: impl AsyncFn() -> bool,
) -> bool {
    let cancel = CancellationToken::new();
    let handle = spawn_adapter_delivery_worker(
        state.adapter_delivery.clone(),
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
        "SELECT status, attempts, last_error FROM adapter_delivery WHERE kind = $1 AND event_id = $2",
    )
    .bind(kind)
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .expect("read adapter_delivery")
    .map(|row| {
        (
            row.get::<String, _>("status"),
            row.get::<i32, _>("attempts"),
            row.get::<Option<String>, _>("last_error"),
        )
    })
}

/// Every record that has not reached a terminal state, with why it is stuck.
///
/// The "did they all finish?" assertions are a wait loop over several kinds, so
/// a bare `false` names none of them. This turns the failure into the list of
/// kinds still in flight and the last error each one recorded.
async fn stuck(pool: &PgPool, kinds: &[(&'static str, String)]) -> String {
    let mut lines = Vec::new();
    for (kind, event_id) in kinds {
        match status_of(pool, kind, event_id).await {
            Some((status, _, _)) if status == "succeeded" || status == "failed" => {}
            Some((status, attempts, last_error)) => lines.push(format!(
                "{kind}/{event_id}: {status} after {attempts} attempt(s): {}",
                last_error.unwrap_or_else(|| "no error recorded".to_string())
            )),
            None => lines.push(format!("{kind}/{event_id}: no record")),
        }
    }
    lines.join("; ")
}

/// Serve every GitHub read as a repo whose default branch is `default_branch`.
///
/// `github.post_merge` is the one kind here that cannot decide anything
/// locally: its work is keyed by a commit sha, and only GitHub can say which PR
/// that sha merged, so it authenticates and reads the repo before any skip is
/// available to it. With no credential and nowhere to send the read it fails
/// *retryably* — a record that never settles, which in a test about reaching
/// terminal states is a 30-second timeout.
///
/// The branch reported here is deliberately not the run's `head_branch`, which
/// is the handler's documented "this run is not a merge result" skip: the read
/// happens, the delivery succeeds on its first attempt, and nothing is posted.
async fn default_branch_stub(default_branch: &'static str) -> (String, AbortOnDrop) {
    let app = axum::Router::new().fallback(axum::routing::any(move || async move {
        axum::Json(serde_json::json!({ "default_branch": default_branch }))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the GitHub stub");
    let addr = listener.local_addr().expect("the stub reports its address");
    let serving = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), AbortOnDrop(serving))
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn count_records(pool: &PgPool, kind: &str, event_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery WHERE kind = $1 AND event_id = $2")
        .bind(kind)
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("count adapter_delivery")
}

/// Verification: crash/restart recovery for every admitted work kind.
///
/// The admitting state is dropped without ever running a worker — from the
/// queue's side that is indistinguishable from a process killed after it
/// acknowledged the provider. A different process then has to finish every one
/// of them.
#[sqlx::test]
async fn every_admitted_kind_is_executed_by_the_process_that_comes_after_the_crash(pool: PgPool) {
    let (base_url, _stub) = default_branch_stub("trunk").await;
    let _github_api = common::github_api_base(&base_url).await;
    let credentialed = test_config;

    seed_connection(&pool, CONNECTION_ID, "github", "acme").await;
    seed_connection(&pool, LINEAR_CONNECTION_ID, "linear", "org-1").await;

    let requests = every_kind(CONNECTION_ID, LINEAR_CONNECTION_ID);
    let kinds: Vec<(&'static str, String)> = requests
        .iter()
        .map(|r| (r.kind, r.event_id.clone()))
        .collect();

    {
        let admitting = test_app_state_with_adapters(pool.clone(), credentialed());
        for request in requests {
            let kind = request.kind;
            assert_eq!(
                admitting
                    .adapter_delivery
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

    let recovering = test_app_state_with_adapters(pool.clone(), credentialed());
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

    assert!(
        finished,
        "every admitted kind reached a terminal state; still in flight: {}",
        stuck(&pool, &kinds).await
    );
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
    let state = test_app_state_with_adapters(pool.clone(), test_config());

    let mut message = github_context(CONNECTION_ID);
    message["message"] = serde_json::json!({
        "adapter_kind": "github_pr",
        "source_id": "acme/widgets",
        "thread_id": "42",
        "message_thread_id": "42",
        "user_id": "user-1",
        "text": "a comment on the PR",
        "file_ids": [],
        "thread_properties": {"owner": "acme", "repo": "widgets", "pr_number": 42},
        "adapter_context": {},
        "external_event_id": "github:ic:9001",
        "source_task_id": serde_json::Value::Null,
    });
    let request = || AdapterDeliveryRequest {
        adapter: GITHUB_ADAPTER,
        kind: GITHUB_PR_MESSAGE,
        event_id: "delivery-dup".to_string(),
        payload: message.clone(),
    };

    assert_eq!(
        state
            .adapter_delivery
            .admit(request())
            .await
            .expect("first"),
        Admission::Admitted
    );
    assert_eq!(
        state
            .adapter_delivery
            .admit(request())
            .await
            .expect("redelivery"),
        Admission::Duplicate,
        "GitHub's own delivery id is the idempotency key"
    );
    assert_eq!(
        count_records(&pool, GITHUB_PR_MESSAGE, "delivery-dup").await,
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
            .adapter_delivery
            .admit(AdapterDeliveryRequest {
                adapter: GITHUB_ADAPTER,
                kind: GITHUB_MERGE_CONFLICT,
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
            status_of(&pool_for_check, GITHUB_PR_MESSAGE, "delivery-dup").await,
            Some((ref status, ..)) if status == "succeeded" || status == "failed"
        )
    })
    .await;

    let (status, attempts, last_error) = status_of(&pool, GITHUB_PR_MESSAGE, "delivery-dup")
        .await
        .expect("record");
    assert_eq!(status, "succeeded", "{last_error:?}");
    assert_eq!(attempts, 1, "the redelivery cost no extra execution");

    // A late redelivery still dedups against the finished record.
    assert_eq!(
        state
            .adapter_delivery
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
    let state = test_app_state_with_adapters(pool.clone(), test_config());

    let session_request = |session_id: &str| AdapterDeliveryRequest {
        adapter: linear::delivery::ADAPTER,
        kind: linear::delivery::SESSION_PROVISION,
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
            .adapter_delivery
            .admit(session_request("session-A"))
            .await
            .expect("first"),
        Admission::Admitted
    );
    assert_eq!(
        state
            .adapter_delivery
            .admit(session_request("session-A"))
            .await
            .expect("retry"),
        Admission::Duplicate,
        "a Linear retry of one session is one unit of work"
    );
    assert_eq!(
        state
            .adapter_delivery
            .admit(session_request("session-B"))
            .await
            .expect("second session"),
        Admission::Admitted,
        "two sessions sharing a webhookId must not collapse into one"
    );

    assert_eq!(
        count_records(
            &pool,
            linear::delivery::SESSION_PROVISION,
            "AgentSessionEvent:created:session-A"
        )
        .await,
        1
    );
}

/// Verification: each Linear kind retries on its own budget.
///
/// The kinds are separate records rather than one shared `linear.webhook`
/// precisely so that a delivery whose work can never succeed cannot spend the
/// attempts of the work admitted beside it. Both records here carry the same
/// delivery identity, so the only thing keeping them apart is the kind.
#[sqlx::test]
async fn a_permanently_failing_linear_kind_leaves_its_siblings_budget_alone(pool: PgPool) {
    seed_connection(&pool, LINEAR_CONNECTION_ID, "linear", "org-1").await;
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    let event_id = "Comment:create:comment-1";

    // Written by a build that is gone: the payload has a connection but none of
    // the fields its kind needs, so no attempt can ever read it.
    state
        .adapter_delivery
        .admit(AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::ISSUE_UPDATE,
            event_id: event_id.to_string(),
            payload: serde_json::json!({"connection_id": LINEAR_CONNECTION_ID}),
        })
        .await
        .expect("admitted");
    state
        .adapter_delivery
        .admit(AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::COMMENT,
            event_id: event_id.to_string(),
            payload: serde_json::json!({
                "connection_id": LINEAR_CONNECTION_ID,
                "issue_id": "issue-1",
                "comment": {"id": "comment-1", "body": "have a look", "issueId": "issue-1"},
            }),
        })
        .await
        .expect("one delivery's distinct kinds do not dedup against each other");

    let pool_for_check = pool.clone();
    let finished = run_worker_until(&state, Duration::from_secs(30), async || {
        for kind in [linear::delivery::ISSUE_UPDATE, linear::delivery::COMMENT] {
            match status_of(&pool_for_check, kind, event_id).await {
                Some((status, ..)) if status == "succeeded" || status == "failed" => {}
                _ => return false,
            }
        }
        true
    })
    .await;
    assert!(finished, "both records reached a terminal state");

    let (status, attempts, last_error) = status_of(&pool, linear::delivery::ISSUE_UPDATE, event_id)
        .await
        .expect("the poisoned record is still there");
    assert_eq!(status, "failed");
    assert_eq!(
        attempts, 1,
        "a permanent failure spends no budget on retrying"
    );
    let last_error = last_error.unwrap_or_default();
    assert!(
        last_error.contains("does not match its kind"),
        "a terminal record says why it was abandoned: {last_error}"
    );

    let (status, attempts, last_error) = status_of(&pool, linear::delivery::COMMENT, event_id)
        .await
        .expect("the sibling is its own record");
    assert_eq!(
        status, "succeeded",
        "the sibling ran despite its neighbour failing: {last_error:?}"
    );
    assert_eq!(
        attempts, 1,
        "the sibling kept its full budget — the failure next to it cost it nothing"
    );
}

/// Verification: a Linear delivery interrupted mid-execution is retried.
///
/// Crash recovery above covers a process that died *before* executing. This is
/// the other half: the record was claimed, the handler was running, and the
/// replica disappeared. Nothing acknowledges it back to Linear, so the only
/// thing that can rescue it is the claim lease — and the record has to complete
/// under the next worker rather than sit `claimed` forever.
#[sqlx::test]
async fn a_linear_delivery_interrupted_mid_execution_is_retried(pool: PgPool) {
    seed_connection(&pool, LINEAR_CONNECTION_ID, "linear", "org-1").await;
    let state = test_app_state_with_adapters(pool.clone(), test_config());
    let event_id = "AgentSessionEvent:prompted:activity-1";

    state
        .adapter_delivery
        .admit(AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::SESSION_PROMPT,
            event_id: event_id.to_string(),
            payload: serde_json::json!({
                "connection_id": LINEAR_CONNECTION_ID,
                "session_id": "session-A",
                "activity_id": "activity-1",
                "body": "please look at this again",
            }),
        })
        .await
        .expect("admitted");

    // The kill: claimed by a replica that is never coming back, long enough ago
    // that its lease has expired. Set directly rather than waited for, because
    // the lease is minutes and the property is not about the clock.
    sqlx::query(
        "UPDATE adapter_delivery \
         SET status = 'claimed', attempts = 1, claimed_at = NOW() - INTERVAL '1 hour', \
             claimed_by = 'dead-replica' \
         WHERE kind = $1 AND event_id = $2",
    )
    .bind(linear::delivery::SESSION_PROMPT)
    .bind(event_id)
    .execute(&pool)
    .await
    .expect("simulate the interrupted execution");

    let pool_for_check = pool.clone();
    let finished = run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, linear::delivery::SESSION_PROMPT, event_id).await,
            Some((ref status, ..)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert!(finished, "the abandoned claim was picked up again");

    let (status, _, last_error) = status_of(&pool, linear::delivery::SESSION_PROMPT, event_id)
        .await
        .expect("record");
    assert_eq!(
        status, "succeeded",
        "the interrupted delivery was retried, not lost: {last_error:?}"
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
    let state = test_app_state_with_adapters(pool.clone(), test_config());

    let mut orphaned = github_context("conn-that-was-deleted");
    orphaned["owner"] = serde_json::json!("acme");
    orphaned["repo"] = serde_json::json!("widgets");
    orphaned["pr_number"] = serde_json::json!(42);
    orphaned["sender_login"] = serde_json::json!("octocat");
    state
        .adapter_delivery
        .admit(AdapterDeliveryRequest {
            adapter: GITHUB_ADAPTER,
            kind: GITHUB_MERGE_CONFLICT,
            event_id: "delivery-orphan".to_string(),
            payload: orphaned,
        })
        .await
        .expect("admitted");

    // A payload an older build wrote and this one cannot read is equally
    // hopeless, and must be terminal rather than retried forever.
    state
        .adapter_delivery
        .admit(AdapterDeliveryRequest {
            adapter: linear::delivery::ADAPTER,
            kind: linear::delivery::SESSION_PROVISION,
            event_id: "AgentSessionEvent:created:unreadable".to_string(),
            payload: serde_json::json!({"connection_id": LINEAR_CONNECTION_ID}),
        })
        .await
        .expect("admitted");

    let cases = [
        (GITHUB_MERGE_CONFLICT, "delivery-orphan"),
        (
            linear::delivery::SESSION_PROVISION,
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

    let (status, attempts, last_error) = status_of(&pool, GITHUB_MERGE_CONFLICT, "delivery-orphan")
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
        linear::delivery::SESSION_PROVISION,
        "AgentSessionEvent:created:unreadable",
    )
    .await
    .expect("record");
    assert_eq!(status, "failed");
    assert_eq!(attempts, 1);
    let last_error = last_error.unwrap_or_default();
    assert!(
        last_error.contains("does not match its kind"),
        "an unreadable payload is terminal, not a retry loop: {last_error}"
    );
}
