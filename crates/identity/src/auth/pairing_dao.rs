//! Device pairing table queries, and the API key a redeemed code mints.

use std::sync::Arc;

use super::api_key;
use super::api_keys_dao::ApiKeyDao;
use super::pairing::{
    PAIRED_DEVICE_KEY_TTL, PairedDevice, PairedDeviceKey, PairedDeviceKeys, PairingCode,
    PairingError,
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
    async fn list_pending_pairing_codes(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<PairingCode>, sqlx::Error>;
    async fn delete_pending_pairing_code(
        &self,
        user_id: &str,
        code_id: &str,
    ) -> Result<bool, sqlx::Error>;
}

type PairingCodeRow = (
    String,
    String,
    String,
    Option<String>,
    DateTime<Utc>,
    DateTime<Utc>,
);

fn pairing_code_from_row(
    (id, code_hash, user_id, label, created_at, expires_at): PairingCodeRow,
) -> PairingCode {
    PairingCode {
        id,
        code_hash,
        user_id,
        label,
        created_at,
        expires_at,
    }
}

/// PostgreSQL implementation of [`PairingCodeDao`].
pub struct PgPairingCodeDao(pub DbPool);

#[async_trait]
impl PairingCodeDao for PgPairingCodeDao {
    async fn insert_pairing_code(&self, code: &PairingCode) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO device_pairing_codes
                (id, code_hash, user_id, label, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(&code.id)
        .bind(&code.code_hash)
        .bind(&code.user_id)
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
        sqlx::query_as::<_, PairingCodeRow>(
            r#"
            UPDATE device_pairing_codes
               SET consumed_at = $2
             WHERE code_hash = $1
               AND consumed_at IS NULL
               AND expires_at > $2
            RETURNING id, code_hash, user_id, label, created_at, expires_at
            "#,
        )
        .bind(code_hash)
        .bind(now)
        .fetch_optional(&self.0)
        .await
        .map(|row| row.map(pairing_code_from_row))
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

    async fn list_pending_pairing_codes(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<PairingCode>, sqlx::Error> {
        sqlx::query_as::<_, PairingCodeRow>(
            r#"
            SELECT id, code_hash, user_id, label, created_at, expires_at
              FROM device_pairing_codes
             WHERE user_id = $1
               AND consumed_at IS NULL
               AND expires_at > $2
             ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .bind(now)
        .fetch_all(&self.0)
        .await
        .map(|rows| rows.into_iter().map(pairing_code_from_row).collect())
    }

    async fn delete_pending_pairing_code(
        &self,
        user_id: &str,
        code_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM device_pairing_codes \
             WHERE id = $1 AND user_id = $2 AND consumed_at IS NULL",
        )
        .bind(code_id)
        .bind(user_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

pub struct PgPairedDeviceKeys(pub Arc<dyn ApiKeyDao>);

#[async_trait]
impl PairedDeviceKeys for PgPairedDeviceKeys {
    async fn issue_api_key(
        &self,
        user_id: &str,
        name: &str,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
    ) -> Result<PairedDevice, PairingError> {
        let key = api_key::generate_key(
            self.0.as_ref(),
            user_id,
            name,
            Some(scopes.clone()),
            Some(expires_at),
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

    async fn list_paired_keys(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<PairedDeviceKey>, PairingError> {
        let keys = self.0.list_api_keys_by_user(user_id).await?;
        Ok(keys
            .into_iter()
            .filter(|key| key.idle_ttl_secs.is_some())
            .filter_map(|key| {
                let expires_at = key.expires_at.filter(|expires_at| *expires_at > now)?;
                Some(PairedDeviceKey {
                    id: key.id,
                    name: key.name,
                    created_at: key.created_at,
                    expires_at,
                    last_used_at: key.last_used_at,
                })
            })
            .collect())
    }

    async fn delete_paired_key(&self, user_id: &str, key_id: &str) -> Result<bool, PairingError> {
        let Some(key) = self.0.get_api_key_by_id(key_id).await? else {
            return Ok(false);
        };
        if key.user_id != user_id || key.idle_ttl_secs.is_none() {
            return Ok(false);
        }
        self.0.delete_api_key(key_id).await?;
        Ok(true)
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "pairing_dao_tests.rs"]
mod db_tests;
