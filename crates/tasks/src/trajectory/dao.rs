//! `trajectory_sessions` and `trajectory_events`: append, chain walk, cut
//! resolution and fork sessions.

use moatless_types::agent::Harness;

use moatless_core::db::DbPool;
use uuid::Uuid;

use super::lineage::LineageRow;
use super::models::{
    AppendError, AppendOutcome, AppendRequest, ChainError, CutAnchors, Head, LineProjection,
    NewEvent, TrajectorySession,
};
use crate::conversation::projector::Projector;

macro_rules! session_columns {
    () => {
        "id, task_id, harness, harness_session_id, forked_from_event_id, parent_event_id, \
         transcoded_from_event_id, own_from_seq, last_event_id, last_seq"
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

/// Every event on `$1`'s lineage: its chain, and from each line of a switch
/// session's prefix (and its first own line) the other harness's tip the
/// prefix was rendered from.
macro_rules! lineage {
    () => {
        "WITH RECURSIVE lineage(id, parent_event_id, jump) AS (
            SELECT e.id, e.parent_event_id,
                   CASE WHEN e.seq <= s.own_from_seq THEN s.transcoded_from_event_id END
              FROM trajectory_events e JOIN trajectory_sessions s ON s.id = e.session_id
             WHERE e.id = $1
            UNION
            SELECT e.id, e.parent_event_id,
                   CASE WHEN e.seq <= s.own_from_seq THEN s.transcoded_from_event_id END
              FROM lineage l
              JOIN trajectory_events e ON e.id = l.parent_event_id OR e.id = l.jump
              JOIN trajectory_sessions s ON s.id = e.session_id
        )
        SELECT e.id, e.session_id, s.harness, e.turn_number, e.content,
               COALESCE(e.seq < s.own_from_seq AND s.transcoded_from_event_id IS NOT NULL,
                        false) AS prefix
          FROM lineage JOIN trajectory_events e USING (id)
          JOIN trajectory_sessions s ON s.id = e.session_id
         ORDER BY e.ord"
    };
}

pub(crate) struct PgTrajectoryDao(pub(crate) DbPool);

/// The first tool call on `event_id`'s line: the call a subagent session
/// hangs under.
pub(crate) async fn spawning_call_in(
    tx: &mut sqlx::PgConnection,
    event_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT b ->> 'id'
             FROM trajectory_events e,
                  jsonb_array_elements(COALESCE(e.content -> 'blocks', '[]'::jsonb)) b
            WHERE e.id = $1 AND b ->> 'type' = 'tool_use'
            LIMIT 1"#,
    )
    .bind(event_id)
    .fetch_optional(tx)
    .await
}

/// How many blocks the session's lines of `item` hold before `seq`.
pub(crate) async fn blocks_before_in(
    tx: &mut sqlx::PgConnection,
    session_id: Uuid,
    item: &str,
    seq: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(jsonb_array_length(content -> 'blocks')), 0)::bigint
             FROM trajectory_events
            WHERE session_id = $1 AND harness_item_id = $2 AND seq < $3
              AND jsonb_typeof(content -> 'blocks') = 'array'"#,
    )
    .bind(session_id)
    .bind(item)
    .bind(seq)
    .fetch_one(tx)
    .await
}

impl PgTrajectoryDao {
    /// Append `lines` as seqs `first_seq..`, creating the session on its first
    /// append. Lines the session already holds are skipped; a batch starting
    /// past `last_seq + 1` inserts nothing and is not accepted.
    pub async fn append(&self, req: AppendRequest<'_>) -> Result<AppendOutcome, AppendError> {
        let AppendRequest {
            task_id,
            harness,
            harness_session_id,
            first_seq,
            lines,
            turn_number,
            subagent_parent,
            own_from_seq,
        } = req;
        let mut tx = self.0.begin().await?;
        let parent_event_id: Option<Uuid> = match subagent_parent {
            Some(parent) => {
                // `COALESCE` evaluates the sibling subquery only when the root
                // holds no match, so the root wins a shared id and the
                // `<root>:agent-*` sessions are scanned only for a nested subagent.
                sqlx::query_scalar(
                    r#"SELECT COALESCE(
                           (SELECT e.id
                              FROM trajectory_events e
                              JOIN trajectory_sessions s ON s.id = e.session_id
                             WHERE s.harness_session_id = $1 AND s.task_id = $2
                               AND e.content -> 'blocks'
                                   @> jsonb_build_array(jsonb_build_object('type', 'tool_use', 'id', $3::text))
                             ORDER BY e.seq
                             LIMIT 1),
                           (SELECT e.id
                              FROM trajectory_events e
                              JOIN trajectory_sessions s ON s.id = e.session_id
                             WHERE s.task_id = $2 AND starts_with(s.harness_session_id, $1 || ':agent-')
                               AND e.content -> 'blocks'
                                   @> jsonb_build_array(jsonb_build_object('type', 'tool_use', 'id', $3::text))
                             ORDER BY s.created_at, e.seq
                             LIMIT 1))"#,
                )
                .bind(parent.harness_session_id)
                .bind(task_id)
                .bind(parent.tool_use_id)
                .fetch_one(&mut *tx)
                .await?
            }
            None => None,
        };
        let created: Option<Uuid> = sqlx::query_scalar(
            r#"INSERT INTO trajectory_sessions
                   (id, task_id, harness, harness_session_id, parent_event_id)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (harness_session_id) DO NOTHING
               RETURNING id"#,
        )
        .bind(Uuid::now_v7())
        .bind(task_id)
        .bind(harness.as_str())
        .bind(harness_session_id)
        .bind(parent_event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if created.is_some() && subagent_parent.is_some() && parent_event_id.is_none() {
            return Err(AppendError::ParentMissing(harness_session_id.to_string()));
        }
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
                projected: LineProjection::default(),
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
                projected: LineProjection::default(),
            });
        }

        let mut session = session;
        if session.own_from_seq.is_none() {
            let switch = session.transcoded_from_event_id.is_some();
            // A frozen Task's first session file holds its pre-freeze lines,
            // which its frozen Messages already show.
            let frozen = !switch
                && own_from_seq.is_some()
                && session.last_seq < 0
                && session.parent_event_id.is_none()
                && session.forked_from_event_id.is_none()
                && crate::conversation::replay_dao::frozen_at(&mut tx, task_id)
                    .await?
                    .is_some();
            match own_from_seq {
                None if switch => {
                    return Err(AppendError::OwnFromSeqMissing(
                        harness_session_id.to_string(),
                    ));
                }
                Some(own_from_seq) if switch || frozen => {
                    sqlx::query("UPDATE trajectory_sessions SET own_from_seq = $2 WHERE id = $1")
                        .bind(session.id)
                        .bind(own_from_seq)
                        .execute(&mut *tx)
                        .await?;
                    session.own_from_seq = Some(own_from_seq);
                }
                _ => {}
            }
        }
        let fresh: Vec<_> = fresh.iter().map(NewEvent::jsonb_safe).collect();
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
                   AS l(id, seq, parent, kind, item, turn, payload, content)
                ORDER BY seq"#,
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
        let projected = Projector
            .on_lines(
                &mut tx,
                &session,
                session.last_seq + 1,
                &ids.iter()
                    .copied()
                    .zip(fresh.iter().map(|l| &**l))
                    .collect::<Vec<_>>(),
                turn_number,
            )
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
            projected,
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

    /// The event `task_id` continues from: its latest recorded top-level
    /// session's tip, else its fork cut. `None` when it has neither.
    pub async fn head(&self, task_id: &str) -> Result<Option<Head>, sqlx::Error> {
        let own = sqlx::query_as::<_, Head>(
            r#"SELECT s.last_event_id AS event_id, s.harness, s.harness_session_id,
                      false AS forked
                 FROM trajectory_sessions s
                 JOIN trajectory_events e ON e.id = s.last_event_id
                WHERE s.task_id = $1 AND s.parent_event_id IS NULL
                ORDER BY e.ord DESC LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;
        if own.is_some() {
            return Ok(own);
        }
        sqlx::query_as::<_, Head>(
            r#"SELECT f.source_event_id AS event_id, s.harness, s.harness_session_id,
                      true AS forked
                 FROM task_forks f
                 JOIN trajectory_events e ON e.id = f.source_event_id
                 JOIN trajectory_sessions s ON s.id = e.session_id
                WHERE f.task_id = $1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    /// The anchors of `event_id`; `None` when unknown.
    pub async fn cut_anchors(&self, event_id: Uuid) -> Result<Option<CutAnchors>, sqlx::Error> {
        sqlx::query_as(
            "SELECT s.harness, s.harness_session_id, e.harness_item_id, e.harness_turn_id \
               FROM trajectory_events e \
               JOIN trajectory_sessions s ON s.id = e.session_id WHERE e.id = $1",
        )
        .bind(event_id)
        .fetch_optional(&self.0)
        .await
    }

    /// The harness of the session `event_id` is in; `None` when unknown.
    pub async fn event_harness(&self, event_id: Uuid) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT s.harness FROM trajectory_events e \
               JOIN trajectory_sessions s ON s.id = e.session_id WHERE e.id = $1",
        )
        .bind(event_id)
        .fetch_optional(&self.0)
        .await
    }

    /// Every event on `tip`'s lineage, in insertion (`ord`) order.
    pub(crate) async fn lineage(&self, tip: Uuid) -> Result<Vec<LineageRow>, sqlx::Error> {
        sqlx::query_as(lineage!())
            .bind(tip)
            .fetch_all(&self.0)
            .await
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

    /// The switch session `task_id` opens in `harness`: `forked_from`'s file
    /// verbatim, if any, then a prefix rendered from `transcoded_from`'s
    /// lineage. Idempotent like [`Self::insert_fork_session`], under the same
    /// lock.
    pub async fn insert_switch_session(
        &self,
        task_id: &str,
        harness: Harness,
        forked_from: Option<Uuid>,
        transcoded_from: Uuid,
    ) -> Result<TrajectorySession, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('trajectory_fork:' || $1))")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        let existing = sqlx::query_as::<_, TrajectorySession>(concat!(
            "SELECT ",
            session_columns!(),
            " FROM trajectory_sessions
             WHERE task_id = $1 AND harness = $2 AND transcoded_from_event_id = $3
               AND forked_from_event_id IS NOT DISTINCT FROM $4 AND last_event_id IS NULL
             ORDER BY created_at DESC LIMIT 1"
        ))
        .bind(task_id)
        .bind(harness.as_str())
        .bind(transcoded_from)
        .bind(forked_from)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(session) = existing {
            tx.commit().await?;
            return Ok(session);
        }
        let session = sqlx::query_as::<_, TrajectorySession>(concat!(
            "INSERT INTO trajectory_sessions
                 (id, task_id, harness, harness_session_id, forked_from_event_id,
                  transcoded_from_event_id, last_seq)
             VALUES ($1, $2, $3, $4, $5, $6,
                     COALESCE((SELECT seq FROM trajectory_events WHERE id = $5), -1))
             RETURNING ",
            session_columns!()
        ))
        .bind(Uuid::now_v7())
        .bind(task_id)
        .bind(harness.as_str())
        .bind(Uuid::now_v7().to_string())
        .bind(forked_from)
        .bind(transcoded_from)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(session)
    }
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
