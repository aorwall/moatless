//! Device pairing table queries, and the API key a redeemed code mints.

use std::sync::Arc;

use super::api_key;
use super::api_keys_dao::ApiKeyDao;
use super::pairing::{
    PAIRED_DEVICE_KEY_TTL, PairedDevice, PairingApiKeyIssuer, PairingCode, PairingError,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

#[async_trait]
pub(crate) trait PairingCodeDao: Send + Sync {
    async fn insert_pairing_code(&self, code: &PairingCode) -> Result<(), sqlx::Error>;
    async fn consume_pairing_code(
        &self,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PairingCode>, sqlx::Error>;
    async fn delete_finished_pairing_codes(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error>;
}

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
            Some(PAIRED_DEVICE_KEY_TTL),
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

#[cfg(all(test, feature = "db-tests"))]
#[path = "pairing_dao_tests.rs"]
mod db_tests;
