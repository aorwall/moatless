//! PostgreSQL tests for the Projector's Trajectory-line rules (plan section 7),
//! each through `append`, with lines shaped as each harness's `SessionFormat`
//! derives them.

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::message::test_support::messages_for;
use crate::task::test_support::make_task_row;
use crate::task_activities::mapping::{ActivityKind, ActivityStatus};
use crate::trajectory::dao::PgTrajectoryDao;
use crate::trajectory::models::{AppendOutcome, AppendRequest, NewEvent, SubagentParent};
use moatless_types::agent::Harness;
use moatless_workspaces::test_support::make_repository_row;

const TASK: &str = "task-1";

async fn seed(pool: &PgPool) -> PgTrajectoryDao {
    let repo = make_repository_row("repo-1", "user-1");
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .unwrap();
    crate::task::test_support::seed_task(pool, &make_task_row(TASK, "user-1"))
        .await
        .unwrap();
    PgTrajectoryDao(pool.clone())
}

fn event(item: Option<&str>, content: Value) -> NewEvent {
    NewEvent {
        kind: "line".into(),
        harness_item_id: item.map(str::to_string),
        harness_turn_id: None,
        payload: "{}".into(),
        content,
    }
}

fn said(item: &str, role: &str, blocks: Value) -> NewEvent {
    event(Some(item), json!({ "role": role, "blocks": blocks }))
}

fn text(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn call(id: &str, name: &str, input: Value) -> Value {
    json!({ "type": "tool_use", "id": id, "name": name, "input": input })
}

fn result(id: &str, content: &str, is_error: bool) -> Value {
    json!({ "type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error })
}

async fn append(
    dao: &PgTrajectoryDao,
    harness: Harness,
    session: &str,
    first_seq: i64,
    lines: &[NewEvent],
) -> AppendOutcome {
    dao.append(AppendRequest {
        task_id: TASK,
        harness,
        harness_session_id: session,
        first_seq,
        lines,
        turn_number: Some(1),
        subagent_parent: None,
        own_from_seq: None,
    })
    .await
    .unwrap()
}

/// `(uuid, message_type, text, model)` of each top-level Message.
async fn messages(pool: &PgPool) -> Vec<(String, String, Option<String>, Option<String>)> {
    let (rows, _, _) = messages_for(pool, TASK).await.unwrap();
    rows.into_iter()
        .map(|m| {
            (
                m.uuid,
                m.message_type.as_str().to_string(),
                m.message_text,
                m.assistant_message_model,
            )
        })
        .collect()
}

/// `(message_uuid, parent_uuid, kind, status, tool_use_id)` of every activity,
/// subagent ones included, in write order.
async fn activities(
    pool: &PgPool,
) -> Vec<(
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
)> {
    sqlx::query_as(
        "SELECT message_uuid, parent_uuid, kind, status, tool_use_id
           FROM task_activities WHERE task_id = $1 ORDER BY position",
    )
    .bind(TASK)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn usage(pool: &PgPool) -> Vec<(String, Option<String>, i32, i32)> {
    sqlx::query_as(
        "SELECT message_uuid, parent_uuid, input_tokens, output_tokens
           FROM task_message_usage WHERE task_id = $1 ORDER BY message_uuid",
    )
    .bind(TASK)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn activity(
    message: &str,
    kind: ActivityKind,
    status: ActivityStatus,
    call: &str,
) -> (
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
) {
    (
        Some(message.to_string()),
        None,
        kind.as_str().to_string(),
        status.as_str().to_string(),
        Some(call.to_string()),
    )
}

/// Claude writes one response over several lines sharing its message id, and
/// the tail may send them in separate appends.
#[sqlx::test]
async fn a_claude_response_folds_into_one_message_across_appends(pool: PgPool) {
    let dao = seed(&pool).await;
    let first = event(
        Some("msg_1"),
        json!({
            "role": "assistant",
            "blocks": [text("Looking at the file.")],
            "model": "claude-opus-4-5",
            "usage": { "input_tokens": 10, "output_tokens": 3,
                       "cache_read_tokens": 0, "cache_write_tokens": 0 },
        }),
    );
    let outcome = append(&dao, Harness::ClaudeCode, "s", 0, &[first]).await;
    append(
        &dao,
        Harness::ClaudeCode,
        "s",
        1,
        &[said("msg_1", "assistant", json!([text("Done.")]))],
    )
    .await;

    assert_eq!(outcome.projected.messages, ["msg_1"]);
    assert_eq!(
        messages(&pool).await,
        [(
            "msg_1".to_string(),
            "assistant".to_string(),
            Some("Looking at the file.\n\nDone.".to_string()),
            Some("claude-opus-4-5".to_string()),
        )]
    );
    assert_eq!(usage(&pool).await, [("msg_1".to_string(), None, 10, 3)]);
}

/// A continuation that is not the first line of its append still follows the
/// blocks an earlier append stored, rather than overwriting the first.
#[sqlx::test]
async fn a_continuation_after_a_housekeeping_line_keeps_the_earlier_blocks(pool: PgPool) {
    let dao = seed(&pool).await;
    append(
        &dao,
        Harness::ClaudeCode,
        "s",
        0,
        &[said(
            "msg_1",
            "assistant",
            json!([text("Looking at the file.")]),
        )],
    )
    .await;
    append(
        &dao,
        Harness::ClaudeCode,
        "s",
        1,
        &[
            event(None, Value::Null),
            said("msg_1", "assistant", json!([text("Done.")])),
        ],
    )
    .await;

    assert_eq!(
        messages(&pool).await[0].2.as_deref(),
        Some("Looking at the file.\n\nDone.")
    );
}

/// A call opens an in-progress activity; the result on a later user line
/// completes it and changes the Message that made the call.
#[sqlx::test]
async fn a_tool_result_settles_the_call_on_the_calling_message(pool: PgPool) {
    let dao = seed(&pool).await;
    let calling = append(
        &dao,
        Harness::ClaudeCode,
        "s",
        0,
        &[said(
            "msg_1",
            "assistant",
            json!([call("toolu_1", "Bash", json!({ "command": "ls" }))]),
        )],
    )
    .await;
    assert_eq!(
        activities(&pool).await,
        [activity(
            "msg_1",
            ActivityKind::ToolCall,
            ActivityStatus::InProgress,
            "toolu_1"
        )]
    );

    let settling = append(
        &dao,
        Harness::ClaudeCode,
        "s",
        1,
        &[said(
            "u_1",
            "user",
            json!([result("toolu_1", "a.rs", true)]),
        )],
    )
    .await;

    assert_eq!(calling.projected.messages, ["msg_1"]);
    assert_eq!(settling.projected.messages, ["msg_1"]);
    assert_eq!(
        settling.projected.settled,
        [(Some("Bash".to_string()), true)]
    );
    assert_eq!(
        activities(&pool).await,
        [activity(
            "msg_1",
            ActivityKind::ToolCall,
            ActivityStatus::Failed,
            "toolu_1"
        )]
    );
    assert!(
        messages(&pool).await.is_empty(),
        "a call with no text writes no Message row"
    );
}

/// The inbox owns what was sent, the harness's own lines are never shown, a
/// housekeeping line has no content, and a switch session's prefix is
/// another harness's history.
#[sqlx::test]
async fn lines_no_reader_sees_project_nothing(pool: PgPool) {
    let dao = seed(&pool).await;
    let lines = [
        said("u_1", "user", json!([text("fix the bug")])),
        event(
            Some("u_2"),
            json!({ "role": "user", "blocks": [text("<system-reminder>")], "injected": true }),
        ),
        event(None, Value::Null),
    ];
    let outcome = append(&dao, Harness::ClaudeCode, "s", 0, &lines).await;

    let switch = crate::trajectory::operations::Trajectories::new(PgTrajectoryDao(pool.clone()))
        .open_switch_session(TASK, Harness::Codex)
        .await
        .unwrap()
        .expect("a Claude head to switch from");
    let prefix = [
        said(
            "msg_0",
            "assistant",
            json!([text("From before the switch.")]),
        ),
        said("msg_0b", "assistant", json!([text("Also before it.")])),
    ];
    dao.append(AppendRequest {
        task_id: TASK,
        harness: Harness::Codex,
        harness_session_id: &switch.session.harness_session_id,
        first_seq: 0,
        lines: &prefix,
        turn_number: Some(2),
        subagent_parent: None,
        own_from_seq: Some(2),
    })
    .await
    .unwrap();

    assert_eq!(outcome.projected, Default::default());
    assert!(messages(&pool).await.is_empty());
    assert!(activities(&pool).await.is_empty());
}

/// A subagent's lines carry the spawning call as their parent, and change the
/// subagent transcript rather than a top-level Message.
#[sqlx::test]
async fn a_subagent_line_hangs_under_its_spawning_call(pool: PgPool) {
    let dao = seed(&pool).await;
    append(
        &dao,
        Harness::ClaudeCode,
        "parent",
        0,
        &[said(
            "msg_1",
            "assistant",
            json!([call(
                "toolu_1",
                "Agent",
                json!({ "description": "explore" })
            )]),
        )],
    )
    .await;
    let line = event(
        Some("msg_sub"),
        json!({
            "role": "assistant",
            "blocks": [text("Found it."), call("toolu_2", "Read", json!({}))],
            "usage": { "input_tokens": 5, "output_tokens": 2,
                       "cache_read_tokens": 0, "cache_write_tokens": 0 },
        }),
    );
    let outcome = dao
        .append(AppendRequest {
            task_id: TASK,
            harness: Harness::ClaudeCode,
            harness_session_id: "parent:agent-a",
            first_seq: 0,
            lines: &[line],
            turn_number: Some(1),
            subagent_parent: Some(SubagentParent {
                harness_session_id: "parent",
                tool_use_id: "toolu_1",
            }),
            own_from_seq: None,
        })
        .await
        .unwrap();

    assert!(outcome.projected.messages.is_empty());
    assert_eq!(outcome.projected.subagents, ["toolu_1"]);
    let sub = activities(&pool).await.pop().unwrap();
    assert_eq!(sub.1.as_deref(), Some("toolu_1"));
    assert_eq!(
        usage(&pool).await,
        [("msg_sub".to_string(), Some("toolu_1".to_string()), 5, 2)]
    );
    let parent: Option<String> =
        sqlx::query_scalar("SELECT parent_uuid FROM task_messages WHERE uuid = 'msg_sub'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(parent.as_deref(), Some("toolu_1"));
}

/// Codex names each item on its own; a command the exec call ran is a view of
/// that call, shown under it, and its usage arrives on a role-less line.
#[sqlx::test]
async fn a_codex_command_shows_under_the_exec_call_that_ran_it(pool: PgPool) {
    let dao = seed(&pool).await;
    let lines = [
        said("rs_1", "assistant", json!([text("Running the tests.")])),
        said(
            "call_1",
            "assistant",
            json!([call("call_1", "codex.exec", json!({ "code": "run()" }))]),
        ),
        event(
            Some("cmd_1"),
            json!({
                "role": "assistant",
                "blocks": [
                    call("cmd_1", "codex.shell", json!({ "command": "cargo test" })),
                    result("cmd_1", "ok", false),
                ],
                "derived_from": "call_1",
            }),
        ),
        said("out_1", "user", json!([result("call_1", "done", false)])),
        event(
            Some("tok_1"),
            json!({ "role": null, "blocks": [],
                    "usage": { "input_tokens": 40, "output_tokens": 9,
                               "cache_read_tokens": 0, "cache_write_tokens": 0 } }),
        ),
    ];
    let outcome = append(&dao, Harness::Codex, "rollout", 0, &lines).await;

    assert_eq!(outcome.projected.messages, ["rs_1", "call_1"]);
    assert_eq!(
        activities(&pool).await,
        [
            activity(
                "call_1",
                ActivityKind::ToolCall,
                ActivityStatus::Completed,
                "call_1"
            ),
            activity(
                "call_1",
                ActivityKind::ToolCall,
                ActivityStatus::Completed,
                "cmd_1"
            ),
        ]
    );
    assert_eq!(usage(&pool).await, [("tok_1".to_string(), None, 40, 9)]);
}

/// A `Skill` call names the skill its Task is tagged with.
#[sqlx::test]
async fn a_skill_call_is_reported_for_tagging(pool: PgPool) {
    let dao = seed(&pool).await;
    let outcome = append(
        &dao,
        Harness::ClaudeCode,
        "s",
        0,
        &[said(
            "msg_1",
            "assistant",
            json!([
                call("toolu_1", "Skill", json!({ "skill": "commit" })),
                call("toolu_2", "Bash", json!({ "command": "ls" })),
            ]),
        )],
    )
    .await;

    assert_eq!(outcome.projected.skills, ["commit"]);
}

/// The Projector reads `content` alone: two lines that differ only in their
/// `payload` and `kind` project the same rows.
#[sqlx::test]
async fn projection_never_reads_payload_or_kind(pool: PgPool) {
    let dao = seed(&pool).await;
    let blocks = json!([text("Same content."), call("toolu_1", "Read", json!({}))]);
    let mut odd = said("msg_2", "assistant", blocks.clone());
    odd.kind = "user".into();
    odd.payload = r#"{"type":"user","message":{"content":"other"}}"#.into();
    append(
        &dao,
        Harness::ClaudeCode,
        "s",
        0,
        &[said("msg_1", "assistant", blocks)],
    )
    .await;
    let mut shifted = odd;
    shifted.content["blocks"][1]["id"] = "toolu_2".into();
    append(&dao, Harness::ClaudeCode, "s", 1, &[shifted]).await;

    let rows = messages(&pool).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, rows[1].1);
    assert_eq!(rows[0].2, rows[1].2);
    let kinds: Vec<_> = activities(&pool)
        .await
        .into_iter()
        .map(|a| (a.2, a.3))
        .collect();
    assert_eq!(kinds[0], kinds[1]);
}

/// A line whose projection fails is not stored: the tail resends it.
#[sqlx::test]
async fn a_failed_projection_rolls_back_the_append(pool: PgPool) {
    let dao = seed(&pool).await;
    sqlx::raw_sql(
        "CREATE FUNCTION refuse() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN RAISE EXCEPTION 'refused'; END $$;
         CREATE TRIGGER refuse BEFORE INSERT ON task_activities
           FOR EACH ROW EXECUTE FUNCTION refuse();",
    )
    .execute(&pool)
    .await
    .unwrap();
    let refused = dao
        .append(AppendRequest {
            task_id: TASK,
            harness: Harness::ClaudeCode,
            harness_session_id: "s",
            first_seq: 0,
            lines: &[said(
                "msg_1",
                "assistant",
                json!([text("Hi."), call("toolu_1", "Read", json!({}))]),
            )],
            turn_number: Some(1),
            subagent_parent: None,
            own_from_seq: None,
        })
        .await;

    assert!(refused.is_err());
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trajectory_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
    assert!(messages(&pool).await.is_empty());
}

/// A backend failure is a `task.error` event first; its Error Message carries
/// the event's id.
#[sqlx::test]
async fn an_error_result_is_an_event_projected_as_an_error_message(pool: PgPool) {
    use crate::message::dao::MessageDao;
    seed(&pool).await;
    crate::message::dao::PgMessageDao(pool.clone())
        .append_error_result(TASK, Some(1), "The sandbox failed to start.")
        .await
        .unwrap();

    let event: String = sqlx::query_scalar(
        "SELECT event_id FROM task_events
          WHERE task_id = $1 AND type = 'task' AND sub_type = 'error' AND is_error",
    )
    .bind(TASK)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        messages(&pool).await,
        [(
            event,
            "error".to_string(),
            Some("The sandbox failed to start.".to_string()),
            None,
        )]
    );
}
