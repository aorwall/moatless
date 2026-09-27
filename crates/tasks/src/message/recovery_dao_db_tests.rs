//! PostgreSQL tests for the turn-recovery reads over Trajectory lines.

use sqlx::PgPool;

use crate::message::dao::{MessageDao, PgMessageDao};
use crate::task::test_support::make_task_row;
use moatless_workspaces::test_support::make_repository_row;

async fn seed_task(pool: &PgPool, task_id: &str, user: &str) {
    let repo = make_repository_row("repo-1", user);
    moatless_workspaces::compose::seed_repository(pool, &repo)
        .await
        .expect("seed repo");
    crate::task::test_support::seed_task(pool, &make_task_row(task_id, user))
        .await
        .expect("seed task");
}

/// Insert an `agent.<sub_type>` event on `turn_number` at an explicit
/// `created_at`, so a test can order recovery markers against output.
async fn seed_agent_event_at(
    pool: &PgPool,
    task_id: &str,
    sub_type: &str,
    turn_number: i32,
    created_at: &str,
) {
    sqlx::query(
        r#"INSERT INTO task_events
               (event_id, task_id, type, sub_type, turn_number, agent_profile, payload, timestamp, created_at)
           VALUES ($1, $2, 'agent', $3, $4, 'claude-code', '{}'::jsonb, $5, $5::timestamptz)"#,
    )
    .bind(format!("{task_id}-agent-{sub_type}-{turn_number}-{created_at}"))
    .bind(task_id)
    .bind(sub_type)
    .bind(turn_number)
    .bind(created_at)
    .execute(pool)
    .await
    .unwrap();
}

/// An assistant Trajectory line of `task_id`'s turn, written at `created_at`.
async fn seed_assistant_line_at(pool: &PgPool, task_id: &str, turn_number: i32, created_at: &str) {
    let session = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO trajectory_sessions (id, task_id, harness, harness_session_id)
         VALUES ($1, $2, 'claude-code', $3)",
    )
    .bind(session)
    .bind(task_id)
    .bind(session.to_string())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO trajectory_events
               (id, session_id, seq, kind, turn_number, payload, content, created_at)
           VALUES ($1, $2, 0, 'assistant', $3, '{}', '{"role":"assistant","blocks":[]}',
                   $4::timestamptz)"#,
    )
    .bind(uuid::Uuid::new_v4())
    .bind(session)
    .bind(turn_number)
    .bind(created_at)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn turn_progressed_since_last_recovery_tracks_output_after_the_newest_recovery(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());

    // Nothing published yet: no progress.
    assert!(
        !repo
            .turn_progressed_since_last_recovery("task-1", 1)
            .await
            .unwrap()
    );

    // A recovery re-drive that then produced assistant output: progress.
    seed_agent_event_at(
        &pool,
        "task-1",
        "recovery_started",
        1,
        "2026-01-01T00:00:00Z",
    )
    .await;
    seed_assistant_line_at(&pool, "task-1", 1, "2026-01-01T00:01:00Z").await;
    assert!(
        repo.turn_progressed_since_last_recovery("task-1", 1)
            .await
            .unwrap()
    );

    // A newer recovery re-drive with no output after it: a crash loop, no
    // progress — the earlier message predates this attempt.
    seed_agent_event_at(
        &pool,
        "task-1",
        "recovery_started",
        1,
        "2026-01-01T00:02:00Z",
    )
    .await;
    assert!(
        !repo
            .turn_progressed_since_last_recovery("task-1", 1)
            .await
            .unwrap()
    );
}

/// One top-level session of `task_id`, whose lines the tests below append.
async fn seed_session(pool: &PgPool, task_id: &str) -> uuid::Uuid {
    let session = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO trajectory_sessions (id, task_id, harness, harness_session_id)
         VALUES ($1, $2, 'claude-code', $3)",
    )
    .bind(session)
    .bind(task_id)
    .bind(session.to_string())
    .execute(pool)
    .await
    .unwrap();
    session
}

/// Appends a line with `content` on `turn_number` at `seq`.
async fn seed_line(
    pool: &PgPool,
    session: uuid::Uuid,
    seq: i64,
    turn_number: i32,
    content: serde_json::Value,
) {
    sqlx::query(
        "INSERT INTO trajectory_events (id, session_id, seq, kind, turn_number, payload, content)
         VALUES ($1, $2, $3, 'line', $4, '{}', $5)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(session)
    .bind(seq)
    .bind(turn_number)
    .bind(content)
    .execute(pool)
    .await
    .unwrap();
}

fn user_line(text: &str) -> serde_json::Value {
    serde_json::json!({"role": "user", "blocks": [{"type": "text", "text": text}]})
}

/// A final reply, which the harness recorded as ending the turn.
fn reply_line(text: &str) -> serde_json::Value {
    let mut line = prelude_line(text);
    line["ends_turn"] = true.into();
    line
}

/// Assistant text whose response had not finished when it was written.
fn prelude_line(text: &str) -> serde_json::Value {
    serde_json::json!({"role": "assistant", "blocks": [{"type": "text", "text": text}]})
}

fn call_line(id: &str) -> serde_json::Value {
    serde_json::json!({"role": "assistant", "blocks": [
        {"type": "tool_use", "id": id, "name": "Bash", "input": {}}
    ]})
}

async fn completed(repo: &PgMessageDao, turn_number: Option<i32>) -> bool {
    repo.last_message_is_completed_reply("task-1", turn_number)
        .await
        .unwrap()
}

#[sqlx::test]
async fn an_assistant_text_line_last_is_a_completed_reply(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());
    assert!(!completed(&repo, Some(1)).await);

    let session = seed_session(&pool, "task-1").await;
    seed_line(&pool, session, 0, 1, user_line("hi")).await;
    assert!(!completed(&repo, Some(1)).await, "the agent never replied");

    seed_line(&pool, session, 1, 1, reply_line("done")).await;
    assert!(completed(&repo, Some(1)).await);
    assert!(
        completed(&repo, None).await,
        "unscoped reads the Task's last line"
    );

    // Housekeeping and injected lines after the reply are not what a reader sees.
    seed_line(&pool, session, 2, 1, serde_json::json!({"blocks": []})).await;
    let mut injected = user_line("<system-reminder>");
    injected["injected"] = true.into();
    seed_line(&pool, session, 3, 1, injected).await;
    assert!(completed(&repo, Some(1)).await);
}

#[sqlx::test]
async fn a_turn_cut_off_mid_tool_is_not_a_completed_reply(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());
    let session = seed_session(&pool, "task-1").await;
    seed_line(&pool, session, 0, 1, prelude_line("let me look")).await;
    seed_line(&pool, session, 1, 1, call_line("toolu_1")).await;
    assert!(!completed(&repo, Some(1)).await);

    let result = serde_json::json!({"role": "user", "blocks": [
        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"}
    ]});
    seed_line(&pool, session, 2, 1, result).await;
    assert!(!completed(&repo, Some(1)).await);
}

#[sqlx::test]
async fn a_completed_reply_is_scoped_to_its_turn(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());
    let session = seed_session(&pool, "task-1").await;
    seed_line(&pool, session, 0, 1, call_line("toolu_1")).await;
    seed_line(&pool, session, 1, 2, reply_line("done")).await;

    assert!(
        !completed(&repo, Some(1)).await,
        "turn 2's reply does not close turn 1, which has output of its own"
    );
    assert!(completed(&repo, Some(2)).await);
}

#[sqlx::test]
async fn a_coalesced_turn_is_closed_by_the_later_reply(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());
    // Turn 1 took an item and said nothing; the run's reply landed on turn 2.
    sqlx::query(
        "INSERT INTO task_inbox (id, task_id, text, origin, sent_by, status, delivered_at, \
         session_id, turn_number) \
         VALUES (gen_random_uuid(), 'task-1', '', 'user', 'user-1', 'delivered', now(), 's', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let session = seed_session(&pool, "task-1").await;
    seed_line(&pool, session, 0, 1, user_line("first")).await;
    seed_line(
        &pool,
        session,
        1,
        1,
        serde_json::json!({"role": "assistant", "blocks": []}),
    )
    .await;
    seed_line(&pool, session, 2, 2, user_line("second")).await;
    seed_line(&pool, session, 3, 2, reply_line("both done")).await;

    assert!(completed(&repo, Some(1)).await);
    assert!(completed(&repo, Some(2)).await);
}

#[sqlx::test]
async fn a_switch_sessions_transcoded_prefix_is_no_reply(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());
    let source = seed_session(&pool, "task-1").await;
    seed_line(&pool, source, 0, 1, reply_line("from claude")).await;
    let switch = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO trajectory_sessions
             (id, task_id, harness, harness_session_id, transcoded_from_event_id, own_from_seq)
         SELECT $1, 'task-1', 'codex', $2, id, 1 FROM trajectory_events WHERE session_id = $3",
    )
    .bind(switch)
    .bind(switch.to_string())
    .bind(source)
    .execute(&pool)
    .await
    .unwrap();
    seed_line(&pool, switch, 0, 2, reply_line("from claude")).await;

    assert!(!completed(&repo, Some(2)).await);
}

#[sqlx::test]
async fn a_text_prelude_before_an_interrupted_tool_call_is_not_a_completed_reply(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());
    let session = seed_session(&pool, "task-1").await;
    seed_line(&pool, session, 0, 1, user_line("fix it")).await;
    // The sandbox died before the response's `tool_use` line was written.
    seed_line(&pool, session, 1, 1, prelude_line("I'll run the tests")).await;

    assert!(!completed(&repo, Some(1)).await);
    assert!(!completed(&repo, None).await);
}

#[sqlx::test]
async fn a_role_less_turn_end_closes_the_turn(pool: PgPool) {
    seed_task(&pool, "task-1", "user-1").await;
    let repo = PgMessageDao(pool.clone());
    let session = seed_session(&pool, "task-1").await;
    seed_line(&pool, session, 0, 1, user_line("hi")).await;
    seed_line(&pool, session, 1, 1, prelude_line("done")).await;
    assert!(!completed(&repo, Some(1)).await);

    // Codex records the turn's end as its own `task_complete` line.
    let end = serde_json::json!({"role": null, "blocks": [], "ends_turn": true});
    seed_line(&pool, session, 2, 1, end).await;
    assert!(completed(&repo, Some(1)).await);
}
