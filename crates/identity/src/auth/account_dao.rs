//! Atomic account persistence across identity-owned tables.

use super::error::AuthError;
use moatless_core::db::DbPool;
use moatless_types::user::UserRole;

/// A fully validated local account with a precomputed password hash.
pub(super) struct LocalAccount<'a> {
    pub user_id: uuid::Uuid,
    pub login: &'a str,
    pub email: &'a str,
    pub provider_user_id: &'a str,
    pub role: UserRole,
    pub password_hash: &'a str,
    pub must_change: bool,
}

/// Persist the account, provider identity and password in one transaction.
pub(super) async fn insert_local(db: &DbPool, account: LocalAccount<'_>) -> Result<(), AuthError> {
    let LocalAccount {
        user_id,
        login,
        email,
        provider_user_id,
        role,
        password_hash,
        must_change,
    } = account;
    let now = chrono::Utc::now();
    let mut tx = db.begin().await.map_err(AuthError::Database)?;

    let inserted_user = sqlx::query(
        r#"
        INSERT INTO users (id, login, email, name, avatar_url, provider, provider_user_id, role, created_at, last_login_at, is_bot)
        VALUES ($1, $2, $3, $4, NULL, 'local', $5, $6, $7, $7, false)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(login)
    .bind(email)
    .bind(login)
    .bind(provider_user_id)
    .bind(role.as_str())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    if inserted_user.rows_affected() == 0 {
        return Err(AuthError::Conflict(format!(
            "a user with email '{email}' already exists"
        )));
    }

    sqlx::query(
        r#"INSERT INTO user_provider_ids (user_id, provider, provider_user_id, created_at)
           VALUES ($1, 'local', $2, NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(user_id)
    .bind(provider_user_id)
    .execute(&mut *tx)
    .await?;

    let inserted = sqlx::query(
        r#"INSERT INTO user_credentials (user_id, username, password_hash, must_change)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(user_id)
    .bind(email)
    .bind(password_hash)
    .bind(must_change)
    .execute(&mut *tx)
    .await?;

    // A racing request won the unique index — surface 409, not a 500.
    if inserted.rows_affected() == 0 {
        return Err(AuthError::Conflict(format!(
            "a user with email '{email}' already exists"
        )));
    }

    tx.commit().await.map_err(AuthError::Database)?;

    Ok(())
}

/// Atomically refresh the bot account, provider identity and installation-token
/// row.
pub(super) async fn upsert_bot(
    db: &DbPool,
    app_key: &str,
    bot_identity: &super::github::service::BotIdentity,
) -> Result<uuid::Uuid, AuthError> {
    let now = chrono::Utc::now();
    // The App key is preserved as the bot's provider identity; the internal id
    // is a uuid, reused across re-runs so the endpoint stays idempotent.
    let provider_user_id = format!("bot:{app_key}");

    // Atomic: users + provider_tokens row succeed together. A failure of the
    // provider_tokens insert must not leave a half-bot row in `users` that
    // silently falls back to "user with no creds". Re-running the endpoint with
    // the same app key refreshes the bot's login/email from the GitHub App
    // metadata via the ON CONFLICT clauses.
    let mut tx = db.begin().await.map_err(AuthError::Database)?;

    let existing_bot: Option<(uuid::Uuid,)> =
        sqlx::query_as("SELECT id FROM users WHERE provider_user_id = $1")
            .bind(&provider_user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let user_id = existing_bot.map_or_else(uuid::Uuid::new_v4, |(id,)| id);
    let user_id_str = user_id.to_string();

    sqlx::query(
        r#"
        INSERT INTO users (id, login, email, name, avatar_url, provider, provider_user_id, role, created_at, last_login_at, is_bot)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (id) DO UPDATE SET
            login = EXCLUDED.login,
            email = COALESCE(users.email, EXCLUDED.email),
            name = EXCLUDED.name,
            avatar_url = EXCLUDED.avatar_url,
            provider_user_id = COALESCE(EXCLUDED.provider_user_id, users.provider_user_id),
            last_login_at = EXCLUDED.last_login_at,
            is_bot = EXCLUDED.is_bot
        "#,
    )
    .bind(user_id)
    .bind(&bot_identity.login)
    .bind(Some(&bot_identity.email))
    .bind(Some(&bot_identity.login))
    .bind::<Option<String>>(None)
    .bind("github_app")
    .bind(&provider_user_id)
    .bind(UserRole::User.as_str())
    .bind(now)
    .bind(now)
    .bind(true)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO user_provider_ids (user_id, provider, provider_user_id, created_at)
           VALUES ($1, 'github_app', $2, NOW())
           ON CONFLICT (user_id, provider) DO NOTHING"#,
    )
    .bind(user_id)
    .bind(&provider_user_id)
    .execute(&mut *tx)
    .await?;

    // Bot's provider_tokens row. `provider_user_id` carries the App key so
    // the canonical resolver can pick the right App when minting installation
    // tokens. `access_token` holds the documented sentinel — any code path that
    // bypasses the resolver and reads the column directly hits a clear 401
    // instead of silently sending an empty Bearer token.
    // team_id is NULL (migration 0097 dropped the team FK requirement).
    sqlx::query(
        r#"
        INSERT INTO provider_tokens (
            user_id, provider, auth_method, access_token, refresh_token,
            token_type, oauth_scope, provider_user_id, provider_login,
            connected_at, expires_at, scope, owner_user_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'user', $1)
        ON CONFLICT (user_id, provider, team_id) DO UPDATE SET
            auth_method = EXCLUDED.auth_method,
            access_token = EXCLUDED.access_token,
            refresh_token = EXCLUDED.refresh_token,
            token_type = EXCLUDED.token_type,
            oauth_scope = EXCLUDED.oauth_scope,
            scope = EXCLUDED.scope,
            owner_user_id = EXCLUDED.owner_user_id,
            provider_user_id = COALESCE(EXCLUDED.provider_user_id, provider_tokens.provider_user_id),
            provider_login = COALESCE(EXCLUDED.provider_login, provider_tokens.provider_login),
            connected_at = EXCLUDED.connected_at,
            expires_at = EXCLUDED.expires_at
        "#,
    )
    .bind(&user_id_str)
    .bind("github")
    .bind(super::provider_tokens::AUTH_METHOD_APP_INSTALLATION)
    .bind(super::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER)
    .bind::<Option<String>>(None) // refresh_token
    .bind::<Option<String>>(None) // token_type
    .bind::<Option<String>>(None) // oauth_scope
    .bind(Some(app_key))
    .bind(Some(&bot_identity.login))
    .bind(now)
    .bind::<Option<chrono::DateTime<chrono::Utc>>>(None)
    .execute(&mut *tx)
    .await?;

    tx.commit().await.map_err(AuthError::Database)?;

    Ok(user_id)
}
