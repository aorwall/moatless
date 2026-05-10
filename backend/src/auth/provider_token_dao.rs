//! Provider token DAO — queries for credential resolution.

use async_trait::async_trait;

use super::models::{ProviderAccessToken, ProviderToken};
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

    /// Upsert a team-level provider token.
    async fn upsert_team_token(
        &self,
        team_id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error>;

    /// Delete a team-level provider token.
    async fn delete_team_token(&self, team_id: &str, provider: &str) -> Result<(), sqlx::Error>;

    /// List all provider tokens for a team.
    async fn list_by_team(&self, team_id: &str) -> Result<Vec<ProviderToken>, sqlx::Error>;
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
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO provider_tokens (
                   team_id, provider, access_token, refresh_token, token_type,
                   metadata, updated_at, scope, owner_user_id
               )
               VALUES ($1, $2, $3, $4, $5, $6, NOW(), 'team', NULL)
               ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq
               DO UPDATE SET access_token = $3, refresh_token = $4, token_type = $5, metadata = $6,
                             scope = 'team', owner_user_id = NULL, updated_at = NOW()"#,
        )
        .bind(team_id)
        .bind(provider)
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
                      token_type, auth_method, expires_at, metadata, created_at, updated_at
               FROM provider_tokens
               WHERE team_id = $1 AND user_id IS NULL"#,
        )
        .bind(team_id)
        .fetch_all(&self.0)
        .await
    }
}
