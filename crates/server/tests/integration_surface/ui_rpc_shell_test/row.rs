//! What one conversation row says.

use super::contract::assert_decodable_thread;
use super::feed::{ingress, start_turn};
use super::harness::{parse, snapshot, thread, viewer};
use super::seeding::{
    MATRIX_WS, follow, follower_state, seed_agent_message, seed_conversation_matrix,
    seed_public_task, seed_sandbox, seed_task, seed_turn, seed_user_message,
    set_settled_and_snoozed,
};
use crate::common::make_message_row;
use crate::common::make_task_row;
use crate::common::make_workspace_row;
use crate::common::test_app_state;
use chrono::DateTime;
use chrono::SubsecRound;
use chrono::Utc;
use moatless_tasks::activities::mapping::BlockContext;
use moatless_tasks::activities::mapping::map_content;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::service as message_service;
use moatless_workspaces::compose;
use serde_json::Value;
use serde_json::json;
use sqlx::PgPool;

// =========================================================================
// What one conversation row says
// =========================================================================

#[sqlx::test]
async fn two_followers_receive_their_own_settlement_state(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-shared-state", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-shared-state").await;
    moatless_tasks::task::test_support::settle_task(&pool, "user-1", "t-shared-state", Utc::now())
        .await
        .unwrap();

    assert_eq!(
        thread(&pool, &viewer("user-1"), "t-shared-state").await["settledOverride"],
        json!("settled")
    );
    assert_eq!(
        thread(&pool, &viewer("user-2"), "t-shared-state").await["settledOverride"],
        Value::Null
    );
}

#[sqlx::test]
async fn a_user_message_clears_settle_for_all_followers_and_snooze_only_for_sender(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-message-wake", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-message-wake").await;
    set_settled_and_snoozed(&pool, "user-1", "t-message-wake").await;
    set_settled_and_snoozed(&pool, "user-2", "t-message-wake").await;

    let state = test_app_state(pool.clone());
    let sender_scope = viewer("user-1");
    message_service::send_user_message(
        state.tasks.as_ref(),
        state.message_send.as_ref(),
        state.sandbox_waker.as_ref(),
        &message_service::Sender {
            user_id: "user-1",
            scope: &sender_scope,
            is_agent_context: false,
        },
        "t-message-wake",
        message_service::UserMessage {
            text: "wake it".to_string(),
            resume: true,
            ..Default::default()
        },
    )
    .await
    .expect("message is accepted");

    let sender = follower_state(&pool, "user-1", "t-message-wake").await;
    let colleague = follower_state(&pool, "user-2", "t-message-wake").await;
    assert!(sender.0.is_none(), "sender settle was not cleared");
    assert!(sender.2.is_none(), "sender snooze was not cleared");
    assert!(colleague.0.is_none(), "colleague settle was not cleared");
    assert!(colleague.2.is_some(), "colleague snooze was cleared");
}

/// REST send is a write: a reader who cannot write is refused before anything
/// lands, and a public Task's visitor is let through.
#[sqlx::test]
async fn a_user_message_needs_write_not_read(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    // A `global` private row reads to everyone but writes for admins only.
    let mut readable = make_task_row("t-readable", "user-1");
    readable.workspace_id = Some("ws-a".to_string());
    readable.scope = moatless_types::access_control::Scope::Global;
    readable.owner_user_id = None;
    moatless_tasks::task::test_support::seed_task(&pool, &readable)
        .await
        .unwrap();
    seed_public_task(&pool, "t-public", "user-1", Some("ws-a")).await;

    let state = test_app_state(pool.clone());
    let visitor = viewer("user-2");
    let send = |task_id: &'static str| {
        let state = &state;
        let visitor = &visitor;
        async move {
            message_service::send_user_message(
                state.tasks.as_ref(),
                state.message_send.as_ref(),
                state.sandbox_waker.as_ref(),
                &message_service::Sender {
                    user_id: "user-2",
                    scope: visitor,
                    is_agent_context: false,
                },
                task_id,
                message_service::UserMessage {
                    text: "hello".to_string(),
                    ..Default::default()
                },
            )
            .await
        }
    };

    assert!(send("t-readable").await.is_err());
    let follows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_follows WHERE user_id = 'user-2' AND task_id = 't-readable'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(follows, 0, "a refused send must not follow the task");
    send("t-public")
        .await
        .expect("a visitor may write to a public task");
}

#[sqlx::test]
async fn turn_start_clears_settle_for_every_follower_and_leaves_snooze(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-turn-wake", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-turn-wake").await;
    set_settled_and_snoozed(&pool, "user-1", "t-turn-wake").await;
    set_settled_and_snoozed(&pool, "user-2", "t-turn-wake").await;

    start_turn(&pool, "t-turn-wake", "run-1").await;

    for user in ["user-1", "user-2"] {
        let state = follower_state(&pool, user, "t-turn-wake").await;
        assert!(state.0.is_none(), "{user} settle was not cleared");
        assert!(state.2.is_some(), "{user} snooze was cleared");
    }
}

#[sqlx::test]
async fn ask_user_question_clears_settle_for_every_follower_and_leaves_snooze(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-question-wake", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-question-wake").await;
    set_settled_and_snoozed(&pool, "user-1", "t-question-wake").await;
    set_settled_and_snoozed(&pool, "user-2", "t-question-wake").await;

    moatless_tasks::message::test_support::seed_agent_blocks(
        &pool,
        "t-question-wake",
        "session-wake",
        Some(1),
        json!([{"type": "tool_use", "id": "tu-wake", "name": "AskUserQuestion", "input": {}}]),
    )
    .await;

    for user in ["user-1", "user-2"] {
        let state = follower_state(&pool, user, "t-question-wake").await;
        assert!(state.0.is_none(), "{user} settle was not cleared");
        assert!(state.2.is_some(), "{user} snooze was cleared");
    }
}

#[sqlx::test]
async fn completed_turn_clears_no_settlement_or_snooze_state(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-complete-keeps-state", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-complete-keeps-state").await;
    let turn = start_turn(&pool, "t-complete-keeps-state", "run-1").await;
    set_settled_and_snoozed(&pool, "user-1", "t-complete-keeps-state").await;
    set_settled_and_snoozed(&pool, "user-2", "t-complete-keeps-state").await;

    ingress(&pool, "t-complete-keeps-state", turn, "agent", "completed").await;

    for user in ["user-1", "user-2"] {
        let state = follower_state(&pool, user, "t-complete-keeps-state").await;
        assert_eq!(state.0.as_deref(), Some("settled"), "{user}");
        assert!(state.1.is_some(), "{user} settled_at was cleared");
        assert!(state.2.is_some(), "{user} snooze was cleared");
        assert!(state.3.is_some(), "{user} snoozed_at was cleared");
    }
}

/// The activity timestamp counts messages a person wrote. `last_message_at`,
/// the nearest stored value, also advances when the agent writes — a sidebar
/// sorted or badged on it reorders itself while nobody is talking.
#[sqlx::test]
async fn the_activity_timestamp_does_not_move_when_the_agent_writes(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-chat", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-chat", 1, "active").await;
    seed_user_message(&pool, "t-chat", "m-person").await;

    let viewer = viewer("user-1");
    let after_person = thread(&pool, &viewer, "t-chat").await["latestUserMessageAt"].clone();
    assert!(
        !after_person.is_null(),
        "a person wrote, so there is a time"
    );

    seed_agent_message(&pool, "t-chat", "m-agent-1", 1, Some("claude-opus-4-5")).await;
    seed_agent_message(&pool, "t-chat", "m-agent-2", 1, Some("claude-opus-4-5")).await;
    sqlx::query("UPDATE tasks SET last_message_at = $1 WHERE task_id = $2")
        .bind(Utc::now().to_rfc3339())
        .bind("t-chat")
        .execute(&pool)
        .await
        .unwrap();

    let after_agent = thread(&pool, &viewer, "t-chat").await["latestUserMessageAt"].clone();
    assert_eq!(
        after_agent, after_person,
        "the agent's messages moved the activity timestamp"
    );

    // And it really is the person's message time, not the row's. Compared at
    // the wire's precision: `timestamptz` keeps microseconds and every instant
    // on this wire is RFC 3339 with milliseconds, so the stored value has to be
    // truncated the same way the projection truncates it before they can be
    // equal at all.
    let wrote_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM conversation_messages WHERE uuid = $1")
            .bind(moatless_tasks::message::test_support::seeded_uuid(
                "m-person",
            ))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(parse(&after_person), wrote_at.trunc_subsecs(3));
}

/// A conversation that has run: a turn, a sandbox, a person's message and the
/// agent's reply. Every derived value the row carries comes from one of them.
#[sqlx::test]
async fn a_conversation_that_has_run_reports_its_turn_session_and_model(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-live", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-live", 1, "completed").await;
    seed_turn(&pool, "t-live", 2, "active").await;
    seed_sandbox(&pool, "t-live", "running", true).await;
    seed_user_message(&pool, "t-live", "m-1").await;
    seed_agent_message(&pool, "t-live", "m-2", 2, Some("claude-opus-4-5-20260101")).await;

    let thread = thread(&pool, &viewer("user-1"), "t-live").await;
    assert_decodable_thread(&thread);

    // The newest turn, not the first.
    assert_eq!(thread["latestTurn"]["turnId"], "t-live:2");
    assert_eq!(thread["latestTurn"]["state"], "running");
    // An open turn is the session's active turn.
    assert_eq!(thread["session"]["activeTurnId"], "t-live:2");
    assert_eq!(thread["session"]["status"], "running");
    assert_eq!(thread["session"]["providerInstanceId"], "claudeAgent");
    // What the agent said it was running beats any configured default.
    assert_eq!(
        thread["modelSelection"]["model"],
        "claude-opus-4-5-20260101"
    );
    assert_eq!(thread["modelSelection"]["instanceId"], "claudeAgent");
    // Distinct per conversation, because the client sends it straight back as
    // the `cwd` of every file request it makes for this thread.
    assert_eq!(thread["worktreePath"], "/moatless/tasks/t-live");
    assert_eq!(thread["runtimeMode"], "full-access");
    assert_eq!(thread["hasPendingApprovals"], false);
}

/// Nothing has run: no turn, no sandbox, no message. Every derived value is
/// null and the row still has to decode, because this is what a conversation
/// looks like for its whole first minute.
#[sqlx::test]
async fn a_conversation_that_never_ran_still_decodes(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-fresh", "user-1", Some("ws-a")).await;

    let thread = thread(&pool, &viewer("user-1"), "t-fresh").await;
    assert_decodable_thread(&thread);

    assert!(thread["latestTurn"].is_null());
    assert_eq!(thread["session"]["status"], "stopped");
    assert!(thread["session"]["activeTurnId"].is_null());
    assert!(thread["latestUserMessageAt"].is_null());
    assert_eq!(thread["worktreePath"], "/moatless/tasks/t-fresh");
    // The deployment's configured default is the answer until something runs.
    assert_eq!(thread["modelSelection"]["model"], "claude-opus-4-5");
}

/// The agent's model default is per agent kind, and the provider instance the
/// client routes on comes from the same place.
#[sqlx::test]
async fn a_codex_conversation_reports_the_codex_instance_and_default(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    let mut task = make_task_row("t-codex", "user-1");
    task.workspace_id = Some("ws-a".to_string());
    task.agent_profile = moatless_tasks::agent::catalog::CODEX.profile;
    moatless_tasks::task::test_support::seed_task(&pool, &task)
        .await
        .unwrap();
    seed_sandbox(&pool, "t-codex", "running", true).await;

    let thread = thread(&pool, &viewer("user-1"), "t-codex").await;
    assert_decodable_thread(&thread);
    assert_eq!(thread["modelSelection"]["instanceId"], "codex");
    assert_eq!(thread["modelSelection"]["model"], "gpt-5.6-sol");
    assert_eq!(thread["session"]["providerInstanceId"], "codex");
}

/// An unanswered question from the agent is what "waiting on a person" means
/// here; an answered one is not.
#[sqlx::test]
async fn an_unanswered_question_is_pending_user_input(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-asking", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-asking", 1, "active").await;

    let mut asking = make_message_row("t-asking", "m-ask");
    asking.message_type = MessageType::Assistant;
    asking.user_type = "system".into();
    asking.turn_number = Some(1);
    asking.message_text = Some("Which one?".into());
    let question = map_content(
        BlockContext {
            task_id: "t-asking",
            message_uuid: "m-ask",
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 1,
            source: "m-ask",
            source_block: 0,
        },
        &json!([{"type": "tool_use", "id": "tu-1", "name": "AskUserQuestion", "input": {}}]),
    );
    moatless_tasks::message::test_support::seed_message(&pool, &asking, &[], &question)
        .await
        .unwrap();

    let viewer = viewer("user-1");
    let waiting = thread(&pool, &viewer, "t-asking").await;
    assert_eq!(waiting["hasPendingUserInput"], true);
    assert_eq!(waiting["hasActionableProposedPlan"], false);

    // The answer settles the call the question wrote. There is no second
    // Message carrying a `tool_result` any more, so the flag has to come off
    // the activity's own status.
    let answer = map_content(
        BlockContext {
            task_id: "t-asking",
            message_uuid: "m-answer",
            parent_uuid: None,
            message_type: MessageType::User,
            block_index: 0,
            source: "m-answer",
            source_block: 0,
        },
        &json!([{"type": "tool_result", "tool_use_id": "tu-1", "content": "the first one"}]),
    );
    moatless_tasks::message::test_support::seed_activities(&pool, "t-asking", "m-ask", &answer)
        .await
        .unwrap();

    assert_eq!(
        thread(&pool, &viewer, "t-asking").await["hasPendingUserInput"],
        false,
        "the question was answered"
    );
}

/// An unanswered question the agent then called past stops counting: the scan
/// reads only the newest turn that did anything, which is the narrow reading of
/// "waiting" — the agent has moved on.
#[sqlx::test]
async fn a_question_the_agent_called_past_is_no_longer_pending(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-moved-on", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-moved-on", 1, "active").await;

    let mut asking = make_message_row("t-moved-on", "m-ask");
    asking.message_type = MessageType::Assistant;
    asking.user_type = "system".into();
    asking.turn_number = Some(1);
    asking.message_text = Some("Which one?".into());
    let question = map_content(
        BlockContext {
            task_id: "t-moved-on",
            message_uuid: "m-ask",
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 1,
            source: "m-ask",
            source_block: 0,
        },
        &json!([{"type": "tool_use", "id": "tu-1", "name": "AskUserQuestion", "input": {}}]),
    );
    moatless_tasks::message::test_support::seed_message(&pool, &asking, &[], &question)
        .await
        .unwrap();

    let viewer = viewer("user-1");
    assert_eq!(
        thread(&pool, &viewer, "t-moved-on").await["hasPendingUserInput"],
        true
    );

    // A later turn that called a tool and said nothing at all — no Message row.
    let moved_on = map_content(
        BlockContext {
            task_id: "t-moved-on",
            message_uuid: "m-next",
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 0,
            source: "m-next",
            source_block: 0,
        },
        &json!([{"type": "tool_use", "id": "tu-2", "name": "Bash", "input": {}}]),
    );
    moatless_tasks::message::test_support::seed_activities(
        &pool,
        "t-moved-on",
        "m-next",
        &moved_on,
    )
    .await
    .unwrap();

    let thread = thread(&pool, &viewer, "t-moved-on").await;
    assert_decodable_thread(&thread);
    assert_eq!(
        thread["hasPendingUserInput"], false,
        "the question is not on the newest turn that did anything"
    );
}

/// A conversation with no name still needs a title: the contract refuses an
/// empty string, so an unnamed conversation shows the "New thread" placeholder
/// until it is auto-titled.
#[sqlx::test]
async fn a_conversation_with_no_name_falls_back_to_new_thread(pool: PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    let mut task = make_task_row("t-nameless", "user-1");
    task.workspace_id = Some("ws-a".to_string());
    task.name = None;
    moatless_tasks::task::test_support::seed_task(&pool, &task)
        .await
        .unwrap();

    let thread = thread(&pool, &viewer("user-1"), "t-nameless").await;
    assert_decodable_thread(&thread);
    assert_eq!(thread["title"], "New thread");
}

/// Every row in a listing built from every combination the filters distinguish
/// has to decode — one bad row is not one missing conversation, it is a blank
/// sidebar.
#[sqlx::test]
async fn every_listed_conversation_decodes(pool: PgPool) {
    seed_conversation_matrix(&pool).await;
    seed_turn(&pool, "own-private-user-followed", 1, "error").await;
    seed_sandbox(&pool, "own-private-user-followed", "removed", false).await;
    seed_turn(&pool, "own-public-global-followed", 1, "stopped").await;
    seed_sandbox(&pool, "own-public-global-followed", "stopped", true).await;
    seed_user_message(&pool, "other-public-user-followed", "m-1").await;

    let snapshot = snapshot(&pool, &viewer("user-1")).await;
    let threads = snapshot["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 7);
    for thread in threads {
        assert_decodable_thread(thread);
        assert_eq!(
            thread["projectId"], MATRIX_WS,
            "every row hangs under a project in the same snapshot"
        );
    }
}
