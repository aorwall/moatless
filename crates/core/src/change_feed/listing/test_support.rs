//! Lock-order probes for `record_change`'s "call this last" rule.
//!
//! Both watch `pg_blocking_pids` rather than a clock: a fixed sleep or timeout
//! either fails on a slow database or passes before the writer took any lock.

use std::time::Duration;

use sqlx::PgPool;
use tokio::task::JoinHandle;

use super::record_change;

/// How often the probes poll `pg_blocking_pids`.
const POLL: Duration = Duration::from_millis(10);

/// Polls until some backend in this database waits on a lock. Returns `false`
/// when `waiter` finishes first, so it never reached one.
///
/// Matches any blocked backend, so the caller must run no other writer that
/// can block while it polls.
pub async fn waits_for_a_lock<T>(
    pool: &PgPool,
    waiter: &JoinHandle<T>,
) -> Result<bool, sqlx::Error> {
    loop {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
             WHERE datname = current_database() AND cardinality(pg_blocking_pids(pid)) > 0)",
        )
        .fetch_one(pool)
        .await?;
        if blocked {
            return Ok(true);
        }
        if waiter.is_finished() {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// What [`probe_counter`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Probe {
    /// The probe took the counter and committed at this position.
    Recorded(i64),
    /// The probe waited on a lock these backends hold.
    Blocked(Vec<i32>),
}

/// Records a change for `task_id` in a transaction of its own and commits it,
/// unless it has to wait on a lock first.
///
/// Takes two connections from `pool`: the probe's and the observer's. A
/// [`Probe::Blocked`] probe stays blocked until its holders let go.
pub async fn probe_counter(pool: &PgPool, task_id: &str) -> Result<Probe, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await?;
    let task_id = task_id.to_owned();
    let probe = tokio::spawn(async move {
        let position = record_change(&mut tx, &task_id).await?;
        tx.commit().await?;
        Ok::<_, sqlx::Error>(position)
    });
    loop {
        if probe.is_finished() {
            return match probe.await {
                Ok(position) => position.map(Probe::Recorded),
                Err(join) => std::panic::resume_unwind(join.into_panic()),
            };
        }
        let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
            .bind(pid)
            .fetch_one(pool)
            .await?;
        if !blockers.is_empty() {
            return Ok(Probe::Blocked(blockers));
        }
        tokio::time::sleep(POLL).await;
    }
}
