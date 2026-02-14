//! Database connection pool and migrations.

use sqlx::postgres::PgPoolOptions;

pub type DbPool = sqlx::PgPool;

/// Create a connection pool to the PostgreSQL database.
pub async fn create_pool(database_url: &str) -> Result<DbPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}

/// Run pending SQL migrations from `./migrations/`.
/// Uses PostgreSQL advisory locks — safe for concurrent replicas.
pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
}
