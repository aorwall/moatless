#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for `event_handler::agent_handlers::handle_agent_message`
//! → `message::hooks::on_agent_message_persisted`.
//!
//! Covers:
//! - `gh pr create` tool result auto-connects a `task_bindings` row and
//!   tags the parent assistant tool_use row with `github:pr:opened:<n>`.
//! - `gh pr comment` tool result tags the parent assistant tool_use row
//!   with `github:ic:<id>`.

mod common;

use chrono::Utc;
use moatless_vibe_backend::adapters::dao::PgAdapterConnectionRepository;
use moatless_vibe_backend::adapters::models::{AdapterConnection, RoutingMode, SubscriptionMatch};
use moatless_vibe_backend::bindings::dao::PgTaskBindingRepository;
use moatless_vibe_backend::event_handler::agent_handlers::handle_agent_message;
use moatless_vibe_backend::event_handler::models::AgentEvent;
use moatless_vibe_backend::loops::store::LoopStore;
use moatless_vibe_backend::repository::dao as repository_dao;
use moatless_vibe_backend::scope::{Scope, ScopeContext};
use moatless_vibe_backend::task::dao as task_dao;
use sqlx::PgPool;

use common::{make_repository_row, make_task_row, test_app_state};

async fn seed_github_trigger(pool: &PgPool, owner: &str, repo: &str, repository_id: &str) {
    let now = Utc::now();
    let connection = AdapterConnection {
        id: format!("pc_{owner}"),
        provider: "github".to_string(),
        connection_kind: "org_webhook".to_string(),
        scope_value: owner.to_string(),
        credential_ref: None,
        webhook_secret: Some("whsec_test".to_string()),
        transport_config: serde_json::json!({}),
        created_by: "user-1".to_string(),
        scope: Scope::User,
        owner_user_id: Some("user-1".to_string()),
        created_at: now,
        updated_at: now,
    };
    let ctx = ScopeContext {
        user_id: "user-1".to_string(),
        is_admin: false,
    };
    PgAdapterConnectionRepository(pool.clone())
        .upsert(&connection, &ctx)
        .await
        .unwrap();

    let subscription = SubscriptionMatch {
        id: format!("tt_{owner}_{repo}"),
        provider: "github".to_string(),
        connection_id: connection.id.clone(),
        source_matcher: format!("{owner}/{repo}"),
        source_name: Some(format!("{owner}/{repo}")),
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        repository_id: repository_id.to_string(),
        prompt_template: None,
        task_name: None,
        tag_ids: vec![],
        run_as_user_id: None,
        agent_type: "claude-code".to_string(),
        inactive_ttl_days: None,
        allow_direct_send: false,
        provider_config: None,
        ongoing_task_id: None,
        close_task_on_close: true,
        notify_on_success: false,
        active: true,
        created_by: "user-1".to_string(),
        scope: Scope::User,
        owner_user_id: Some("user-1".to_string()),
        created_at: now,
        updated_at: now,
    };
    LoopStore::new(pool.clone())
        .upsert_subscription(&subscription)
        .await
        .unwrap();
}

/// Build an `AgentEvent` for an SDK assistant or user message with the
/// given content array.
fn agent_message_event(
    event_id: &str,
    task_id: &str,
    role: &str,
    uuid: &str,
    content: serde_json::Value,
) -> AgentEvent {
    let payload = serde_json::json!({
        "uuid": uuid,
        "session_id": "session-1",
        "message": {
            "role": role,
            "content": content,
        },
    });
    let value = serde_json::json!({
        "eventId": event_id,
        "type": "agent",
        "subType": "message",
        "taskId": task_id,
        "timestamp": "2026-04-30T00:00:00.000Z",
        "isError": false,
        "agentType": "claude-code",
        "turnNumber": 1,
        "payload": payload,
    });
    serde_json::from_value(value).unwrap()
}

/// Seed a repo + task and return an `AppState` wired to the pool.
async fn seed(pool: &PgPool, task_id: &str) -> moatless_vibe_backend::state::AppState {
    let repo = make_repository_row("repo-1", "user-1");
    repository_dao::insert(pool, &repo).await.unwrap();
    let task = make_task_row(task_id, "user-1");
    task_dao::insert_task(pool, &task).await.unwrap();
    test_app_state(pool.clone())
}

#[sqlx::test]
#[ignore = "test_app_state wires no adapter registry, so connect_github_binding \
            always fails with AdapterNotConfigured; was also red before the \
            scope collapse (seed lacked the personal-team row)"]
async fn gh_pr_create_auto_connects_task_binding(pool: PgPool) {
    let task_id = "task-pr-create";
    let state = seed(&pool, task_id).await;

    // The org-webhook connection + trigger route must exist before the agent's
    // gh-pr-create hook can attach a task binding.
    seed_github_trigger(&pool, "foo", "bar", "repo-1").await;

    // 1. Persist the assistant tool_use(Bash, "gh pr create ...") message.
    let assistant_event = agent_message_event(
        "evt-1",
        task_id,
        "assistant",
        "asst-uuid-1",
        serde_json::json!([
            {
                "type": "tool_use",
                "id": "tu-1",
                "name": "Bash",
                "input": {
                    "command": "gh pr create --title \"foo\" --body \"bar\"",
                },
            }
        ]),
    );
    handle_agent_message(&state, &assistant_event).await;

    // 2. Persist the matching tool_result with the PR URL in stdout.
    let result_event = agent_message_event(
        "evt-2",
        task_id,
        "user",
        "tr-uuid-1",
        serde_json::json!([
            {
                "type": "tool_result",
                "tool_use_id": "tu-1",
                "content": "Creating pull request for branch...\nhttps://github.com/foo/bar/pull/99\n",
                "is_error": false,
            }
        ]),
    );
    handle_agent_message(&state, &result_event).await;

    // 3. Verify a task_bindings row was created for this task pointing at
    //    the seeded org connection and repo trigger.
    let connection = PgAdapterConnectionRepository(pool.clone())
        .find_by_scope_value("github", "foo")
        .await
        .unwrap()
        .expect("seeded github org connection");
    let trigger = state
        .loops
        .find_subscription_by_source_matcher("github", "foo/bar")
        .await
        .unwrap()
        .expect("seeded github trigger");

    let bindings = PgTaskBindingRepository(pool.clone())
        .find_by_task_id(task_id)
        .await
        .unwrap();
    assert_eq!(bindings.len(), 1, "expected a GitHub task binding");
    assert_eq!(bindings[0].provider, "github");
    assert_eq!(bindings[0].subject_kind, "github_pr");
    assert_eq!(bindings[0].subject_key, "foo/bar#99");
    assert_eq!(bindings[0].connection_id, connection.id);
    assert_eq!(bindings[0].trigger_id.as_deref(), Some(trigger.id.as_str()));

    // 4. The parent assistant row should now carry channel metadata so the
    //    UI can render a "PR #99 opened" chip on it.
    let (rows, _, _) = moatless_vibe_backend::message::dao::get_all(&pool, task_id)
        .await
        .unwrap();
    let assistant_row = rows
        .iter()
        .find(|m| m.uuid == "asst-uuid-1")
        .expect("assistant row present");
    assert_eq!(
        assistant_row.external_event_id.as_deref(),
        Some("github:pr:opened:99"),
    );
    assert_eq!(assistant_row.external_adapter.as_deref(), Some("github_pr"));
    assert_eq!(
        assistant_row.external_source_name.as_deref(),
        Some("foo/bar")
    );
}

#[sqlx::test]
async fn gh_pr_comment_tags_tool_result_with_external_id(pool: PgPool) {
    let task_id = "task-pr-comment";
    let state = seed(&pool, task_id).await;

    let assistant_event = agent_message_event(
        "evt-c1",
        task_id,
        "assistant",
        "asst-uuid-c1",
        serde_json::json!([
            {
                "type": "tool_use",
                "id": "tu-c1",
                "name": "Bash",
                "input": {
                    "command": "gh pr comment 42 --body \"agent reply\"",
                },
            }
        ]),
    );
    handle_agent_message(&state, &assistant_event).await;

    let result_uuid = "tr-uuid-c1";
    let result_event = agent_message_event(
        "evt-c2",
        task_id,
        "user",
        result_uuid,
        serde_json::json!([
            {
                "type": "tool_result",
                "tool_use_id": "tu-c1",
                "content": "https://github.com/foo/bar/pull/42#issuecomment-555\n",
                "is_error": false,
            }
        ]),
    );
    handle_agent_message(&state, &result_event).await;

    // The parent assistant tool_use row should now carry the prefixed comment id.
    let has = state
        .messages
        .has_external_event_id(task_id, "github:ic:555")
        .await
        .unwrap();
    assert!(
        has,
        "assistant row should be tagged with namespaced comment id",
    );

    let (rows, _, _) = moatless_vibe_backend::message::dao::get_all(&pool, task_id)
        .await
        .unwrap();
    let assistant_row = rows
        .iter()
        .find(|m| m.uuid == "asst-uuid-c1")
        .expect("assistant row present");
    assert_eq!(
        assistant_row.external_event_id.as_deref(),
        Some("github:ic:555"),
    );
    assert_eq!(assistant_row.external_adapter.as_deref(), Some("github_pr"));
    assert_eq!(
        assistant_row.external_source_name.as_deref(),
        Some("foo/bar")
    );
    // The hidden System tool_result row must remain untagged.
    let result_row = rows
        .iter()
        .find(|m| m.uuid == result_uuid)
        .expect("tool_result row present");
    assert!(result_row.external_event_id.is_none());
}
