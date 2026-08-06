#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on, here \
              and in the shared `common` module compiled into this binary; both lints only \
              fire when the db-tests feature is enabled"
)]
//! What each Linear workflow does, and what it costs.
//!
//! Characterization for stages 12–15 of `.plans/linear-adapter-breakup.md`.
//! Everything here passes before the move and must pass unchanged after it —
//! these are not tests of the new modules, they are the definition of "nothing
//! observable changed".
//!
//! # Why the call budgets
//!
//! Half of this file asserts *how many* Linear calls a workflow made, and in
//! what order. That is not incidental rigour: `agent_api.rs` had no base-URL
//! injection until this stage, so no test could observe a Linear call at all,
//! and a refactor that added a round trip to an unchanged workflow passed every
//! test in the repository. The budgets in the plan's API-call table are the
//! only thing that catches it — a heartbeat emitted twice, an issue fetched per
//! rule instead of once, an attachment linked on a path that used not to.
//!
//! Every call goes to `POST /graphql`, so the budget is read off the GraphQL
//! operation in each recorded body (`ProviderStub::graphql_operations`). A call
//! that is not GraphQL — an OAuth token refresh, say — is reported by its path
//! rather than hidden, because it is still a round trip.
//!
//! # Why a stub and not an unreachable address
//!
//! The neighbouring identity tests point Linear at a dead port, which proves
//! the forwards survive an unreachable sender. That cannot show what was
//! *attempted*. Here the sender answers, so the attempts are observable — and
//! so the success paths downstream of a call (the binding written after
//! `agentSessionUpdate`, the comment id returned by `commentCreate`) actually
//! run.

use crate::common;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::{Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use sqlx::Row;

use moatless_vibe_backend::adapters::linear::adapter::LinearAdapter;
use moatless_vibe_backend::adapters::linear::handlers::linear_webhook;
use moatless_vibe_backend::adapters::models::{RoutingMode, SubscriptionMatch, TaskBinding};
use moatless_vibe_backend::adapters::test_support::ProviderStub;
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_core::scope::Scope;
use moatless_vibe_backend::state::AppState;
use moatless_vibe_backend::task::dao as task_dao;

use common::{
    drain_linear_adapter_delivery, linear_config, make_repository_row, make_task_row,
    seed_linear_app, seed_team, test_app_state_with_linear, test_config, USER_1,
};

const SECRET: &str = "linear-webhook-secret";
const ORG: &str = "org-1";
const TEAM: &str = "team-1";
const ISSUE: &str = "issue-1";
const CONNECTION: &str = "conn-linear-1";
const SESSION: &str = "session-1";
const TASK: &str = "task-linear-1";
const RULE: &str = "trigger-linear-1";

type HmacSha256 = Hmac<Sha256>;

// =========================================================================
// The stub sender
// =========================================================================

/// A Linear that answers every operation successfully.
///
/// One union body serves every call, because a GraphQL response is keyed by
/// field name: a request for `agentActivityCreate` reads
/// `/data/agentActivityCreate` and ignores the rest. That keeps the stub from
/// having to parse the request, and keeps this file's budget assertions about
/// *what was called* rather than about what the stub knows.
///
/// The issue it returns carries no project and no labels, which is the shape
/// the rule filters in this file expect.
async fn linear_stub() -> ProviderStub {
    ProviderStub::start(|_path| {
        serde_json::json!({
            "data": {
                "agentActivityCreate": {"success": true, "agentActivity": {"id": "activity-stub"}},
                "agentSessionUpdate": {"success": true},
                "attachmentLinkURL": {"success": true, "attachment": {"id": "att-1", "url": "u"}},
                "commentCreate": {
                    "success": true,
                    "comment": {"id": "comment-stub", "url": "u", "body": "b", "createdAt": "now"},
                },
                "issueUpdate": {"success": true, "issue": {"id": ISSUE, "identifier": "ENG-7"}},
                "issue": {
                    "id": ISSUE,
                    "identifier": "ENG-7",
                    "title": "Fix it",
                    "url": "https://linear.app/acme/issue/ENG-7",
                    "project": null,
                    "labels": {"nodes": []},
                },
                "workflowStates": {"nodes": [{"id": "state-started", "name": "In Progress"}]},
            }
        })
    })
    .await
}

/// A replica whose Linear clients all speak to `stub`.
///
/// The base URL travels on `Config`, because a Linear client is built deep
/// inside webhook and worker paths that hold only an `AppState` — there is no
/// other seam through which a workflow's calls can be redirected, and a
/// workflow that cannot be redirected cannot be budgeted.
fn replica(pool: &PgPool, stub: &ProviderStub) -> AppState {
    let mut config = test_config();
    config.linear = linear_config(Some(SECRET), Some(stub.base_url()));
    let linear = Arc::new(LinearAdapter::new(&config.linear, pool.clone()));
    test_app_state_with_linear(pool.clone(), linear, config)
}

// =========================================================================
// Delivery
// =========================================================================

async fn deliver(state: &AppState, envelope: &serde_json::Value) -> StatusCode {
    let body = serde_json::to_vec(envelope).expect("serialize envelope");
    let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).expect("hmac key");
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let mut headers = HeaderMap::new();
    headers.insert("linear-signature", signature.parse().expect("header value"));

    linear_webhook(
        State(state.clone()),
        State(state.provider_handles.linear.clone()),
        headers,
        Bytes::from(body),
    )
    .await
    .expect("Linear is acknowledged")
}

/// Run the real worker until every admitted record is terminal.
async fn drain(pool: &PgPool, stub: &ProviderStub) {
    drain_linear_adapter_delivery(pool, replica(pool, stub)).await;
}

// =========================================================================
// Fixtures
// =========================================================================

/// The connection, plus the `adapter_apps` row its client is resolved from.
///
/// Both are required before any Linear call leaves the process: the connection
/// names the workspace, and the app row is where `api_for_connection` reads the
/// `api_base_url` that points the client at `stub`. Without the app row the
/// client cannot be built, every emitter skips its call best-effort, and each
/// budget below reads as zero.
async fn seed_connection(pool: &PgPool, stub: &ProviderStub) {
    sqlx::query(
        "INSERT INTO adapter_connections \
         (id, adapter_kind, connection_kind, external_account_id, created_by, scope, owner_user_id) \
         VALUES ($1, 'linear', 'org', $2, $3, 'user', $3)",
    )
    .bind(CONNECTION)
    .bind(ORG)
    .bind(USER_1)
    .execute(pool)
    .await
    .expect("seed Linear connection");
    seed_linear_app(pool, stub.base_url()).await;
}

/// The workspace OAuth token every Linear call resolves before it reaches HTTP.
///
/// Without it no request is ever made and every budget below reads as zero —
/// a green test that proves nothing. Deliberately far from expiry so no call
/// triggers the token-refresh round trip, which would show up in the budget as
/// a `POST /oauth/token` it is not this file's job to assert.
async fn seed_workspace_token(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO provider_tokens \
         (provider, auth_method, access_token, token_type, connected_at, updated_at, \
          scope, owner_user_id, expires_at) \
         VALUES ('linear', 'oauth', 'stub-access-token', 'Bearer', NOW(), NOW(), \
                 'global', NULL, $1)",
    )
    .bind(Utc::now() + Duration::days(30))
    .execute(pool)
    .await
    .expect("seed Linear workspace token");
}

/// A repository that needs no GitHub preflight, so task creation succeeds.
///
/// `make_repository_row` produces a GitHub-backed repository, and creating a
/// task against one with no token available fails preflight. That failure has
/// its own budget row below; it must not be every test's silent default.
async fn seed_local_repository(pool: &PgPool) {
    seed_team(pool, &format!("personal-{USER_1}")).await;
    let mut repo = make_repository_row("repo-1", USER_1);
    repo.provider = None;
    repo.remote_url = None;
    repository_dao::insert(pool, &repo)
        .await
        .expect("seed repository");
}

/// A GitHub-backed repository with no reachable credentials: creating a task
/// against it fails preflight.
async fn seed_preflight_failing_repository(pool: &PgPool) {
    seed_team(pool, &format!("personal-{USER_1}")).await;
    repository_dao::insert(pool, &make_repository_row("repo-1", USER_1))
        .await
        .expect("seed repository");
}

/// An active Linear Subscription for the team.
async fn seed_rule(state: &AppState, config: Option<serde_json::Value>) {
    seed_rule_with_id(state, RULE, config).await;
}

/// A second Subscription, so a binding can name a rule that is not the one the
/// close path is acting for.
async fn seed_rule_with_id(state: &AppState, id: &str, config: Option<serde_json::Value>) {
    let now = Utc::now();
    state
        .loops
        .upsert_subscription(&SubscriptionMatch {
            id: id.to_string(),
            adapter_kind: "linear".to_string(),
            connection_id: CONNECTION.to_string(),
            source_matcher: TEAM.to_string(),
            source_name: Some("Engineering".to_string()),
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            repository_id: "repo-1".to_string(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            // A Subscription with no run-as user is invisible to every lookup
            // that resolves rules — `run_as_user_id IS NOT NULL` is in the
            // query. Without it the rule exists and matches nothing.
            run_as_user_id: Some(USER_1.to_string()),
            agent_type: "claude-code".to_string(),
            inactive_ttl_days: None,
            allow_direct_send: false,
            // `close_task_on_close` is a scalar column on the Subscription, not
            // Linear config — mirrored out of the same literal so a test reads
            // as one rule rather than two half-specified ones.
            close_task_on_close: config
                .as_ref()
                .and_then(|c| c.get("close_task_on_close"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            adapter_config: config,
            ongoing_task_id: None,
            notify_on_success: false,
            active: true,
            created_by: USER_1.to_string(),
            scope: Scope::User,
            owner_user_id: Some(USER_1.to_string()),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("seed Linear rule");
}

/// A task with one active binding of `subject_kind` on `subject_key`.
#[allow(clippy::too_many_arguments)]
async fn seed_binding(
    state: &AppState,
    pool: &PgPool,
    binding_id: &str,
    task_id: &str,
    subject_kind: &str,
    subject_key: &str,
    destination_config: serde_json::Value,
    loop_id: Option<&str>,
    origin: &str,
) {
    task_dao::insert_task(pool, &make_task_row(task_id, USER_1))
        .await
        .expect("seed task");

    let now = Utc::now();
    state
        .task_bindings
        .upsert(&TaskBinding {
            id: binding_id.to_string(),
            task_id: task_id.to_string(),
            adapter_kind: "linear".to_string(),
            subject_kind: subject_kind.to_string(),
            subject_key: subject_key.to_string(),
            connection_id: CONNECTION.to_string(),
            loop_id: loop_id.map(str::to_string),
            destination_config,
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: origin.to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        })
        .await
        .expect("seed binding");
}

/// The Subscription and then a session bound under it.
///
/// Order matters: a binding's `loop_id` references the Subscription, so the
/// rule has to exist first.
async fn seed_rule_then_session(
    state: &AppState,
    pool: &PgPool,
    config: Option<serde_json::Value>,
) {
    seed_rule(state, config).await;
    seed_bound_session(state, pool).await;
}

/// A bound agent session on the issue — the common fixture.
async fn seed_bound_session(state: &AppState, pool: &PgPool) {
    seed_binding(
        state,
        pool,
        "tb-session-1",
        TASK,
        "linear_agent_session",
        SESSION,
        serde_json::json!({"session_id": SESSION, "issue_id": ISSUE}),
        Some(RULE),
        moatless_vibe_backend::adapters::models::ADAPTER_EVENT_BINDING_ORIGIN,
    )
    .await;
}

// =========================================================================
// Envelopes
// =========================================================================

fn session_created() -> serde_json::Value {
    serde_json::json!({
        "action": "created",
        "type": "AgentSessionEvent",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_000i64,
        "agentSession": {
            "id": SESSION,
            "issue": {
                "id": ISSUE,
                "identifier": "ENG-7",
                "title": "Fix it",
                "teamId": TEAM,
                "url": "https://linear.app/acme/issue/ENG-7",
                "state": {"id": "s0", "name": "Todo", "type": "unstarted"},
            },
        },
    })
}

fn session_prompted(activity_id: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "prompted",
        "type": "AgentSessionEvent",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_001i64,
        "agentSession": {
            "id": SESSION,
            "issue": {"id": ISSUE, "identifier": "ENG-7", "title": "Fix it", "teamId": TEAM},
        },
        "agentActivity": {"id": activity_id, "type": "prompt", "body": body},
    })
}

fn comment_created(comment_id: &str, parent_id: Option<&str>, body: &str) -> serde_json::Value {
    let mut data = serde_json::json!({
        "id": comment_id,
        "body": body,
        "issueId": ISSUE,
        "issue": {"id": ISSUE, "identifier": "ENG-7", "title": "Fix it", "teamId": TEAM},
        "user": {"id": "user-linear-1", "name": "Ada"},
    });
    if let Some(parent) = parent_id {
        data["parentId"] = serde_json::json!(parent);
    }
    serde_json::json!({
        "action": "create",
        "type": "Comment",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_002i64,
        "data": data,
    })
}

/// An issue update. `from` equal to `to` is the no-transition case.
fn issue_updated(from: &str, to: &str, to_name: &str, to_kind: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "update",
        "type": "Issue",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_003i64,
        "updatedFrom": {"stateId": from},
        "data": {
            "id": ISSUE,
            "identifier": "ENG-7",
            "title": "Fix it",
            "teamId": TEAM,
            "projectId": "project-1",
            "url": "https://linear.app/acme/issue/ENG-7",
            "state": {"id": to, "name": to_name, "type": to_kind},
        },
    })
}

// =========================================================================
// Assertions on state
// =========================================================================

async fn task_status(pool: &PgPool, task_id: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM tasks WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .expect("read task status")
}

/// The durable Sandbox intent for a task. `None` means no Sandbox was ever
/// requested — what a task nobody woke looks like.
async fn sandbox_desired_state(pool: &PgPool, task_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT desired_state FROM sandboxes WHERE task_id = $1")
        .bind(task_id)
        .fetch_optional(pool)
        .await
        .expect("read sandbox intent")
}

/// Every external id the Linear adapter has written onto a forwarded message,
/// oldest first. The literal strings are the persisted formats.
async fn forwarded_ids(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT external_event_id FROM task_messages \
         WHERE external_adapter = 'linear' AND external_event_id IS NOT NULL ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
    .expect("read forwarded messages")
}

/// The tasks that received a forwarded Linear message.
async fn forwarded_task_ids(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT task_id FROM task_messages \
         WHERE external_adapter = 'linear' ORDER BY task_id ASC",
    )
    .fetch_all(pool)
    .await
    .expect("read forwarded message tasks")
}

/// `(status, state_kind)` persisted onto each binding, by binding id.
async fn binding_status(pool: &PgPool) -> Vec<(String, Option<String>, Option<String>)> {
    sqlx::query(
        "SELECT id, external_status, external_state_kind FROM task_bindings ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
    .expect("read binding status")
    .into_iter()
    .map(|row| {
        (
            row.get::<String, _>("id"),
            row.get::<Option<String>, _>("external_status"),
            row.get::<Option<String>, _>("external_state_kind"),
        )
    })
    .collect()
}

// =========================================================================
// AgentSessionEvent.created — the response path
// =========================================================================

/// Exactly one `agentActivityCreate` before the `200`, and nothing else.
///
/// This is the one workflow that still calls Linear on the response path, and
/// deliberately: the session panel goes dark after ten seconds. The budget is
/// the guard on that exception — no issue fetch, no attachment, no session
/// update. Those belong to the admitted record, which has not run yet.
#[sqlx::test]
async fn a_created_session_posts_one_heartbeat_and_admits_the_rest(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(&state, None).await;

    assert_eq!(deliver(&state, &session_created()).await, StatusCode::OK);

    assert_eq!(
        stub.graphql_operations(),
        vec!["agentActivityCreate"],
        "the response path makes exactly one Linear call"
    );

    let pending: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM adapter_delivery WHERE status = 'pending'")
            .fetch_all(&pool)
            .await
            .expect("read admitted work");
    assert_eq!(
        pending,
        vec!["linear.session_provision"],
        "everything past the heartbeat is admitted, not run"
    );
}

/// No Subscription, no admission — and the session is closed rather than left
/// spinning.
///
/// A session Linear opened must not sit `active` forever while nothing runs
/// behind it. No provisioning is admitted, so nothing downstream will post a
/// terminal activity; the drop itself closes the session with a terminal
/// `error` so the panel reaches a resolved state.
#[sqlx::test]
async fn a_created_session_with_no_rule_closes_the_session(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;

    assert_eq!(deliver(&state, &session_created()).await, StatusCode::OK);

    assert_eq!(stub.graphql_operations(), vec!["agentActivityCreate"]);
    let content_type = serde_json::from_str::<serde_json::Value>(&stub.requests()[0].body)
        .expect("stub recorded a JSON body")
        .pointer("/variables/input/content/type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    assert_eq!(
        content_type.as_deref(),
        Some("error"),
        "the session is closed with a terminal error, not left on the ephemeral heartbeat"
    );
    let admitted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM adapter_delivery")
        .fetch_one(&pool)
        .await
        .expect("count admitted work");
    assert_eq!(admitted, 0, "no rule means nothing durable");
}

// =========================================================================
// session_provision — the worker
// =========================================================================

/// The provisioning budget for an unscoped rule with no start state.
///
/// No `issue` fetch, because no rule filters on a project or a label — the
/// selector takes the cheap path. Then the two links, in order: the session's
/// external URL first, the issue attachment second. No `issueUpdate`, because
/// the rule configures no start state.
#[sqlx::test]
async fn provisioning_an_unscoped_session_makes_two_calls_in_order(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(&state, None).await;

    deliver(&state, &session_created()).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec![
            "agentActivityCreate",
            "agentSessionUpdate",
            "attachmentLinkURL",
        ],
        "one heartbeat on the response path, two links in the worker"
    );

    let bound: Vec<String> = sqlx::query_scalar(
        "SELECT subject_key FROM task_bindings WHERE adapter_kind = 'linear' AND active",
    )
    .fetch_all(&pool)
    .await
    .expect("read bindings");
    assert_eq!(bound, vec![SESSION], "the session is bound to a new task");
    assert_eq!(
        forwarded_ids(&pool).await,
        vec![format!("linear:session:created:{SESSION}")],
        "the inbound prompt carries the session-created external id"
    );
}

/// A rule scoped to a project pays for one `issue` fetch, and only one.
///
/// The fetch exists to learn the issue's project and labels, which the webhook
/// payload does not carry. It is skipped entirely when no rule needs it (the
/// test above) — that asymmetry is the budget row, and it is invisible without
/// an observable sender.
#[sqlx::test]
async fn provisioning_a_project_scoped_session_fetches_the_issue_once(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({"project_ids": ["project-1"]})),
    )
    .await;

    deliver(&state, &session_created()).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations()
            .iter()
            .filter(|op| *op == "issue")
            .count(),
        1,
        "the issue is fetched once, not per rule: {:?}",
        stub.graphql_operations()
    );
}

/// A rule with a start state the issue is not already in pays for one
/// `issueUpdate`, last.
#[sqlx::test]
async fn provisioning_moves_the_issue_to_the_start_state_once(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({"start_state": "In Progress"})),
    )
    .await;

    deliver(&state, &session_created()).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec![
            "agentActivityCreate",
            "agentSessionUpdate",
            "attachmentLinkURL",
            "workflowStates",
            "issueUpdate",
        ],
        "the board move is last, and resolving the state name is part of its cost"
    );
}

/// A start state the issue is already in costs nothing.
///
/// The guard matters beyond the round trip: a loop whose start state overlaps
/// its trigger states would otherwise move the issue to where it already is,
/// and every move is another `Issue.update` webhook.
#[sqlx::test]
async fn provisioning_skips_the_board_move_when_the_issue_is_already_there(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(&state, Some(serde_json::json!({"start_state": "Todo"}))).await;

    deliver(&state, &session_created()).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec![
            "agentActivityCreate",
            "agentSessionUpdate",
            "attachmentLinkURL",
        ],
        "no state resolution and no update when the issue is already in the start state"
    );
}

/// A second execution of the same record provisions nothing a second time.
///
/// The record is at-least-once — an attempt abandoned at its deadline is
/// claimed again — so convergence is the binding guard, not admission.
#[sqlx::test]
async fn a_second_provisioning_attempt_creates_no_second_task(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(&state, None).await;

    deliver(&state, &session_created()).await;
    drain(&pool, &stub).await;

    // Re-run the admitted work by hand: reset the record to pending and drain
    // again, which is what a restart mid-attempt produces.
    sqlx::query(
        "UPDATE adapter_delivery SET status = 'pending', attempts = 0, next_attempt_at = NOW()",
    )
    .execute(&pool)
    .await
    .expect("re-arm the record");
    drain(&pool, &stub).await;

    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(&pool)
        .await
        .expect("count tasks");
    assert_eq!(tasks, 1, "the binding guard makes the retry a no-op");
}

// =========================================================================
// AgentSessionEvent.prompted
// =========================================================================

/// One heartbeat per active session binding, no issue fetch, and the prompt
/// forwarded under the activity's id.
#[sqlx::test]
async fn a_prompt_heartbeats_once_and_forwards_under_the_activity_id(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;

    deliver(&state, &session_prompted("activity-1", "try again")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec!["agentActivityCreate"],
        "one heartbeat, and nothing fetched"
    );
    assert_eq!(
        forwarded_ids(&pool).await,
        vec!["linear:prompt:activity-1"],
        "the prompt is identified by the activity carrying it"
    );
}

/// A closed task's binding is skipped: no heartbeat, no forward.
///
/// A closed task is terminal. Reaching it with a follow-up prompt would
/// resurrect work someone deliberately ended.
#[sqlx::test]
async fn a_prompt_to_a_deactivated_binding_reaches_nothing(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;
    state
        .task_bindings
        .deactivate_by_task(TASK)
        .await
        .expect("deactivate the binding");

    deliver(&state, &session_prompted("activity-2", "try again")).await;
    drain(&pool, &stub).await;

    assert!(
        stub.graphql_operations().is_empty(),
        "no active binding, no call: {:?}",
        stub.graphql_operations()
    );
    assert!(forwarded_ids(&pool).await.is_empty());
}

// =========================================================================
// Comment.create
// =========================================================================

/// An issue-level comment reaches every task bound to the issue, and heartbeats
/// only into the ones that have a Linear session.
///
/// The asymmetry is the budget row: an issue binding has no session panel to
/// keep alive, so it costs no call.
#[sqlx::test]
async fn a_comment_heartbeats_only_session_bindings_and_reaches_both(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;
    seed_binding(
        &state,
        &pool,
        "tb-issue-1",
        "task-linear-2",
        "linear_issue",
        ISSUE,
        serde_json::json!({"issue_id": ISSUE}),
        Some(RULE),
        moatless_vibe_backend::adapters::models::ADAPTER_EVENT_BINDING_ORIGIN,
    )
    .await;

    deliver(&state, &comment_created("comment-1", None, "please look")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec!["agentActivityCreate"],
        "one heartbeat for the session binding, none for the issue binding"
    );
    assert_eq!(
        forwarded_task_ids(&pool).await,
        vec![TASK.to_string(), "task-linear-2".to_string()],
        "an issue-level comment is for everyone bound to the issue"
    );
    assert_eq!(
        forwarded_ids(&pool).await.first().map(String::as_str),
        Some("linear:comment:comment-1"),
        "the comment's persisted identity is its Linear comment id"
    );
}

/// One task holding both a session binding and an issue binding on the same
/// issue receives the comment once.
#[sqlx::test]
async fn a_task_bound_twice_to_one_issue_receives_a_comment_once(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;

    // A second binding on the same task — the issue-level one a direct send
    // would have created.
    let now = Utc::now();
    state
        .task_bindings
        .upsert(&TaskBinding {
            id: "tb-issue-dup".to_string(),
            task_id: TASK.to_string(),
            adapter_kind: "linear".to_string(),
            subject_kind: "linear_issue".to_string(),
            subject_key: ISSUE.to_string(),
            connection_id: CONNECTION.to_string(),
            loop_id: Some(RULE.to_string()),
            destination_config: serde_json::json!({"issue_id": ISSUE}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: moatless_vibe_backend::adapters::models::ADAPTER_EVENT_BINDING_ORIGIN
                .to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        })
        .await
        .expect("seed second binding");

    deliver(&state, &comment_created("comment-2", None, "please look")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        forwarded_ids(&pool).await,
        vec!["linear:comment:comment-2"],
        "one delivery per task, not per binding"
    );
}

/// A comment carrying the `Moatless task:` footer of the task that wrote it is
/// not forwarded back to that task.
///
/// The echo filter is what stops an agent talking to itself. It works off the
/// task id recovered from the footer link, so this pins the footer's role as
/// provenance rather than decoration.
#[sqlx::test]
async fn a_task_does_not_receive_back_its_own_comment(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;

    let frontend = state
        .config
        .auth
        .frontend_url
        .trim_end_matches('/')
        .to_string();
    let body = format!("done\n\n---\nMoatless task: [T]({frontend}/tasks/{TASK})");
    deliver(&state, &comment_created("comment-3", None, &body)).await;
    drain(&pool, &stub).await;

    assert!(
        forwarded_ids(&pool).await.is_empty(),
        "the authoring task is excluded, and it was the only recipient"
    );
    assert!(
        stub.graphql_operations().is_empty(),
        "no recipient means no heartbeat either"
    );
}

// =========================================================================
// Issue.update
// =========================================================================

/// An update that does not change the workflow state does nothing at all.
///
/// Linear sends `Issue.update` for a title edit, a label change, an assignee —
/// this Adapter acts on state transitions only. The guard is first, before the
/// status write, so a non-transition costs neither a call nor a row.
#[sqlx::test]
async fn an_issue_update_with_no_transition_does_nothing(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;

    deliver(&state, &issue_updated("s0", "s0", "Todo", "unstarted")).await;
    drain(&pool, &stub).await;

    assert!(
        stub.graphql_operations().is_empty(),
        "no transition, no call: {:?}",
        stub.graphql_operations()
    );
    assert!(forwarded_ids(&pool).await.is_empty());
    assert_eq!(
        binding_status(&pool).await,
        vec![("tb-session-1".to_string(), None, None)],
        "no transition, no status write"
    );
}

/// A real transition persists the new state onto every binding for the issue
/// and forwards a notification, and fetches nothing.
///
/// Zero `issue` fetches is the budget row: no candidate rule filters on labels
/// and the payload carries the project, so there is nothing to look up.
#[sqlx::test]
async fn an_issue_transition_persists_status_and_forwards_without_fetching(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;

    deliver(&state, &issue_updated("s0", "s1", "In Progress", "started")).await;
    drain(&pool, &stub).await;

    assert!(
        stub.graphql_operations().is_empty(),
        "a plain transition costs no Linear call: {:?}",
        stub.graphql_operations()
    );
    assert_eq!(
        binding_status(&pool).await,
        vec![(
            "tb-session-1".to_string(),
            Some("In Progress".to_string()),
            Some("started".to_string()),
        )],
        "the new workflow state is persisted onto the binding"
    );
    assert_eq!(
        forwarded_ids(&pool).await,
        vec![format!("linear:issue-state:{ISSUE}:1700000000003")],
        "the state change is identified by the issue and Linear's delivery timestamp"
    );
}

/// Forwarding a state change also wakes the task that has to act on it.
///
/// The message alone is not enough: a pending user message nobody was notified
/// about never opens a turn, so the Task keeps reading as working — its newest
/// user message stays newer than its last turn — while no agent ever runs.
#[sqlx::test]
async fn an_issue_transition_wakes_the_task_it_forwards_to(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(&state, &pool, None).await;

    deliver(&state, &issue_updated("s0", "s1", "In Progress", "started")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        sandbox_desired_state(&pool, TASK).await.as_deref(),
        Some("running"),
        "the forwarded state change wakes the bound task's sandbox"
    );
}

/// A terminal transition with `close_task_on_close` closes the bound task and
/// posts one `response` activity into its session.
#[sqlx::test]
async fn a_terminal_transition_closes_the_task_and_posts_one_response(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(
        &state,
        &pool,
        Some(serde_json::json!({"close_task_on_close": true, "terminal_states": ["completed"]})),
    )
    .await;

    deliver(&state, &issue_updated("s0", "s9", "Done", "completed")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec!["agentActivityCreate"],
        "one close response into the bound session, and nothing else"
    );
    assert_eq!(task_status(&pool, TASK).await, "closed");
}

/// The same transition without `close_task_on_close` leaves the task open.
#[sqlx::test]
async fn a_terminal_transition_without_the_flag_leaves_the_task_open(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(
        &state,
        &pool,
        Some(serde_json::json!({"terminal_states": ["completed"]})),
    )
    .await;

    deliver(&state, &issue_updated("s0", "s9", "Done", "completed")).await;
    drain(&pool, &stub).await;

    assert!(
        stub.graphql_operations().is_empty(),
        "no close means no close response: {:?}",
        stub.graphql_operations()
    );
    assert_eq!(task_status(&pool, TASK).await, "open");
}

// =========================================================================
// Close eligibility — each rejection separately
// =========================================================================

/// A task with a second binding is left open.
///
/// The Linear issue closing says nothing about the Slack thread or the GitHub
/// PR the same task is also bound to.
#[sqlx::test]
async fn a_task_with_another_binding_is_not_closed(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(
        &state,
        &pool,
        Some(serde_json::json!({"close_task_on_close": true, "terminal_states": ["completed"]})),
    )
    .await;

    let now = Utc::now();
    state
        .task_bindings
        .upsert(&TaskBinding {
            id: "tb-sibling".to_string(),
            task_id: TASK.to_string(),
            adapter_kind: "linear".to_string(),
            subject_kind: "linear_issue".to_string(),
            subject_key: "issue-other".to_string(),
            connection_id: CONNECTION.to_string(),
            loop_id: Some(RULE.to_string()),
            destination_config: serde_json::json!({"issue_id": "issue-other"}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: moatless_vibe_backend::adapters::models::ADAPTER_EVENT_BINDING_ORIGIN
                .to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        })
        .await
        .expect("seed sibling binding");

    deliver(&state, &issue_updated("s0", "s9", "Done", "completed")).await;
    drain(&pool, &stub).await;

    assert_eq!(task_status(&pool, TASK).await, "open");
}

/// A binding created by something other than a Linear event is left open — an
/// operator who bound a task to an issue by hand did not ask for auto-close.
#[sqlx::test]
async fn a_binding_from_a_different_origin_is_not_closed(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({"close_task_on_close": true, "terminal_states": ["completed"]})),
    )
    .await;
    seed_binding(
        &state,
        &pool,
        "tb-session-1",
        TASK,
        "linear_agent_session",
        SESSION,
        serde_json::json!({"session_id": SESSION, "issue_id": ISSUE}),
        Some(RULE),
        "manual",
    )
    .await;

    deliver(&state, &issue_updated("s0", "s9", "Done", "completed")).await;
    drain(&pool, &stub).await;

    assert_eq!(task_status(&pool, TASK).await, "open");
}

/// A binding created by a different Subscription is left open.
#[sqlx::test]
async fn a_binding_from_a_different_rule_is_not_closed(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({"close_task_on_close": true, "terminal_states": ["completed"]})),
    )
    .await;
    seed_rule_with_id(&state, "trigger-linear-2", None).await;
    seed_binding(
        &state,
        &pool,
        "tb-session-1",
        TASK,
        "linear_agent_session",
        SESSION,
        serde_json::json!({"session_id": SESSION, "issue_id": ISSUE}),
        Some("trigger-linear-2"),
        moatless_vibe_backend::adapters::models::ADAPTER_EVENT_BINDING_ORIGIN,
    )
    .await;

    deliver(&state, &issue_updated("s0", "s9", "Done", "completed")).await;
    drain(&pool, &stub).await;

    assert_eq!(task_status(&pool, TASK).await, "open");
}

/// An already-closed task is left alone: no second close, and no close response
/// posted into a session nobody is watching.
#[sqlx::test]
async fn an_already_closed_task_gets_no_second_close(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(
        &state,
        &pool,
        Some(serde_json::json!({"close_task_on_close": true, "terminal_states": ["completed"]})),
    )
    .await;
    sqlx::query("UPDATE tasks SET status = 'closed' WHERE task_id = $1")
        .bind(TASK)
        .execute(&pool)
        .await
        .expect("close the task up front");

    deliver(&state, &issue_updated("s0", "s9", "Done", "completed")).await;
    drain(&pool, &stub).await;

    assert!(
        stub.graphql_operations().is_empty(),
        "an already-closed task gets no close response: {:?}",
        stub.graphql_operations()
    );
}

// =========================================================================
// Status-entry creation
// =========================================================================

/// A rule opted into `create_on_status_entry` spawns a task when the issue
/// enters a trigger state, links it back, and forwards the read-the-ticket
/// prompt under the issue-created id.
#[sqlx::test]
async fn a_status_entry_creates_a_task_and_links_it_back(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({
            "create_on_status_entry": true,
            "trigger_states": ["In Progress"],
        })),
    )
    .await;

    deliver(&state, &issue_updated("s0", "s1", "In Progress", "started")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec!["attachmentLinkURL"],
        "one attachment, and no comment because preflight succeeded"
    );
    assert_eq!(
        forwarded_ids(&pool).await,
        vec![format!("linear:issue:created:{ISSUE}")],
        "the issue-initiated prompt carries the issue-created external id"
    );
}

/// A preflight failure posts exactly one `commentCreate` and creates no
/// binding.
///
/// The status-entry path has no agent session to write an error activity into,
/// so the reason reaches the person as an issue comment. It is the only path
/// that posts one, which is why it is a budget row of its own.
#[sqlx::test]
async fn a_status_entry_preflight_failure_posts_one_comment_and_stops(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_preflight_failing_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({
            "create_on_status_entry": true,
            "trigger_states": ["In Progress"],
        })),
    )
    .await;

    deliver(&state, &issue_updated("s0", "s1", "In Progress", "started")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec!["commentCreate"],
        "the reason is posted, and nothing is linked to a task that never started"
    );
    let bindings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_bindings")
        .fetch_one(&pool)
        .await
        .expect("count bindings");
    assert_eq!(bindings, 0, "an errored task gets no binding");
}

/// Re-entering the same state does not create a second task.
#[sqlx::test]
async fn a_second_status_entry_creates_no_second_task(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({
            "create_on_status_entry": true,
            "trigger_states": ["In Progress"],
        })),
    )
    .await;

    deliver(&state, &issue_updated("s0", "s1", "In Progress", "started")).await;
    drain(&pool, &stub).await;

    let mut second = issue_updated("s2", "s1", "In Progress", "started");
    second["webhookTimestamp"] = serde_json::json!(1_700_000_000_004i64);
    deliver(&state, &second).await;
    drain(&pool, &stub).await;

    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(&pool)
        .await
        .expect("count tasks");
    assert_eq!(tasks, 1, "one active binding per (issue, trigger)");
}

/// A rule whose required labels the issue does not carry creates nothing — and
/// the label check costs the one `issue` fetch that answers it.
#[sqlx::test]
async fn a_status_entry_with_unmet_labels_creates_nothing(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({
            "create_on_status_entry": true,
            "trigger_states": ["In Progress"],
            "require_labels": ["security"],
        })),
    )
    .await;

    deliver(&state, &issue_updated("s0", "s1", "In Progress", "started")).await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec!["issue"],
        "one fetch to learn the labels, and nothing after it"
    );
    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(&pool)
        .await
        .expect("count tasks");
    assert_eq!(tasks, 0);
}

// =========================================================================
// AppUserNotification
// =========================================================================

/// The inbox-stream close path reaches the same close as `Issue.update`.
///
/// Used by workspaces that subscribe to `AppUserNotification` and not to
/// `Issue`. It acts on the current state alone — the notification carries no
/// diff — so there is no no-transition guard here to assert.
#[sqlx::test]
async fn a_status_notification_closes_the_task_the_same_way(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_then_session(
        &state,
        &pool,
        Some(serde_json::json!({"close_task_on_close": true, "terminal_states": ["completed"]})),
    )
    .await;

    let envelope = serde_json::json!({
        "action": "issueStatusChanged",
        "type": "AppUserNotification",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_005i64,
        "notification": {
            "id": "notif-1",
            "type": "issueStatusChanged",
            "issueId": ISSUE,
            "issue": {
                "id": ISSUE,
                "identifier": "ENG-7",
                "title": "Fix it",
                "teamId": TEAM,
                "state": {"id": "s9", "name": "Done", "type": "completed"},
            },
        },
    });
    deliver(&state, &envelope).await;
    drain(&pool, &stub).await;

    assert_eq!(stub.graphql_operations(), vec!["agentActivityCreate"]);
    assert_eq!(task_status(&pool, TASK).await, "closed");
}
