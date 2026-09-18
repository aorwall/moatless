//! Postgres [`LifecycleLock`] — per-Sandbox serialization via **session**
//! advisory locks on a dedicated lock-only pool.
//!
//! Every operation takes `pg_try_advisory_lock` keyed on the Sandbox, using the
//! two-int lock space so it never collides with the one-int `pg_advisory_*`
//! locks elsewhere (codex refresh, task turns). The holder keeps its lock
//! connection for the operation's duration; a detached connection closes its
//! session on drop, so the lock dies with it even if the unlock never runs.

use async_trait::async_trait;
use sqlx::Postgres;
use sqlx::pool::PoolConnection;

use moatless_core::db::DbPool;

use super::super::orchestrator::{LifecycleLock, LockGuard};
use moatless_sandbox_runtime::operation::OperationKind;

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
        let Some(conn) = self.conn.take() else {
            return;
        };
        let key = self.key;
        // Detach from the pool *before* handing the connection to the spawned
        // unlock. A detached connection closes its session on drop, so even if
        // this task is never polled (runtime shutting down, executor dropped),
        // the session — and its advisory lock — die with the dropped connection.
        // A pooled connection would instead return to the pool still holding the
        // lock, silently leaking a foreign per-Task lock to the next checkout.
        let mut conn = conn.detach();
        tokio::spawn(async move {
            if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1, $2)")
                .bind(LIFECYCLE_LOCK_NS)
                .bind(key)
                .execute(&mut conn)
                .await
            {
                tracing::debug!(error = %e, "lifecycle advisory unlock failed; session close releases the lock");
            }
            // `conn` is detached: dropping it here closes the session, releasing
            // any lock the explicit unlock did not.
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
                tracing::warn!(sandbox_id = task_id, error = %e, "lifecycle lock pool acquire failed");
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
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = $1")
                .bind(task_id)
                .fetch_optional(&mut **conn)
                .await
                .map(Option::flatten);
        match marker {
            Ok(Some(s)) => OperationKind::from_db_str(&s).unwrap_or(fallback),
            Ok(None) => {
                // Contended in the window between lock acquisition and the intent
                // write: no marker yet, so the true holder's kind is unknowable.
                tracing::debug!(
                    task_id,
                    "lifecycle lock contended before marker write; reporting requester kind"
                );
                fallback
            }
            Err(e) => {
                tracing::warn!(task_id, error = %e, "failed to read lifecycle holder kind; reporting requester kind");
                fallback
            }
        }
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::super::super::test_support::seed_task as seed;
    use super::*;
    use moatless_core::db::create_pool;
    use sqlx::PgPool;

    #[sqlx::test]
    async fn contended_acquire_returns_holder_kind_from_marker(pool: PgPool) {
        seed(&pool, "t1").await;
        // Mark an in-flight remove so the holder kind is readable.
        sqlx::query("UPDATE sandboxes SET operation_kind = 'remove' WHERE sandbox_id = 't1'")
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
        let guard = a
            .try_acquire("t1", OperationKind::Start)
            .await
            .expect("a acquires");
        assert!(
            b.try_acquire("t1", OperationKind::Start).await.is_err(),
            "replica B is serialized behind replica A"
        );
        drop(guard);
    }
}
