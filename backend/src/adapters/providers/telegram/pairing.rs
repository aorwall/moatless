//! Telegram pairing — code-based DM access approval.
#![expect(
    dead_code,
    reason = "the approval half of pairing (look a code up, list what is \
              pending, approve it, prune) has no caller since this module \
              stopped being `pub`: it was reachable only from outside the \
              Adapter, and nothing outside calls it. Kept rather than deleted \
              because pairing without approval is a dead end for the user who \
              was just handed a code; deleting it is the pairing UI's call, \
              not this breakup's."
)]

use chrono::{DateTime, Utc};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::adapters::error::AdapterError;
use moatless_core::db::DbPool;
use crate::loops::dao::LoopDao;

use super::config::TelegramAdapterConfig;

/// Row from the `integration_pairing_requests` table.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PairingRequest {
    pub id: i32,
    pub loop_id: String,
    pub adapter_kind: String,
    pub provider_user_id: String,
    pub code: String,
    pub status: String,
    pub meta: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Alphabet for pairing codes — no O/I/0/1 to avoid confusion.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 8;

/// Generate a random pairing code.
pub fn generate_code() -> String {
    let mut rng = rand::rng();
    (0..CODE_LENGTH)
        .map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

/// Upsert a pending pairing request. Returns existing code if already pending.
pub async fn upsert_pairing_request(
    db: &DbPool,
    loop_id: &str,
    provider_user_id: &str,
    meta: Option<serde_json::Value>,
) -> Result<PairingRequest, AdapterError> {
    // Check for existing pending request
    let existing: Option<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE loop_id = $1 AND adapter_kind = 'telegram' AND provider_user_id = $2 AND status = 'pending' \
         LIMIT 1",
    )
    .bind(loop_id)
    .bind(provider_user_id)
    .fetch_optional(db)
    .await
    .map_err(AdapterError::Db)?;

    if let Some(req) = existing {
        return Ok(req);
    }

    let code = generate_code();

    let row: PairingRequest = sqlx::query_as(
        "INSERT INTO integration_pairing_requests (loop_id, adapter_kind, provider_user_id, code, status, meta) \
         VALUES ($1, 'telegram', $2, $3, 'pending', $4) \
         RETURNING *",
    )
    .bind(loop_id)
    .bind(provider_user_id)
    .bind(&code)
    .bind(&meta)
    .fetch_one(db)
    .await
    .map_err(AdapterError::Db)?;

    info!(
        loop_id,
        provider_user_id,
        code = %row.code,
        "Created pairing request"
    );

    Ok(row)
}

/// Find a pending request by code.
pub async fn find_by_code(
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
    .map_err(AdapterError::Db)?;

    Ok(row)
}

/// List all pending requests for a config.
pub async fn list_pending(db: &DbPool, loop_id: &str) -> Result<Vec<PairingRequest>, AdapterError> {
    let rows: Vec<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE loop_id = $1 AND status = 'pending' \
         ORDER BY created_at DESC",
    )
    .bind(loop_id)
    .fetch_all(db)
    .await
    .map_err(AdapterError::Db)?;

    Ok(rows)
}

/// List all pending pairing requests for loops owned by the given user.
pub async fn list_pending_for_user(
    db: &DbPool,
    user_id: &str,
) -> Result<Vec<PairingRequest>, AdapterError> {
    let rows: Vec<PairingRequest> = sqlx::query_as(
        "SELECT p.* FROM integration_pairing_requests p \
         JOIN loops t ON t.id = p.loop_id \
         WHERE t.created_by = $1 AND p.status = 'pending' \
         ORDER BY p.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    .map_err(AdapterError::Db)?;

    Ok(rows)
}

/// Approve a pairing request: mark approved + add user ID to dm_allowlist.
pub async fn approve_request(
    db: &DbPool,
    loop_id: &str,
    code: &str,
) -> Result<PairingRequest, AdapterError> {
    // Find the pending request
    let req = find_by_code(db, loop_id, code)
        .await?
        .ok_or_else(|| AdapterError::BadRequest("Pairing code not found or already used".into()))?;

    // Mark as approved
    sqlx::query(
        "UPDATE integration_pairing_requests SET status = 'approved', updated_at = now() \
         WHERE id = $1",
    )
    .bind(req.id)
    .execute(db)
    .await
    .map_err(AdapterError::Db)?;

    // Read current adapter_config and append user to dm_allowlist
    let store = LoopDao::new(db.clone());
    let current_config = store
        .get_adapter_config(loop_id)
        .await
        .map_err(AdapterError::Db)?;

    let mut tg_config: TelegramAdapterConfig = current_config
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Add user if not already present
    let user_id_str = req.provider_user_id.clone();
    if !tg_config.dm_allowlist.contains(&user_id_str) {
        tg_config.dm_allowlist.push(user_id_str);
    }

    let updated_config =
        serde_json::to_value(&tg_config).map_err(AdapterError::ConfigSerialization)?;

    store
        .set_adapter_config(loop_id, &updated_config)
        .await
        .map_err(AdapterError::Db)?;

    info!(
        loop_id,
        code,
        provider_user_id = %req.provider_user_id,
        "Pairing request approved, user added to dm_allowlist"
    );

    // Return updated request
    let updated: PairingRequest =
        sqlx::query_as("SELECT * FROM integration_pairing_requests WHERE id = $1")
            .bind(req.id)
            .fetch_one(db)
            .await
            .map_err(AdapterError::Db)?;

    Ok(updated)
}

/// Delete expired pending requests (older than 1 hour).
pub async fn prune_expired(db: &DbPool) -> Result<u64, AdapterError> {
    let result = sqlx::query(
        "DELETE FROM integration_pairing_requests \
         WHERE status = 'pending' AND created_at < now() - interval '1 hour'",
    )
    .execute(db)
    .await
    .map_err(AdapterError::Db)?;

    let count = result.rows_affected();
    if count > 0 {
        info!(count, "Pruned expired pairing requests");
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_generation_format() {
        let code = generate_code();
        assert_eq!(code.len(), CODE_LENGTH);
        for ch in code.chars() {
            assert!(
                CODE_ALPHABET.contains(&(ch as u8)),
                "Code contains invalid char: {ch}"
            );
        }
    }

    #[test]
    fn code_excludes_ambiguous_chars() {
        // Verify no O, I, 0, 1 in alphabet
        assert!(!CODE_ALPHABET.contains(&b'O'));
        assert!(!CODE_ALPHABET.contains(&b'I'));
        assert!(!CODE_ALPHABET.contains(&b'0'));
        assert!(!CODE_ALPHABET.contains(&b'1'));
    }

    #[test]
    fn codes_are_unique() {
        let codes: Vec<String> = (0..100).map(|_| generate_code()).collect();
        let unique: std::collections::HashSet<&String> = codes.iter().collect();
        // With 8 chars from 30-char alphabet, collisions in 100 samples are astronomically unlikely
        assert_eq!(codes.len(), unique.len());
    }
}

#[cfg(test)]
mod error_source_tests {
    use std::error::Error as _;
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum::response::IntoResponse as _;

    use super::*;
    use crate::adapters::test_support::response_snapshot;

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

        assert!(matches!(err, AdapterError::Db(_)), "got {err:?}");
        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<sqlx::Error>())
            .is_some());
    }

    #[tokio::test]
    async fn config_serialization_keeps_serde_source_and_redacts() {
        let serde_err = serde_json::from_str::<serde_json::Value>("nope").unwrap_err();
        let err = AdapterError::ConfigSerialization(serde_err);

        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<serde_json::Error>())
            .is_some());

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }
}
