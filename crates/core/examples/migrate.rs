//! Apply the workspace migrations to the database URL given as the argument.
//!
//! CI runs it against `template1`, which every `#[sqlx::test]` database is a
//! `CREATE DATABASE` copy of, so each test starts migrated and sqlx's own run
//! finds nothing to apply.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args()
        .nth(1)
        .ok_or("usage: migrate <database-url>")?;
    let pool = sqlx::PgPool::connect(&url).await?;
    moatless_core::db::run_migrations(&pool).await?;
    pool.close().await;
    Ok(())
}
