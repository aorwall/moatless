//! PostgreSQL tests for a rewind across a harness switch.

use serde_json::json;
use sqlx::PgPool;

use moatless_types::access_control::ScopeContext;
use moatless_types::agent::Harness;

use super::TaskRewindService;
use crate::message::dao::PgMessageDao;
use crate::message::models::MessageType;
use crate::message::test_support::{make_message_row, seed_message};
use crate::task::dao::PgTaskDao;
use crate::task::test_support::{make_task_row, seed_task};
use crate::task_turn::dao::PgTaskTurnDao;
use crate::trajectory::dao::PgTrajectoryDao;
use crate::trajectory::models::{AppendRequest, NewEvent};
use crate::trajectory::operations::Trajectories;
use std::sync::Arc;

fn line(n: i64, turn_id: &str) -> NewEvent {
    NewEvent {
        kind: "user".into(),
        harness_item_id: Some(format!("item-{n}")),
        harness_turn_id: Some(turn_id.into()),
        payload: format!(r#"{{"n":{n}}}"#),
        content: json!({"role": "user", "blocks": [{"type": "text", "text": n.to_string()}]}),
    }
}

/// An assistant Message in `session` on `turn`, and the turn's row.
async fn seed_turn(pool: &PgPool, task_id: &str, turn: i32, session: &str, agent_turn_id: &str) {
    let mut msg = make_message_row(task_id, &format!("msg-{turn}"));
    msg.message_type = MessageType::Assistant;
    msg.turn_number = Some(turn);
    msg.session_id = Some(session.into());
    seed_message(pool, &msg, &[], &Default::default())
        .await
        .expect("seed message");
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status, agent_turn_id) \
         VALUES ($1, $2, 'completed', $3)",
    )
    .bind(task_id)
    .bind(turn)
    .bind(agent_turn_id)
    .execute(pool)
    .await
    .expect("seed turn");
}

/// Turn 1 ran in Codex, turn 2 in Claude Code after a switch; the Task is on
/// `claude-code`. A rewind to turn 1 anchors on the Codex turn, and the next
/// turn's session renders only turn 1 into Claude Code.
#[sqlx::test]
async fn a_rewind_into_a_codex_turn_transcodes_it_for_claude(pool: PgPool) {
    let repo = moatless_workspaces::test_support::make_repository_row("repo-1", "user-1");
    moatless_workspaces::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");
    seed_task(&pool, &make_task_row("t", "user-1"))
        .await
        .expect("seed task");
    seed_turn(&pool, "t", 1, "thread-1", "turn-1").await;

    let dao = PgTrajectoryDao(pool.clone());
    let trajectories = Arc::new(Trajectories::new(PgTrajectoryDao(pool.clone())));
    let mut codex: Vec<NewEvent> = (0..4).map(|n| line(n, "turn-1")).collect();
    codex[3].kind = "event_msg/task_complete".into();
    let appended = dao
        .append(AppendRequest {
            task_id: "t",
            harness: Harness::Codex,
            harness_session_id: "thread-1",
            first_seq: 0,
            lines: &codex,
            turn_number: Some(1),
            subagent_parent: None,
            own_from_seq: None,
        })
        .await
        .unwrap();
    assert!(appended.accepted);

    let switched = trajectories
        .open_switch_session("t", Harness::ClaudeCode)
        .await
        .unwrap()
        .expect("a head to switch from");
    let claude_session = switched.session.harness_session_id.clone();
    let claude: Vec<NewEvent> = (10..12).map(|n| line(n, "claude-turn")).collect();
    assert!(
        dao.append(AppendRequest {
            task_id: "t",
            harness: Harness::ClaudeCode,
            harness_session_id: &claude_session,
            first_seq: 0,
            lines: &claude,
            turn_number: Some(2),
            subagent_parent: None,
            own_from_seq: Some(0),
        })
        .await
        .unwrap()
        .accepted
    );
    seed_turn(&pool, "t", 2, &claude_session, "claude-turn").await;

    let service = TaskRewindService::new(
        pool.clone(),
        Arc::new(PgTaskDao::new(pool.clone())),
        Arc::new(PgTaskTurnDao(pool.clone())),
        Arc::new(PgMessageDao(pool.clone())),
        trajectories.clone(),
    );
    let owner = ScopeContext {
        user_id: "user-1".into(),
        is_admin: false,
    };
    service.rewind(&owner, "t", 1).await.expect("rewind");

    let cut: uuid::Uuid =
        sqlx::query_scalar("SELECT source_event_id FROM task_forks WHERE task_id = 't'")
            .fetch_one(&pool)
            .await
            .expect("the rewind leaves an anchor");
    let anchors = trajectories
        .cut_anchors(cut)
        .await
        .unwrap()
        .expect("the cut's anchors");
    assert_eq!(
        anchors.claude_message_id(),
        None,
        "a Codex cut carries no Claude uuid"
    );
    assert_eq!(anchors.harness_turn_id.as_deref(), Some("turn-1"));
    let codex_tip: (String, i64) = sqlx::query_as(
        "SELECT s.harness, e.seq FROM trajectory_events e \
             JOIN trajectory_sessions s ON s.id = e.session_id WHERE e.id = $1",
    )
    .bind(cut)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        codex_tip,
        ("codex".to_string(), 3),
        "the cut is turn 1's last line"
    );

    let resumed = trajectories
        .open_fork_session("t", Harness::ClaudeCode, cut)
        .await
        .unwrap()
        .expect("a session to resume");
    assert_eq!(resumed.session.forked_from_event_id, None);
    let texts: Vec<&str> = resumed
        .transcode
        .iter()
        .filter_map(|item| item.content["blocks"][0]["text"].as_str())
        .collect();
    assert_eq!(texts, ["0", "1", "2", "3"], "turn 1 only");
    assert!(
        resumed
            .transcode
            .iter()
            .all(|item| item.source_harness == "codex")
    );
}
