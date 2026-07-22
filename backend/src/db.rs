//! Database connection pool and migrations.

use sqlx::postgres::PgPoolOptions;

pub type DbPool = sqlx::PgPool;

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

/// Run pending SQL migrations from `./migrations/`.
/// Uses PostgreSQL advisory locks — safe for concurrent replicas.
pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
