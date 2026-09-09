//! The `forgejo_instances` table: one registered OAuth2 application per host.

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;

use crate::auth::error::AuthError;
use crate::secret::models::EncryptedSecretValue;

/// A registered instance with its client secret still sealed.
///
/// [`Self::host`] is the additional authenticated data the secret was encrypted
/// under, so a ciphertext cannot be replayed onto another instance's row.
pub(crate) struct ForgejoInstanceRecord {
    pub id: String,
    pub host: String,
    pub client_id: String,
    pub client_secret: EncryptedSecretValue,
}

/// What an admin screen may see: everything but the secret.
pub(crate) struct ForgejoInstanceSummary {
    pub host: String,
    pub client_id: String,
    pub updated_at: DateTime<Utc>,
}

pub(crate) struct PgForgejoInstanceDao(pub(crate) DbPool);

#[derive(sqlx::FromRow)]
struct ForgejoInstanceRow {
    id: String,
    host: String,
    client_id: String,
    client_secret_ciphertext: Vec<u8>,
    client_secret_nonce: Vec<u8>,
    client_secret_data_key: Vec<u8>,
    kms_key_id: String,
    encryption_version: i32,
}

impl From<ForgejoInstanceRow> for ForgejoInstanceRecord {
    fn from(row: ForgejoInstanceRow) -> Self {
        Self {
            id: row.id,
            host: row.host,
            client_id: row.client_id,
            client_secret: EncryptedSecretValue {
                value_ciphertext: row.client_secret_ciphertext,
                encryption_nonce: row.client_secret_nonce,
                encrypted_data_key: row.client_secret_data_key,
                kms_key_id: row.kms_key_id,
                encryption_version: row.encryption_version,
            },
        }
    }
}

impl PgForgejoInstanceDao {
    pub(crate) async fn find_by_host(
        &self,
        host: &str,
    ) -> Result<Option<ForgejoInstanceRecord>, AuthError> {
        let row: Option<ForgejoInstanceRow> = sqlx::query_as(
            "SELECT id, host, client_id, client_secret_ciphertext, client_secret_nonce, \
                    client_secret_data_key, kms_key_id, encryption_version \
             FROM forgejo_instances WHERE host = $1",
        )
        .bind(host)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(Into::into))
    }

    pub(crate) async fn list(&self) -> Result<Vec<ForgejoInstanceSummary>, AuthError> {
        let rows: Vec<(String, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT host, client_id, updated_at FROM forgejo_instances ORDER BY host",
        )
        .fetch_all(&self.0)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(host, client_id, updated_at)| ForgejoInstanceSummary {
                host,
                client_id,
                updated_at,
            })
            .collect())
    }

    /// Insert `record`, or replace the application registered for its host, and
    /// return the row's `updated_at`.
    ///
    /// A replace keeps the stored `id`, and the incoming ciphertext is sealed
    /// under `host` rather than that id: `host` is the conflict key, so it is
    /// the one value the caller can seal against without first reading the row
    /// it is racing another registration to write.
    pub(crate) async fn upsert(
        &self,
        record: &ForgejoInstanceRecord,
        actor_id: &str,
    ) -> Result<DateTime<Utc>, AuthError> {
        let (updated_at,): (DateTime<Utc>,) = sqlx::query_as(
            r#"INSERT INTO forgejo_instances (
                   id, host, client_id, client_secret_ciphertext, client_secret_nonce,
                   client_secret_data_key, kms_key_id, encryption_version,
                   created_by, updated_by
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
               ON CONFLICT (host) DO UPDATE SET
                   client_id = $3,
                   client_secret_ciphertext = $4,
                   client_secret_nonce = $5,
                   client_secret_data_key = $6,
                   kms_key_id = $7,
                   encryption_version = $8,
                   updated_by = $9,
                   updated_at = now()
               RETURNING updated_at"#,
        )
        .bind(&record.id)
        .bind(&record.host)
        .bind(&record.client_id)
        .bind(&record.client_secret.value_ciphertext)
        .bind(&record.client_secret.encryption_nonce)
        .bind(&record.client_secret.encrypted_data_key)
        .bind(&record.client_secret.kms_key_id)
        .bind(record.client_secret.encryption_version)
        .bind(actor_id)
        .fetch_one(&self.0)
        .await?;
        Ok(updated_at)
    }

    pub(crate) async fn delete(&self, host: &str) -> Result<bool, AuthError> {
        let deleted = sqlx::query("DELETE FROM forgejo_instances WHERE host = $1")
            .bind(host)
            .execute(&self.0)
            .await?;
        Ok(deleted.rows_affected() > 0)
    }
}
