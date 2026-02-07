//! Database access layer for the sandbox controller.
//!
//! Uses sqlx with PostgreSQL for reading task and repository configuration
//! needed for auto-provisioning workspaces.

pub mod models;
pub mod provider_token_repo;
pub mod repository_repo;
pub mod task_repo;

use sqlx::postgres::PgPoolOptions;

pub type DbPool = sqlx::PgPool;

/// Create a connection pool to the PostgreSQL database.
pub async fn create_pool(database_url: &str) -> Result<DbPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}
