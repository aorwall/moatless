use super::pairing::{PairingCode, PairingCodeDao};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

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
