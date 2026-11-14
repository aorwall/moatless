//! The `pull_request_host_changes` sequence.

use sqlx::{PgConnection, PgPool};

/// Count one host-state change of a bound pull request.
pub async fn record_host_change(tx: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT nextval('pull_request_host_changes')")
        .fetch_one(tx)
        .await
}

/// The count so far; 0 before the first change.
pub async fn host_changes(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (last, called): (i64, bool) =
        sqlx::query_as("SELECT last_value, is_called FROM pull_request_host_changes")
            .fetch_one(pool)
            .await?;
    Ok(if called { last } else { 0 })
}
