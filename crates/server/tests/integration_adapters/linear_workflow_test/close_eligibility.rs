//! Close eligibility: each reason a Linear issue update does not close its
//! Task, tested separately.

use super::*;

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
    upsert_binding(
        &state.task_bindings,
        &TaskBinding {
            id: "tb-sibling".to_string(),
            task_id: TASK.to_string(),
            adapter_kind: AdapterKind::Linear,
            subject_kind: "linear_issue".to_string(),
            subject_key: "issue-other".to_string(),
            connection_id: CONNECTION.to_string(),
            loop_id: Some(RULE.to_string()),
            destination_config: serde_json::json!({"issue_id": "issue-other"}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN.to_string(),
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
            watching: true,
        },
    )
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
        moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN,
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
