//! Telegram pairing — code-based DM access approval.

use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::adapters::error::AdapterError;
use crate::db::DbPool;

use super::config::TelegramAdapterConfig;

/// Row from the `integration_pairing_requests` table.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PairingRequest {
    pub id: i32,
    pub trigger_id: String,
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
    trigger_id: &str,
    provider_user_id: &str,
    meta: Option<serde_json::Value>,
) -> Result<PairingRequest, AdapterError> {
    // Check for existing pending request
    let existing: Option<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE trigger_id = $1 AND adapter_kind = 'telegram' AND provider_user_id = $2 AND status = 'pending' \
         LIMIT 1",
    )
    .bind(trigger_id)
    .bind(provider_user_id)
    .fetch_optional(db)
    .await
    .map_err(AdapterError::Db)?;

    if let Some(req) = existing {
        return Ok(req);
    }

    let code = generate_code();

    let row: PairingRequest = sqlx::query_as(
        "INSERT INTO integration_pairing_requests (trigger_id, adapter_kind, provider_user_id, code, status, meta) \
         VALUES ($1, 'telegram', $2, $3, 'pending', $4) \
         RETURNING *",
    )
    .bind(trigger_id)
    .bind(provider_user_id)
    .bind(&code)
    .bind(&meta)
    .fetch_one(db)
    .await
    .map_err(AdapterError::Db)?;

    info!(
        trigger_id,
        provider_user_id,
        code = %row.code,
        "Created pairing request"
    );

    Ok(row)
}

/// Find a pending request by code.
pub async fn find_by_code(
    db: &DbPool,
    trigger_id: &str,
    code: &str,
) -> Result<Option<PairingRequest>, AdapterError> {
    let row: Option<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE trigger_id = $1 AND code = $2 AND status = 'pending' \
         LIMIT 1",
    )
    .bind(trigger_id)
    .bind(code)
    .fetch_optional(db)
    .await
    .map_err(AdapterError::Db)?;

    Ok(row)
}

/// List all pending requests for a config.
pub async fn list_pending(
    db: &DbPool,
    trigger_id: &str,
) -> Result<Vec<PairingRequest>, AdapterError> {
    let rows: Vec<PairingRequest> = sqlx::query_as(
        "SELECT * FROM integration_pairing_requests \
         WHERE trigger_id = $1 AND status = 'pending' \
         ORDER BY created_at DESC",
    )
    .bind(trigger_id)
    .fetch_all(db)
    .await
    .map_err(AdapterError::Db)?;

    Ok(rows)
}

/// List all pending pairing requests for triggers owned by the given user.
pub async fn list_pending_for_user(
    db: &DbPool,
    user_id: &str,
) -> Result<Vec<PairingRequest>, AdapterError> {
    let rows: Vec<PairingRequest> = sqlx::query_as(
        "SELECT p.* FROM integration_pairing_requests p \
         JOIN triggers t ON t.id = p.trigger_id \
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
    trigger_id: &str,
    code: &str,
) -> Result<PairingRequest, AdapterError> {
    // Find the pending request
    let req = find_by_code(db, trigger_id, code)
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

    // Read current provider_config and append user to dm_allowlist
    let row: (Option<serde_json::Value>,) = sqlx::query_as(
        "SELECT adapter_config FROM adapter_event_trigger_sources WHERE trigger_id = $1",
    )
    .bind(trigger_id)
    .fetch_one(db)
    .await
    .map_err(AdapterError::Db)?;

    let mut tg_config: TelegramAdapterConfig = row
        .0
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Add user if not already present
    let user_id_str = req.provider_user_id.clone();
    if !tg_config.dm_allowlist.contains(&user_id_str) {
        tg_config.dm_allowlist.push(user_id_str);
    }

    let updated_config = serde_json::to_value(&tg_config)
        .map_err(|e| AdapterError::Internal(format!("Failed to serialize config: {e}")))?;

    sqlx::query(
        "UPDATE adapter_event_trigger_sources SET adapter_config = $1 WHERE trigger_id = $2",
    )
    .bind(&updated_config)
    .bind(trigger_id)
    .execute(db)
    .await
    .map_err(AdapterError::Db)?;
    sqlx::query("UPDATE triggers SET updated_at = now() WHERE id = $1")
        .bind(trigger_id)
        .execute(db)
        .await
        .map_err(AdapterError::Db)?;

    info!(
        trigger_id,
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
