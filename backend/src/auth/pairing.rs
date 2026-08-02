//! Device pairing persistence and API-key issuance adapters.
//!
//! The pairing flow itself lives in `moatless-backend-core`; the backend owns
//! the database table that stores codes and the API-key service that mints the
//! credential a paired device keeps.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_backend_core::pairing as core_pairing;

use super::api_key;
use super::api_keys::ApiKeyDao;
use crate::db::DbPool;

pub use core_pairing::{
    hash_key, DevicePairing, IssuedPairingCode, PairedDevice, PairingApiKeyIssuer, PairingClient,
    PairingCode, PairingCodeDao, PairingError, PAIRED_DEVICE_KEY_TTL, PAIRING_CODE_TTL,
};

/// PostgreSQL implementation of [`PairingCodeDao`].
pub struct PgPairingCodeDao(pub DbPool);

#[async_trait]
impl PairingCodeDao for PgPairingCodeDao {
    async fn insert_pairing_code(&self, code: &PairingCode) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO device_pairing_codes
                (id, code_hash, user_id, group_id, label, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(&code.id)
        .bind(&code.code_hash)
        .bind(&code.user_id)
        .bind(&code.group_id)
        .bind(&code.label)
        .bind(code.created_at)
        .bind(code.expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn consume_pairing_code(
        &self,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PairingCode>, sqlx::Error> {
        // One statement, so two backends racing the same code produce one
        // winner: the loser's UPDATE matches no row and returns nothing.
        sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                DateTime<Utc>,
                DateTime<Utc>,
            ),
        >(
            r#"
            UPDATE device_pairing_codes
               SET consumed_at = $2
             WHERE code_hash = $1
               AND consumed_at IS NULL
               AND expires_at > $2
            RETURNING id, code_hash, user_id, group_id, label, created_at, expires_at
            "#,
        )
        .bind(code_hash)
        .bind(now)
        .fetch_optional(&self.0)
        .await
        .map(|row| {
            row.map(
                |(id, code_hash, user_id, group_id, label, created_at, expires_at)| PairingCode {
                    id,
                    code_hash,
                    user_id,
                    group_id,
                    label,
                    created_at,
                    expires_at,
                },
            )
        })
    }

    async fn delete_finished_pairing_codes(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM device_pairing_codes WHERE expires_at < $1")
            .bind(cutoff)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }
}

pub struct PgPairingApiKeyIssuer(pub Arc<dyn ApiKeyDao>);

#[async_trait]
impl PairingApiKeyIssuer for PgPairingApiKeyIssuer {
    async fn issue_api_key(
        &self,
        user_id: &str,
        name: &str,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
        group_id: Option<&str>,
    ) -> Result<PairedDevice, PairingError> {
        let key = api_key::generate_key(
            self.0.as_ref(),
            user_id,
            name,
            Some(scopes.clone()),
            Some(expires_at),
            group_id,
        )
        .await
        .map_err(|error| PairingError::Storage(error.to_string()))?;

        Ok(PairedDevice {
            key_id: key.id,
            token: key.key,
            expires_at,
            scopes,
        })
    }
}
