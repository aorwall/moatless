//! Secret DAO — metadata and ciphertext persistence.

use async_trait::async_trait;

use super::models::SecretRecord;
use crate::db::DbPool;

pub struct SecretInsert<'a> {
    pub id: &'a str,
    pub scope_type: &'a str,
    pub scope_id: Option<&'a str>,
    pub key: &'a str,
    pub kind: &'a str,
    pub provider: Option<&'a str>,
    pub target: &'a str,
    pub value_ciphertext: Option<&'a [u8]>,
    pub encryption_nonce: Option<&'a [u8]>,
    pub encrypted_data_key: Option<&'a [u8]>,
    pub external_ref: Option<&'a str>,
    pub kms_key_id: Option<&'a str>,
    pub encryption_version: i32,
    pub actor_id: &'a str,
}

pub struct SecretReplace<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub provider: Option<&'a str>,
    pub target: &'a str,
    pub value_ciphertext: Option<&'a [u8]>,
    pub encryption_nonce: Option<&'a [u8]>,
    pub encrypted_data_key: Option<&'a [u8]>,
    pub external_ref: Option<&'a str>,
    pub kms_key_id: Option<&'a str>,
    pub encryption_version: i32,
    pub actor_id: &'a str,
}

#[async_trait]
pub trait SecretRepository: Send + Sync {
    async fn list_by_scope(
        &self,
        scope_type: &str,
        scope_id: Option<&str>,
    ) -> Result<Vec<SecretRecord>, sqlx::Error>;

    async fn find_by_id(&self, id: &str) -> Result<Option<SecretRecord>, sqlx::Error>;

    async fn find_by_scope_key_target(
        &self,
        scope_type: &str,
        scope_id: Option<&str>,
        key: &str,
        target: &str,
    ) -> Result<Option<SecretRecord>, sqlx::Error>;

    async fn insert(&self, input: SecretInsert<'_>) -> Result<SecretRecord, sqlx::Error>;

    async fn replace(&self, input: SecretReplace<'_>) -> Result<SecretRecord, sqlx::Error>;

    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error>;

    async fn mark_used(&self, id: &str) -> Result<(), sqlx::Error>;
}

pub struct PgSecretRepository(pub DbPool);

#[async_trait]
impl SecretRepository for PgSecretRepository {
    async fn list_by_scope(
        &self,
        scope_type: &str,
        scope_id: Option<&str>,
    ) -> Result<Vec<SecretRecord>, sqlx::Error> {
        sqlx::query_as::<_, SecretRecord>(
            r#"SELECT * FROM secrets
               WHERE scope_type = $1
                 AND (($2::text IS NULL AND scope_id IS NULL) OR scope_id = $2)
               ORDER BY key ASC, target ASC"#,
        )
        .bind(scope_type)
        .bind(scope_id)
        .fetch_all(&self.0)
        .await
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<SecretRecord>, sqlx::Error> {
        sqlx::query_as::<_, SecretRecord>("SELECT * FROM secrets WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.0)
            .await
    }

    async fn find_by_scope_key_target(
        &self,
        scope_type: &str,
        scope_id: Option<&str>,
        key: &str,
        target: &str,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        sqlx::query_as::<_, SecretRecord>(
            r#"SELECT * FROM secrets
               WHERE scope_type = $1
                 AND (($2::text IS NULL AND scope_id IS NULL) OR scope_id = $2)
                 AND key = $3
                 AND target = $4"#,
        )
        .bind(scope_type)
        .bind(scope_id)
        .bind(key)
        .bind(target)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert(&self, input: SecretInsert<'_>) -> Result<SecretRecord, sqlx::Error> {
        sqlx::query_as::<_, SecretRecord>(
            r#"INSERT INTO secrets (
                   id, scope_type, scope_id, key, kind, provider, target,
                   value_ciphertext, encryption_nonce, encrypted_data_key,
                   external_ref, kms_key_id, encryption_version,
                   created_by, updated_by, created_at, updated_at, version
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                       $14, $14, now(), now(), 1)
               RETURNING *"#,
        )
        .bind(input.id)
        .bind(input.scope_type)
        .bind(input.scope_id)
        .bind(input.key)
        .bind(input.kind)
        .bind(input.provider)
        .bind(input.target)
        .bind(input.value_ciphertext)
        .bind(input.encryption_nonce)
        .bind(input.encrypted_data_key)
        .bind(input.external_ref)
        .bind(input.kms_key_id)
        .bind(input.encryption_version)
        .bind(input.actor_id)
        .fetch_one(&self.0)
        .await
    }

    async fn replace(&self, input: SecretReplace<'_>) -> Result<SecretRecord, sqlx::Error> {
        sqlx::query_as::<_, SecretRecord>(
            r#"UPDATE secrets
               SET kind = $2,
                   provider = $3,
                   target = $4,
                   value_ciphertext = $5,
                   encryption_nonce = $6,
                   encrypted_data_key = $7,
                   external_ref = $8,
                   kms_key_id = $9,
                   encryption_version = $10,
                   updated_by = $11,
                   updated_at = now(),
                   version = version + 1
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(input.id)
        .bind(input.kind)
        .bind(input.provider)
        .bind(input.target)
        .bind(input.value_ciphertext)
        .bind(input.encryption_nonce)
        .bind(input.encrypted_data_key)
        .bind(input.external_ref)
        .bind(input.kms_key_id)
        .bind(input.encryption_version)
        .bind(input.actor_id)
        .fetch_one(&self.0)
        .await
    }

    async fn delete(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM secrets WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn mark_used(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE secrets SET last_used_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}
