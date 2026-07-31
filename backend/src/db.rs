//! Database connection pool and migrations.

use sqlx::postgres::PgPoolOptions;

pub type DbPool = sqlx::PgPool;

const POOL_CONNECTIONS: &str = "moatless_database_pool_connections";
const READINESS_FAILURES: &str = "moatless_database_readiness_failures_total";

/// How a pool reclaims idle connections.
///
/// The main pool trims idle connections after ten minutes. The dedicated
/// lock-only pool ([`PoolKind::Lock`]) keeps them: a held session advisory lock
/// pins its connection for the operation's whole duration, and idle-reaping it
/// would drop the lock. Keeping the lock pool separate from the main pool means
/// a blocking or long-held lock can never starve application queries.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PoolKind {
    Main,
    Lock,
}

/// Create a connection pool to the PostgreSQL database.
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<DbPool, sqlx::Error> {
    create_pool_kind(database_url, max_connections, PoolKind::Main).await
}

/// Create a pool with kind-specific defaults, so the main and lock pools share
/// one constructor and their common settings cannot drift.
pub async fn create_pool_kind(
    database_url: &str,
    max_connections: u32,
    kind: PoolKind,
) -> Result<DbPool, sqlx::Error> {
    let mut opts = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(std::time::Duration::from_secs(5));
    if kind == PoolKind::Main {
        opts = opts.idle_timeout(std::time::Duration::from_mins(10));
    }
    opts.connect(database_url).await
}

/// Register and refresh the main pool's operator-facing metrics.
///
/// SQLx exposes these values synchronously, so readiness can report pool
/// starvation without first needing the connection whose absence it is trying
/// to diagnose.
pub fn record_pool_metrics(pool: &DbPool) {
    let size = pool.size() as usize;
    let idle = pool.num_idle();
    let in_use = size.saturating_sub(idle);
    let max = pool.options().get_max_connections();

    metrics::describe_gauge!(
        POOL_CONNECTIONS,
        "SQLx main database pool connections by state"
    );
    metrics::describe_counter!(READINESS_FAILURES, "Database readiness failures by reason");
    metrics::gauge!(POOL_CONNECTIONS, "state" => "open").set(size as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "idle").set(idle as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "in_use").set(in_use as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "max").set(f64::from(max));
}

/// Count one failed database readiness check.
pub fn record_readiness_failure(reason: &'static str) {
    metrics::counter!(READINESS_FAILURES, "reason" => reason).increment(1);
}

/// Run pending SQL migrations from `./migrations/`.
/// Uses PostgreSQL advisory locks — safe for concurrent replicas.
pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

#[cfg(test)]
mod tests {
    /// Two files sharing a version is not a conflict git can see: both merge,
    /// and the loser is silently skipped against a database that recorded the
    /// winner, leaving its schema change unapplied and startup refusing to boot.
    #[test]
    fn every_migration_has_its_own_version() {
        let migrations = sqlx::migrate!();
        let mut seen = std::collections::HashMap::new();

        for migration in migrations.iter() {
            if let Some(other) = seen.insert(migration.version, &migration.description) {
                panic!(
                    "migration {} is claimed by both {:?} and {:?}",
                    migration.version, other, migration.description
                );
            }
        }
    }

    #[test]
    fn shell_index_migrations_run_outside_a_transaction() {
        let migrations = sqlx::migrate!();

        for version in 107..=110 {
            let migration = migrations
                .iter()
                .find(|migration| migration.version == version)
                .expect("shell index migration");
            assert!(migration.no_tx, "migration {version}");
        }
    }
}
