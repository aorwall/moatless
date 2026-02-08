//! Database access layer for the sandbox controller.
//!
//! Uses sqlx with PostgreSQL for reading task and repository configuration
//! needed for auto-provisioning workspaces, and full CRUD for tasks,
//! messages, commands, tags, and file metadata.

pub mod api_keys;
pub mod command_dao;
pub mod file_meta_dao;
pub mod message_dao;
pub mod models;
pub mod provider_token_dao;
pub mod provider_tokens;
pub mod repository_dao;
pub mod sessions;
pub mod tag_dao;
pub mod task_dao;
pub mod users;

use sqlx::postgres::PgPoolOptions;

pub type DbPool = sqlx::PgPool;

/// Create a connection pool to the PostgreSQL database.
pub async fn create_pool(database_url: &str) -> Result<DbPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}
