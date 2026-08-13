//! Workspace credential DAO — ciphertext persistence for `workspace_credentials`.
//!
//! Every method here is unscoped: access control for a workspace credential is
//! control of the owning workspace row, which
//! [`super::service::WorkspaceCredentialService`] checks through `WorkspaceDao`
//! before it calls in. Duplicating a scope filter on this table would give the
//! two places to disagree, and the sandbox resolution path has no actor to
//! filter by in the first place.

use async_trait::async_trait;

use sqlx::AssertSqlSafe;

use moatless_core::db::DbPool;

use super::models::{WorkspaceCredential, WorkspaceCredentialWrite};

const CREDENTIAL_COLUMNS: &str = r#"id, workspace_id, provider,
    value_ciphertext, encryption_nonce, encrypted_data_key,
    kms_key_id, encryption_version,
    created_by, updated_by, created_at, updated_at"#;

#[async_trait]
pub trait WorkspaceCredentialDao: Send + Sync {
    /// Every credential pinned to a workspace, ordered by provider.
    async fn list(&self, workspace_id: &str) -> Result<Vec<WorkspaceCredential>, sqlx::Error>;

    /// One credential by its natural key.
    async fn find(
        &self,
        workspace_id: &str,
        provider: &str,
    ) -> Result<Option<WorkspaceCredential>, sqlx::Error>;

    /// Insert or replace the value for `(workspace_id, provider)`, returning
    /// the stored row.
    async fn upsert(
        &self,
        write: WorkspaceCredentialWrite,
    ) -> Result<WorkspaceCredential, sqlx::Error>;

    /// Remove a credential. `false` when there was none to remove.
    async fn delete(&self, workspace_id: &str, provider: &str) -> Result<bool, sqlx::Error>;
}

pub struct PgWorkspaceCredentialDao(pub DbPool);

#[async_trait]
impl WorkspaceCredentialDao for PgWorkspaceCredentialDao {
    async fn list(&self, workspace_id: &str) -> Result<Vec<WorkspaceCredential>, sqlx::Error> {
        sqlx::query_as::<_, WorkspaceCredential>(AssertSqlSafe(format!(
            "SELECT {CREDENTIAL_COLUMNS} FROM workspace_credentials \
             WHERE workspace_id = $1 ORDER BY provider"
        )))
        .bind(workspace_id)
        .fetch_all(&self.0)
        .await
    }

    async fn find(
        &self,
        workspace_id: &str,
        provider: &str,
    ) -> Result<Option<WorkspaceCredential>, sqlx::Error> {
        sqlx::query_as::<_, WorkspaceCredential>(AssertSqlSafe(format!(
            "SELECT {CREDENTIAL_COLUMNS} FROM workspace_credentials \
             WHERE workspace_id = $1 AND provider = $2"
        )))
        .bind(workspace_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert(
        &self,
        write: WorkspaceCredentialWrite,
    ) -> Result<WorkspaceCredential, sqlx::Error> {
        // `id` is the ciphertext's additional authenticated data, so a
        // conflicting row keeps its own id and the caller re-encrypts under it
        // (`WorkspaceCredentialService::set` reads the existing row first).
        sqlx::query_as::<_, WorkspaceCredential>(AssertSqlSafe(format!(
            "INSERT INTO workspace_credentials (
                 id, workspace_id, provider,
                 value_ciphertext, encryption_nonce, encrypted_data_key,
                 kms_key_id, encryption_version,
                 created_by, updated_by, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, now(), now())
             ON CONFLICT (workspace_id, provider) DO UPDATE SET
                 value_ciphertext = EXCLUDED.value_ciphertext,
                 encryption_nonce = EXCLUDED.encryption_nonce,
                 encrypted_data_key = EXCLUDED.encrypted_data_key,
                 kms_key_id = EXCLUDED.kms_key_id,
                 encryption_version = EXCLUDED.encryption_version,
                 updated_by = EXCLUDED.updated_by,
                 updated_at = now()
             RETURNING {CREDENTIAL_COLUMNS}"
        )))
        .bind(&write.id)
        .bind(&write.workspace_id)
        .bind(&write.provider)
        .bind(&write.value.value_ciphertext)
        .bind(&write.value.encryption_nonce)
        .bind(&write.value.encrypted_data_key)
        .bind(&write.value.kms_key_id)
        .bind(write.value.encryption_version)
        .bind(&write.actor_id)
        .fetch_one(&self.0)
        .await
    }

    async fn delete(&self, workspace_id: &str, provider: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM workspace_credentials WHERE workspace_id = $1 AND provider = $2",
        )
        .bind(workspace_id)
        .bind(provider)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
