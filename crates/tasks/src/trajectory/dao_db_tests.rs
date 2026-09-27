//! PostgreSQL tests for the trajectory store.

use serde_json::json;
use sqlx::PgPool;

use super::*;
use crate::trajectory::lineage::include_pending_results;
use crate::trajectory::models::{NewEvent, SubagentParent};

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

fn req<'a>(
    task_id: &'a str,
    harness: Harness,
    harness_session_id: &'a str,
    first_seq: i64,
    lines: &'a [NewEvent],
    turn_number: Option<i32>,
) -> AppendRequest<'a> {
    AppendRequest {
        task_id,
        harness,
        harness_session_id,
        first_seq,
        lines,
        turn_number,
        subagent_parent: None,
        own_from_seq: None,
    }
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
        .append(req(
            "source",
            Harness::ClaudeCode,
            session,
            0,
            &events,
            Some(1),
        ))
        .await
        .unwrap();
    assert!(outcome.accepted);
    dao
}

#[sqlx::test]
async fn append_is_idempotent_and_rejects_gaps(pool: PgPool) {
    let dao = PgTrajectoryDao(pool.clone());
    let first = dao
        .append(req("t", Harness::Codex, "s", 0, &lines(0..10), None))
        .await
        .unwrap();
    let resent = dao
        .append(req("t", Harness::Codex, "s", 5, &lines(5..15), None))
        .await
        .unwrap();
    let early = dao
        .append(req("t", Harness::Codex, "s", 20, &lines(20..25), None))
        .await
        .unwrap();

    assert_eq!(
        first,
        AppendOutcome {
            accepted: true,
            last_seq: 9,
            last_payload: None,
            projected: Default::default(),
        }
    );
    assert_eq!(
        resent,
        AppendOutcome {
            accepted: true,
            last_seq: 14,
            last_payload: None,
            projected: Default::default(),
        }
    );
    assert_eq!(
        early,
        AppendOutcome {
            accepted: false,
            last_seq: 14,
            last_payload: None,
            projected: Default::default(),
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
        .append(req(
            "intruder",
            Harness::ClaudeCode,
            "s",
            2,
            &lines(2..3),
            None,
        ))
        .await;
    assert!(matches!(other, Err(AppendError::Foreign(_))));
    assert_eq!(event_count(&pool).await, 2);
}

/// The event `session` recorded for `harness_item_id`.
async fn event(pool: &PgPool, session: &str, harness_item_id: &str) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM trajectory_events e JOIN trajectory_sessions s ON s.id = e.session_id \
          WHERE s.harness_session_id = $1 AND e.harness_item_id = $2",
    )
    .bind(session)
    .bind(harness_item_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn fork_copies_no_events(pool: PgPool) {
    let dao = seeded(&pool, "source-session", lines(0..5)).await;
    let before = event_count(&pool).await;

    let cut = event(&pool, "source-session", "item-2").await;
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
        .append(req(
            "fork",
            Harness::ClaudeCode,
            &fork.harness_session_id,
            3,
            &lines(30..32),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        own,
        AppendOutcome {
            accepted: true,
            last_seq: 4,
            last_payload: None,
            projected: Default::default(),
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
    let cut = event(&pool, "root", "item-1").await;
    let first = dao
        .insert_fork_session("f1", Harness::ClaudeCode, cut)
        .await
        .unwrap();
    dao.append(req(
        "f1",
        Harness::ClaudeCode,
        &first.harness_session_id,
        2,
        &lines(10..13),
        None,
    ))
    .await
    .unwrap();

    let cut = event(&pool, &first.harness_session_id, "item-11").await;
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
async fn a_cut_at_a_tool_use_moves_to_its_result(pool: PgPool) {
    let mut events = lines(0..5);
    events[1].content = json!({"role": "assistant", "blocks": [
        {"type": "tool_use", "id": "call-1", "name": "Bash", "input": {}}]});
    events[3].content = json!({"role": "user", "blocks": [
        {"type": "tool_result", "tool_use_id": "call-1"}]});
    let dao = seeded(&pool, "s", events).await;

    let tip = event(&pool, "s", "item-4").await;
    let rows = dao.lineage(tip).await.unwrap();
    let refs: Vec<&LineageRow> = rows.iter().collect();
    assert_eq!(include_pending_results(&refs, 1), 3);
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
            dao.append(req(task, Harness::ClaudeCode, &session, 0, &[], None))
                .await
                .unwrap()
        }
    };

    let held = probe("source", "source-session".into()).await;
    assert_eq!(held.last_payload, Some(line(2).payload));

    let cut = event(&pool, "source-session", "item-1").await;
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

fn tool_call(id: &str) -> NewEvent {
    NewEvent {
        kind: "assistant".into(),
        harness_item_id: Some(format!("msg-{id}")),
        harness_turn_id: None,
        payload: format!(r#"{{"tool":"{id}","sessionId":"s"}}"#),
        content: json!({
            "role": "assistant",
            "blocks": [{"type": "tool_use", "id": id, "name": "Agent", "input": {}}],
        }),
    }
}

#[sqlx::test]
async fn a_subagent_session_links_to_its_spawning_tool_call(pool: PgPool) {
    let dao = seeded(&pool, "parent", vec![line(0), tool_call("toolu_1")]).await;
    let parent = SubagentParent {
        harness_session_id: "parent",
        tool_use_id: "toolu_1",
    };
    let subagent = |task, session, parent| AppendRequest {
        subagent_parent: Some(parent),
        ..req(task, Harness::ClaudeCode, session, 0, &[], None)
    };

    let unrecorded = SubagentParent {
        tool_use_id: "toolu_2",
        ..parent
    };
    for (task, session, parent) in [
        ("source", "parent:agent-b", unrecorded),
        ("other", "parent:agent-c", parent),
    ] {
        let refused = dao.append(subagent(task, session, parent)).await;
        assert!(matches!(refused, Err(AppendError::ParentMissing(_))));
        assert!(dao.find_session(session).await.unwrap().is_none());
    }

    let probe = dao
        .append(subagent("source", "parent:agent-a", parent))
        .await
        .unwrap();
    assert_eq!(probe.last_seq, -1);
    let first = lines(0..2);
    dao.append(AppendRequest {
        lines: &first,
        ..subagent("source", "parent:agent-a", parent)
    })
    .await
    .unwrap();
    let session = dao.find_session("parent:agent-a").await.unwrap().unwrap();
    let spawning: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM trajectory_events e
           JOIN trajectory_sessions s ON s.id = e.session_id
          WHERE s.harness_session_id = 'parent' AND e.seq = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(session.parent_event_id, Some(spawning));

    let forked_too =
        sqlx::query("UPDATE trajectory_sessions SET forked_from_event_id = $2 WHERE id = $1")
            .bind(session.id)
            .bind(spawning)
            .execute(&pool)
            .await;
    assert!(forked_too.is_err(), "a subagent session never forks");
}

#[sqlx::test]
async fn a_retried_switch_read_returns_the_same_session(pool: PgPool) {
    let dao = seeded(&pool, "claude-1", lines(0..3)).await;
    let trajectories =
        crate::trajectory::operations::Trajectories::new(PgTrajectoryDao(pool.clone()));

    let first = trajectories
        .open_switch_session("source", Harness::Codex)
        .await
        .unwrap()
        .expect("a head to switch from");
    let again = trajectories
        .open_switch_session("source", Harness::Codex)
        .await
        .unwrap()
        .expect("a head to switch from");
    assert_eq!(first.session.id, again.session.id);
    assert_eq!(first.session.forked_from_event_id, None);
    assert_eq!(first.transcode.len(), 3, "every Claude line is rendered");

    let codex = first.session.harness_session_id.clone();
    let codex_lines = lines(10..12);
    let missing = dao
        .append(req(
            "source",
            Harness::Codex,
            &codex,
            0,
            &codex_lines,
            Some(2),
        ))
        .await;
    assert!(matches!(missing, Err(AppendError::OwnFromSeqMissing(_))));
    let mut own = req("source", Harness::Codex, &codex, 0, &codex_lines, Some(2));
    own.own_from_seq = Some(1);
    assert!(dao.append(own).await.unwrap().accepted);

    let back = trajectories
        .open_switch_session("source", Harness::ClaudeCode)
        .await
        .unwrap()
        .expect("a head to switch back from");
    let claude_tip = dao.find_session("claude-1").await.unwrap().unwrap();
    assert_eq!(
        back.session.forked_from_event_id, claude_tip.last_event_id,
        "switching back forks the Claude file verbatim"
    );
    assert_eq!(back.transcode.len(), 1, "only the Codex turn is rendered");
    let chain = dao.chain(&back.session).await.unwrap();
    assert_eq!(chain.len(), 3);
}

#[sqlx::test]
async fn the_fork_trigger_rejects_a_cross_harness_fork(pool: PgPool) {
    let dao = seeded(&pool, "claude-1", lines(0..2)).await;
    let tip = dao
        .find_session("claude-1")
        .await
        .unwrap()
        .unwrap()
        .last_event_id
        .unwrap();

    let refused = dao.insert_fork_session("fork", Harness::Codex, tip).await;
    let Err(sqlx::Error::Database(e)) = refused else {
        panic!("a Codex session forking a Claude event is refused: {refused:?}");
    };
    assert_eq!(e.code().as_deref(), Some("23514"));
    dao.insert_fork_session("fork", Harness::ClaudeCode, tip)
        .await
        .expect("the same harness forks");
}

/// A backend whose clock runs behind stamps a later session's events with ids
/// that sort before its source's: head and lineage follow insertion instead.
#[sqlx::test]
async fn a_switch_back_renders_lines_whose_ids_sort_first(pool: PgPool) {
    seeded(&pool, "claude-1", lines(0..3)).await;
    let trajectories =
        crate::trajectory::operations::Trajectories::new(PgTrajectoryDao(pool.clone()));
    let codex = trajectories
        .open_switch_session("source", Harness::Codex)
        .await
        .unwrap()
        .expect("a head to switch from")
        .session;

    let (first, second) = (Uuid::from_u128(1), Uuid::from_u128(2));
    for (id, seq, parent, text) in [(first, 0, None, "10"), (second, 1, Some(first), "11")] {
        sqlx::query(
            "INSERT INTO trajectory_events
                 (id, session_id, seq, parent_event_id, kind, turn_number, payload, content)
             VALUES ($1, $2, $3, $4, 'user', 2, '{}', $5)",
        )
        .bind(id)
        .bind(codex.id)
        .bind(seq)
        .bind(parent)
        .bind(json!({"role": "user", "blocks": [{"type": "text", "text": text}]}))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "UPDATE trajectory_sessions SET last_seq = 1, last_event_id = $2, own_from_seq = 0
          WHERE id = $1",
    )
    .bind(codex.id)
    .bind(second)
    .execute(&pool)
    .await
    .unwrap();

    let head = trajectories.head("source").await.unwrap().unwrap();
    assert_eq!(head.harness_session_id, codex.harness_session_id);
    let back = trajectories
        .open_switch_session("source", Harness::ClaudeCode)
        .await
        .unwrap()
        .expect("a head to switch back from");
    let texts: Vec<&str> = back
        .transcode
        .iter()
        .filter_map(|item| item.content["blocks"][0]["text"].as_str())
        .collect();
    assert_eq!(texts, ["10", "11"], "the Codex turn is rendered, in order");
}

/// Rows recorded before `ord` existed get it from the migration's backfill,
/// which must order a later session's low ids after its source's lines too.
#[sqlx::test]
async fn the_ord_backfill_orders_existing_rows_by_when_they_were_written(pool: PgPool) {
    const MIGRATION: &str = include_str!("../../../core/migrations/0175_trajectory_switch.sql");
    let start = MIGRATION
        .find("UPDATE trajectory_events e")
        .expect("the backfill statement");
    let backfill = &MIGRATION[start..start + MIGRATION[start..].find(';').unwrap()];

    seeded(&pool, "claude-1", lines(0..3)).await;
    let trajectories =
        crate::trajectory::operations::Trajectories::new(PgTrajectoryDao(pool.clone()));
    let codex = trajectories
        .open_switch_session("source", Harness::Codex)
        .await
        .unwrap()
        .expect("a head to switch from")
        .session;
    let low = Uuid::from_u128(1);
    sqlx::query(
        "INSERT INTO trajectory_events
             (id, session_id, seq, kind, turn_number, payload, content, created_at)
         VALUES ($1, $2, 0, 'user', 2, '{}', '{}', NOW() + interval '1 second')",
    )
    .bind(low)
    .bind(codex.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE trajectory_sessions SET last_seq = 0, last_event_id = $2, own_from_seq = 0
          WHERE id = $1",
    )
    .bind(codex.id)
    .bind(low)
    .execute(&pool)
    .await
    .unwrap();

    for statement in [
        "ALTER TABLE trajectory_events ALTER COLUMN ord DROP NOT NULL",
        "UPDATE trajectory_events SET ord = NULL",
        backfill,
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    let head = trajectories.head("source").await.unwrap().unwrap();
    assert_eq!(head.harness_session_id, codex.harness_session_id);
}
