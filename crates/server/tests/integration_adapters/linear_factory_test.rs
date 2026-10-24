#![cfg(feature = "db-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures assert on invariants a broken fixture must fail loudly on; both \
              lints only fire when the db-tests feature is enabled"
)]
//! The Linear factory triggers: label triggers, ongoing Loops, exclusions
//! and re-entry. Fixtures come from [`super::linear_workflow_test`].

use sqlx::PgPool;

use moatless_types::loops::RoutingMode;

use crate::linear_workflow_test::{
    ISSUE, ORG, RULE, TASK, TEAM, deliver, drain, forwarded_ids, issue_updated, linear_stub,
    linear_stub_with_labels, replica, seed_binding, seed_connection, seed_local_repository,
    seed_rule, seed_rule_routed, seed_rule_with_id, seed_workspace_token, session_created,
};

/// An update that adds `added` to an issue already carrying `kept`.
fn issue_labeled(kept: &[(&str, &str)], added: &[(&str, &str)]) -> serde_json::Value {
    let all: Vec<_> = kept.iter().chain(added).collect();
    serde_json::json!({
        "action": "update",
        "type": "Issue",
        "organizationId": ORG,
        "webhookId": "wh-stable",
        "webhookTimestamp": 1_700_000_000_010i64,
        "updatedFrom": {"labelIds": kept.iter().map(|(id, _)| *id).collect::<Vec<_>>()},
        "actor": {"id": "user-linear-1", "name": "Ada"},
        "data": {
            "id": ISSUE,
            "identifier": "ENG-7",
            "title": "Fix it",
            "teamId": TEAM,
            "projectId": "project-1",
            "updatedAt": "2026-08-10T03:20:08.000Z",
            "url": "https://linear.app/acme/issue/ENG-7",
            "state": {"id": "s0", "name": "Todo", "type": "unstarted"},
            "labelIds": all.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            "labels": all
                .iter()
                .map(|(id, name)| serde_json::json!({"id": id, "name": name}))
                .collect::<Vec<_>>(),
        },
    })
}

async fn task_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await
        .expect("count tasks")
}

/// The text of every message forwarded to `task_id`, oldest first.
async fn inbox_texts(pool: &PgPool, task_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT text FROM task_inbox WHERE task_id = $1 ORDER BY created_at, id")
        .bind(task_id)
        .fetch_all(pool)
        .await
        .expect("read inbox")
}

#[sqlx::test]
async fn adding_a_trigger_label_creates_a_task_bound_to_the_issue(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({"trigger_labels": ["factory"]})),
    )
    .await;

    deliver(
        &state,
        &issue_labeled(&[("l-bug", "bug")], &[("l-factory", "Factory")]),
    )
    .await;
    drain(&pool, &stub).await;

    assert_eq!(
        stub.graphql_operations(),
        vec!["attachmentLinkURL"],
        "the webhook named the labels, so nothing is fetched"
    );
    assert_eq!(
        forwarded_ids(&pool).await,
        vec![format!("linear:issue:created:{ISSUE}")]
    );
    let loops: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT loop_id FROM task_bindings WHERE subject_kind = 'linear_issue' AND active",
    )
    .fetch_all(&pool)
    .await
    .expect("read bindings");
    assert_eq!(loops, vec![Some(RULE.to_string())]);
}

#[sqlx::test]
async fn an_issue_created_with_a_trigger_label_creates_a_task(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({"trigger_labels": ["factory"]})),
    )
    .await;

    let mut created = issue_labeled(&[], &[("l-factory", "factory")]);
    created["action"] = serde_json::json!("create");
    created
        .as_object_mut()
        .expect("object")
        .remove("updatedFrom");
    deliver(&state, &created).await;
    drain(&pool, &stub).await;

    assert_eq!(task_count(&pool).await, 1);
}

/// A label Loop that names `trigger_states` fires only in those states, and one
/// carrying an excluded label never fires.
#[sqlx::test]
async fn a_label_trigger_respects_trigger_states_and_exclude_labels(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_with_id(
        &state,
        "rule-gated",
        Some(serde_json::json!({"trigger_labels": ["factory"], "trigger_states": ["Ready"]})),
    )
    .await;
    seed_rule_with_id(
        &state,
        "rule-excluding",
        Some(serde_json::json!({"trigger_labels": ["factory"], "exclude_labels": ["no-agent"]})),
    )
    .await;

    deliver(
        &state,
        &issue_labeled(&[("l-no", "No-Agent")], &[("l-factory", "factory")]),
    )
    .await;
    drain(&pool, &stub).await;

    assert_eq!(task_count(&pool).await, 0);
    assert!(stub.graphql_operations().is_empty());
}

#[sqlx::test]
async fn an_excluded_label_stops_a_status_entry_and_declines_a_mention(pool: PgPool) {
    let stub =
        linear_stub_with_labels(serde_json::json!([{"id": "l-no", "name": "no-agent"}])).await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(
        &state,
        Some(serde_json::json!({
            "create_on_status_entry": true,
            "trigger_states": ["In Progress"],
            "exclude_labels": ["no-agent"],
        })),
    )
    .await;
    // Newer, so a mention selects the excluding Loop and must not fall through.
    seed_rule_with_id(&state, "trigger-linear-2", None).await;

    deliver(&state, &issue_updated("s0", "s1", "In Progress", "started")).await;
    drain(&pool, &stub).await;
    deliver(&state, &session_created()).await;
    drain(&pool, &stub).await;

    assert_eq!(task_count(&pool).await, 0);
    assert_eq!(
        stub.graphql_operations(),
        vec![
            "issue",
            "agentActivityCreate",
            "issue",
            "agentActivityCreate"
        ],
        "each path fetches the labels once; the session also heartbeats, then is declined"
    );
}

/// An ongoing Loop's events reach its one Task, opened by the first of them,
/// with no Binding on the issue.
#[sqlx::test]
async fn an_ongoing_loop_delivers_every_issue_event_to_its_one_task(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule_routed(
        &state,
        RULE,
        RoutingMode::Ongoing,
        Some(serde_json::json!({
            "create_on_status_entry": true,
            "trigger_states": ["Inbox", "Todo"],
            "trigger_labels": ["factory"],
        })),
    )
    .await;

    deliver(&state, &issue_updated("s0", "s1", "Inbox", "unstarted")).await;
    drain(&pool, &stub).await;
    deliver(&state, &issue_labeled(&[], &[("l-factory", "factory")])).await;
    drain(&pool, &stub).await;

    assert_eq!(task_count(&pool).await, 1);
    let ongoing: Option<String> =
        sqlx::query_scalar("SELECT ongoing_task_id FROM loop_configs WHERE loop_id = $1")
            .bind(RULE)
            .fetch_one(&pool)
            .await
            .expect("read ongoing task");
    let ongoing = ongoing.expect("the first event records the ongoing task");
    let texts = inbox_texts(&pool, &ongoing).await;
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts[0].starts_with("Linear issue ENG-7 \"Fix it\" entered \"Inbox\"."));
    assert!(texts[0].contains("https://linear.app/acme/issue/ENG-7"));
    assert!(texts[1].starts_with("Linear issue ENG-7 \"Fix it\" was labeled \"factory\" by Ada."));
    let bindings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_bindings")
        .fetch_one(&pool)
        .await
        .expect("count bindings");
    assert_eq!(
        bindings, 0,
        "the Loop owns its task; the issue binds nothing"
    );
}

/// A state change reaches a Task bound to the whole issue through a
/// `linear_issue` Binding, and not one bound only to a comment it posted.
#[sqlx::test]
async fn a_state_change_reaches_an_issue_binding_but_not_a_comment_binding(pool: PgPool) {
    let stub = linear_stub().await;
    let state = replica(&pool, &stub);
    seed_connection(&pool, &stub).await;
    seed_workspace_token(&pool).await;
    seed_local_repository(&pool).await;
    seed_rule(&state, None).await;
    seed_binding(
        &state,
        &pool,
        "tb-issue-1",
        TASK,
        "linear_issue",
        ISSUE,
        serde_json::json!({"issue_id": ISSUE}),
        Some(RULE),
        moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN,
    )
    .await;
    seed_binding(
        &state,
        &pool,
        "tb-comment-1",
        "task-commenter",
        "linear_issue",
        ISSUE,
        serde_json::json!({"issue_id": ISSUE, "origin_comment_id": "comment-1"}),
        None,
        moatless_adapter_model::binding::TASK_OWNED_BINDING_ORIGIN,
    )
    .await;

    let mut moved = issue_updated("s0", "s1", "In Review", "started");
    moved["actor"] = serde_json::json!({"id": "user-linear-1", "name": "Ada"});
    deliver(&state, &moved).await;
    drain(&pool, &stub).await;

    assert_eq!(
        inbox_texts(&pool, TASK).await,
        vec![
            "Linear issue ENG-7 moved to \"In Review\" by Ada.\n\nhttps://linear.app/acme/issue/ENG-7"
        ]
    );
    assert!(inbox_texts(&pool, "task-commenter").await.is_empty());
}

/// Re-entering a trigger state tells the still-active Task rather than starting
/// a second one.
#[sqlx::test]
async fn re_entering_a_trigger_state_tells_the_active_task(pool: PgPool) {
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
    let mut back = issue_updated("s2", "s1", "In Progress", "started");
    back["webhookTimestamp"] = serde_json::json!(1_700_000_000_004i64);
    back["actor"] = serde_json::json!({"id": "user-linear-1", "name": "Ada"});
    deliver(&state, &back).await;
    drain(&pool, &stub).await;

    assert_eq!(task_count(&pool).await, 1);
    let task_id: String = sqlx::query_scalar("SELECT task_id FROM tasks")
        .fetch_one(&pool)
        .await
        .expect("read task");
    let texts = inbox_texts(&pool, &task_id).await;
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(
        texts[1].starts_with("Linear issue ENG-7 re-entered \"In Progress\" (moved by Ada)."),
        "{}",
        texts[1]
    );
}
