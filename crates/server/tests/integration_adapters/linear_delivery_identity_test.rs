#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on, here \
              and in the shared `common` module compiled into this binary; both lints only \
              fire when the db-tests feature is enabled"
)]
//! Two Linear deliveries a person made separately must reach the Agent
//! separately, and one delivery Linear retried must reach it once.
//!
//! These drive the real inbound route — signed body in, `200` out — because the
//! defect they cover lives between the signature check and the handler: the
//! delivery identity the dispatcher derives. A unit test on that identity says
//! the two keys differ; only the route says the second message was actually
//! delivered.
//!
//! The route admits the delivery rather than performing it, so every
//! assertion here runs the real worker afterwards. That is the shape of the
//! guarantee being asserted: the `200` means the delivery is durably owned, and what
//! reaches the Agent is decided by the record, not by the process that answered
//! Linear.
//!
//! Linear's own API is unreachable from here, which is deliberate and matches
//! production behaviour on an expired install: the session heartbeat fails and
//! is logged, and the forward to the task carries on regardless.

use crate::common;
use moatless_types::adapter::AdapterKind;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use moatless_backend::adapters::providers::linear::handlers::linear_webhook;
use moatless_backend::state::AppState;
use moatless_tasks::task::dao as task_dao;
use moatless_types::access_control::Scope;
use moatless_types::adapter::TaskBinding;
use moatless_types::loops::RoutingMode;
use moatless_types::loops::SubscriptionMatch;
use moatless_workspaces::repository::dao as repository_dao;

use common::{
    USER_1, drain_linear_adapter_delivery, linear_config, make_repository_row, make_task_row,
    seed_linear_app, seed_team, test_app_state_with_linear, test_config,
};

const SECRET: &str = "linear-webhook-secret";
const ORG: &str = "org-1";
const TEAM: &str = "team-1";
const ISSUE: &str = "issue-1";
const SESSION: &str = "session-A";
const CONNECTION: &str = "conn-linear-1";
const TASK: &str = "task-linear-1";

type HmacSha256 = Hmac<Sha256>;

/// A backend replica with the Linear adapter installed. It holds no state
/// between deliveries, so two of these stand in for two pods — and for one pod
/// restarted between them.
fn replica(pool: &PgPool) -> AppState {
    let mut config = test_config();
    config.linear = linear_config(Some(SECRET), None);
    test_app_state_with_linear(pool.clone(), config)
}

/// Deliver a signed envelope the way Linear does, and return the status the
/// sender sees.
async fn deliver(state: &AppState, envelope: &serde_json::Value) -> StatusCode {
    let body = serde_json::to_vec(envelope).expect("serialize envelope");

    let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).expect("hmac key");
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());

    let mut headers = HeaderMap::new();
    headers.insert("linear-signature", signature.parse().expect("header value"));

    linear_webhook(
        State(
            state
                .adapter_installations
                .as_ref()
                .map(|installations| installations.linear()),
        ),
        headers,
        Bytes::from(body),
    )
    .await
    .expect("Linear is acknowledged")
}

/// Run the real worker until every admitted record is terminal, then stop it,
/// and assert that nothing was abandoned along the way.
///
/// A delivery is answered once it is durably owned, so nothing has reached the
/// Agent when `deliver` returns. This is the second half of every assertion
/// below.
async fn drain_admitted_work(pool: &PgPool) {
    drain_linear_adapter_delivery(pool, replica(pool)).await;

    let failed: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT kind, event_id, last_error FROM adapter_delivery WHERE status = 'failed'",
    )
    .fetch_all(pool)
    .await
    .expect("read failed records");
    assert!(
        failed.is_empty(),
        "no Linear record should have been abandoned: {failed:?}"
    );
}

/// Every message the Linear adapter has forwarded to the task, oldest first,
/// as `(external_event_id, text)`.
async fn delivered_messages(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT m.external_event_id, m.message_text \
         FROM task_messages m \
         WHERE m.task_id = $1 AND m.external_adapter = 'linear' \
         ORDER BY m.id ASC",
    )
    .bind(TASK)
    .fetch_all(pool)
    .await
    .expect("read forwarded messages")
    .into_iter()
    .map(|(id, text)| (id, text.unwrap_or_default()))
    .collect()
}

/// Whether some forwarded message carries `needle`, regardless of position.
///
/// Nothing below asserts the *order* the messages arrived in, and that is a
/// property of the seam rather than a weakened test: the worker claims a batch
/// ordered by when each record became due and then executes it concurrently, so
/// two records from two deliveries can complete in either order. The behaviour
/// under test is that both arrive and that each keeps its own identity.
fn carries(messages: &[(String, String)], needle: &str) -> bool {
    messages.iter().any(|(_, text)| text.contains(needle))
}

/// How many distinct identities the forwarded messages carry.
fn distinct_identities(messages: &[(String, String)]) -> usize {
    messages
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// The fixtures every delivery below needs: a task, the Linear connection the
/// envelope's `organizationId` resolves to, and an active session binding
/// pointing at both the session and its issue.
async fn seed_bound_session(state: &AppState, pool: &PgPool) {
    seed_team(pool, &format!("personal-{USER_1}")).await;
    let _ = repository_dao::insert(pool, &make_repository_row("repo-1", USER_1)).await;
    common::seed_workspace_placing(pool, common::FIXTURE_WORKSPACE, "repo-1", USER_1).await;
    task_dao::insert_task(pool, &make_task_row(TASK, USER_1))
        .await
        .expect("seed task");

    let now = Utc::now();
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
    // Every Linear handler resolves its client from this row before it looks
    // for work, so without it no delivery ever reaches the forward: the record
    // fails on "linear app 'default' not configured" and retries. The address
    // is one nothing listens on, which is the unreachable API this file is
    // written against.
    seed_linear_app(pool, "http://127.0.0.1:1").await;

    let binding = TaskBinding {
        id: "tb-linear-1".to_string(),
        task_id: TASK.to_string(),
        adapter_kind: AdapterKind::Linear,
        subject_kind: "linear_agent_session".to_string(),
        subject_key: SESSION.to_string(),
        connection_id: CONNECTION.to_string(),
        loop_id: None,
        destination_config: serde_json::json!({"session_id": SESSION, "issue_id": ISSUE}),
        metadata: serde_json::json!({}),
        last_external_event_id: None,
        origin: "provider_event".to_string(),
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
    };
    state
        .task_bindings
        .upsert(&binding)
        .await
        .expect("seed session binding");
}

/// An active Linear rule for the team. `Issue.update` is ignored outright
/// without one, so the issue tests need it; the prompt path does not.
async fn seed_rule(state: &AppState) {
    let now = Utc::now();
    state
        .loops
        .upsert_subscription(&SubscriptionMatch {
            name: "test-loop".to_string(),
            id: "trigger-linear-1".to_string(),
            adapter_kind: AdapterKind::Linear,
            connection_id: CONNECTION.to_string(),
            source_matcher: TEAM.to_string(),
            source_name: Some("Engineering".to_string()),
            routing_mode: RoutingMode::BySubject,
            event_filter: serde_json::json!([]),
            workspace_id: "ws-1".to_string(),
            prompt_template: None,
            task_name: None,
            tag_ids: vec![],
            skills: Vec::new(),
            // A Subscription with no run-as user is invisible to every lookup
            // that resolves rules — `run_as_user_id IS NOT NULL` is in the
            // query. Without it the rule exists and matches nothing, and the
            // issue-update handler returns before it forwards anything.
            run_as_user_id: Some(USER_1.to_string()),
            agent_type: "claude-code".into(),
            model: None,
            inactive_ttl_days: None,
            allow_direct_send: false,
            adapter_config: None,
            ongoing_task_id: None,
            // The task under test must survive the transition; closing it is a
            // different behaviour with its own coverage.
            close_task_on_close: false,
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

/// A follow-up prompt on the bound session, carried by one `agentActivity`.
fn prompted(activity_id: &str, body: &str, delivered_at: i64) -> serde_json::Value {
    serde_json::json!({
        "action": "prompted",
        "type": "AgentSessionEvent",
        "organizationId": ORG,
        // Stable across every delivery — this is the webhook *configuration*.
        "webhookId": "wh-stable",
        "webhookTimestamp": delivered_at,
        "agentSession": {
            "id": SESSION,
            "issue": {"id": ISSUE, "identifier": "ENG-7", "title": "Fix it", "teamId": TEAM},
        },
        "agentActivity": {"id": activity_id, "type": "prompt", "body": body},
    })
}

/// An issue moving from one workflow state to another.
fn issue_moved(from_state: &str, to_state: &str, delivered_at: i64) -> serde_json::Value {
    serde_json::json!({
        "action": "update",
        "type": "Issue",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": delivered_at,
        "updatedFrom": {"stateId": from_state},
        "data": {
            "id": ISSUE,
            "identifier": "ENG-7",
            "title": "Fix it",
            "teamId": TEAM,
            "state": {"id": to_state, "name": to_state, "type": "started"},
        },
    })
}

/// Regression: a person sending a second message to a Linear agent session
/// within the dedup window had it discarded, because the delivery identity was
/// built from the session rather than from the activity carrying the prompt.
#[sqlx::test]
async fn two_prompts_on_one_session_both_reach_the_agent(pool: PgPool) {
    let state = replica(&pool);
    seed_bound_session(&state, &pool).await;

    assert_eq!(
        deliver(
            &state,
            &prompted("activity-1", "first prompt", 1_700_000_000_000)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        deliver(
            &state,
            &prompted("activity-2", "second prompt", 1_700_000_030_000)
        )
        .await,
        StatusCode::OK
    );

    drain_admitted_work(&pool).await;
    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        2,
        "both prompts must reach the Agent, got {messages:?}"
    );
    assert_eq!(
        distinct_identities(&messages),
        2,
        "each prompt carries its own identity: {messages:?}"
    );
    assert!(
        carries(&messages, "first prompt") && carries(&messages, "second prompt"),
        "both bodies arrive: {messages:?}"
    );
}

/// The other half of the same property. The retry is delivered to a second
/// replica, which has never seen the first — so what collapses it is the
/// durable identity itself, the only deduplication left and the only one that
/// holds across a restart or across pods.
#[sqlx::test]
async fn a_retried_prompt_reaches_the_agent_once(pool: PgPool) {
    seed_bound_session(&replica(&pool), &pool).await;
    let delivery = prompted("activity-1", "please look at this", 1_700_000_000_000);

    assert_eq!(deliver(&replica(&pool), &delivery).await, StatusCode::OK);
    assert_eq!(deliver(&replica(&pool), &delivery).await, StatusCode::OK);

    drain_admitted_work(&pool).await;
    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        1,
        "a retry of one prompt is one message, got {messages:?}"
    );
}

/// Regression: two state changes on one issue shared an identity, because it
/// was built from the issue alone. The second was dropped as a duplicate.
#[sqlx::test]
async fn two_state_changes_on_one_issue_both_reach_the_task(pool: PgPool) {
    let state = replica(&pool);
    seed_bound_session(&state, &pool).await;
    seed_rule(&state).await;

    assert_eq!(
        deliver(
            &state,
            &issue_moved("todo", "in-progress", 1_700_000_000_000)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        deliver(
            &state,
            &issue_moved("in-progress", "in-review", 1_700_000_060_000)
        )
        .await,
        StatusCode::OK
    );

    drain_admitted_work(&pool).await;
    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        2,
        "both state changes must reach the Task, got {messages:?}"
    );
    assert_eq!(
        distinct_identities(&messages),
        2,
        "each state change carries its own identity: {messages:?}"
    );
}

/// Regression, one layer further down: an issue that leaves a state and later
/// returns to it makes two changes that differ in nothing a person can see —
/// same issue, same destination state. Identifying the forwarded message by
/// the state it moved *into* collapsed the return into the first arrival, so
/// the Task never learned the issue came back.
#[sqlx::test]
async fn an_issue_returning_to_a_state_reaches_the_task_again(pool: PgPool) {
    let state = replica(&pool);
    seed_bound_session(&state, &pool).await;
    seed_rule(&state).await;

    for delivery in [
        issue_moved("todo", "in-progress", 1_700_000_000_000),
        issue_moved("in-progress", "todo", 1_700_000_060_000),
        issue_moved("todo", "in-progress", 1_700_000_120_000),
    ] {
        assert_eq!(deliver(&state, &delivery).await, StatusCode::OK);
    }

    drain_admitted_work(&pool).await;
    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        3,
        "every change reaches the Task, including the return, got {messages:?}"
    );
    assert_eq!(
        distinct_identities(&messages),
        3,
        "the two arrivals in one state are two deliveries: {messages:?}"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|(_, text)| text.contains("in-progress"))
            .count(),
        2,
        "the Task learns about both arrivals in that state: {messages:?}"
    );
}

/// Linear repeats `webhookTimestamp` unchanged on a retry, so the identity of
/// an issue change survives redelivery to a replica that never saw the first.
#[sqlx::test]
async fn a_retried_state_change_reaches_the_task_once(pool: PgPool) {
    let first = replica(&pool);
    seed_bound_session(&first, &pool).await;
    let delivery = issue_moved("todo", "in-progress", 1_700_000_000_000);

    seed_rule(&first).await;
    assert_eq!(deliver(&first, &delivery).await, StatusCode::OK);
    assert_eq!(deliver(&replica(&pool), &delivery).await, StatusCode::OK);

    drain_admitted_work(&pool).await;
    let messages = delivered_messages(&pool).await;
    assert_eq!(
        messages.len(),
        1,
        "a retried state change is one message, got {messages:?}"
    );
}
