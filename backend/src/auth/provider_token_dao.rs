//! Provider token DAO — queries for credential resolution.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{GithubTokenPresence, ProviderAccessToken, ProviderToken};
use crate::auth::codex_scope::CodexScope;
use crate::auth::github_scope::GitHubScope;
use moatless_core::db::DbPool;

/// DAO trait for provider token credential resolution.
///
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` / `Option<&Value>` in async-trait
/// methods.  Tests use `PgProviderTokenDao` via `#[sqlx::test]`.
// reason: provider-token upserts already pass 7+ atomic columns; wrapping
// into a struct would require touching every call-site and impl for no real
// readability gain.
#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait ProviderTokenDao: Send + Sync {
    /// Find a provider token (access_token only) by user_id and provider name.
    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error>;

    /// Find the full provider token row by user_id and provider name.
    async fn find_provider_token_full(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert a provider token (insert or update on conflict). Inserts with
    /// `auth_method = 'manual'` and leaves an existing row's `auth_method`
    /// untouched — use [`upsert_token_with_auth_method`] to set it explicitly.
    ///
    /// [`upsert_token_with_auth_method`]: ProviderTokenDao::upsert_token_with_auth_method
    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Upsert a provider token, setting `auth_method` on both insert and
    /// update. Used for reference rows (`secret_ref`) whose resolution branch
    /// must switch even when a row already exists.
    async fn upsert_token_with_auth_method(
        &self,
        user_id: &str,
        provider: &str,
        auth_method: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Delete a provider token for a user.
    async fn delete_token(&self, user_id: &str, provider: &str) -> Result<(), sqlx::Error>;

    // ---- Scope-aware codex methods ----

    /// Find the codex `provider_tokens` row addressed by `scope`. Returns the
    /// full row (including `metadata`) so callers can build a
    /// `CodexConfigResponse` without a follow-up read.
    async fn find_codex_blob(
        &self,
        scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert a codex blob at the requested scope. The unified shape replaces
    /// the user / global triplet so the user / admin endpoints share one
    /// persistence path.
    async fn upsert_codex_blob(
        &self,
        scope: &CodexScope,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Delete the codex row at the requested scope.
    async fn delete_codex_blob(&self, scope: &CodexScope) -> Result<(), sqlx::Error>;

    // ---- Scope-aware github methods ----

    /// Find the github `provider_tokens` row addressed by `scope`.
    async fn find_github_row(
        &self,
        scope: &GitHubScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert the github row at the requested scope.
    async fn upsert_github_row(
        &self,
        scope: &GitHubScope,
        auth_method: &str,
        access_token: &str,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        provider_user_id: Option<&str>,
        provider_login: Option<&str>,
    ) -> Result<(), sqlx::Error>;

    /// Delete the github row at the requested scope.
    async fn delete_github_row(&self, scope: &GitHubScope) -> Result<(), sqlx::Error>;

    /// Resolve the `user > global` scope walk in one query, returning
    /// only addressing info — no token material, no decrypt, no write. Drives
    /// both `/me`'s setup evaluation and the token resolver's scope selection.
    async fn find_github_scope_presence(
        &self,
        user_id: &str,
    ) -> Result<Option<GithubTokenPresence>, sqlx::Error>;
}

/// PostgreSQL implementation of [`ProviderTokenDao`].
pub struct PgProviderTokenDao(pub DbPool);

#[async_trait]
impl ProviderTokenDao for PgProviderTokenDao {
    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderAccessToken>(
            "SELECT access_token FROM provider_tokens WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_provider_token_full(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderToken>(
            "SELECT * FROM provider_tokens WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   user_id, provider, access_token, refresh_token, token_type, metadata,
                   auth_method, connected_at, updated_at, scope, owner_user_id
               )
               VALUES ($1, $2, $3, $4, $5, $6, 'manual', NOW(), NOW(), 'user', $1)
               ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
               DO UPDATE SET access_token = $3, refresh_token = $4, token_type = $5, metadata = $6,
                             scope = 'user', owner_user_id = $1, updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(access_token)
        .bind(refresh_token)
        .bind(token_type)
        .bind(metadata)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn upsert_token_with_auth_method(
        &self,
        user_id: &str,
        provider: &str,
        auth_method: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   user_id, provider, access_token, refresh_token, token_type, metadata,
                   auth_method, connected_at, updated_at, scope, owner_user_id
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW(), 'user', $1)
               ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
               DO UPDATE SET access_token = $3, refresh_token = $4, token_type = $5, metadata = $6,
                             auth_method = $7, scope = 'user', owner_user_id = $1, updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(access_token)
        .bind(refresh_token)
        .bind(token_type)
        .bind(metadata)
        .bind(auth_method)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete_token(&self, user_id: &str, provider: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM provider_tokens WHERE user_id = $1 AND provider = $2")
            .bind(user_id)
            .bind(provider)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn find_codex_blob(
        &self,
        scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        let where_clause = scope.where_clause("$1");
        // `ProviderToken::user_id` is `NOT NULL` in the Rust struct so
        // coalesce NULL to '' for global rows.
        let sql = format!(
            r#"SELECT COALESCE(user_id, '') AS user_id, provider, access_token, refresh_token,
                      token_type, auth_method, expires_at, metadata, created_at, updated_at,
                      provider_user_id, provider_login
               FROM provider_tokens
               WHERE {where_clause}"#,
        );
        sqlx::query_as::<_, ProviderToken>(AssertSqlSafe(sql))
            .bind(scope.bind_value())
            .fetch_optional(&self.0)
            .await
    }

    async fn upsert_codex_blob(
        &self,
        scope: &CodexScope,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        match scope {
            CodexScope::User(user_id) => {
                // User scope: delegate to upsert_token so the user path stays
                // in one place.
                self.upsert_token(
                    user_id,
                    "codex",
                    access_token,
                    refresh_token,
                    token_type,
                    metadata,
                )
                .await
            }
            CodexScope::Global => {
                // Global codex row: user_id=NULL, team_id=NULL, scope='global'.
                // The unique constraint provider_tokens_scope_uq is
                // UNIQUE NULLS NOT DISTINCT (user_id, provider, team_id),
                // so the NULL/NULL combination is unique per provider.
                sqlx::query(
                    r#"INSERT INTO provider_tokens (
                           provider, auth_method, access_token, refresh_token,
                           token_type, metadata, connected_at, updated_at, scope,
                           owner_user_id
                       )
                       VALUES ('codex', 'manual', $1, $2, $3, $4, NOW(), NOW(),
                               'global', NULL)
                       ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
                       DO UPDATE SET access_token = $1, refresh_token = $2,
                                     token_type = $3, metadata = $4,
                                     auth_method = 'manual', scope = 'global',
                                     owner_user_id = NULL, updated_at = NOW()"#,
                )
                .bind(access_token)
                .bind(refresh_token)
                .bind(token_type)
                .bind(metadata)
                .execute(&self.0)
                .await?;
                Ok(())
            }
        }
    }

    async fn delete_codex_blob(&self, scope: &CodexScope) -> Result<(), sqlx::Error> {
        let where_clause = scope.where_clause("$1");
        let sql = format!("DELETE FROM provider_tokens WHERE {where_clause}");
        sqlx::query(AssertSqlSafe(sql))
            .bind(scope.bind_value())
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn find_github_row(
        &self,
        scope: &GitHubScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        let where_clause = scope.where_clause("$1");
        // `ProviderToken::user_id` is `NOT NULL` in the Rust struct so
        // coalesce NULL to '' for global rows.
        let sql = format!(
            r#"SELECT COALESCE(user_id, '') AS user_id, provider, access_token, refresh_token,
                      token_type, auth_method, expires_at, metadata, created_at, updated_at,
                      provider_user_id, provider_login
               FROM provider_tokens
               WHERE {where_clause}"#,
        );
        sqlx::query_as::<_, ProviderToken>(AssertSqlSafe(sql))
            .bind(scope.bind_value())
            .fetch_optional(&self.0)
            .await
    }

    async fn upsert_github_row(
        &self,
        scope: &GitHubScope,
        auth_method: &str,
        access_token: &str,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        provider_user_id: Option<&str>,
        provider_login: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        match scope {
            GitHubScope::User(user_id) => {
                self.upsert_token_with_auth_method(
                    user_id,
                    "github",
                    auth_method,
                    access_token,
                    None,
                    token_type,
                    metadata,
                )
                .await?;
                if provider_user_id.is_some() || provider_login.is_some() {
                    sqlx::query(
                        r#"UPDATE provider_tokens
                           SET provider_user_id = $2, provider_login = $3
                           WHERE user_id = $1 AND provider = 'github' AND scope = 'user'"#,
                    )
                    .bind(user_id)
                    .bind(provider_user_id)
                    .bind(provider_login)
                    .execute(&self.0)
                    .await?;
                }
                Ok(())
            }
            GitHubScope::Global => {
                // Global github row: user_id=NULL, team_id=NULL, scope='global'.
                // The unique constraint provider_tokens_scope_uq covers this row
                // uniquely as UNIQUE NULLS NOT DISTINCT (user_id, provider, team_id).
                sqlx::query(
                    r#"INSERT INTO provider_tokens (
                           provider, auth_method, access_token, refresh_token,
                           token_type, metadata, provider_user_id, provider_login,
                           connected_at, updated_at, scope,
                           owner_user_id
                       )
                       VALUES ('github', $1, $2, NULL, $3, $4, $5, $6, NOW(), NOW(),
                               'global', NULL)
                       ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
                       DO UPDATE SET access_token = $2, refresh_token = NULL,
                                     token_type = $3, metadata = $4,
                                     provider_user_id = $5, provider_login = $6,
                                     auth_method = $1, scope = 'global',
                                     owner_user_id = NULL, updated_at = NOW()"#,
                )
                .bind(auth_method)
                .bind(access_token)
                .bind(token_type)
                .bind(metadata)
                .bind(provider_user_id)
                .bind(provider_login)
                .execute(&self.0)
                .await?;
                Ok(())
            }
        }
    }

    async fn delete_github_row(&self, scope: &GitHubScope) -> Result<(), sqlx::Error> {
        let where_clause = scope.where_clause("$1");
        let sql = format!("DELETE FROM provider_tokens WHERE {where_clause}");
        sqlx::query(AssertSqlSafe(sql))
            .bind(scope.bind_value())
            .execute(&self.0)
            .await?;
        Ok(())
    }

    async fn find_github_scope_presence(
        &self,
        user_id: &str,
    ) -> Result<Option<GithubTokenPresence>, sqlx::Error> {
        sqlx::query_as::<_, GithubTokenPresence>(
            r#"SELECT scope, user_id, team_id, auth_method,
                      metadata->>'login' AS login,
                      (metadata->>'refresh_failure') IS NOT NULL AS refresh_failure
               FROM provider_tokens
               WHERE provider = 'github'
                 AND (   (scope = 'user'   AND user_id = $1)
                      OR (scope = 'global' AND user_id IS NULL AND team_id IS NULL))
               ORDER BY CASE scope WHEN 'user' THEN 0 ELSE 1 END
               LIMIT 1"#,
        )
        .bind(user_id)
        .fetch_optional(&self.0)
        .await
    }
}
