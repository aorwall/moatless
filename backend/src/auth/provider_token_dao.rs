//! Provider token DAO — queries for credential resolution.

use async_trait::async_trait;
use sqlx::AssertSqlSafe;

use super::models::{ProviderAccessToken, ProviderToken};
use crate::credentials::codex::scope::CodexScope;
use crate::db::DbPool;

pub(crate) async fn ensure_personal_team(
    pool: &DbPool,
    user_id: &str,
) -> Result<String, sqlx::Error> {
    let team_id = format!("personal-{user_id}");
    let team_name = format!("{user_id} Personal");
    let team_slug = personal_team_slug(user_id);

    sqlx::query(
        r#"INSERT INTO users (id, login, provider, role, created_at, last_login_at, settings)
           VALUES ($1, $1, 'local', 'user', NOW(), NOW(), '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"INSERT INTO teams (id, name, slug, description, is_default, settings, created_at, updated_at)
           VALUES ($1, $2, $3, 'Personal workspace', false, '{}'::jsonb, NOW(), NOW())
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(&team_id)
    .bind(team_name)
    .bind(team_slug)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"INSERT INTO team_members (team_id, user_id, role, created_at)
           VALUES ($1, $2, 'owner', NOW())
           ON CONFLICT (team_id, user_id) DO UPDATE SET role = EXCLUDED.role"#,
    )
    .bind(&team_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(team_id)
}

fn personal_team_slug(user_id: &str) -> String {
    let mut slug = String::with_capacity(user_id.len() + "personal-".len());
    slug.push_str("personal-");
    for ch in user_id.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// Repository trait for provider token credential resolution.
///
/// Note: `automock` is intentionally omitted — mockall cannot synthesise
/// correct lifetimes for `Option<&str>` / `Option<&Value>` in async-trait
/// methods.  Tests use `PgProviderTokenRepository` via `#[sqlx::test]`.
// reason: provider-token upserts already pass 7+ atomic columns; wrapping
// into a struct would require touching every call-site and impl for no real
// readability gain.
#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait ProviderTokenRepository: Send + Sync {
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

    /// Upsert a provider token (insert or update on conflict).
    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Delete a provider token for a user.
    async fn delete_token(&self, user_id: &str, provider: &str) -> Result<(), sqlx::Error>;

    // ---- Team-level credential methods ----

    /// Find a team-level provider token (access_token only).
    async fn find_team_token(
        &self,
        team_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error>;

    /// Upsert a team-level provider token. `auth_method` is typically
    /// `"manual"` for admin-supplied PATs and `"oauth"` for workspace OAuth.
    async fn upsert_team_token(
        &self,
        team_id: &str,
        provider: &str,
        auth_method: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Delete a team-level provider token.
    async fn delete_team_token(&self, team_id: &str, provider: &str) -> Result<(), sqlx::Error>;

    /// List all provider tokens for a team.
    async fn list_by_team(&self, team_id: &str) -> Result<Vec<ProviderToken>, sqlx::Error>;

    // ---- Scope-aware codex methods ----

    /// Find the codex `provider_tokens` row addressed by `scope`. Returns the
    /// full row (including `metadata`) so callers can build a
    /// `CodexConfigResponse` without a follow-up read.
    async fn find_codex_blob(
        &self,
        scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error>;

    /// Upsert a codex blob at the requested scope. The unified shape replaces
    /// the user / team / global triplet so the user / team / admin endpoints
    /// share one persistence path.
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
}

/// PostgreSQL implementation of [`ProviderTokenRepository`].
pub struct PgProviderTokenRepository(pub DbPool);

#[async_trait]
impl ProviderTokenRepository for PgProviderTokenRepository {
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
        let team_id = ensure_personal_team(&self.0, user_id).await?;
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   user_id, provider, access_token, refresh_token, token_type, metadata,
                   auth_method, connected_at, updated_at, scope, team_id, owner_user_id
               )
               VALUES ($1, $2, $3, $4, $5, $6, 'manual', NOW(), NOW(), 'user', $7, $1)
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
        .bind(team_id)
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

    async fn find_team_token(
        &self,
        team_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderAccessToken>(
            "SELECT access_token FROM provider_tokens WHERE team_id = $1 AND provider = $2 AND user_id IS NULL",
        )
        .bind(team_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert_team_token(
        &self,
        team_id: &str,
        provider: &str,
        auth_method: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   team_id, provider, auth_method, access_token, refresh_token, token_type,
                   metadata, connected_at, updated_at, scope, owner_user_id
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW(), 'team', NULL)
               ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
               DO UPDATE SET access_token = $4, refresh_token = $5, token_type = $6, metadata = $7,
                             auth_method = $3, scope = 'team', owner_user_id = NULL, updated_at = NOW()"#,
        )
        .bind(team_id)
        .bind(provider)
        .bind(auth_method)
        .bind(access_token)
        .bind(refresh_token)
        .bind(token_type)
        .bind(metadata)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete_team_token(&self, team_id: &str, provider: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "DELETE FROM provider_tokens WHERE team_id = $1 AND provider = $2 AND user_id IS NULL",
        )
        .bind(team_id)
        .bind(provider)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list_by_team(&self, team_id: &str) -> Result<Vec<ProviderToken>, sqlx::Error> {
        sqlx::query_as::<_, ProviderToken>(
            r#"SELECT COALESCE(user_id, '') AS user_id, provider, access_token, refresh_token,
                      token_type, auth_method, expires_at, metadata, created_at, updated_at,
                      provider_user_id, provider_login
               FROM provider_tokens
               WHERE team_id = $1 AND user_id IS NULL"#,
        )
        .bind(team_id)
        .fetch_all(&self.0)
        .await
    }

    async fn find_codex_blob(
        &self,
        scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        let where_clause = scope.where_clause("$1");
        // `ProviderToken::user_id` is `NOT NULL` in the Rust struct so
        // coalesce NULL to '' for team / global rows.
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
                // User scope already has a scaffolded personal team via
                // `upsert_token`; delegate so the personal-team ensure path
                // stays in one place.
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
            CodexScope::Team(team_id) => {
                // 'manual' is the auth_method for admin-supplied codex blobs.
                self.upsert_team_token(
                    team_id,
                    "codex",
                    "manual",
                    access_token,
                    refresh_token,
                    token_type,
                    metadata,
                )
                .await
            }
            CodexScope::Global => {
                // Global codex row: pinned to the reserved `'system'` team.
                // `provider_tokens_scope_uq UNIQUE NULLS NOT DISTINCT
                // (user_id, provider, team_id)` covers this row uniquely.
                sqlx::query(
                    r#"INSERT INTO provider_tokens (
                           team_id, provider, auth_method, access_token, refresh_token,
                           token_type, metadata, connected_at, updated_at, scope,
                           owner_user_id, user_id
                       )
                       VALUES ($1, 'codex', 'manual', $2, $3, $4, $5, NOW(), NOW(),
                               'global', NULL, NULL)
                       ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
                       DO UPDATE SET access_token = $2, refresh_token = $3,
                                     token_type = $4, metadata = $5,
                                     auth_method = 'manual', scope = 'global',
                                     owner_user_id = NULL, updated_at = NOW()"#,
                )
                .bind(crate::credentials::codex::scope::SYSTEM_TEAM_ID)
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
}
