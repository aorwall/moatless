//! PostgreSQL tests for the trajectory store.

use serde_json::json;
use sqlx::PgPool;

use super::*;

fn line(n: i64) -> NewEvent {
    NewEvent {
        kind: "user".into(),
        harness_item_id: Some(format!("item-{n}")),
        harness_turn_id: None,
        payload: format!(r#"{{"n":{n},"sessionId":"s"}}"#),
        content: json!({"role": "user", "blocks": [{"type": "text", "text": n.to_string()}]}),
    }
}

fn lines(range: std::ops::Range<i64>) -> Vec<NewEvent> {
    range.map(line).collect()
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM trajectory_events")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn seeded(pool: &PgPool, session: &str, events: Vec<NewEvent>) -> PgTrajectoryDao {
    let dao = PgTrajectoryDao(pool.clone());
    let outcome = dao
        .append("source", Harness::ClaudeCode, session, 0, &events, Some(1))
        .await
        .unwrap();
    assert!(outcome.accepted);
    dao
}

#[sqlx::test]
async fn append_is_idempotent_and_rejects_gaps(pool: PgPool) {
    let dao = PgTrajectoryDao(pool.clone());
    let first = dao
        .append("t", Harness::Codex, "s", 0, &lines(0..10), None)
        .await
        .unwrap();
    let resent = dao
        .append("t", Harness::Codex, "s", 5, &lines(5..15), None)
        .await
        .unwrap();
    let early = dao
        .append("t", Harness::Codex, "s", 20, &lines(20..25), None)
        .await
        .unwrap();

    assert_eq!(
        first,
        AppendOutcome {
            accepted: true,
            last_seq: 9,
            last_payload: None,
        }
    );
    assert_eq!(
        resent,
        AppendOutcome {
            accepted: true,
            last_seq: 14,
            last_payload: None,
        }
    );
    assert_eq!(
        early,
        AppendOutcome {
            accepted: false,
            last_seq: 14,
            last_payload: None,
        }
    );
    assert_eq!(event_count(&pool).await, 15);

    let session = dao.find_session("s").await.unwrap().unwrap();
    let chain = dao.chain(&session).await.unwrap();
    let expected: Vec<String> = lines(0..15).into_iter().map(|l| l.payload).collect();
    assert_eq!(
        chain, expected,
        "one event per line, each parented on the one before"
    );
}

#[sqlx::test]
async fn append_refuses_another_tasks_session(pool: PgPool) {
    let dao = seeded(&pool, "s", lines(0..2)).await;
    let other = dao
        .append("intruder", Harness::ClaudeCode, "s", 2, &lines(2..3), None)
        .await;
    assert!(matches!(other, Err(AppendError::Foreign(_))));
    assert_eq!(event_count(&pool).await, 2);
}

#[sqlx::test]
async fn fork_copies_no_events(pool: PgPool) {
    let dao = seeded(&pool, "source-session", lines(0..5)).await;
    let before = event_count(&pool).await;

    let cut = dao
        .resolve_cut("source-session", &ForkCut::Message("item-2".into()))
        .await
        .unwrap()
        .expect("item-2 resolves");
    let fork = dao
        .insert_fork_session("fork", Harness::ClaudeCode, cut)
        .await
        .unwrap();
    let again = dao
        .insert_fork_session("fork", Harness::ClaudeCode, cut)
        .await
        .unwrap();

    assert_eq!(fork.id, again.id, "a retried call returns the same session");
    assert_eq!(event_count(&pool).await, before);
    assert_eq!(fork.last_seq, 2);
    assert_ne!(fork.harness_session_id, "source-session");
    let prefix: Vec<String> = lines(0..3).into_iter().map(|l| l.payload).collect();
    assert_eq!(dao.chain(&fork).await.unwrap(), prefix);

    let own = dao
        .append(
            "fork",
            Harness::ClaudeCode,
            &fork.harness_session_id,
            3,
            &lines(30..32),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        own,
        AppendOutcome {
            accepted: true,
            last_seq: 4,
            last_payload: None,
        }
    );
    let fork = dao
        .find_session(&fork.harness_session_id)
        .await
        .unwrap()
        .unwrap();
    let chain = dao.chain(&fork).await.unwrap();
    assert_eq!(&chain[..3], &prefix[..]);
    assert_eq!(chain[3], line(30).payload);
    let source = dao.find_session("source-session").await.unwrap().unwrap();
    assert_eq!(dao.chain(&source).await.unwrap().len(), 5, "source intact");

    let later = dao
        .insert_fork_session("fork", Harness::ClaudeCode, cut)
        .await
        .unwrap();
    assert_ne!(later.id, fork.id, "a session that recorded is never reused");
}

#[sqlx::test]
async fn fork_of_a_fork_walks_both_fork_points(pool: PgPool) {
    let dao = seeded(&pool, "root", lines(0..4)).await;
    let cut = dao
        .resolve_cut("root", &ForkCut::Message("item-1".into()))
        .await
        .unwrap()
        .unwrap();
    let first = dao
        .insert_fork_session("f1", Harness::ClaudeCode, cut)
        .await
        .unwrap();
    dao.append(
        "f1",
        Harness::ClaudeCode,
        &first.harness_session_id,
        2,
        &lines(10..13),
        None,
    )
    .await
    .unwrap();

    let cut = dao
        .resolve_cut(
            &first.harness_session_id,
            &ForkCut::Message("item-11".into()),
        )
        .await
        .unwrap()
        .unwrap();
    let second = dao
        .insert_fork_session("f2", Harness::ClaudeCode, cut)
        .await
        .unwrap();

    let payloads: Vec<String> = [0, 1, 10, 11]
        .into_iter()
        .map(|n| line(n).payload)
        .collect();
    assert_eq!(dao.chain(&second).await.unwrap(), payloads);
}

#[sqlx::test]
async fn an_unresolved_cut_uses_the_old_fork_path(pool: PgPool) {
    let dao = seeded(&pool, "source-session", lines(0..3)).await;

    let unknown_message = dao
        .resolve_cut("source-session", &ForkCut::Message("not-recorded".into()))
        .await
        .unwrap();
    let unknown_turn = dao
        .resolve_cut("source-session", &ForkCut::Turn("turn-x".into()))
        .await
        .unwrap();
    let unknown_session = dao
        .resolve_cut("never-recorded", &ForkCut::End)
        .await
        .unwrap();

    assert_eq!(unknown_message, None);
    assert_eq!(unknown_turn, None);
    assert_eq!(unknown_session, None);
}

#[sqlx::test]
async fn a_codex_cut_lands_on_the_turns_task_complete(pool: PgPool) {
    let mut events = lines(0..5);
    for event in &mut events {
        event.harness_turn_id = Some("turn-1".into());
    }
    events[3].kind = "event_msg/task_complete".into();
    let dao = seeded(&pool, "thread", events).await;

    let cut = dao
        .resolve_cut("thread", &ForkCut::Turn("turn-1".into()))
        .await
        .unwrap()
        .unwrap();
    let fork = dao
        .insert_fork_session("fork", Harness::ClaudeCode, cut)
        .await
        .unwrap();
    assert_eq!(fork.last_seq, 3);
}

#[sqlx::test]
async fn a_cut_at_a_tool_use_moves_to_its_result(pool: PgPool) {
    let mut events = lines(0..5);
    events[1].content = json!({"role": "assistant", "blocks": [
        {"type": "tool_use", "id": "call-1", "name": "Bash", "input": {}}]});
    events[3].content = json!({"role": "user", "blocks": [
        {"type": "tool_result", "tool_use_id": "call-1"}]});
    let dao = seeded(&pool, "s", events).await;

    let cut = dao
        .resolve_cut("s", &ForkCut::Message("item-1".into()))
        .await
        .unwrap()
        .unwrap();
    let fork = dao
        .insert_fork_session("fork", Harness::ClaudeCode, cut)
        .await
        .unwrap();
    assert_eq!(fork.last_seq, 3);
}

#[sqlx::test]
async fn a_missing_seq_is_a_gap(pool: PgPool) {
    let dao = seeded(&pool, "s", lines(0..3)).await;
    sqlx::query("UPDATE trajectory_sessions SET last_seq = 5")
        .execute(&pool)
        .await
        .unwrap();
    let session = dao.find_session("s").await.unwrap().unwrap();
    assert!(matches!(
        dao.chain(&session).await,
        Err(ChainError::Gap { expected: 3 })
    ));
}

#[sqlx::test]
async fn an_empty_append_returns_the_tip_payload(pool: PgPool) {
    let dao = seeded(&pool, "source-session", lines(0..3)).await;
    let probe = |task: &'static str, session: String| {
        let dao = &dao;
        async move {
            dao.append(task, Harness::ClaudeCode, &session, 0, &[], None)
                .await
                .unwrap()
        }
    };

    let held = probe("source", "source-session".into()).await;
    assert_eq!(held.last_payload, Some(line(2).payload));

    let cut = dao
        .resolve_cut("source-session", &ForkCut::Message("item-1".into()))
        .await
        .unwrap()
        .unwrap();
    let fork = dao
        .insert_fork_session("fork", Harness::ClaudeCode, cut)
        .await
        .unwrap();
    let forked = probe("fork", fork.harness_session_id).await;
    assert_eq!(forked.last_seq, 1);
    assert_eq!(
        forked.last_payload,
        Some(line(1).payload),
        "the cut's payload"
    );

    let unknown = probe("t", "never-recorded".into()).await;
    assert_eq!((unknown.last_seq, unknown.last_payload), (-1, None));
    assert!(dao.find_session("never-recorded").await.unwrap().is_none());
}
