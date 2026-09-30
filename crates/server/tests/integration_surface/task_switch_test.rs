#![cfg(feature = "db-tests")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `TaskSwitchService` against the REST table for `PATCH /tasks/{id}` with an
//! `agent_profile`, row by row, and the T3 sink's switch through it.

use crate::common;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use moatless_backend::state::AppState;
use moatless_backend::t3code::backend::AppStateCommands;
use moatless_t3code::ports::{CommandRefusal, ThreadCommandSink, ThreadMeta};
use moatless_tasks::activities::mapping::ConversationWrite;
use moatless_tasks::message::models::{MessageRole, MessageType};
use moatless_tasks::message::test_support as message_fixtures;
use moatless_tasks::task::error::TaskError;
use moatless_tasks::task::switch::{SwitchCredentials, SwitchOutcome};
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::Harness;
use sqlx::PgPool;

use common::{USER_1, make_task_row, test_app_state};

fn viewer() -> ScopeContext {
    ScopeContext {
        user_id: USER_1.into(),
        is_admin: false,
    }
}

async fn seed_task(pool: &PgPool, task_id: &str, profile: &str) {
    let mut task = make_task_row(task_id, USER_1);
    task.workspace_id = Some("ws-switch".to_string());
    task.agent_profile = profile.parse().unwrap();
    moatless_tasks::task::test_support::seed_task(pool, &task)
        .await
        .unwrap();
}

/// A user-scoped secret under `key`, held by reference so no envelope key is
/// needed; the switch reads only that the key is there.
async fn seed_secret(pool: &PgPool, key: &str) {
    sqlx::query(
        r#"INSERT INTO secrets (id, scope, owner_user_id, key, kind, external_ref,
                                created_by, updated_by)
           VALUES ($1, 'user', $2, $3, 'env', 'ref', $2, $2)"#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(USER_1)
    .bind(key)
    .execute(pool)
    .await
    .unwrap();
}

/// A recorded agent reply with no trajectory behind it: a Task created before
/// recording.
async fn seed_unrecorded_reply(pool: &PgPool, task_id: &str) {
    let mut reply = message_fixtures::make_message_row(task_id, &format!("{task_id}-reply"));
    reply.message_type = MessageType::Assistant;
    reply.message_role = MessageRole::Assistant;
    reply.session_id = Some("claude-1".to_string());
    message_fixtures::seed_message(pool, &reply, &[], &ConversationWrite::default())
        .await
        .unwrap();
    message_fixtures::freeze(pool, task_id, None).await.unwrap();
}

async fn switch(
    state: &AppState,
    task_id: &str,
    profile: &str,
    model: Option<&str>,
) -> Result<SwitchOutcome, TaskError> {
    // The handler parses the id before it switches; an unknown one is its 400.
    let profile = profile.parse().map_err(TaskError::BadRequest)?;
    state
        .task_switch
        .switch_profile(
            &viewer(),
            task_id,
            profile,
            model,
            SwitchCredentials {
                secrets: state.secrets.as_ref(),
                provider_tokens: state.harness_credentials.as_ref(),
                credential_sync: &state.credential_sync,
            },
        )
        .await
}

fn status(result: Result<SwitchOutcome, TaskError>) -> (StatusCode, String) {
    let error = result.expect_err("refused");
    let message = error.to_string();
    (error.into_response().status(), message)
}

async fn profile_and_model(pool: &PgPool, task_id: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT agent_profile, model FROM tasks WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn the_current_profile_changes_nothing(pool: PgPool) {
    seed_task(&pool, "t-same", "claude-code").await;
    let state = test_app_state(pool.clone());
    let outcome = switch(&state, "t-same", "claude-code", None).await.unwrap();
    assert_eq!(outcome, SwitchOutcome::Unchanged);
}

#[sqlx::test]
async fn an_unknown_profile_is_a_bad_request(pool: PgPool) {
    seed_task(&pool, "t-unknown", "claude-code").await;
    let state = test_app_state(pool.clone());
    let (code, _) = status(switch(&state, "t-unknown", "gemini", None).await);
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn a_missing_credential_is_a_bad_request_naming_it(pool: PgPool) {
    seed_task(&pool, "t-cred", "codex").await;
    let state = test_app_state(pool.clone());
    let (code, message) = status(switch(&state, "t-cred", "claude-code", None).await);
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(message.contains("CLAUDE_CODE_OAUTH_TOKEN"), "{message}");
}

#[sqlx::test]
async fn a_model_the_profile_does_not_offer_is_a_bad_request(pool: PgPool) {
    seed_task(&pool, "t-model", "codex").await;
    seed_secret(&pool, "CLAUDE_CODE_OAUTH_TOKEN").await;
    let state = test_app_state(pool.clone());
    let (code, _) = status(switch(&state, "t-model", "claude-code", Some("gpt-5.6-sol")).await);
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn a_frozen_task_cannot_switch_harness(pool: PgPool) {
    seed_task(&pool, "t-frozen", "codex").await;
    seed_secret(&pool, "CLAUDE_CODE_OAUTH_TOKEN").await;
    seed_unrecorded_reply(&pool, "t-frozen").await;
    let state = test_app_state(pool.clone());
    let (code, message) = status(switch(&state, "t-frozen", "claude-code", None).await);
    assert_eq!(code, StatusCode::CONFLICT);
    assert!(message.contains("cannot switch harness"), "{message}");
}

#[sqlx::test]
async fn a_claude_profile_on_another_transport_is_a_conflict(pool: PgPool) {
    seed_task(&pool, "t-tmux", "claude-code").await;
    seed_secret(&pool, "CLAUDE_CODE_OAUTH_TOKEN").await;
    let state = test_app_state(pool.clone());
    let (code, message) = status(switch(&state, "t-tmux", "claude-code-tui", None).await);
    assert_eq!(code, StatusCode::CONFLICT);
    assert!(message.contains("This Sandbox cannot run"), "{message}");
}

#[sqlx::test]
async fn a_switch_writes_the_task_and_its_sandbox_and_clears_the_model(pool: PgPool) {
    seed_task(&pool, "t-ok", "codex").await;
    sqlx::query("UPDATE tasks SET model = 'gpt-5.6-sol' WHERE task_id = 't-ok'")
        .execute(&pool)
        .await
        .unwrap();
    seed_secret(&pool, "CLAUDE_CODE_OAUTH_TOKEN").await;
    let state = test_app_state(pool.clone());

    let outcome = switch(&state, "t-ok", "claude-code", None).await.unwrap();

    assert!(matches!(outcome, SwitchOutcome::Switched { .. }));
    assert_eq!(
        profile_and_model(&pool, "t-ok").await,
        ("claude-code".to_string(), None)
    );
    let sandbox: Option<String> =
        sqlx::query_scalar("SELECT agent_profile FROM sandboxes WHERE sandbox_id = 't-ok'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sandbox.as_deref(), Some("claude-code"));
}

#[sqlx::test]
async fn an_openai_key_switches_into_codex_without_a_connection(pool: PgPool) {
    seed_task(&pool, "t-key", "claude-code").await;
    seed_secret(&pool, "OPENAI_API_KEY").await;
    let state = test_app_state(pool.clone());

    let outcome = switch(&state, "t-key", "codex", None).await.unwrap();

    assert!(matches!(outcome, SwitchOutcome::Switched { .. }));
}

/// T3 names a harness; the sink picks the profile and surfaces a refusal.
#[sqlx::test]
async fn a_t3_meta_update_switches_or_surfaces_the_refusal(pool: PgPool) {
    seed_task(&pool, "t-t3", "codex").await;
    let sink = AppStateCommands(test_app_state(pool.clone()));
    let meta = || ThreadMeta {
        task_id: "t-t3".into(),
        title: None,
        model: None,
        harness: Some(Harness::ClaudeCode),
    };

    let refusal = sink.update_meta(&viewer(), meta()).await.unwrap_err();
    assert!(
        matches!(refusal, CommandRefusal::Rejected { .. }),
        "no Claude credential: {refusal:?}"
    );

    seed_secret(&pool, "CLAUDE_CODE_OAUTH_TOKEN").await;
    sink.update_meta(&viewer(), meta()).await.unwrap();
    assert_eq!(profile_and_model(&pool, "t-t3").await.0, "claude-code");
}
