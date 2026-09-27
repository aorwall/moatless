//! A conversation replayed from its sources: `rebuild`, a fork's inheritance
//! and what an ancestor's rewind or delete leaves a fork (4.9a).

use std::sync::Arc;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use moatless_types::access_control::ScopeContext;
use moatless_types::agent::Harness;

use crate::message::dao::{MessageDao, PgMessageDao};
use crate::message::test_support::{
    acknowledge_messages, freeze, queue_named, record_external_event, seed_agent_blocks,
    seed_agent_line,
};
use crate::task::dao::PgTaskDao;
use crate::task::fork_dao::{PgTaskForkDao, TaskForkDao};
use crate::task::models::TaskFork;
use crate::task::rewind::TaskRewindService;
use crate::task::test_support::{remove_task, seed_bare_task};
use crate::task_turn::dao::PgTaskTurnDao;
use crate::trajectory::dao::PgTrajectoryDao;
use crate::trajectory::operations::Trajectories;

fn owner() -> ScopeContext {
    ScopeContext {
        user_id: "user-1".to_string(),
        is_admin: false,
    }
}

/// One closed turn: the person's item, delivered to `session`, and the
/// agent's reply. Returns the reply's event, the cut a fork after it names.
async fn run_turn(pool: &PgPool, task: &str, session: &str, turn: i32, prompt: &str) -> Uuid {
    let item = queue_named(pool, task, &format!("{task}:{turn}"), prompt).await;
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status, started_at) \
         VALUES ($1, $2, 'active', now())",
    )
    .bind(task)
    .bind(turn)
    .execute(pool)
    .await
    .unwrap();
    acknowledge_messages(pool, task, &[item], session, turn)
        .await
        .unwrap();
    let reply = seed_agent_line(pool, task, session, Some(turn), &format!("re: {prompt}")).await;
    sqlx::query(
        "UPDATE task_turns SET status = 'completed', ended_at = now() \
         WHERE task_id = $1 AND turn_number = $2",
    )
    .bind(task)
    .bind(turn)
    .execute(pool)
    .await
    .unwrap();
    reply
}

/// Forks `source` at `cut` into `fork` as fork creation does: the fork row,
/// then the inherited conversation. Returns the fork's own session id.
async fn fork(pool: &PgPool, source: &str, fork: &str, at_turn: i32, cut: Uuid) -> String {
    seed_bare_task(pool, fork, "open").await;
    PgTaskForkDao(pool.clone())
        .insert(&TaskFork {
            task_id: fork.to_string(),
            source_task_id: source.to_string(),
            at_turn: Some(at_turn),
            source_event_id: cut,
        })
        .await
        .unwrap();
    PgMessageDao(pool.clone())
        .inherit_conversation(fork, source, cut)
        .await
        .unwrap();
    PgTrajectoryDao(pool.clone())
        .insert_fork_session(fork, Harness::ClaudeCode, cut)
        .await
        .unwrap()
        .harness_session_id
}

/// `task`'s top-level Messages in order: uuid, author and the text a reader
/// shows, the item's for a user Message.
async fn conversation(pool: &PgPool, task: &str) -> Vec<(Uuid, String, Option<String>)> {
    sqlx::query_as(
        r#"SELECT m.uuid, m.author, COALESCE(m.text, i.text)
             FROM conversation_messages m LEFT JOIN task_inbox i ON i.id = m.inbox_id
            WHERE m.task_id = $1 AND m.parent_tool_use_id IS NULL
            ORDER BY m.position"#,
    )
    .bind(task)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn texts(rows: &[(Uuid, String, Option<String>)]) -> Vec<(&str, &str)> {
    rows.iter()
        .map(|(_, author, text)| (author.as_str(), text.as_deref().unwrap_or("")))
        .collect()
}

fn uuids(rows: &[(Uuid, String, Option<String>)]) -> Vec<Uuid> {
    rows.iter().map(|(uuid, _, _)| *uuid).collect()
}

async fn activity_ids(pool: &PgPool, task: &str) -> Vec<(Uuid, Option<Uuid>, String)> {
    sqlx::query_as(
        r#"SELECT activity_id, message_uuid, kind FROM conversation_activities
            WHERE task_id = $1 ORDER BY position, ordinal"#,
    )
    .bind(task)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn rebuild(pool: &PgPool, task: &str) {
    PgMessageDao(pool.clone())
        .rebuild_conversation(task)
        .await
        .unwrap();
}

#[sqlx::test]
async fn rebuild_reproduces_rows_and_ids(pool: PgPool) {
    seed_bare_task(&pool, "a", "open").await;
    run_turn(&pool, "a", "sa", 1, "one").await;
    run_turn(&pool, "a", "sa", 2, "two").await;
    seed_agent_blocks(
        &pool,
        "a",
        "sa",
        Some(2),
        json!([{ "type": "tool_use", "id": "tu-1", "name": "Bash", "input": {} }]),
    )
    .await;
    let messages = conversation(&pool, "a").await;
    let activities = activity_ids(&pool, "a").await;
    assert_eq!(
        texts(&messages),
        [
            ("user", "one"),
            ("agent", "re: one"),
            ("user", "two"),
            ("agent", "re: two"),
        ]
    );
    assert_eq!(activities.len(), 1);

    rebuild(&pool, "a").await;

    assert_eq!(conversation(&pool, "a").await, messages);
    assert_eq!(activity_ids(&pool, "a").await, activities);
}

/// A fork's cut turn is closed and decided, so nothing reads the inherited
/// user Messages as a turn still owed (#1005).
#[sqlx::test]
async fn a_fork_gets_its_closed_cut_turn_with_no_trigger(pool: PgPool) {
    seed_bare_task(&pool, "a", "open").await;
    run_turn(&pool, "a", "sa", 1, "one").await;
    let cut = run_turn(&pool, "a", "sa", 2, "two").await;
    sqlx::query("UPDATE task_turns SET status = 'active', ended_at = NULL WHERE turn_number = 2")
        .execute(&pool)
        .await
        .unwrap();

    fork(&pool, "a", "b", 2, cut).await;

    let turns: Vec<(i32, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT turn_number, status, trigger_decided, triggered_by_task_id FROM task_turns \
         WHERE task_id = 'b' ORDER BY turn_number",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(turns, [(2, "stopped".to_string(), true, None)]);
}

/// Each generation holds its ancestors' rows up to its cut under ids of its
/// own, reproduced by a rebuild; nothing it inherits claims an echo.
#[sqlx::test]
async fn a_fork_of_a_fork_holds_each_ancestors_rows_up_to_its_cut(pool: PgPool) {
    seed_bare_task(&pool, "a", "open").await;
    let a1 = run_turn(&pool, "a", "sa", 1, "a one").await;
    run_turn(&pool, "a", "sa", 2, "a two").await;
    record_external_event(&pool, "a", "slack:1", "slack", None)
        .await
        .unwrap();

    let sb = fork(&pool, "a", "b", 1, a1).await;
    let b2 = run_turn(&pool, "b", &sb, 2, "b two").await;
    run_turn(&pool, "b", &sb, 3, "b three").await;

    fork(&pool, "b", "c", 2, b2).await;

    let a = conversation(&pool, "a").await;
    let b = conversation(&pool, "b").await;
    let c = conversation(&pool, "c").await;
    assert_eq!(
        texts(&c),
        [
            ("user", "a one"),
            ("agent", "re: a one"),
            ("user", "b two"),
            ("agent", "re: b two"),
        ]
    );
    for id in uuids(&c) {
        assert!(!uuids(&a).contains(&id), "{id} is also a's");
        assert!(!uuids(&b).contains(&id), "{id} is also b's");
    }
    let claims: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM task_outbound_events WHERE task_id = 'c') \
              + (SELECT count(*) FROM task_inbox WHERE task_id = 'c')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claims, 0, "an inherited row claims nothing of its own");

    rebuild(&pool, "c").await;
    assert_eq!(conversation(&pool, "c").await, c);
    rebuild(&pool, "b").await;
    assert_eq!(conversation(&pool, "b").await, b);
}

/// An ancestor rewound past the cut keeps the items a fork inherited, and
/// the fork's rebuild still replays them. Its item is `unknown`, as one a
/// frozen history recorded, since that origin constrains the status.
#[sqlx::test]
async fn an_ancestor_rewound_past_the_cut_leaves_the_fork_whole(pool: PgPool) {
    seed_bare_task(&pool, "a", "open").await;
    let a1 = run_turn(&pool, "a", "sa", 1, "a one").await;
    sqlx::query("UPDATE task_inbox SET origin = 'unknown' WHERE task_id = 'a'")
        .execute(&pool)
        .await
        .unwrap();
    let sb = fork(&pool, "a", "b", 1, a1).await;
    run_turn(&pool, "b", &sb, 2, "b two").await;
    let before = conversation(&pool, "b").await;

    TaskRewindService::new(
        pool.clone(),
        Arc::new(PgTaskDao::new(pool.clone())),
        Arc::new(PgTaskTurnDao(pool.clone())),
        Arc::new(PgMessageDao(pool.clone())),
        Arc::new(Trajectories::new(PgTrajectoryDao(pool.clone()))),
    )
    .rewind(&owner(), "a", 0)
    .await
    .expect("rewind");

    assert!(texts(&conversation(&pool, "a").await).is_empty());
    assert_eq!(conversation(&pool, "b").await, before);
    rebuild(&pool, "b").await;
    assert_eq!(conversation(&pool, "b").await, before);
}

/// Deleting an ancestor keeps the items its forks' Messages read.
#[sqlx::test]
async fn deleting_an_ancestor_keeps_its_forks_user_messages(pool: PgPool) {
    seed_bare_task(&pool, "a", "open").await;
    let a1 = run_turn(&pool, "a", "sa", 1, "a one").await;
    run_turn(&pool, "a", "sa", 2, "a two").await;
    fork(&pool, "a", "b", 1, a1).await;
    let before = conversation(&pool, "b").await;

    assert!(remove_task(&pool, "a", &owner()).await.unwrap());

    assert_eq!(conversation(&pool, "b").await, before);
    let left: Vec<String> = sqlx::query_scalar("SELECT text FROM task_inbox WHERE task_id = 'a'")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, ["a one"], "the item no fork reads went with its Task");
}

/// A frozen call settles only when the turn open at the freeze wrote it.
#[sqlx::test]
async fn a_frozen_call_settles_only_in_the_turn_open_at_the_freeze(pool: PgPool) {
    use crate::activities::mapping::{ActivityStatus, ToolOutcome};

    seed_bare_task(&pool, "a", "open").await;
    let mut asking = crate::message::test_support::make_message_row("a", "m-ask");
    asking.message_type = crate::message::models::MessageType::Assistant;
    let call = crate::activities::mapping::map_content(
        crate::activities::mapping::BlockContext {
            task_id: "a",
            message_uuid: "m-ask",
            parent_uuid: None,
            message_type: asking.message_type,
            block_index: 0,
            source: "m-ask",
            source_block: 0,
        },
        &json!([{ "type": "tool_use", "id": "tu-old", "name": "Bash", "input": {} }]),
    );
    crate::message::test_support::seed_message(&pool, &asking, &[], &call)
        .await
        .unwrap();
    let outcome = ToolOutcome {
        task_id: "a".to_string(),
        tool_use_id: "tu-old".to_string(),
        status: ActivityStatus::Completed,
        result: json!("done"),
        typed_result: None,
        is_error: false,
    };
    let status = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM conversation_activities WHERE tool_use_id = 'tu-old'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let settle = || async {
        let mut tx = pool.begin().await.unwrap();
        super::dao::settle(&mut tx, &outcome, None).await.unwrap();
        tx.commit().await.unwrap();
    };

    freeze(&pool, "a", None).await.unwrap();
    settle().await;
    assert_eq!(status().await, "in_progress", "no turn was open");

    freeze(
        &pool,
        "a",
        Some(chrono::Utc::now() + chrono::Duration::hours(1)),
    )
    .await
    .unwrap();
    settle().await;
    assert_eq!(status().await, "in_progress", "an earlier turn wrote it");

    freeze(
        &pool,
        "a",
        Some(chrono::Utc::now() - chrono::Duration::hours(1)),
    )
    .await
    .unwrap();
    settle().await;
    assert_eq!(status().await, "completed");
}
