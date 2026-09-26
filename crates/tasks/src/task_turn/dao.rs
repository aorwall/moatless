//! Task turn DAO — authoritative per-turn lifecycle persistence.
//!
//! [`TaskTurnDao::apply_ingress_event`] commits the event row, turn row, and
//! task-status flip in ONE transaction before the gRPC handler publishes the
//! event, so turn state is never lost to a dropped event; the event dispatcher
//! re-applies it idempotently via [`TaskTurnDao::project_turn`] as a backstop.
//!
//! This is ordering, not an outbox: nothing here is spooled, relayed, or
//! deleted.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Postgres;
use sqlx::postgres::PgArguments;
use sqlx::query::Query;

use super::models::{
    ForkPoint, SettledTurn, TurnSignals, TurnStateEvent, TurnStatus, TurnTransition, UnclosedTurn,
};
use crate::task::dao as task_dao;
use moatless_core::db::DbPool;

/// Build the idempotent `task_turns` upsert for a transition.
///
/// The `WHERE` guards make every transition idempotent and monotonic: never
/// resurrecting a terminal turn, and preserving the first terminal status on
/// re-delivery.
fn turn_projection_query<'a>(
    task_id: &'a str,
    turn_number: i32,
    agent_profile: Option<&'a str>,
    transition: TurnTransition,
    at: DateTime<Utc>,
) -> Query<'a, Postgres, PgArguments> {
    match transition {
        TurnTransition::Open => sqlx::query(
            r#"INSERT INTO task_turns (task_id, turn_number, status, agent_profile, started_at)
               VALUES ($1, $2, 'active', $3, $4)
               ON CONFLICT (task_id, turn_number) DO UPDATE
                 SET status = 'active',
                     ended_at = NULL,
                     agent_profile = COALESCE(task_turns.agent_profile, EXCLUDED.agent_profile)
                 WHERE task_turns.status = 'interrupted'"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .bind(agent_profile)
        .bind(at),
        TurnTransition::Interrupted => sqlx::query(
            r#"UPDATE task_turns
                 SET status = 'interrupted'
               WHERE task_id = $1 AND turn_number = $2 AND status = 'active'"#,
        )
        .bind(task_id)
        .bind(turn_number),
        TurnTransition::Terminal(status) => sqlx::query(
            r#"INSERT INTO task_turns (task_id, turn_number, status, agent_profile, started_at, ended_at)
               VALUES ($1, $2, $3, $4, $5, $5)
               ON CONFLICT (task_id, turn_number) DO UPDATE
                 SET status = EXCLUDED.status,
                     ended_at = EXCLUDED.ended_at,
                     agent_profile = COALESCE(task_turns.agent_profile, EXCLUDED.agent_profile)
                 WHERE task_turns.status IN ('active', 'interrupted')"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .bind(status.as_str())
        .bind(agent_profile)
        .bind(at),
    }
}

/// Record that `task_id`'s Turn moved, on the transaction that moved it.
///
/// Guarded by `rows_affected`, not transition kind, since the dispatcher
/// re-applies committed events; ordered conversation-first, listing-last per
/// `record_change`'s rule.
async fn record_turn_change(
    tx: &mut sqlx::PgConnection,
    task_id: &str,
    rows_affected: u64,
) -> Result<(), sqlx::Error> {
    if rows_affected > 0 {
        moatless_core::change_feed::conversation::record_session_change(tx, task_id).await?;
        moatless_core::change_feed::listing::record_change(tx, task_id).await?;
    }
    Ok(())
}

/// Repository for authoritative task-turn state.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait TaskTurnDao: Send + Sync {
    /// Transactionally apply a state-bearing event at the gRPC ingress.
    ///
    /// Persists the event row (idempotent on `event_id`), projects the turn
    /// row, and reopens the task on a terminal transition unless it closed
    /// mid-run — all committed before the caller publishes the event.
    async fn apply_ingress_event(&self, ev: &TurnStateEvent) -> Result<(), sqlx::Error>;

    /// Idempotent turn projection used by the event dispatcher as a fallback.
    /// Touches only `task_turns` (the event row and task-status flip stay with
    /// the dispatcher's existing writes). A no-op when the row is already
    /// current.
    async fn project_turn(
        &self,
        task_id: &str,
        turn_number: i32,
        agent_profile: Option<String>,
        transition: TurnTransition,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;

    /// List tasks whose latest turn is still open (`active`/`interrupted`) and
    /// started before `older_than` — the reaper's stuck-turn backstop. Indexed
    /// by the partial `idx_task_turns_open` index.
    async fn list_unclosed_active_turns(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error>;

    /// Tasks whose newest turn is closed yet show activity since — an agent
    /// still working under a turn row that says it finished. `run_id`
    /// distinguishes a newer run from a contradicting one.
    async fn list_settled_turns_with_later_activity(&self)
    -> Result<Vec<SettledTurn>, sqlx::Error>;

    /// Open turns a higher-numbered turn already supersedes, started before
    /// `older_than`.
    ///
    /// [`Self::list_unclosed_active_turns`] only returns the newest open turn,
    /// so without this a stranded earlier one stays open forever.
    async fn list_superseded_open_turns(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error>;

    /// The turn number for a sandbox agent run, allocating it on first sight.
    ///
    /// The sole allocator; idempotent and order-independent — whichever signal
    /// arrives first (ack, `taskTurn.active`, or an overtaking terminal event)
    /// allocates, and the rest join.
    async fn resolve_turn_for_run(&self, task_id: &str, run_id: &str) -> Result<i32, sqlx::Error>;

    /// The Task band's half of the running rule, for one task, in one query.
    ///
    /// Turn state and newest-user-message time travel together because the rule
    /// reads them together: a turn closed before the newest message is one
    /// that's owed.
    async fn latest_running_signals(&self, task_id: &str) -> Result<TurnSignals, sqlx::Error>;

    /// The task's highest turn number, or `None` when it has none yet.
    ///
    /// For rows the backend writes outside any run (harness errors, internal
    /// replies, outbound stubs), which belong to the turn already in progress.
    async fn latest_turn_number(&self, task_id: &str) -> Result<Option<i32>, sqlx::Error>;

    /// The task whose message opened `turn_number`, the one told how it ended.
    /// `None` for a turn a person or nobody opened.
    async fn triggered_by_task_id(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<Option<String>, sqlx::Error>;

    /// The status of the task's most recently *closed* turn, or `None`.
    ///
    /// Open turns are skipped so the answer doesn't flip once the next run
    /// opens; read when deriving `--resume` (see
    /// [`TurnStatus::cut_off_session`]).
    async fn latest_closed_turn_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TurnStatus>, sqlx::Error>;

    /// Every turn's stored metrics, oldest first. Turns that never completed
    /// (or predate the column) are omitted.
    async fn list_turn_metrics(&self, task_id: &str)
    -> Result<Vec<serde_json::Value>, sqlx::Error>;

    /// Store the SDK result message's turn totals on the turn row.
    ///
    /// Authoritative counts live only on the terminal result — per-message
    /// `output_tokens` is partial — kept off message rows, where summing would
    /// double-count.
    async fn set_turn_metrics(
        &self,
        task_id: &str,
        turn_number: i32,
        metrics: &serde_json::Value,
    ) -> Result<bool, sqlx::Error>;

    /// Record the agent's native turn id (Codex's `turn/start` id, used by
    /// [`Self::fork_point`]).
    ///
    /// First writer wins — the id is fixed for the turn's life, so a
    /// re-delivered `session_init` must not overwrite it. Only Codex runs call
    /// this.
    async fn set_agent_turn_id(
        &self,
        task_id: &str,
        turn_number: i32,
        agent_turn_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Bump a turn's recovery counter and return the new value. Creates the row
    /// as `interrupted` when the turn predates `task_turns` — a turn being
    /// recovered is by definition one that never closed.
    async fn increment_recovery_attempts(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<i32, sqlx::Error>;

    /// Where a fork of `task_id` would cut, and the harness id that names it.
    ///
    /// One read, not three: the cut and its `agent_turn_id` must come off the
    /// same row to decide whether the cut clears the turn still in flight.
    async fn fork_point(
        &self,
        task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<ForkPoint, sqlx::Error>;

    /// Zero a turn's recovery counter so repeated external reschedules never
    /// exhaust the crash-loop cap. Called after a re-drive makes progress; a
    /// no-op when the row is absent.
    async fn reset_recovery_attempts(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<(), sqlx::Error>;
}

/// The next free turn number for a task.
///
/// `GREATEST` over both tables is required: migration 0071 seeded `task_turns`
/// only for turns active at rollout, so most tasks' table is empty while
/// `task_messages` holds full history.
const NEXT_TURN_NUMBER_SQL: &str = r#"
    SELECT GREATEST(
        COALESCE((SELECT MAX(turn_number) FROM task_turns WHERE task_id = $1), 0),
        COALESCE((SELECT MAX(turn_number) FROM task_messages WHERE task_id = $1), 0)
    ) + 1"#;

/// PostgreSQL implementation of [`TaskTurnDao`].
pub struct PgTaskTurnDao(pub DbPool);

#[async_trait]
impl TaskTurnDao for PgTaskTurnDao {
    async fn apply_ingress_event(&self, ev: &TurnStateEvent) -> Result<(), sqlx::Error> {
        let at = Utc::now();
        let mut tx = self.0.begin().await?;

        // 1) Persist the event row (idempotent on event_id) so task_events
        //    stays consistent with the turn/status write even if the feed
        //    never round-trips it. Shares the caller's pre-generated event_id
        //    with the publish, so the dispatcher's insert dedups against this.
        sqlx::query(
            r#"INSERT INTO task_events (
                   event_id, task_id, type, sub_type, is_error,
                   agent_profile, turn_number, payload, timestamp
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (event_id) DO NOTHING"#,
        )
        .bind(&ev.event_id)
        .bind(&ev.task_id)
        .bind(&ev.event_type)
        .bind(&ev.sub_type)
        .bind(ev.is_error)
        .bind(&ev.agent_profile)
        .bind(ev.turn_number)
        .bind(&ev.payload)
        .bind(&ev.timestamp)
        .execute(&mut *tx)
        .await?;

        // 2) Project the turn state (idempotent — see `turn_projection_query`).
        //    Whether it moved the row is what decides, in step 4, if there is a
        //    change to record; nothing between here and there disturbs it.
        let projected = turn_projection_query(
            &ev.task_id,
            ev.turn_number,
            ev.agent_profile.as_deref(),
            ev.transition,
            at,
        )
        .execute(&mut *tx)
        .await?;

        // 3) A terminal turn returns the task to Open unless it was Closed
        //    mid-run. Mirrors `update_status_if_not_closed` so a closed task is
        //    never resurrected by an automated agent-lifecycle event.
        if ev.transition.is_terminal() {
            sqlx::query(
                "UPDATE tasks SET status = 'open', updated_at = NOW() \
                 WHERE task_id = $1 AND status <> 'closed'",
            )
            .bind(&ev.task_id)
            .execute(&mut *tx)
            .await?;
        }

        // 4) Record the change, when — and only when — step 2 moved the row.
        //    Last, after the `tasks` row lock step 3 takes: `record_change`'s
        //    "call this last".
        record_turn_change(&mut tx, &ev.task_id, projected.rows_affected()).await?;

        tx.commit().await?;
        Ok(())
    }

    async fn project_turn(
        &self,
        task_id: &str,
        turn_number: i32,
        agent_profile: Option<String>,
        transition: TurnTransition,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        // The projection and its listing record have to land together or not at
        // all. This path is the consumer's backstop and the reaper's stuck-turn
        // close, and the reaper's is the one place a turn reaches a terminal
        // state without any event committing for it — so a partial write leaves
        // the sidebar showing a reconciled turn as running.
        let mut tx = self.0.begin().await?;
        let projected = turn_projection_query(
            task_id,
            turn_number,
            agent_profile.as_deref(),
            transition,
            at,
        )
        .execute(&mut *tx)
        .await?;
        record_turn_change(&mut tx, task_id, projected.rows_affected()).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn list_unclosed_active_turns(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
        let rows: Vec<(String, i32, Option<String>)> = sqlx::query_as(
            r#"SELECT tt.task_id, tt.turn_number, tt.agent_profile
               FROM task_turns tt
               WHERE tt.status IN ('active', 'interrupted')
                 AND tt.started_at < $1
                 AND tt.turn_number = GREATEST(
                   COALESCE((
                     SELECT MAX(later_tt.turn_number)
                     FROM task_turns later_tt
                     WHERE later_tt.task_id = tt.task_id
                   ), 0),
                   COALESCE((
                     SELECT MAX(m.turn_number)
                     FROM task_messages m
                     WHERE m.task_id = tt.task_id
                   ), 0)
                 )"#,
        )
        .bind(older_than)
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(task_id, turn_number, agent_profile)| UnclosedTurn {
                task_id,
                turn_number: Some(turn_number),
                agent_profile,
            })
            .collect())
    }

    async fn list_settled_turns_with_later_activity(
        &self,
    ) -> Result<Vec<SettledTurn>, sqlx::Error> {
        // `ended_at IS NOT NULL` is load-bearing, not defensive: a turn closed
        // without recording when cannot be compared against anything, and
        // treating it as infinitely old would make every such row a candidate
        // forever.
        //
        // The activity is dated by `created_at` (timestamptz), not by
        // `timestamp`, which is free-form text a publisher chooses.
        let rows: Vec<(String, i32, Option<String>, Option<String>)> = sqlx::query_as(
            r#"SELECT tt.task_id, tt.turn_number, tt.agent_profile, tt.run_id
               FROM task_turns tt
               WHERE tt.status IN ('completed', 'error', 'stopped')
                 AND tt.ended_at IS NOT NULL
                 AND tt.turn_number = (
                   SELECT MAX(later_tt.turn_number)
                   FROM task_turns later_tt
                   WHERE later_tt.task_id = tt.task_id
                 )
                 AND EXISTS (
                   SELECT 1
                   FROM task_events e
                   WHERE e.task_id = tt.task_id
                     AND e.type = 'agent'
                     AND e.sub_type = 'activity'
                     AND e.created_at > tt.ended_at
                 )"#,
        )
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(task_id, turn_number, agent_profile, run_id)| SettledTurn {
                    task_id,
                    turn_number,
                    agent_profile,
                    run_id,
                },
            )
            .collect())
    }

    async fn list_superseded_open_turns(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
        // The complement of `list_unclosed_active_turns`: open turns strictly
        // below the task's high-water mark. That later turn is the proof of
        // supersession, so no age of the row itself is required — the floor is
        // only a guard against racing a terminal event still in flight.
        let rows: Vec<(String, i32, Option<String>)> = sqlx::query_as(
            r#"SELECT tt.task_id, tt.turn_number, tt.agent_profile
               FROM task_turns tt
               WHERE tt.status IN ('active', 'interrupted')
                 AND tt.started_at < $1
                 AND tt.turn_number < GREATEST(
                   COALESCE((
                     SELECT MAX(later_tt.turn_number)
                     FROM task_turns later_tt
                     WHERE later_tt.task_id = tt.task_id
                   ), 0),
                   COALESCE((
                     SELECT MAX(m.turn_number)
                     FROM task_messages m
                     WHERE m.task_id = tt.task_id
                   ), 0)
                 )"#,
        )
        .bind(older_than)
        .fetch_all(&self.0)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(task_id, turn_number, agent_profile)| UnclosedTurn {
                task_id,
                turn_number: Some(turn_number),
                agent_profile,
            })
            .collect())
    }

    async fn latest_running_signals(&self, task_id: &str) -> Result<TurnSignals, sqlx::Error> {
        // One row, three scalars, so the answer is a single consistent read
        // rather than three that can interleave with a turn opening.
        let row: Option<(Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
            sqlx::query_as(
                r#"SELECT
                       turn.status,
                       turn.ended_at,
                       person.created_at
                   FROM (SELECT $1::text AS task_id) t
                   LEFT JOIN LATERAL (
                       SELECT tt.status, tt.ended_at
                       FROM task_turns tt
                       WHERE tt.task_id = t.task_id
                       ORDER BY tt.turn_number DESC
                       LIMIT 1
                   ) turn ON TRUE
                   LEFT JOIN LATERAL (
                       SELECT m.created_at
                       FROM task_inbox m
                       WHERE m.task_id = t.task_id
                         AND m.status <> 'cancelled'
                       ORDER BY m.created_at DESC
                       LIMIT 1
                   ) person ON TRUE"#,
            )
            .bind(task_id)
            .fetch_optional(&self.0)
            .await?;

        let (status, ended_at, latest_user_message_at) = row.unwrap_or((None, None, None));
        Ok(TurnSignals {
            // An unrecognised status is `None` — "this reader cannot say" —
            // rather than a guess. The running rule already treats that as
            // "not known to this caller".
            turn_status: status.as_deref().and_then(TurnStatus::parse),
            turn_ended_at: ended_at,
            latest_user_message_at,
        })
    }

    async fn resolve_turn_for_run(&self, task_id: &str, run_id: &str) -> Result<i32, sqlx::Error> {
        // Fast path: most calls are the second and later signals of a run whose
        // turn already exists (every `agent.message` it emits comes through
        // here), and those need no lock.
        let allocated: Option<(i32,)> =
            sqlx::query_as("SELECT turn_number FROM task_turns WHERE task_id = $1 AND run_id = $2")
                .bind(task_id)
                .bind(run_id)
                .fetch_optional(&self.0)
                .await?;
        if let Some((turn_number,)) = allocated {
            return Ok(turn_number);
        }

        let mut tx = self.0.begin().await?;

        // Serialize allocation per task. Read-then-write would otherwise race
        // two of the run's own signals against each other, and the reaper
        // against both.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1)::bigint)")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;

        let existing: Option<(i32,)> =
            sqlx::query_as("SELECT turn_number FROM task_turns WHERE task_id = $1 AND run_id = $2")
                .bind(task_id)
                .bind(run_id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some((turn_number,)) = existing {
            tx.commit().await?;
            return Ok(turn_number);
        }

        let (turn_number,): (i32,) = sqlx::query_as(NEXT_TURN_NUMBER_SQL)
            .bind(task_id)
            .fetch_one(&mut *tx)
            .await?;

        // At most one open turn per task. Without this a turn left open by a
        // lost terminal event (or seeded stale by migration 0071) sits
        // alongside the live one, and the reaper reconciles whichever it finds.
        sqlx::query(
            "UPDATE task_turns SET status = 'interrupted' \
             WHERE task_id = $1 AND status = 'active'",
        )
        .bind(task_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO task_turns (task_id, turn_number, run_id, status, started_at)
               VALUES ($1, $2, $3, 'active', NOW())
               ON CONFLICT (task_id, turn_number) DO UPDATE
                 SET run_id = COALESCE(task_turns.run_id, EXCLUDED.run_id)"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .bind(run_id)
        .execute(&mut *tx)
        .await?;

        // Allocation is usually the only place a turn start shows: by the time
        // `taskTurn.active` reaches `apply_ingress_event` the row is already
        // `active` and its guarded projection changes nothing. Last, after the
        // advisory lock and both `task_turns` writes — `record_change`'s "call
        // this last", with the conversation's allocator ahead of it.
        task_dao::clear_settled_for_all_followers(&mut tx, task_id).await?;
        moatless_core::change_feed::conversation::record_session_change(&mut tx, task_id).await?;
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;

        tx.commit().await?;
        Ok(turn_number)
    }

    async fn latest_turn_number(&self, task_id: &str) -> Result<Option<i32>, sqlx::Error> {
        let (latest,): (Option<i32>,) = sqlx::query_as(
            r#"SELECT NULLIF(GREATEST(
                   COALESCE((SELECT MAX(turn_number) FROM task_turns WHERE task_id = $1), 0),
                   COALESCE((SELECT MAX(turn_number) FROM task_messages WHERE task_id = $1), 0)
               ), 0)"#,
        )
        .bind(task_id)
        .fetch_one(&self.0)
        .await?;
        Ok(latest)
    }

    async fn triggered_by_task_id(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT triggered_by_task_id FROM task_turns WHERE task_id = $1 AND turn_number = $2",
        )
        .bind(task_id)
        .bind(turn_number)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.and_then(|(trigger,)| trigger))
    }

    async fn latest_closed_turn_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TurnStatus>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"SELECT status FROM task_turns
               WHERE task_id = $1
                 AND status IN ('completed', 'error', 'stopped')
               ORDER BY turn_number DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.and_then(|(status,)| TurnStatus::parse(&status)))
    }

    async fn increment_recovery_attempts(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<i32, sqlx::Error> {
        let (attempts,): (i32,) = sqlx::query_as(
            r#"INSERT INTO task_turns (task_id, turn_number, status, recovery_attempts)
               VALUES ($1, $2, 'interrupted', 1)
               ON CONFLICT (task_id, turn_number) DO UPDATE
                 SET recovery_attempts = task_turns.recovery_attempts + 1
               RETURNING recovery_attempts"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .fetch_one(&self.0)
        .await?;
        Ok(attempts)
    }

    async fn fork_point(
        &self,
        task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<ForkPoint, sqlx::Error> {
        // The newest turn only, because that is the one an agent could still be
        // writing. An earlier turn left `active` or `interrupted` behind a
        // higher-numbered one is a lost terminal event, not work in flight: the
        // turns after it prove the agent moved on, and nothing will ever resume
        // it. Asking across every turn instead refused these Tasks forever, and
        // told the person to wait for a turn that had ended a day earlier.
        let open_turn: Option<i32> = sqlx::query_scalar(
            "SELECT turn_number FROM task_turns \
             WHERE task_id = $1 AND status IN ('active', 'interrupted') \
               AND turn_number = (SELECT MAX(turn_number) FROM task_turns WHERE task_id = $1)",
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?;

        // `$2` both selects and relaxes: naming a turn takes exactly that row,
        // whatever it ended as, while omitting it takes the newest row that
        // ended at all.
        let cut: Option<(i32, Option<String>)> = sqlx::query_as(
            r#"SELECT turn_number, agent_turn_id FROM task_turns
               WHERE task_id = $1
                 AND ($2::int IS NULL OR turn_number = $2)
                 AND ($2::int IS NOT NULL OR status NOT IN ('active', 'interrupted'))
               ORDER BY turn_number DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .bind(at_turn)
        .fetch_optional(&self.0)
        .await?;

        Ok(ForkPoint {
            open_turn,
            turn: cut.as_ref().map(|(turn, _)| *turn),
            agent_turn_id: cut.and_then(|(_, id)| id),
        })
    }

    async fn reset_recovery_attempts(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE task_turns SET recovery_attempts = 0 \
             WHERE task_id = $1 AND turn_number = $2",
        )
        .bind(task_id)
        .bind(turn_number)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list_turn_metrics(
        &self,
        task_id: &str,
    ) -> Result<Vec<serde_json::Value>, sqlx::Error> {
        let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
            r#"SELECT metrics FROM task_turns
               WHERE task_id = $1 AND metrics IS NOT NULL
               ORDER BY turn_number"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().map(|(m,)| m).collect())
    }

    async fn set_turn_metrics(
        &self,
        task_id: &str,
        turn_number: i32,
        metrics: &serde_json::Value,
    ) -> Result<bool, sqlx::Error> {
        // Only updates an existing turn: the row is projected by the lifecycle
        // transitions, and a result for a turn we never opened is not a reason
        // to invent one.
        let result = sqlx::query(
            r#"UPDATE task_turns SET metrics = $3
               WHERE task_id = $1 AND turn_number = $2"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .bind(metrics)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn set_agent_turn_id(
        &self,
        task_id: &str,
        turn_number: i32,
        agent_turn_id: &str,
    ) -> Result<bool, sqlx::Error> {
        // Only updates an existing turn, and only when the id is not already
        // set: `session_init` can be re-delivered, and the first turn id the run
        // reported is the one `fork_point` cuts on.
        let result = sqlx::query(
            r#"UPDATE task_turns SET agent_turn_id = $3
               WHERE task_id = $1 AND turn_number = $2 AND agent_turn_id IS NULL"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .bind(agent_turn_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
