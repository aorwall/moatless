//! Delivery-record fixtures for a DB-backed test, so no test names the private
//! store.

use moatless_core::db::DbPool;

/// Re-arm every admitted record for another attempt, which is what a restart
/// mid-attempt produces.
pub async fn requeue_deliveries(db: &DbPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE adapter_delivery SET status = 'pending', attempts = 0, next_attempt_at = NOW()",
    )
    .execute(db)
    .await?;
    Ok(())
}
