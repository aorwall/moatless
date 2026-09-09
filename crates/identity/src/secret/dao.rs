//! Secret DAO — metadata and ciphertext persistence.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::SecretRecord;
use moatless_core::db::DbPool;
use moatless_types::access_control::{ScopeContext, ScopedEntity};

pub(crate) struct SecretInsert<'a> {
    pub id: &'a str,
    pub scope: &'a str,
    pub owner_user_id: Option<&'a str>,
    pub key: &'a str,
    pub kind: &'a str,
    pub provider: Option<&'a str>,
    pub value_ciphertext: Option<&'a [u8]>,
    pub encryption_nonce: Option<&'a [u8]>,
    pub encrypted_data_key: Option<&'a [u8]>,
    pub external_ref: Option<&'a str>,
    pub kms_key_id: Option<&'a str>,
    pub encryption_version: i32,
    pub actor_id: &'a str,
}

pub(crate) struct SecretReplace<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub provider: Option<&'a str>,
    pub value_ciphertext: Option<&'a [u8]>,
    pub encryption_nonce: Option<&'a [u8]>,
    pub encrypted_data_key: Option<&'a [u8]>,
    pub external_ref: Option<&'a str>,
    pub kms_key_id: Option<&'a str>,
    pub encryption_version: i32,
    /// `None` preserves the current flag; `Some` sets it (re-enable on set,
    /// toggle on patch).
    pub enabled: Option<bool>,
    pub actor_id: &'a str,
}

const SECRET_COLUMNS: &str = r#"id, scope, owner_user_id, key, kind, provider,
                      value_ciphertext, encryption_nonce, encrypted_data_key,
                      external_ref, kms_key_id, encryption_version,
                      created_by, updated_by, created_at, updated_at,
                      last_used_at, version, enabled"#;

#[async_trait]
pub(crate) trait SecretDao: Send + Sync {
    async fn list(
        &self,
        ctx: &ScopeContext,
        scope_filter: Option<&str>,
    ) -> Result<Vec<SecretRecord>, sqlx::Error>;

    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<SecretRecord>, sqlx::Error>;

    /// System-only lookup by natural key for secret resolution paths.
    async fn find_by_key(
        &self,
        scope: &str,
        owner_user_id: Option<&str>,
        key: &str,
    ) -> Result<Option<SecretRecord>, sqlx::Error>;

    async fn insert(&self, input: SecretInsert<'_>) -> Result<SecretRecord, sqlx::Error>;

    async fn replace(
        &self,
        input: SecretReplace<'_>,
        ctx: &ScopeContext,
    ) -> Result<Option<SecretRecord>, sqlx::Error>;

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error>;

    // reason: system bookkeeping — called after resolution, not scoped to an actor
    async fn mark_used(&self, id: &str) -> Result<(), sqlx::Error>;
}

pub(crate) struct PgSecretDao(pub(crate) DbPool);

#[async_trait]
impl SecretDao for PgSecretDao {
    async fn list(
        &self,
        ctx: &ScopeContext,
        scope_filter: Option<&str>,
    ) -> Result<Vec<SecretRecord>, sqlx::Error> {
        // access_filter uses $1 (is_admin) and $2 (user_id)
        let af = moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 1);
        let sql = if let Some(scope) = scope_filter {
            format!(
                "SELECT {SECRET_COLUMNS} FROM secrets \
                 WHERE scope = '{scope}' AND {} ORDER BY key ASC",
                af.sql()
            )
        } else {
            format!(
                "SELECT {SECRET_COLUMNS} FROM secrets \
                 WHERE {} ORDER BY key ASC",
                af.sql()
            )
        };
        sqlx::query_as::<_, SecretRecord>(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_all(&self.0)
            .await
    }

    async fn find_by_id(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        // $1 = id; access_filter uses $2 (is_admin), $3 (user_id)
        let af = moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 2);
        let sql = format!(
            "SELECT {SECRET_COLUMNS} FROM secrets WHERE id = $1 AND {}",
            af.sql()
        );
        sqlx::query_as::<_, SecretRecord>(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn find_by_key(
        &self,
        scope: &str,
        owner_user_id: Option<&str>,
        key: &str,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        sqlx::query_as::<_, SecretRecord>(AssertSqlSafe(format!(
            "SELECT {SECRET_COLUMNS} FROM secrets \
                 WHERE scope = $1 \
                   AND COALESCE(owner_user_id, '') = COALESCE($2, '') \
                   AND key = $3"
        )))
        .bind(scope)
        .bind(owner_user_id)
        .bind(key)
        .fetch_optional(&self.0)
        .await
    }

    async fn insert(&self, input: SecretInsert<'_>) -> Result<SecretRecord, sqlx::Error> {
        sqlx::query_as::<_, SecretRecord>(AssertSqlSafe(format!(
            r#"INSERT INTO secrets (
                       id, scope, owner_user_id, key, kind, provider,
                       value_ciphertext, encryption_nonce, encrypted_data_key,
                       external_ref, kms_key_id, encryption_version,
                       created_by, updated_by, created_at, updated_at, version
                   )
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                           $13, $13, now(), now(), 1)
                   RETURNING {SECRET_COLUMNS}"#
        )))
        .bind(input.id)
        .bind(input.scope)
        .bind(input.owner_user_id)
        .bind(input.key)
        .bind(input.kind)
        .bind(input.provider)
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

    async fn replace(
        &self,
        input: SecretReplace<'_>,
        ctx: &ScopeContext,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        // Bind order: $1=id, $2=kind, $3=provider, $4=value_ciphertext,
        //   $5=encryption_nonce, $6=encrypted_data_key, $7=external_ref,
        //   $8=kms_key_id, $9=encryption_version, $10=actor_id, $11=enabled,
        //   then write_filter: $12=is_admin, $13=user_id
        let cf = moatless_core::db::access_control::write_filter("", 12);
        let sql = format!(
            r#"UPDATE secrets
               SET kind = $2,
                   provider = $3,
                   value_ciphertext = $4,
                   encryption_nonce = $5,
                   encrypted_data_key = $6,
                   external_ref = $7,
                   kms_key_id = $8,
                   encryption_version = $9,
                   enabled = COALESCE($11, enabled),
                   updated_by = $10,
                   updated_at = now(),
                   version = version + 1
               WHERE id = $1 AND {}
               RETURNING {SECRET_COLUMNS}"#,
            cf.sql()
        );
        sqlx::query_as::<_, SecretRecord>(AssertSqlSafe(sql))
            .bind(input.id)
            .bind(input.kind)
            .bind(input.provider)
            .bind(input.value_ciphertext)
            .bind(input.encryption_nonce)
            .bind(input.encrypted_data_key)
            .bind(input.external_ref)
            .bind(input.kms_key_id)
            .bind(input.encryption_version)
            .bind(input.actor_id)
            .bind(input.enabled)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        // $1=id; write_filter uses $2=is_admin, $3=user_id
        let cf = moatless_core::db::access_control::write_filter("", 2);
        let sql = format!("DELETE FROM secrets WHERE id = $1 AND {}", cf.sql());
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // reason: system bookkeeping — called after resolution, not scoped to an actor
    async fn mark_used(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE secrets SET last_used_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}
