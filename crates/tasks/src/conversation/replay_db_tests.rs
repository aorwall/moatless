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
use crate::task::dao::{PgTaskDao, TaskDao};
use crate::task::fork_dao::{PgTaskForkDao, TaskForkDao};
use crate::task::models::TaskFork;
use crate::task::rewind::TaskRewindService;
use crate::task::test_support::{remove_task, seed_bare_task};
use crate::task_turn::dao::PgTaskTurnDao;
use crate::trajectory::dao::PgTrajectoryDao;
use crate::trajectory::operations::Trajectories;

fn owner() -> ScopeContext {
    ScopeContext::user("user-1")
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

#[sqlx::test]
async fn rebuild_reproduces_a_stopped_recording(pool: PgPool) {
    use crate::task_event::dao::{PgTaskEventDao, TaskEventDao};
    use crate::task_event::models::InsertTaskEvent;
    seed_bare_task(&pool, "a", "open").await;
    run_turn(&pool, "a", "sa", 1, "one").await;
    PgTaskEventDao(pool.clone())
        .insert_event(&InsertTaskEvent {
            event_id: "evt-stopped".to_string(),
            task_id: "a".to_string(),
            event_type: "agent".to_string(),
            sub_type: "recording_stopped".to_string(),
            is_error: true,
            agent_profile: Some("claude-code".to_string()),
            turn_number: Some(1),
            payload: json!({ "error": "line 7 is too large" }),
            timestamp: "2026-10-15T06:54:00.000Z".to_string(),
        })
        .await
        .unwrap();
    let activities = activity_ids(&pool, "a").await;
    assert_eq!(
        activities
            .iter()
            .map(|(_, _, kind)| kind.as_str())
            .collect::<Vec<_>>(),
        ["sandbox_error"]
    );

    rebuild(&pool, "a").await;

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
    let tasks = PgTaskDao::new(pool.clone());
    assert!(
        !tasks
            .close_if_unused("b", &ScopeContext::user("other"))
            .await
            .unwrap()
    );
    assert!(tasks.close_if_unused("b", &owner()).await.unwrap());
    sqlx::query("UPDATE tasks SET status = 'open' WHERE task_id = 'b'")
        .execute(&pool)
        .await
        .unwrap();
    queue_named(&pool, "b", "own-question", "what does this mean?").await;
    assert!(!tasks.close_if_unused("b", &owner()).await.unwrap());
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

/// A fork of a Task upgraded from before recording shows the frozen history
/// under ids of its own, then the recorded turns up to the cut; a rebuild
/// keeps it as it was.
#[sqlx::test]
async fn a_fork_of_an_upgraded_task_shows_its_frozen_history(pool: PgPool) {
    use crate::activities::mapping::{AdmittedUsage, BlockContext, map_content};
    use crate::message::models::{MessageRole, MessageType};
    use crate::message::test_support::{make_message_row, seed_message};

    seed_bare_task(&pool, "a", "open").await;
    let mut asked = make_message_row("a", "legacy-q");
    asked.turn_number = Some(1);
    asked.sent_by = Some("user-1".to_string());
    asked.message_text = Some("legacy q".to_string());
    let write = |uuid: &str, message_type, content: serde_json::Value| {
        map_content(
            BlockContext {
                task_id: "a",
                message_uuid: uuid,
                parent_uuid: None,
                message_type,
                block_index: 0,
                source: uuid,
                source_block: 0,
            },
            &content,
        )
    };
    seed_message(
        &pool,
        &asked,
        &[],
        &write("legacy-q", MessageType::User, json!("legacy q")),
    )
    .await
    .unwrap();
    let mut answered = make_message_row("a", "legacy-a");
    answered.turn_number = Some(1);
    answered.message_type = MessageType::Assistant;
    answered.message_role = MessageRole::Assistant;
    let mut answer = write(
        "legacy-a",
        MessageType::Assistant,
        json!([
            { "type": "text", "text": "legacy answer" },
            { "type": "tool_use", "id": "tu-legacy", "name": "Bash", "input": {} }
        ]),
    );
    answer.usage = Some(AdmittedUsage {
        input_tokens: 10,
        output_tokens: 20,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
    });
    seed_message(&pool, &answered, &[], &answer).await.unwrap();
    freeze(&pool, "a", None).await.unwrap();
    let cut = run_turn(&pool, "a", "sa", 2, "new q").await;

    fork(&pool, "a", "b", 2, cut).await;

    let a = conversation(&pool, "a").await;
    let b = conversation(&pool, "b").await;
    assert_eq!(
        texts(&b),
        [
            ("user", "legacy q"),
            ("agent", "legacy answer"),
            ("user", "new q"),
            ("agent", "re: new q"),
        ]
    );
    for id in uuids(&b) {
        assert!(!uuids(&a).contains(&id), "{id} is also a's");
    }
    let activities = activity_ids(&pool, "b").await;
    assert_eq!(
        activities,
        [(activities[0].0, Some(b[1].0), "tool_call".to_string())],
        "the frozen call hangs on the fork's copy of its Message"
    );
    let usage: Vec<(Uuid, i32)> = sqlx::query_as(
        "SELECT message_uuid, output_tokens FROM conversation_usage \
         WHERE task_id = 'b' AND message_uuid IS NOT NULL",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(usage, [(b[1].0, 20)]);

    rebuild(&pool, "b").await;
    assert_eq!(conversation(&pool, "b").await, b);
    assert_eq!(activity_ids(&pool, "b").await, activities);
}

#[sqlx::test]
async fn side_chat_context_is_frozen_on_the_first_inbox_item(pool: PgPool) {
    use crate::inbox::view::PendingView;
    use crate::message::enricher::AgentPromptBuilder;
    use crate::message::test_support::{queue_message, user_item};
    use crate::task::relations::record_forked_from;
    use crate::task::relations_dao::PgTaskRelationDao;
    use moatless_types::agent::AgentProfile;

    seed_bare_task(&pool, "a", "open").await;
    let cut = run_turn(&pool, "a", "sa", 1, "inherited").await;
    fork(&pool, "a", "b", 1, cut).await;
    record_forked_from(&PgTaskRelationDao(pool.clone()), "b", "a", Some(1), true)
        .await
        .unwrap();
    run_turn(&pool, "a", "sa", 2, "new work").await;
    queue_named(&pool, "a", "pending-parent", "pending parent question").await;

    let mut first = user_item("b", Uuid::new_v4(), "like this?");
    first.options.queue = true;
    let mut tx = pool.begin().await.unwrap();
    let rolled_back = crate::inbox::dao::submit(&mut tx, &first)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rolled_back.options.contexts.len(), 1);
    tx.rollback().await.unwrap();
    let stored = queue_message(&pool, first.clone()).await.unwrap().unwrap();
    assert!(queue_message(&pool, first).await.unwrap().is_none());
    run_turn(&pool, "a", "sa", 3, "after enqueue").await;
    let second = queue_message(&pool, user_item("b", Uuid::new_v4(), "second question"))
        .await
        .unwrap()
        .unwrap();
    assert!(second.options.contexts.is_empty());
    let reloaded = crate::inbox::dao::find(&pool, stored.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.options.contexts, stored.options.contexts);
    let context = &reloaded.options.contexts[0];
    assert_eq!(context.context_type, "sideChat");
    assert_eq!(context.reference_id, "a");
    let text = context.data["text"].as_str().unwrap();
    assert!(text.contains("do not continue or redo the main thread's task"));
    assert!(text.contains("re: new work"));
    assert!(text.contains("pending parent question"));
    assert!(text.find("pending parent question") < text.find("re: new work"));
    assert!(text.find("re: new work") < text.find("- user: new work"));
    assert!(!text.contains("re: inherited"));
    assert!(!text.contains("after enqueue"));

    let view = PendingView::of(&[reloaded, second]);
    let logins = std::collections::HashMap::new();
    let builder = AgentPromptBuilder::new(
        None,
        None,
        AgentProfile::ClaudeCode,
        None,
        None,
        vec![],
        &logins,
    );
    let deliveries = builder
        .build_pending_deliveries(&view.messages, &view.contexts)
        .await;
    assert!(deliveries[0].prompt_text.starts_with("# Side Chat"));
    let prompt = &deliveries[0].prompt_text;
    assert_eq!(prompt.matches("# Side Chat").count(), 1);
    assert!(prompt.ends_with("like this?\nsecond question"));
}

#[sqlx::test]
async fn concurrent_first_side_chat_messages_attach_framing_once_even_without_a_digest(
    pool: PgPool,
) {
    use crate::message::test_support::{queue_message, user_item};
    use crate::task::relations::record_forked_from;
    use crate::task::relations_dao::PgTaskRelationDao;

    seed_bare_task(&pool, "parent", "open").await;
    seed_bare_task(&pool, "side", "open").await;
    record_forked_from(
        &PgTaskRelationDao(pool.clone()),
        "side",
        "parent",
        None,
        true,
    )
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        queue_message(&pool, user_item("side", Uuid::new_v4(), "one")),
        queue_message(&pool, user_item("side", Uuid::new_v4(), "two")),
    );
    let rows = [a.unwrap().unwrap(), b.unwrap().unwrap()];
    let contexts: Vec<_> = rows.iter().flat_map(|row| &row.options.contexts).collect();
    assert_eq!(contexts.len(), 1);
    assert!(
        contexts[0].data["text"]
            .as_str()
            .unwrap()
            .contains("# Side Chat")
    );
}

#[sqlx::test]
async fn side_chat_context_skips_internal_inputs_and_existing_messages(pool: PgPool) {
    use crate::inbox::InboxOrigin;
    use crate::message::test_support::{queue_message, user_item};
    use crate::task::relations::record_forked_from;
    use crate::task::relations_dao::PgTaskRelationDao;

    seed_bare_task(&pool, "parent", "open").await;
    for (task, side_chat, own_turn) in [
        ("new", true, false),
        ("existing", true, true),
        ("fork", false, false),
    ] {
        seed_bare_task(&pool, task, "open").await;
        if own_turn {
            run_turn(&pool, task, "session", 1, "already asked").await;
        }
        record_forked_from(
            &PgTaskRelationDao(pool.clone()),
            task,
            "parent",
            None,
            side_chat,
        )
        .await
        .unwrap();
        for (origin, answer) in [
            (InboxOrigin::Sandbox, None),
            (
                InboxOrigin::User {
                    sent_by: "user-1".into(),
                },
                Some("tool".into()),
            ),
        ] {
            let mut input = user_item(task, Uuid::new_v4(), "internal input");
            input.origin = origin;
            input.answers_tool_use_id = answer;
            let stored = queue_message(&pool, input).await.unwrap().unwrap();
            assert!(stored.options.contexts.is_empty());
        }
        let stored = queue_message(&pool, user_item(task, Uuid::new_v4(), "question"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.options.contexts.len(),
            usize::from(side_chat && !own_turn)
        );
    }
}

#[sqlx::test]
async fn side_chat_context_waits_through_compact_and_goal_commands(pool: PgPool) {
    use crate::message::test_support::{queue_message, user_item};
    use crate::task::relations::record_forked_from;
    use crate::task::relations_dao::PgTaskRelationDao;

    seed_bare_task(&pool, "parent", "open").await;
    seed_bare_task(&pool, "side", "open").await;
    record_forked_from(
        &PgTaskRelationDao(pool.clone()),
        "side",
        "parent",
        None,
        true,
    )
    .await
    .unwrap();
    for (turn, command) in [(1, "  /compact keep the plan"), (2, "/goal")] {
        run_turn(&pool, "side", "session", turn, command).await;
        let rows = crate::inbox::dao::delivered_in_turn(&pool, "side", turn)
            .await
            .unwrap();
        assert!(rows[0].options.contexts.is_empty());
    }
    let first = queue_message(&pool, user_item("side", Uuid::new_v4(), "question"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.options.contexts.len(), 1);
}

#[sqlx::test]
async fn merge_back_is_guarded_durable_and_delivered_once_with_the_next_message(pool: PgPool) {
    use crate::inbox::view::PendingView;
    use crate::message::enricher::AgentPromptBuilder;
    use crate::message::test_support::{queue_message, user_item};
    use crate::tag::dao::PgTagDao;
    use crate::task::fork::TaskForkService;
    use crate::task::relations::{TaskRelationDao, record_forked_from};
    use crate::task::relations_dao::PgTaskRelationDao;
    use moatless_types::agent::AgentProfile;
    seed_bare_task(&pool, "a", "open").await;
    seed_bare_task(&pool, "other", "open").await;
    let cut = run_turn(&pool, "a", "sa", 1, "inherited main task").await;
    let session = fork(&pool, "a", "b", 1, cut).await;
    let relations = PgTaskRelationDao(pool.clone());
    record_forked_from(&relations, "b", "a", Some(1), true)
        .await
        .unwrap();
    assert!(
        relations
            .find_parent("b")
            .await
            .unwrap()
            .unwrap()
            .is_side_chat()
    );
    run_turn(&pool, "b", &session, 2, "side question").await;
    run_turn(&pool, "b", &session, 3, "later question").await;
    let service = TaskForkService::new(
        Arc::new(PgTaskDao::new(pool.clone())),
        Arc::new(PgTaskTurnDao(pool.clone())),
        Arc::new(PgMessageDao(pool.clone())),
        Arc::new(PgTagDao(pool.clone())),
        Arc::new(Trajectories::new(PgTrajectoryDao(pool.clone()))),
        Arc::new(relations),
        pool.clone(),
    );
    assert!(
        service
            .merge_back(&ScopeContext::user("outsider"), "b", "a", 2)
            .await
            .is_err()
    );
    assert!(service.merge_back(&owner(), "b", "other", 2).await.is_err());
    assert!(service.merge_back(&owner(), "b", "a", 1).await.is_err());
    assert!(service.merge_back(&owner(), "b", "a", 99).await.is_err());
    sqlx::query("UPDATE task_turns SET status = 'active', ended_at = NULL WHERE task_id = 'a'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(service.merge_back(&owner(), "b", "a", 2).await.is_err());
    sqlx::query("UPDATE task_turns SET status = 'completed', ended_at = now() WHERE task_id = 'a'")
        .execute(&pool)
        .await
        .unwrap();
    let busy = queue_named(&pool, "a", "queued-parent", "already queued").await;
    assert!(service.merge_back(&owner(), "b", "a", 2).await.is_err());
    acknowledge_messages(&pool, "a", std::slice::from_ref(&busy), "sa", 1)
        .await
        .unwrap();
    sqlx::query("UPDATE task_turns SET status = 'active', ended_at = NULL WHERE task_id = 'b' AND turn_number = 3").execute(&pool).await.unwrap();
    assert!(service.merge_back(&owner(), "b", "a", 3).await.is_err());
    service.merge_back(&owner(), "b", "a", 2).await.unwrap();
    service.merge_back(&owner(), "b", "a", 2).await.unwrap();
    let pending = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM task_merge_backs WHERE target_task_id = 'a' AND pending",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(pending().await, 1);
    assert!(
        crate::message::test_support::pending_user_messages(&pool, "a")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        queue_message(&pool, user_item("a", busy.parse().unwrap(), "duplicate"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(pending().await, 1);
    for (origin, answers_tool_use_id) in [
        (crate::inbox::InboxOrigin::Sandbox, None),
        (
            crate::inbox::InboxOrigin::User {
                sent_by: "owner".into(),
            },
            Some("tool".into()),
        ),
    ] {
        let mut tx = pool.begin().await.unwrap();
        let mut item = user_item("a", Uuid::new_v4(), "context-free input");
        item.origin = origin;
        item.answers_tool_use_id = answers_tool_use_id;
        let staged = crate::inbox::dao::submit(&mut tx, &item)
            .await
            .unwrap()
            .unwrap();
        assert!(staged.options.contexts.is_empty());
        let mut probe = pool.begin().await.unwrap();
        sqlx::query("SELECT task_id FROM tasks WHERE task_id = 'a' FOR NO KEY UPDATE NOWAIT")
            .execute(&mut *probe)
            .await
            .unwrap();
        probe.rollback().await.unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(pending().await, 1);
    }
    let mut tx = pool.begin().await.unwrap();
    let staged = crate::inbox::dao::submit(&mut tx, &user_item("a", Uuid::new_v4(), "rolled back"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(staged.options.contexts.len(), 1);
    tx.rollback().await.unwrap();
    assert_eq!(pending().await, 1);
    remove_task(&pool, "b", &owner()).await.unwrap();
    let first = user_item("a", Uuid::new_v4(), "next message");
    let second = user_item("a", Uuid::new_v4(), "another message");
    let (first, second) = tokio::join!(queue_message(&pool, first), queue_message(&pool, second));
    let rows = [first.unwrap().unwrap(), second.unwrap().unwrap()];
    assert_eq!(
        rows.iter()
            .map(|row| row.options.contexts.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(pending().await, 0);
    let view = PendingView::of(&rows);
    let logins = std::collections::HashMap::new();
    let builder = AgentPromptBuilder::new(
        None,
        None,
        AgentProfile::ClaudeCode,
        None,
        None,
        vec![],
        &logins,
    );
    let deliveries = builder
        .build_pending_deliveries(&view.messages, &view.contexts)
        .await;
    let text = &deliveries[0].prompt_text;
    assert_eq!(
        text.matches("Merge-back context from forked conversation.")
            .count(),
        1
    );
    assert!(text.contains("side question"));
    assert!(text.contains("re: side question"));
    assert!(text.contains("next message"));
    assert!(!text.contains("inherited main task"));
    assert!(!text.contains("later question"));
}
