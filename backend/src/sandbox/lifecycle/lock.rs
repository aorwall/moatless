//! Postgres [`LifecycleLock`] — per-Task serialization via **session** advisory
//! locks on a dedicated lock-only pool.
//!
//! Every operation takes `pg_try_advisory_lock` keyed on the Task. The two-int
//! lock space is used deliberately: it never collides with the one-int
//! `pg_advisory_*` locks elsewhere (codex refresh, task turns). The holder keeps
//! its lock connection for the operation's whole duration; on drop the lock is
//! released, and if the unlock does not succeed the connection is **closed** so
//! the session — and its lock — die with it, never returning to the pool.

use async_trait::async_trait;
use sqlx::pool::PoolConnection;
use sqlx::{Connection, Postgres};

use crate::db::DbPool;

use super::orchestrator::{LifecycleLock, LockGuard};
use super::types::OperationKind;

/// Two-int advisory-lock namespace ("LF"). The two-int space never conflicts
/// with one-int `pg_advisory_*` keys used by other subsystems.
const LIFECYCLE_LOCK_NS: i32 = 0x4C46;

/// Postgres advisory-lock adapter over a dedicated lock-only pool.
pub struct PgLifecycleLock {
    pool: DbPool,
}

impl PgLifecycleLock {
    /// `pool` must be a **dedicated** lock-only pool: a held session lock pins
    /// its connection for the operation's duration.
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

/// A held session lock. Owns the connection; dropping it releases the lock (or
/// closes the connection so the lock dies with the session).
struct PgLockGuard {
    conn: Option<PoolConnection<Postgres>>,
    key: i32,
}

impl LockGuard for PgLockGuard {}

impl Drop for PgLockGuard {
    fn drop(&mut self) {
        let Some(mut conn) = self.conn.take() else {
            return;
        };
        let key = self.key;
        // Unlock is async; run it on the runtime. A clean unlock returns the
        // connection to the pool; anything else closes it, and closing a session
        // releases every advisory lock it held.
        tokio::spawn(async move {
            match sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1, $2)")
                .bind(LIFECYCLE_LOCK_NS)
                .bind(key)
                .fetch_one(&mut *conn)
                .await
            {
                Ok(true) => {
                    // Released cleanly; connection returns to the pool on drop.
                }
                Ok(false) | Err(_) => {
                    // Unlock did not take. Destroy the session so the lock cannot
                    // outlive us — never return this connection to the pool.
                    let _ = conn.detach().close().await;
                }
            }
        });
    }
}

#[async_trait]
impl LifecycleLock for PgLifecycleLock {
    async fn try_acquire(
        &self,
        task_id: &str,
        kind: OperationKind,
    ) -> Result<Box<dyn LockGuard>, OperationKind> {
        let mut conn = match self.pool.acquire().await {
            Ok(conn) => conn,
            Err(e) => {
                // No error channel on the port — a lock-pool failure reads as
                // contention so the caller retries, and exhausted retries surface
                // OperationInProgress rather than proceeding without the lock.
                tracing::warn!(task_id, error = %e, "lifecycle lock pool acquire failed");
                return Err(kind);
            }
        };

        // Acquire and capture the key in one round-trip so the guard can unlock
        // the exact same key.
        let acquired: Result<(bool, i32), _> = sqlx::query_as(
            "SELECT pg_try_advisory_lock($1, k) AS locked, k
             FROM (SELECT hashtext($2) AS k) sub",
        )
        .bind(LIFECYCLE_LOCK_NS)
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await;

        match acquired {
            Ok((true, key)) => Ok(Box::new(PgLockGuard {
                conn: Some(conn),
                key,
            })),
            Ok((false, _)) => Err(self.holder_kind(&mut conn, task_id, kind).await),
            Err(e) => {
                tracing::warn!(task_id, error = %e, "lifecycle advisory lock query failed");
                Err(kind)
            }
        }
    }
}

impl PgLifecycleLock {
    /// The conflicting operation's kind, read from the in-flight marker. Falls
    /// back to the requesting `kind` when no marker is persisted yet (the brief
    /// window between lock acquisition and the intent write).
    async fn holder_kind(
        &self,
        conn: &mut PoolConnection<Postgres>,
        task_id: &str,
        fallback: OperationKind,
    ) -> OperationKind {
        let marker: Result<Option<String>, _> =
            sqlx::query_scalar("SELECT sandbox_operation_kind FROM tasks WHERE task_id = $1")
                .bind(task_id)
                .fetch_optional(&mut **conn)
                .await
                .map(Option::flatten);
        marker
            .ok()
            .flatten()
            .and_then(|s| OperationKind::from_db_str(&s))
            .unwrap_or(fallback)
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;
    use crate::db::create_pool;
    use sqlx::PgPool;

    async fn seed(pool: &PgPool, task_id: &str) {
        sqlx::query("INSERT INTO teams (id, name, slug) VALUES ('team-x', 'Team X', 'team-x') ON CONFLICT (id) DO NOTHING")
            .execute(pool)
            .await
            .expect("seed team");
        sqlx::query(
            r#"INSERT INTO tasks (task_id, created_at, last_message_at, agent_type, repository_id,
                   status, total_cost_usd, visibility, scope, team_id, created_by, owner_user_id)
               VALUES ($1, NOW()::text, NOW()::text, 'claude-code', 'repo-x', 'open', 0,
                   'private', 'user', 'team-x', 'user-1', 'user-1')"#,
        )
        .bind(task_id)
        .execute(pool)
        .await
        .expect("seed task");
    }

    #[sqlx::test]
    async fn contended_acquire_returns_holder_kind_from_marker(pool: PgPool) {
        seed(&pool, "t1").await;
        // Mark an in-flight remove so the holder kind is readable.
        sqlx::query("UPDATE tasks SET sandbox_operation_kind = 'remove' WHERE task_id = 't1'")
            .execute(&pool)
            .await
            .unwrap();

        let lock = PgLifecycleLock::new(pool.clone());
        let guard = lock
            .try_acquire("t1", OperationKind::Start)
            .await
            .expect("first acquire succeeds");

        // A second, independent connection contends and gets the holder's kind.
        let contender = PgLifecycleLock::new(pool.clone());
        match contender.try_acquire("t1", OperationKind::Start).await {
            Err(kind) => assert_eq!(kind, OperationKind::Remove),
            Ok(_) => panic!("expected contention"),
        }

        drop(guard);
    }

    #[sqlx::test]
    async fn lock_is_released_after_guard_drop(pool: PgPool) {
        seed(&pool, "t1").await;
        let lock = PgLifecycleLock::new(pool.clone());

        {
            let _guard = lock
                .try_acquire("t1", OperationKind::Stop)
                .await
                .expect("acquire");
            // A different Task never contends.
            seed(&pool, "t2").await;
            assert!(lock.try_acquire("t2", OperationKind::Stop).await.is_ok());
        }

        // After the guard drops, the async unlock releases the lock. Retry
        // briefly to absorb the spawned unlock's scheduling.
        let mut acquired = false;
        for _ in 0..50 {
            if lock.try_acquire("t1", OperationKind::Start).await.is_ok() {
                acquired = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(acquired, "lock released after guard drop");
    }

    #[sqlx::test]
    async fn distinct_pools_serialize_across_replicas(pool: PgPool) {
        // Two pools model two backend replicas against the same database.
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let replica_a = create_pool(&url, 2).await.expect("pool a");
        let replica_b = create_pool(&url, 2).await.expect("pool b");
        seed(&pool, "t1").await;

        let a = PgLifecycleLock::new(replica_a);
        let b = PgLifecycleLock::new(replica_b);
        let guard = a.try_acquire("t1", OperationKind::Start).await.expect("a acquires");
        assert!(
            b.try_acquire("t1", OperationKind::Start).await.is_err(),
            "replica B is serialized behind replica A"
        );
        drop(guard);
    }
}
