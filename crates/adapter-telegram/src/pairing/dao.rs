use super::models::PairingRequest;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_core::db::DbPool;
use tracing::info;

/// Every query here reads or writes `integration_pairing_requests`. The failing
/// statement is in the source, so one context serves them all.
pub(super) fn pairing_store_error(error: sqlx::Error) -> SharedAdapterError {
    SharedAdapterError::Store(moatless_types::store::StoreError::with_source(
        "a pairing-request store call",
        error,
    ))
}

pub(super) async fn find_pending(
    db: &DbPool,
    loop_id: &str,
    provider_user_id: &str,
) -> Result<Option<PairingRequest>, AdapterError> {
    let existing: Option<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE loop_id = $1 AND adapter_kind = 'telegram' AND provider_user_id = $2 AND status = 'pending' \
         LIMIT 1",
    )
    .bind(loop_id)
    .bind(provider_user_id)
    .fetch_optional(db)
    .await
    .map_err(pairing_store_error)?;

    Ok(existing)
}
pub(super) async fn insert_pending(
    db: &DbPool,
    loop_id: &str,
    provider_user_id: &str,
    code: &str,
    meta: Option<serde_json::Value>,
) -> Result<PairingRequest, AdapterError> {
    let row: PairingRequest = sqlx::query_as(
        "INSERT INTO integration_pairing_requests (loop_id, adapter_kind, provider_user_id, code, status, meta) \
         VALUES ($1, 'telegram', $2, $3, 'pending', $4) \
         RETURNING *",
    )
    .bind(loop_id)
    .bind(provider_user_id)
    .bind(code)
    .bind(&meta)
    .fetch_one(db)
    .await
    .map_err(pairing_store_error)?;

    Ok(row)
}
/// Find a pending request by code.
pub(super) async fn find_by_code(
    db: &DbPool,
    loop_id: &str,
    code: &str,
) -> Result<Option<PairingRequest>, AdapterError> {
    let row: Option<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE loop_id = $1 AND code = $2 AND status = 'pending' \
         LIMIT 1",
    )
    .bind(loop_id)
    .bind(code)
    .fetch_optional(db)
    .await
    .map_err(pairing_store_error)?;

    Ok(row)
}

/// List all pending requests for a config.
pub(super) async fn list_pending(
    db: &DbPool,
    loop_id: &str,
) -> Result<Vec<PairingRequest>, AdapterError> {
    let rows: Vec<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE loop_id = $1 AND status = 'pending' \
         ORDER BY created_at DESC",
    )
    .bind(loop_id)
    .fetch_all(db)
    .await
    .map_err(pairing_store_error)?;

    Ok(rows)
}

pub(super) async fn pending_for_loops(
    db: &DbPool,
    loop_ids: &[String],
) -> Result<Vec<PairingRequest>, AdapterError> {
    let rows: Vec<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE loop_id = ANY($1) AND status = 'pending' \
         ORDER BY created_at DESC",
    )
    .bind(loop_ids)
    .fetch_all(db)
    .await
    .map_err(pairing_store_error)?;

    Ok(rows)
}
pub(super) async fn mark_approved(db: &DbPool, id: i32) -> Result<(), AdapterError> {
    sqlx::query(
        "UPDATE integration_pairing_requests SET status = 'approved', updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(db)
    .await
    .map_err(pairing_store_error)?;

    Ok(())
}
pub(super) async fn find_by_id(db: &DbPool, id: i32) -> Result<PairingRequest, AdapterError> {
    let updated: PairingRequest =
        sqlx::query_as("SELECT * FROM integration_pairing_requests WHERE id = $1")
            .bind(id)
            .fetch_one(db)
            .await
            .map_err(pairing_store_error)?;

    Ok(updated)
}
/// Delete expired pending requests (older than 1 hour).
pub(super) async fn prune_expired(db: &DbPool) -> Result<u64, AdapterError> {
    let result = sqlx::query(
        "DELETE FROM integration_pairing_requests \
         WHERE status = 'pending' AND created_at < now() - interval '1 hour'",
    )
    .execute(db)
    .await
    .map_err(pairing_store_error)?;

    let count = result.rows_affected();
    if count > 0 {
        info!(count, "Pruned expired pairing requests");
    }
    Ok(count)
}

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum::response::IntoResponse as _;

    use super::*;
    use moatless_adapter_model::test_support::response_snapshot;

    /// A pool pointed at a dead loopback port, with a short acquire timeout so
    /// the pairing queries fail fast instead of waiting out the default.
    fn offline_pool() -> DbPool {
        sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(500))
            .connect_lazy("postgres://pair:pair@127.0.0.1:1/pair")
            .expect("lazy pool")
    }

    /// The pairing queries are the module's main failure path: a database that
    /// cannot be reached must surface as `Db` with the `sqlx` cause attached.
    #[tokio::test]
    async fn pairing_lookup_keeps_sqlx_source() {
        let err = find_by_code(&offline_pool(), "trigger-1", "ABCD2345")
            .await
            .expect_err("unreachable database must fail");

        assert!(
            matches!(err, AdapterError::Shared(SharedAdapterError::Store(_))),
            "got {err:?}"
        );
        assert!(
            err.source()
                .and_then(|s| s.downcast_ref::<sqlx::Error>())
                .is_some()
        );
    }

    #[tokio::test]
    async fn config_serialization_keeps_serde_source_and_redacts() {
        let serde_err = serde_json::from_str::<serde_json::Value>("nope").unwrap_err();
        let err: AdapterError = SharedAdapterError::ConfigSerialization(serde_err).into();

        assert!(
            err.source()
                .and_then(|s| s.downcast_ref::<serde_json::Error>())
                .is_some()
        );

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }
}
