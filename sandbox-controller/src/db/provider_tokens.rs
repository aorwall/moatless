//! Provider tokens table queries.

use chrono::NaiveDateTime;
use super::DbPool;

#[derive(sqlx::FromRow, Debug)]
pub struct ProviderTokenRow {
    pub user_id: String,
    pub provider: String,
    pub auth_method: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub provider_user_id: Option<String>,
    pub provider_login: Option<String>,
    pub connected_at: NaiveDateTime,
    pub expires_at: Option<NaiveDateTime>,
}

/// Upsert a provider token (insert or update on conflict).
pub async fn upsert_provider_token(pool: &DbPool, token: &ProviderTokenRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO provider_tokens (
            user_id, provider, auth_method, access_token, refresh_token,
            token_type, scope, provider_user_id, provider_login,
            connected_at, expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (user_id, provider) DO UPDATE SET
            auth_method = EXCLUDED.auth_method,
            access_token = EXCLUDED.access_token,
            refresh_token = EXCLUDED.refresh_token,
            token_type = EXCLUDED.token_type,
            scope = EXCLUDED.scope,
            provider_user_id = EXCLUDED.provider_user_id,
            provider_login = EXCLUDED.provider_login,
            connected_at = EXCLUDED.connected_at,
            expires_at = EXCLUDED.expires_at
        "#,
    )
    .bind(&token.user_id)
    .bind(&token.provider)
    .bind(&token.auth_method)
    .bind(&token.access_token)
    .bind(&token.refresh_token)
    .bind(&token.token_type)
    .bind(&token.scope)
    .bind(&token.provider_user_id)
    .bind(&token.provider_login)
    .bind(token.connected_at)
    .bind(token.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}
