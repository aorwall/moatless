//! Session service — create, validate, delete sessions.

use chrono::Utc;
use rand::Rng;
use tracing::{error, info, warn};

use crate::auth::types::{OAuthTokens, ProviderUserInfo};
use crate::db::sessions::{self, SessionRow};
use crate::db::users::{self, UserRow};
use crate::db::provider_tokens::{self, ProviderTokenRow};
use crate::db::DbPool;

/// Generate a cryptographically secure session ID (64 hex chars).
fn generate_session_id() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}

/// Create a new session after OAuth login.
///
/// Upserts the user, creates a session row, and optionally persists the
/// provider token (for GitHub OAuth → provider_tokens cross-context access).
pub async fn create_session(
    pool: &DbPool,
    provider_id: &str,
    tokens: &OAuthTokens,
    user_info: &ProviderUserInfo,
    group_id: Option<&str>,
    session_max_age_secs: u64,
) -> Result<String, crate::auth::error::AuthError> {
    let now = Utc::now().naive_utc();
    let expires_at = now + chrono::Duration::seconds(session_max_age_secs as i64);

    // Upsert user
    let existing_user = users::get_user(pool, &user_info.id).await?;
    let role = existing_user
        .as_ref()
        .map(|u| u.role.clone())
        .unwrap_or_else(|| "user".to_string());

    let user_row = UserRow {
        id: user_info.id.clone(),
        login: user_info.login.clone(),
        email: user_info.email.clone(),
        name: user_info.name.clone(),
        avatar_url: user_info.avatar_url.clone(),
        provider: provider_id.to_string(),
        role: role.clone(),
        created_at: existing_user
            .as_ref()
            .map(|u| u.created_at)
            .unwrap_or(now),
        last_login_at: now,
    };
    users::upsert_user(pool, &user_row).await?;

    let action = if existing_user.is_some() {
        "Updated"
    } else {
        "Created"
    };
    info!(
        user_login = %user_info.login,
        user_id = %user_info.id,
        provider = %provider_id,
        "{} user record",
        action,
    );

    // Create session
    let session_id = generate_session_id();
    let session = SessionRow {
        id: session_id.clone(),
        provider: provider_id.to_string(),
        user_id: user_info.id.clone(),
        user_login: user_info.login.clone(),
        user_email: user_info.email.clone(),
        user_name: user_info.name.clone(),
        avatar_url: user_info.avatar_url.clone(),
        role,
        access_token: Some(tokens.access_token.clone()),
        id_token: tokens.id_token.clone(),
        token_type: Some(tokens.token_type.clone()),
        scope: tokens.scope.clone(),
        group_id: group_id.map(String::from),
        created_at: now,
        expires_at,
    };
    sessions::insert_session(pool, &session).await?;

    // For GitHub OAuth, also persist to provider_tokens for cross-context access
    if provider_id == "github" && !tokens.access_token.is_empty() {
        let pt = ProviderTokenRow {
            user_id: user_info.id.clone(),
            provider: "github".to_string(),
            auth_method: "oauth".to_string(),
            access_token: tokens.access_token.clone(),
            refresh_token: None,
            token_type: Some(tokens.token_type.clone()),
            scope: tokens.scope.clone(),
            provider_user_id: Some(user_info.id.clone()),
            provider_login: Some(user_info.login.clone()),
            connected_at: now,
            expires_at: None,
        };
        if let Err(e) = provider_tokens::upsert_provider_token(pool, &pt).await {
            warn!(
                error = %e,
                user_id = %user_info.id,
                "Failed to persist GitHub OAuth token to provider_tokens"
            );
        } else {
            info!(
                user_id = %user_info.id,
                login = %user_info.login,
                "Persisted GitHub OAuth token to provider_tokens"
            );
        }
    }

    info!(
        user_login = %user_info.login,
        provider = %provider_id,
        group_id = ?group_id,
        "Created session"
    );

    Ok(session_id)
}

/// Validate a session by ID. Returns None if expired or not found.
pub async fn validate_session(
    pool: &DbPool,
    session_id: &str,
) -> Result<Option<SessionRow>, crate::auth::error::AuthError> {
    let session = sessions::get_session(pool, session_id).await?;

    let Some(session) = session else {
        return Ok(None);
    };

    // Check expiry
    let now = Utc::now().naive_utc();
    if now > session.expires_at {
        info!(session_id = %session_id, "Session expired");
        if let Err(e) = sessions::delete_session(pool, session_id).await {
            error!(error = %e, "Failed to delete expired session");
        }
        return Ok(None);
    }

    Ok(Some(session))
}

/// Delete a session by ID.
pub async fn delete_session_by_id(
    pool: &DbPool,
    session_id: &str,
) -> Result<(), crate::auth::error::AuthError> {
    sessions::delete_session(pool, session_id).await?;
    info!(session_id = %session_id, "Deleted session");
    Ok(())
}
