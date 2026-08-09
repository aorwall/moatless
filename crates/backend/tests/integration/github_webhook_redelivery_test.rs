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

use crate::common;

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use moatless_backend::adapters::delivery::{
    spawn_adapter_delivery_worker, AdapterDeliveryHandlers, AdapterDeliveryRequest, Admission,
};
use moatless_backend::adapters::routes::adapter_router;
use moatless_backend::compose::compose;
use moatless_backend::repository::dao as repository_dao;
use moatless_backend::state::AppState;
use moatless_backend::task::dao as task_dao;
use sqlx::{PgPool, Row};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use common::{
    make_repository_row, make_task_row, seed_team, test_app_state_with_adapters, test_config,
};

const CONNECTION_ID: &str = "conn-github-redelivery";
const OWNER: &str = "acme";
const REPO_FULL_NAME: &str = "acme/widgets";
const SUBJECT_KEY: &str = "acme/widgets#42";
const GITHUB_PR_MESSAGE: &str = "github.pr_message";
const GITHUB_POST_MERGE: &str = "github.post_merge";
const GITHUB_MERGE_CONFLICT: &str = "github.merge_conflict";
/// The task's owner. Deliberately not the connection's user: an event whose
/// sender resolves to the task owner is filtered as a self-echo, which would
/// hide the delivery this test is counting.
const TASK_USER: &str = "user-task-owner";
const CONNECTION_USER: &str = "user-connection-owner";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedGithubRequest {
    method: String,
    uri: String,
    authorization: Option<String>,
}

static GITHUB_API_ENV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

struct GithubApiBaseRestore(Option<std::ffi::OsString>);

impl Drop for GithubApiBaseRestore {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            std::env::set_var("GITHUB_API_BASE_URL", value);
        } else {
            std::env::remove_var("GITHUB_API_BASE_URL");
        }
    }
}

async fn request_counting_github_stub(
) -> (String, Arc<tokio::sync::Mutex<Vec<RecordedGithubRequest>>>) {
    async fn handle(
        State(requests): State<Arc<tokio::sync::Mutex<Vec<RecordedGithubRequest>>>>,
        request: axum::extract::Request,
    ) -> Response {
        let method = request.method().clone();
        let uri = request
            .uri()
            .path_and_query()
            .map_or_else(|| request.uri().path().to_string(), ToString::to_string);
        let authorization = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        requests.lock().await.push(RecordedGithubRequest {
            method: method.to_string(),
            uri: uri.clone(),
            authorization,
        });

        match (method, request.uri().path()) {
            (Method::GET, "/repos/acme/widgets/pulls/42") => Json(serde_json::json!({
                "number": 42,
                "title": "Add widgets",
                "body": "Body",
                "state": "open",
                "merged": false,
                "mergeable": false,
                "mergeable_state": "dirty",
                "draft": false,
                "html_url": "https://github.com/acme/widgets/pull/42",
                "user": {"login": "octocat"},
                "head": {"ref": "feature", "sha": "abc1234"},
                "base": {"ref": "main", "sha": "def4567"}
            }))
            .into_response(),
            (Method::GET, "/repos/acme/widgets") => {
                Json(serde_json::json!({"default_branch": "main"})).into_response()
            }
            (Method::GET, "/repos/acme/widgets/commits/deadbeef/pulls") => {
                Json(serde_json::json!([{
                    "number": 42,
                    "state": "closed",
                    "merged_at": "2026-07-30T00:00:00Z"
                }]))
                .into_response()
            }
            _ => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"message": format!("unexpected {uri}")})),
            )
                .into_response(),
        }
    }

    let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind GitHub stub");
    let addr = listener.local_addr().expect("GitHub stub address");
    let app = Router::new()
        .fallback(any(handle))
        .with_state(requests.clone());
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("GitHub stub serves");
    });
    (format!("http://{addr}"), requests)
}

// =========================================================================
// Fixtures
// =========================================================================

/// One backend process with its own state and registered adapter, sharing only
/// the database. Two of these is the whole point of these tests.
async fn replica(pool: &PgPool) -> AppState {
    registered_replica(pool, None, None).await
}

async fn registered_replica(
    pool: &PgPool,
    webhook_secret: Option<&str>,
    bot_token: Option<&str>,
) -> AppState {
    let mut config = test_config();
    config.auth.frontend_url = "https://app.example.com".to_string();
    config.github_pr.webhook_secret = webhook_secret.map(str::to_string);
    config.github_pr.bot_token = bot_token.map(str::to_string);
    let ports = compose(pool.clone(), &config);
    let (adapters, handles) = moatless_backend::compose::from_config(
        &config,
        ports.pool.clone(),
        ports.provider_tokens.clone(),
        ports.provider_token_auth.clone(),
        ports.github_apps.clone(),
        ports.github_app_service.clone(),
        ports.secrets.clone(),
    )
    .await;
    test_app_state_with_adapters(pool.clone(), Some(Arc::new(adapters)), handles)
}

async fn seed_fixtures(pool: &PgPool, task_id: &str) {
    seed_team(pool, &format!("personal-{TASK_USER}")).await;
    repository_dao::insert(pool, &make_repository_row("repo-1", TASK_USER))
        .await
        .expect("seed repository");

    sqlx::query(
        r#"INSERT INTO adapter_connections
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

    seed_bound_task(pool, task_id, "binding-1", "connected_thread").await;
}

async fn seed_bound_task(pool: &PgPool, task_id: &str, binding_id: &str, origin: &str) {
    task_dao::insert_task(pool, &make_task_row(task_id, TASK_USER))
        .await
        .expect("seed task");

    // The PR the events name, already connected to the task — so the ingress
    // path has a binding to read and a status column to write.
    sqlx::query(
        r#"INSERT INTO task_bindings
               (id, task_id, adapter_kind, subject_kind, subject_key, connection_id,
                destination_config, origin)
           VALUES ($1, $2, 'github', 'github_pr', $3, $4, $5, $6)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(binding_id)
    .bind(task_id)
    .bind(SUBJECT_KEY)
    .bind(CONNECTION_ID)
    .bind(serde_json::json!({"owner": OWNER, "repo": "widgets", "pr_number": 42}))
    .bind(origin)
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
    event_type: &str,
    delivery_id: &str,
    payload: &serde_json::Value,
) -> StatusCode {
    let body = serde_json::to_vec(payload).unwrap();
    webhook_response(state, headers(event_type, delivery_id), body)
        .await
        .status()
}

async fn webhook_response(state: &AppState, headers: HeaderMap, body: Vec<u8>) -> Response {
    let (router, _) = adapter_router(state.clone());
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/adapters/github/webhook");
    for (name, value) in &headers {
        builder = builder.header(name, value);
    }
    router
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .expect("route request succeeds")
}

fn issue_comment_payload(comment_id: u64, sender: &str) -> serde_json::Value {
    issue_comment_payload_with_body(comment_id, sender, "please take another look")
}

fn issue_comment_payload_with_body(comment_id: u64, sender: &str, body: &str) -> serde_json::Value {
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
            "body": body,
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

fn check_run_payload(check_run_id: u64) -> serde_json::Value {
    serde_json::json!({
        "action": "completed",
        "repository": {
            "full_name": REPO_FULL_NAME,
            "owner": { "login": OWNER },
        },
        "sender": { "login": "octocat" },
        "check_run": {
            "id": check_run_id,
            "name": "CI",
            "conclusion": "failure",
            "head_sha": "abc1234deadbeef",
            "html_url": format!("https://github.com/acme/widgets/runs/{check_run_id}"),
            "app": { "name": "Actions" },
            "pull_requests": [{ "number": 42 }],
            "output": {
                "title": "Tests failed",
                "summary": "one failing test",
            },
        },
    })
}

fn rejected_workflow_run_payload() -> serde_json::Value {
    serde_json::json!({
        "action": "requested",
        "repository": {
            "full_name": REPO_FULL_NAME,
            "owner": { "login": OWNER },
        },
        "sender": { "login": "octocat" },
        "workflow_run": {
            "id": 555,
            "name": "CD - Publish",
            "event": "push",
            "conclusion": "success",
            "head_sha": "deadbeef",
            "head_branch": "main",
            "html_url": "https://github.com/acme/widgets/actions/runs/555",
            "pull_requests": [],
        },
    })
}

fn merge_conflict_request(event_id: &str) -> AdapterDeliveryRequest {
    AdapterDeliveryRequest {
        adapter: "github",
        kind: GITHUB_MERGE_CONFLICT,
        event_id: event_id.to_string(),
        payload: serde_json::json!({
            "connection_id": CONNECTION_ID,
            "loop_id": serde_json::Value::Null,
            "repo_full_name": REPO_FULL_NAME,
            "owner": OWNER,
            "repo": "widgets",
            "pr_number": 42,
            "sender_login": "octocat",
        }),
    }
}

fn post_merge_request(event_id: &str, run_id: u64) -> AdapterDeliveryRequest {
    AdapterDeliveryRequest {
        adapter: "github",
        kind: GITHUB_POST_MERGE,
        event_id: event_id.to_string(),
        payload: serde_json::json!({
            "connection_id": CONNECTION_ID,
            "loop_id": serde_json::Value::Null,
            "repo_full_name": REPO_FULL_NAME,
            "owner": OWNER,
            "repo": "widgets",
            "sender_login": "octocat",
            "run": {
                "run_id": run_id,
                "name": "CI",
                "conclusion": "success",
                "head_sha": "deadbeef",
                "head_branch": "main",
                "html_url": format!("https://github.com/acme/widgets/actions/runs/{run_id}"),
            },
        }),
    }
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

async fn task_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await
        .expect("count tasks")
}

async fn delivery_payload(pool: &PgPool, kind: &str, event_id: &str) -> Option<serde_json::Value> {
    sqlx::query_scalar("SELECT payload FROM adapter_delivery WHERE kind = $1 AND event_id = $2")
        .bind(kind)
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .expect("read adapter delivery payload")
}

/// Run the worker until `done` holds or the budget elapses, then cancel it.
async fn run_worker_until(state: &AppState, budget: Duration, done: impl AsyncFn() -> bool) {
    let mut handlers = AdapterDeliveryHandlers::new();
    moatless_backend::compose::register_builtin_delivery_handlers(&mut handlers, state);
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

#[sqlx::test]
async fn acknowledgement_and_partial_admission_recovery_follow_durable_ownership(pool: PgPool) {
    seed_fixtures(&pool, "task-admission").await;

    let signed_state = registered_replica(&pool, Some("webhook-secret"), None).await;
    let malformed = webhook_response(
        &signed_state,
        headers("issue_comment", "delivery-malformed"),
        b"{not-json".to_vec(),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::OK);

    let payload = issue_comment_payload(9100, "octocat");
    let body = serde_json::to_vec(&payload).unwrap();
    let mut bad_signature_headers = headers("issue_comment", "delivery-bad-signature");
    bad_signature_headers.insert(
        "x-hub-signature-256",
        "sha256=0000000000000000000000000000000000000000000000000000000000000000"
            .parse()
            .unwrap(),
    );
    let bad_signature = webhook_response(&signed_state, bad_signature_headers, body.clone()).await;
    assert_eq!(bad_signature.status(), StatusCode::OK);
    assert_eq!(
        count_records(&pool, GITHUB_PR_MESSAGE, "delivery-bad-signature").await,
        0
    );

    sqlx::query(
        r#"CREATE FUNCTION reject_second_github_kind() RETURNS trigger AS $$
           BEGIN
             IF NEW.kind = 'github.merge_conflict' THEN
               RAISE EXCEPTION 'forced later admission failure';
             END IF;
             RETURN NEW;
           END;
           $$ LANGUAGE plpgsql"#,
    )
    .execute(&pool)
    .await
    .expect("create admission fault");
    sqlx::query(
        r#"CREATE TRIGGER reject_second_github_kind
           BEFORE INSERT ON adapter_delivery
           FOR EACH ROW EXECUTE FUNCTION reject_second_github_kind()"#,
    )
    .execute(&pool)
    .await
    .expect("install admission fault");

    let state = registered_replica(&pool, None, None).await;
    let failed = webhook_response(
        &state,
        headers("issue_comment", "delivery-partial"),
        body.clone(),
    )
    .await;
    assert!(
        failed.status().is_server_error(),
        "admission failure maps to a non-2xx response"
    );
    assert_eq!(
        count_records(&pool, GITHUB_PR_MESSAGE, "delivery-partial").await,
        1,
        "the first independently committed kind remains durable"
    );
    assert_eq!(
        count_records(&pool, GITHUB_MERGE_CONFLICT, "delivery-partial").await,
        0,
        "the failed kind has no durable owner yet"
    );

    sqlx::query("DROP TRIGGER reject_second_github_kind ON adapter_delivery")
        .execute(&pool)
        .await
        .expect("remove admission fault");
    sqlx::query("DROP FUNCTION reject_second_github_kind()")
        .execute(&pool)
        .await
        .expect("remove admission fault function");

    let recovered =
        webhook_response(&state, headers("issue_comment", "delivery-partial"), body).await;
    assert_eq!(recovered.status(), StatusCode::OK);
    assert_eq!(
        count_records(&pool, GITHUB_PR_MESSAGE, "delivery-partial").await,
        1,
        "redelivery deduplicates the first committed kind"
    );
    assert_eq!(
        count_records(&pool, GITHUB_MERGE_CONFLICT, "delivery-partial").await,
        1,
        "redelivery admits the missing later kind before reporting success"
    );
}

#[sqlx::test]
async fn connection_without_subscription_delivers_existing_bindings_but_creates_no_task(
    pool: PgPool,
) {
    let task_id = "task-existing-binding";
    seed_fixtures(&pool, task_id).await;
    let state = replica(&pool).await;

    deliver(
        &state,
        "issue_comment",
        "delivery-existing-comment",
        &issue_comment_payload(9200, "octocat"),
    )
    .await;
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, GITHUB_PR_MESSAGE, "delivery-existing-comment").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        message_count(&pool, task_id).await,
        1,
        "a bare connection still routes comments to an existing binding"
    );

    let before_tasks = task_count(&pool).await;
    deliver(
        &state,
        "pull_request",
        "delivery-open-without-subscription",
        &pull_request_payload("opened", "open", false),
    )
    .await;
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(
                &pool_for_check,
                GITHUB_PR_MESSAGE,
                "delivery-open-without-subscription"
            )
            .await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        task_count(&pool).await,
        before_tasks,
        "an opened PR cannot create a review task without a Subscription"
    );
    assert_eq!(
        message_count(&pool, task_id).await,
        2,
        "the opened event still reaches the already-bound task"
    );
}

#[sqlx::test]
async fn footer_echo_skips_only_its_authoring_task(pool: PgPool) {
    let author = "task-footer-author";
    let sibling = "task-footer-sibling";
    seed_fixtures(&pool, author).await;
    seed_bound_task(&pool, sibling, "binding-footer-sibling", "connected_thread").await;
    let state = replica(&pool).await;
    let body =
        format!("Done.\n\n---\nMoatless task: [Author](https://app.example.com/tasks/{author})");

    deliver(
        &state,
        "issue_comment",
        "delivery-footer-echo",
        &issue_comment_payload_with_body(9300, "octocat", &body),
    )
    .await;
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, GITHUB_PR_MESSAGE, "delivery-footer-echo").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;

    assert_eq!(
        message_count(&pool, author).await,
        0,
        "the footer names exactly the task whose echo is skipped"
    );
    assert_eq!(
        message_count(&pool, sibling).await,
        1,
        "a sibling Binding still receives the same comment"
    );
}

#[sqlx::test]
async fn pushed_commit_claim_skips_only_its_pusher(pool: PgPool) {
    let pusher = "task-commit-pusher";
    let sibling = "task-commit-sibling";
    seed_fixtures(&pool, pusher).await;
    seed_bound_task(&pool, sibling, "binding-commit-sibling", "connected_thread").await;
    let state = replica(&pool).await;
    assert!(state.recently_sent.note_commit(pusher, "7fc0612"));

    let mut payload = pull_request_payload("synchronize", "open", false);
    payload["before"] = serde_json::json!("1111111111111111111111111111111111111111");
    payload["after"] = serde_json::json!("7fc0612a1b2c3d4e5f60718293a4b5c6d7e8f901");
    deliver(&state, "pull_request", "delivery-commit-echo", &payload).await;
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, GITHUB_PR_MESSAGE, "delivery-commit-echo").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;

    assert_eq!(
        message_count(&pool, pusher).await,
        0,
        "the commit claim names exactly the pushing task"
    );
    assert_eq!(
        message_count(&pool, sibling).await,
        1,
        "a sibling Binding still receives the synchronize event"
    );
}

#[sqlx::test]
async fn inbound_sender_prefers_mapped_login_then_connection_owner(pool: PgPool) {
    seed_fixtures(&pool, "task-inbound-identity").await;
    sqlx::query(
        r#"INSERT INTO provider_tokens
               (user_id, provider, access_token, auth_method, provider_login, scope, owner_user_id)
           VALUES ('mapped-octocat', 'github', 'token', 'manual', 'octocat', 'user',
                   'mapped-octocat')"#,
    )
    .execute(&pool)
    .await
    .expect("seed mapped GitHub login");
    let state = replica(&pool).await;

    deliver(
        &state,
        "issue_comment",
        "delivery-mapped-sender",
        &issue_comment_payload(9400, "octocat"),
    )
    .await;
    assert_eq!(
        delivery_payload(&pool, GITHUB_PR_MESSAGE, "delivery-mapped-sender")
            .await
            .and_then(|payload| payload.pointer("/message/user_id").cloned()),
        Some(serde_json::json!("mapped-octocat"))
    );

    deliver(
        &state,
        "issue_comment",
        "delivery-unmapped-sender",
        &issue_comment_payload(9401, "unmapped-user"),
    )
    .await;
    assert_eq!(
        delivery_payload(&pool, GITHUB_PR_MESSAGE, "delivery-unmapped-sender")
            .await
            .and_then(|payload| payload.pointer("/message/user_id").cloned()),
        Some(serde_json::json!(CONNECTION_USER)),
        "without a Subscription, inbound identity falls back to its delivery-only effective user"
    );
}

#[sqlx::test]
async fn partial_pr_payload_projects_status_before_current_acknowledged_no_effect_limit(
    pool: PgPool,
) {
    let _env_lock = GITHUB_API_ENV_LOCK.lock().await;
    let (base_url, requests) = request_counting_github_stub().await;
    let previous = std::env::var_os("GITHUB_API_BASE_URL");
    std::env::set_var("GITHUB_API_BASE_URL", &base_url);
    let _restore = GithubApiBaseRestore(previous);

    seed_fixtures(&pool, "task-partial-status").await;
    let state = registered_replica(&pool, None, Some("global-token")).await;
    let payload = serde_json::json!({
        "action": "ready_for_review",
        "repository": {
            "full_name": REPO_FULL_NAME,
            "owner": {"login": OWNER},
        },
        "sender": {"login": "octocat"},
        // Enough for the pre-admission status projection, deliberately without
        // title/body/head/base fields needed by richer primary effects. In the
        // current handlers no separate post-projection primary parse rejection
        // is reachable: status projection and primary parsing both need this
        // PR-number shape, while the richer primary fields default.
        "pull_request": {
            "number": 42,
            "state": "open",
            "merged": false,
            "draft": false,
        },
    });
    let accepted = webhook_response(
        &state,
        headers("pull_request", "delivery-partial-status"),
        serde_json::to_vec(&payload).unwrap(),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);

    let projected = wait_for(&pool, |state| state.0.is_some()).await;
    assert_eq!(projected.0.as_deref(), Some("open"));
    assert_eq!(
        count_records(&pool, GITHUB_PR_MESSAGE, "delivery-partial-status").await,
        0,
        "the partial payload did not invent a primary message"
    );
    assert_eq!(
        count_records(&pool, GITHUB_MERGE_CONFLICT, "delivery-partial-status").await,
        1,
        "all work the partial payload can plan is admitted before acknowledgment"
    );
    assert!(
        requests.lock().await.is_empty(),
        "webhook status projection performs zero GitHub API calls"
    );
}

#[sqlx::test]
async fn rejected_post_merge_candidate_spends_zero_github_calls_during_webhook_normalization(
    pool: PgPool,
) {
    let _env_lock = GITHUB_API_ENV_LOCK.lock().await;
    let (base_url, requests) = request_counting_github_stub().await;
    let previous = std::env::var_os("GITHUB_API_BASE_URL");
    std::env::set_var("GITHUB_API_BASE_URL", &base_url);
    let _restore = GithubApiBaseRestore(previous);

    seed_fixtures(&pool, "task-rejected-post-merge-webhook").await;
    let state = registered_replica(&pool, None, Some("global-token")).await;
    let accepted = webhook_response(
        &state,
        headers("workflow_run", "delivery-rejected-post-merge-webhook"),
        serde_json::to_vec(&rejected_workflow_run_payload()).unwrap(),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        count_records(
            &pool,
            GITHUB_POST_MERGE,
            "delivery-rejected-post-merge-webhook"
        )
        .await,
        0,
        "normalization rejects the post-merge candidate before durable admission"
    );
    assert!(
        requests.lock().await.is_empty(),
        "candidate rejection on the webhook path spends zero GitHub API calls"
    );
}

#[sqlx::test]
async fn build_result_routes_to_producer_then_shared_fallback(pool: PgPool) {
    let _env_lock = GITHUB_API_ENV_LOCK.lock().await;
    let (base_url, requests) = request_counting_github_stub().await;
    let previous = std::env::var_os("GITHUB_API_BASE_URL");
    std::env::set_var("GITHUB_API_BASE_URL", &base_url);
    let _restore = GithubApiBaseRestore(previous);

    let producer = "task-build-producer";
    let responder = "task-build-responder";
    seed_fixtures(&pool, producer).await;
    seed_bound_task(&pool, responder, "binding-build-responder", "ingress").await;
    sqlx::query("UPDATE task_bindings SET external_mergeable_state = 'dirty'")
        .execute(&pool)
        .await
        .expect("suppress unrelated conflict transition notices");
    let state = registered_replica(&pool, None, Some("global-token")).await;

    let accepted = webhook_response(
        &state,
        headers("check_run", "delivery-build-producer"),
        serde_json::to_vec(&check_run_payload(9500)).unwrap(),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert!(
        requests.lock().await.is_empty(),
        "check_run normalization and admission spend zero GitHub API calls"
    );
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, GITHUB_PR_MESSAGE, "delivery-build-producer").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        ) && matches!(
            status_of(&pool_for_check, GITHUB_MERGE_CONFLICT, "delivery-build-producer").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        message_count(&pool, producer).await,
        1,
        "a build result first targets the PR producer"
    );
    assert_eq!(message_count(&pool, responder).await, 0);

    sqlx::query("UPDATE task_bindings SET active = false WHERE id = 'binding-1'")
        .execute(&pool)
        .await
        .expect("deactivate producer");
    let accepted = webhook_response(
        &state,
        headers("check_run", "delivery-build-fallback"),
        serde_json::to_vec(&check_run_payload(9501)).unwrap(),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, GITHUB_PR_MESSAGE, "delivery-build-fallback").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        ) && matches!(
            status_of(&pool_for_check, GITHUB_MERGE_CONFLICT, "delivery-build-fallback").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        message_count(&pool, responder).await,
        1,
        "when no active producer binding remains, build routing uses the shared fallback"
    );
    assert_eq!(message_count(&pool, producer).await, 1);
}

#[sqlx::test]
async fn durable_workflows_preserve_audience_fallback_and_github_call_budgets(pool: PgPool) {
    let _env_lock = GITHUB_API_ENV_LOCK.lock().await;
    let (base_url, requests) = request_counting_github_stub().await;
    let previous = std::env::var_os("GITHUB_API_BASE_URL");
    std::env::set_var("GITHUB_API_BASE_URL", &base_url);
    let _restore = GithubApiBaseRestore(previous);

    let producer = "task-pr-producer";
    let responder = "task-pr-responder";
    seed_fixtures(&pool, producer).await;
    seed_bound_task(&pool, responder, "binding-responder", "ingress").await;
    let state = registered_replica(&pool, None, Some("global-token")).await;

    assert_eq!(
        state
            .adapter_delivery
            .admit(merge_conflict_request("delivery-conflict-producer"))
            .await
            .unwrap(),
        Admission::Admitted
    );
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(
                &pool_for_check,
                GITHUB_MERGE_CONFLICT,
                "delivery-conflict-producer"
            )
            .await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        message_count(&pool, producer).await,
        1,
        "conflict audience reaches the PR producer"
    );
    assert_eq!(message_count(&pool, responder).await, 0);
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .map(|request| request.uri.as_str())
            .collect::<Vec<_>>(),
        vec!["/repos/acme/widgets/pulls/42"],
        "a settled active-Binding conflict check spends one PR read"
    );

    assert_eq!(
        state
            .adapter_delivery
            .admit(post_merge_request("delivery-post-merge-producer", 7))
            .await
            .unwrap(),
        Admission::Admitted
    );
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(
                &pool_for_check,
                GITHUB_POST_MERGE,
                "delivery-post-merge-producer"
            )
            .await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        message_count(&pool, producer).await,
        2,
        "post-merge audience reaches the PR producer"
    );
    assert_eq!(message_count(&pool, responder).await, 0);
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .map(|request| request.uri.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/repos/acme/widgets/pulls/42",
            "/repos/acme/widgets",
            "/repos/acme/widgets/commits/deadbeef/pulls",
        ],
        "post-merge execution reads the default branch before the commit-to-PR list"
    );

    let mut wrong_branch = post_merge_request("delivery-post-merge-wrong-branch", 70);
    wrong_branch.payload["run"]["head_branch"] = serde_json::json!("feature");
    state.adapter_delivery.admit(wrong_branch).await.unwrap();
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(
                &pool_for_check,
                GITHUB_POST_MERGE,
                "delivery-post-merge-wrong-branch"
            )
            .await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        requests
            .lock()
            .await
            .iter()
            .map(|request| request.uri.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/repos/acme/widgets/pulls/42",
            "/repos/acme/widgets",
            "/repos/acme/widgets/commits/deadbeef/pulls",
            "/repos/acme/widgets",
        ],
        "a non-default branch spends the repository read but no commit-to-PR read"
    );
    assert_eq!(message_count(&pool, producer).await, 2);
    assert_eq!(message_count(&pool, responder).await, 0);

    sqlx::query(
        r#"UPDATE task_bindings
              SET active = false
            WHERE id = 'binding-1'"#,
    )
    .execute(&pool)
    .await
    .expect("deactivate producer");
    sqlx::query(
        r#"UPDATE task_bindings
              SET external_mergeable_state = 'clean'
            WHERE id = 'binding-responder'"#,
    )
    .execute(&pool)
    .await
    .expect("reset conflict transition");

    state
        .adapter_delivery
        .admit(merge_conflict_request("delivery-conflict-fallback"))
        .await
        .unwrap();
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(
                &pool_for_check,
                GITHUB_MERGE_CONFLICT,
                "delivery-conflict-fallback"
            )
            .await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        message_count(&pool, responder).await,
        1,
        "conflict audience falls back to the remaining active Binding"
    );

    state
        .adapter_delivery
        .admit(post_merge_request("delivery-post-merge-fallback", 8))
        .await
        .unwrap();
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(
                &pool_for_check,
                GITHUB_POST_MERGE,
                "delivery-post-merge-fallback"
            )
            .await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    assert_eq!(
        message_count(&pool, responder).await,
        2,
        "post-merge audience falls back to the remaining active Binding"
    );

    let before_no_binding = requests.lock().await.len();
    sqlx::query("UPDATE task_bindings SET active = false")
        .execute(&pool)
        .await
        .expect("deactivate every Binding");
    state
        .adapter_delivery
        .admit(merge_conflict_request("delivery-conflict-no-binding"))
        .await
        .unwrap();
    let pool_for_check = pool.clone();
    run_worker_until(&state, Duration::from_secs(30), async || {
        matches!(
            status_of(
                &pool_for_check,
                GITHUB_MERGE_CONFLICT,
                "delivery-conflict-no-binding"
            )
            .await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;

    let recorded = requests.lock().await.clone();
    assert_eq!(
        recorded.len(),
        before_no_binding,
        "a conflict check with no active Binding spends zero GitHub calls"
    );
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.uri.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/repos/acme/widgets/pulls/42",
            "/repos/acme/widgets",
            "/repos/acme/widgets/commits/deadbeef/pulls",
            "/repos/acme/widgets",
            "/repos/acme/widgets/pulls/42",
            "/repos/acme/widgets",
            "/repos/acme/widgets/commits/deadbeef/pulls",
        ],
        "each fallback workflow keeps the same request budget and order"
    );
    assert!(
        recorded
            .iter()
            .all(|request| request.authorization.as_deref() == Some("Bearer global-token")),
        "durable GitHub work keeps the current global-token fallback: {recorded:?}"
    );
}

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
    let state_a = replica(&pool).await;
    let state_b = replica(&pool).await;

    let first = deliver(&state_a, "issue_comment", "delivery-1", &payload).await;
    let redelivery = deliver(&state_b, "issue_comment", "delivery-1", &payload).await;

    // Neither is dismissed at the door. The second one is processed in full and
    // simply finds the work already admitted.
    assert_eq!(first, StatusCode::OK);
    assert_eq!(
        redelivery,
        StatusCode::OK,
        "the replica that never saw the first delivery still accepts this one"
    );

    assert_eq!(
        count_records(&pool, GITHUB_PR_MESSAGE, "delivery-1").await,
        1,
        "two replicas, one delivery id, one unit of work"
    );
    // The same delivery also derives a merge-conflict check. Different kind,
    // so it is its own record — and it, too, is admitted exactly once.
    assert_eq!(
        count_records(&pool, GITHUB_MERGE_CONFLICT, "delivery-1").await,
        1
    );

    // And the effect really happened once: the work executes on its first
    // attempt and the bound task hears the comment a single time.
    let pool_for_check = pool.clone();
    run_worker_until(&state_a, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, GITHUB_PR_MESSAGE, "delivery-1").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;

    let (status, attempts) = status_of(&pool, GITHUB_PR_MESSAGE, "delivery-1")
        .await
        .expect("record");
    assert_eq!(status, "succeeded");
    assert_eq!(attempts, 1, "the redelivery cost no second execution");
    let pool_for_check = pool.clone();
    run_worker_until(&state_a, Duration::from_secs(30), async || {
        matches!(
            status_of(&pool_for_check, GITHUB_MERGE_CONFLICT, "delivery-1").await,
            Some((ref status, _)) if status == "succeeded" || status == "failed"
        )
    })
    .await;
    let (status, attempts) = status_of(&pool, GITHUB_MERGE_CONFLICT, "delivery-1")
        .await
        .expect("second planned kind record");
    assert_eq!(status, "succeeded");
    assert_eq!(
        attempts, 1,
        "the second planned kind from the redelivery executes exactly once"
    );
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

    let state_a = replica(&pool).await;
    let state_b = replica(&pool).await;

    // 1. PR status, derived from `pull_request.{state,merged,draft}`.
    let opened = pull_request_payload("opened", "open", false);
    deliver(&state_a, "pull_request", "delivery-pr", &opened).await;
    let after_first = wait_for(&pool, |s| s.0.is_some()).await;
    assert_eq!(after_first.0.as_deref(), Some("open"));

    clear_persisted_pr_state(&pool).await;
    let redelivery = deliver(&state_a, "pull_request", "delivery-pr", &opened).await;
    assert_eq!(
        redelivery,
        StatusCode::OK,
        "a redelivery is accepted at the production HTTP boundary"
    );
    let after_redelivery = wait_for(&pool, |s| s.0.is_some()).await;
    assert_eq!(
        after_redelivery, after_first,
        "the same payload derives the same PR status on a replay"
    );

    // The same value again, this time from a replica that never saw the first.
    clear_persisted_pr_state(&pool).await;
    deliver(&state_b, "pull_request", "delivery-pr", &opened).await;
    let after_other_replica = wait_for(&pool, |s| s.0.is_some()).await;
    assert_eq!(after_other_replica, after_first);

    // 2. Review decision, derived from `review.state`.
    let approved = review_payload(555, "approved");
    deliver(
        &state_a,
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
