//! `trajectory_sessions` and `trajectory_events`: append, chain walk, cut
//! resolution and fork sessions.

use std::collections::HashSet;

use moatless_core::db::DbPool;
use serde_json::Value;
use uuid::Uuid;

use super::models::{
    AppendError, AppendOutcome, ChainError, ForkCut, Harness, NewEvent, TrajectorySession,
};

macro_rules! session_columns {
    () => {
        "id, task_id, harness, harness_session_id, forked_from_event_id, last_event_id, last_seq"
    };
}

/// Every event from `$1` back to its root, crossing fork points, as `walk(id)`.
macro_rules! walk {
    () => {
        "WITH RECURSIVE walk(id, parent_event_id) AS (
            SELECT id, parent_event_id FROM trajectory_events WHERE id = $1
            UNION ALL
            SELECT e.id, e.parent_event_id
              FROM trajectory_events e JOIN walk w ON e.id = w.parent_event_id
        ) "
    };
}

#[derive(sqlx::FromRow)]
struct CutRow {
    id: Uuid,
    kind: String,
    harness_item_id: Option<String>,
    harness_turn_id: Option<String>,
    content: Value,
}

pub(crate) struct PgTrajectoryDao(pub(crate) DbPool);

impl PgTrajectoryDao {
    /// Append `lines` as seqs `first_seq..`, creating the session on its first
    /// append. Lines the session already holds are skipped; a batch starting
    /// past `last_seq + 1` inserts nothing and is not accepted.
    pub async fn append(
        &self,
        task_id: &str,
        harness: Harness,
        harness_session_id: &str,
        first_seq: i64,
        lines: &[NewEvent],
        turn_number: Option<i32>,
    ) -> Result<AppendOutcome, AppendError> {
        let mut tx = self.0.begin().await?;
        sqlx::query(
            r#"INSERT INTO trajectory_sessions (id, task_id, harness, harness_session_id)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (harness_session_id) DO NOTHING"#,
        )
        .bind(Uuid::now_v7())
        .bind(task_id)
        .bind(harness.as_str())
        .bind(harness_session_id)
        .execute(&mut *tx)
        .await?;
        let session = sqlx::query_as::<_, TrajectorySession>(concat!(
            "SELECT ",
            session_columns!(),
            " FROM trajectory_sessions
             WHERE harness_session_id = $1 FOR UPDATE"
        ))
        .bind(harness_session_id)
        .fetch_one(&mut *tx)
        .await?;
        if session.task_id != task_id || session.harness != harness.as_str() {
            return Err(AppendError::Foreign(harness_session_id.to_string()));
        }

        if first_seq > session.last_seq + 1 {
            return Ok(AppendOutcome {
                accepted: false,
                last_seq: session.last_seq,
                last_payload: None,
            });
        }
        let held = usize::try_from(session.last_seq + 1 - first_seq).unwrap_or(0);
        let fresh = lines.get(held..).unwrap_or_default();
        if fresh.is_empty() {
            let tip = session.last_event_id.or(session.forked_from_event_id);
            let last_payload = match tip {
                Some(tip) if lines.is_empty() => {
                    sqlx::query_scalar("SELECT payload FROM trajectory_events WHERE id = $1")
                        .bind(tip)
                        .fetch_optional(&mut *tx)
                        .await?
                }
                _ => None,
            };
            return Ok(AppendOutcome {
                accepted: true,
                last_seq: session.last_seq,
                last_payload,
            });
        }

        let ids: Vec<Uuid> = fresh.iter().map(|_| Uuid::now_v7()).collect();
        let mut parents = Vec::with_capacity(ids.len());
        parents.push(session.last_event_id.or(session.forked_from_event_id));
        parents.extend(ids.iter().take(ids.len() - 1).copied().map(Some));
        let seqs: Vec<i64> = (session.last_seq + 1..).take(ids.len()).collect();
        let last_seq = seqs.last().copied().unwrap_or(session.last_seq);

        sqlx::query(
            r#"INSERT INTO trajectory_events
                   (id, session_id, seq, parent_event_id, kind, harness_item_id,
                    harness_turn_id, turn_number, payload, content)
               SELECT id, $1, seq, parent, kind, item, turn, $2, payload, content
                 FROM UNNEST($3::uuid[], $4::bigint[], $5::uuid[], $6::text[], $7::text[],
                             $8::text[], $9::text[], $10::jsonb[])
                   AS l(id, seq, parent, kind, item, turn, payload, content)"#,
        )
        .bind(session.id)
        .bind(turn_number)
        .bind(&ids)
        .bind(&seqs)
        .bind(&parents)
        .bind(fresh.iter().map(|l| l.kind.clone()).collect::<Vec<_>>())
        .bind(
            fresh
                .iter()
                .map(|l| l.harness_item_id.clone())
                .collect::<Vec<_>>(),
        )
        .bind(
            fresh
                .iter()
                .map(|l| l.harness_turn_id.clone())
                .collect::<Vec<_>>(),
        )
        .bind(fresh.iter().map(|l| l.payload.clone()).collect::<Vec<_>>())
        .bind(fresh.iter().map(|l| l.content.clone()).collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE trajectory_sessions SET last_seq = $2, last_event_id = $3 WHERE id = $1",
        )
        .bind(session.id)
        .bind(last_seq)
        .bind(ids.last().copied())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(AppendOutcome {
            accepted: true,
            last_seq,
            last_payload: None,
        })
    }

    /// The session a harness resumes by `harness_session_id`.
    pub async fn find_session(
        &self,
        harness_session_id: &str,
    ) -> Result<Option<TrajectorySession>, sqlx::Error> {
        sqlx::query_as::<_, TrajectorySession>(concat!(
            "SELECT ",
            session_columns!(),
            " FROM trajectory_sessions WHERE harness_session_id = $1"
        ))
        .bind(harness_session_id)
        .fetch_optional(&self.0)
        .await
    }

    /// Every payload of `session`'s file, root-first, crossing fork points.
    /// `Err(Gap)` when a seq is missing.
    pub async fn chain(&self, session: &TrajectorySession) -> Result<Vec<String>, ChainError> {
        let Some(tip) = session.last_event_id.or(session.forked_from_event_id) else {
            return Ok(Vec::new());
        };
        let rows: Vec<(i64, String)> = sqlx::query_as(concat!(
            walk!(),
            " SELECT e.seq, e.payload FROM walk JOIN trajectory_events e USING (id)
             ORDER BY e.seq"
        ))
        .bind(tip)
        .fetch_all(&self.0)
        .await?;
        check_contiguous(rows.iter().map(|(seq, _)| *seq), session.last_seq)?;
        Ok(rows.into_iter().map(|(_, payload)| payload).collect())
    }

    /// The event a fork of `source_harness_session_id` continues from, or
    /// `None` when the session is unknown or no event matches `cut`.
    ///
    /// A cut that leaves a `tool_use` without its `tool_result` moves forward
    /// to the event carrying the last missing result.
    pub async fn resolve_cut(
        &self,
        source_harness_session_id: &str,
        cut: &ForkCut,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        let Some(source) = self.find_session(source_harness_session_id).await? else {
            return Ok(None);
        };
        let Some(tip) = source.last_event_id.or(source.forked_from_event_id) else {
            return Ok(None);
        };
        let rows: Vec<CutRow> = sqlx::query_as(concat!(
            walk!(),
            " SELECT e.id, e.kind, e.harness_item_id, e.harness_turn_id, e.content
               FROM walk JOIN trajectory_events e USING (id) ORDER BY e.seq"
        ))
        .bind(tip)
        .fetch_all(&self.0)
        .await?;

        let at = match cut {
            ForkCut::Message(id) => rows
                .iter()
                .rposition(|r| r.harness_item_id.as_deref() == Some(id.as_str())),
            ForkCut::Turn(turn) => rows.iter().rposition(|r| {
                r.harness_turn_id.as_deref() == Some(turn.as_str())
                    && r.kind == "event_msg/task_complete"
            }),
            ForkCut::End => rows.len().checked_sub(1),
        };
        let Some(at) = at else { return Ok(None) };
        let at = include_pending_results(&rows, at);
        Ok(Some(rows[at].id))
    }

    /// The fork session `fork_task_id` resumes from `forked_from_event_id`,
    /// with a fresh `UUIDv7` harness session id and no event of its own.
    ///
    /// Idempotent per fork task and cut while the session has recorded
    /// nothing: a retried call returns it, and a later branch at the same cut
    /// gets a new one.
    pub async fn insert_fork_session(
        &self,
        fork_task_id: &str,
        harness: Harness,
        forked_from_event_id: Uuid,
    ) -> Result<TrajectorySession, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('trajectory_fork:' || $1))")
            .bind(fork_task_id)
            .execute(&mut *tx)
            .await?;
        let existing = sqlx::query_as::<_, TrajectorySession>(concat!(
            "SELECT ",
            session_columns!(),
            " FROM trajectory_sessions
             WHERE task_id = $1 AND forked_from_event_id = $2 AND last_event_id IS NULL
             ORDER BY created_at DESC LIMIT 1"
        ))
        .bind(fork_task_id)
        .bind(forked_from_event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(session) = existing {
            tx.commit().await?;
            return Ok(session);
        }
        let session = sqlx::query_as::<_, TrajectorySession>(concat!(
            "INSERT INTO trajectory_sessions
                 (id, task_id, harness, harness_session_id, forked_from_event_id, last_seq)
             SELECT $1, $2, $3, $4, e.id, e.seq FROM trajectory_events e WHERE e.id = $5
             RETURNING ",
            session_columns!()
        ))
        .bind(Uuid::now_v7())
        .bind(fork_task_id)
        .bind(harness.as_str())
        .bind(Uuid::now_v7().to_string())
        .bind(forked_from_event_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(session)
    }
}

/// Move `at` forward to the last event carrying a `tool_result` for a
/// `tool_use` at or before `at`, when the source holds one.
fn include_pending_results(rows: &[CutRow], at: usize) -> usize {
    let blocks = |row: &CutRow| {
        row.content
            .get("blocks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut pending = HashSet::new();
    for row in &rows[..=at] {
        for block in blocks(row) {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        pending.insert(id.to_string());
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                        pending.remove(id);
                    }
                }
                _ => {}
            }
        }
    }
    let mut cut = at;
    for (i, row) in rows.iter().enumerate().skip(at + 1) {
        if pending.is_empty() {
            break;
        }
        for block in blocks(row) {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let id = block.get("tool_use_id").and_then(Value::as_str);
            if id.is_some_and(|id| pending.remove(id)) {
                cut = i;
            }
        }
    }
    cut
}

fn check_contiguous(seqs: impl Iterator<Item = i64>, last_seq: i64) -> Result<(), ChainError> {
    let mut expected = 0;
    for seq in seqs {
        if seq != expected {
            return Err(ChainError::Gap { expected });
        }
        expected += 1;
    }
    if expected != last_seq + 1 {
        return Err(ChainError::Gap { expected });
    }
    Ok(())
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
