use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

pub(super) async fn attempts(
    db: &DbPool,
    bucket: &str,
    subject_hash: Vec<u8>,
    window: DateTime<Utc>,
) -> Result<i32, sqlx::Error> {
    let attempts = sqlx::query_scalar::<_, i32>(
        r#"SELECT attempts
               FROM auth_rate_limits
               WHERE bucket = $1 AND subject_hash = $2 AND window_started_at = $3"#,
    )
    .bind(bucket)
    .bind(subject_hash)
    .bind(window)
    .fetch_optional(db)
    .await?
    .unwrap_or(0);
    Ok(attempts)
}

pub(super) async fn record_failure(
    db: &DbPool,
    bucket: &str,
    subject_hash: Vec<u8>,
    window: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"WITH cleaned AS (
                   DELETE FROM auth_rate_limits
                   WHERE window_started_at < now() - interval '1 day'
               )
               INSERT INTO auth_rate_limits
               (bucket, subject_hash, window_started_at, attempts)
               VALUES ($1, $2, $3, 1)
               ON CONFLICT (bucket, subject_hash, window_started_at)
               DO UPDATE SET attempts = auth_rate_limits.attempts + 1"#,
    )
    .bind(bucket)
    .bind(subject_hash)
    .bind(window)
    .execute(db)
    .await?;
    Ok(())
}

pub(super) async fn clear(
    db: &DbPool,
    bucket: &str,
    subject_hash: Vec<u8>,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM auth_rate_limits WHERE bucket = $1 AND subject_hash = $2")
        .bind(bucket)
        .bind(subject_hash)
        .execute(db)
        .await?;
    Ok(())
}
