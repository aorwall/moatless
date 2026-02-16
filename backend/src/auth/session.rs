//! Session service — create, validate, delete sessions.

use chrono::Utc;
use rand::Rng;
use tracing::{error, info, warn};

use super::models::{OAuthTokens, ProviderUserInfo, UserRole};
use super::provider_tokens::{ProviderToken, ProviderTokenAuthRepository};
use super::sessions::{Session, SessionRepository};
use super::users::{User, UserRepository};

/// Generate a cryptographically secure session ID (64 hex chars).
fn generate_session_id() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}

/// Create a new session after OAuth login.
///
/// Upserts the user, creates a session row, and optionally persists the
/// provider token (for GitHub OAuth → provider_tokens cross-context access).
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    users: &dyn UserRepository,
    sessions: &dyn SessionRepository,
    provider_tokens: &dyn ProviderTokenAuthRepository,
    provider_id: &str,
    tokens: &OAuthTokens,
    user_info: &ProviderUserInfo,
    group_id: Option<&str>,
    session_max_age_secs: u64,
) -> Result<String, crate::auth::error::AuthError> {
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(session_max_age_secs as i64);

    // Upsert user
    let existing_user = users.get_user(&user_info.id).await?;
    let role = existing_user
        .as_ref().map_or(UserRole::User, |u| u.role);

    let user_row = User {
        id: user_info.id.clone(),
        login: user_info.login.clone(),
        email: user_info.email.clone(),
        name: user_info.name.clone(),
        avatar_url: user_info.avatar_url.clone(),
        provider: provider_id.to_string(),
        role,
        created_at: existing_user
            .as_ref()
            .map_or(now, |u| u.created_at),
        last_login_at: now,
    };
    users.upsert_user(&user_row).await?;

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
    let session = Session {
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
    sessions.insert_session(&session).await?;

    // For GitHub OAuth, also persist to provider_tokens for cross-context access
    if provider_id == "github" && !tokens.access_token.is_empty() {
        let pt = ProviderToken {
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
        if let Err(e) = provider_tokens.upsert_provider_token(&pt).await {
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

/// Create a new session from a user row (for API key → session exchange).
///
/// Unlike `create_session`, this doesn't require OAuth tokens — it creates
/// a session with `provider = "api-key"` and no token fields.
pub async fn create_session_from_user(
    sessions: &dyn SessionRepository,
    user: &User,
    group_id: Option<&str>,
    session_max_age_secs: u64,
) -> Result<String, crate::auth::error::AuthError> {
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(session_max_age_secs as i64);
    let session_id = generate_session_id();

    let session = Session {
        id: session_id.clone(),
        provider: "api-key".to_string(),
        user_id: user.id.clone(),
        user_login: user.login.clone(),
        user_email: user.email.clone(),
        user_name: user.name.clone(),
        avatar_url: user.avatar_url.clone(),
        role: user.role,
        access_token: None,
        id_token: None,
        token_type: None,
        scope: None,
        group_id: group_id.map(String::from),
        created_at: now,
        expires_at,
    };
    sessions.insert_session(&session).await?;

    info!(
        user_login = %user.login,
        user_id = %user.id,
        "Created session from API key"
    );

    Ok(session_id)
}

/// Validate a session by ID. Returns None if expired or not found.
pub async fn validate_session(
    sessions: &dyn SessionRepository,
    session_id: &str,
) -> Result<Option<Session>, crate::auth::error::AuthError> {
    let session = sessions.get_session(session_id).await?;

    let Some(session) = session else {
        return Ok(None);
    };

    // Check expiry
    let now = Utc::now();
    if now > session.expires_at {
        info!(session_id = %session_id, "Session expired");
        if let Err(e) = sessions.delete_session(session_id).await {
            error!(error = %e, "Failed to delete expired session");
        }
        return Ok(None);
    }

    Ok(Some(session))
}

/// Validate a session with cache-aside. Used by `verify_handler` only.
///
/// 1. Check cache → return if hit (re-check expiry on cached `expires_at`)
/// 2. Miss → call `validate_session()` → build `VerifyPayload` → insert cache → return
/// 3. If cached entry is expired → invalidate + fire-and-forget DB delete
pub async fn validate_session_cached(
    sessions: &dyn SessionRepository,
    session_id: &str,
    cache: &crate::auth::cache::AuthCache,
) -> Result<Option<crate::auth::cache::VerifyPayload>, crate::auth::error::AuthError> {
    // Check cache first
    if let Some(cached) = cache.get_session(session_id).await {
        let now = Utc::now();
        if now > cached.expires_at {
            // Cached entry is expired — evict and delete from DB
            cache.invalidate_session(session_id).await;
            if let Err(e) = sessions.delete_session(session_id).await {
                error!(error = %e, "Failed to delete expired session (from cache)");
            }
            return Ok(None);
        }
        return Ok(Some(cached));
    }

    // Cache miss — go to DB
    let session = validate_session(sessions, session_id).await?;
    let Some(session) = session else {
        return Ok(None);
    };

    let payload = crate::auth::cache::VerifyPayload {
        user_id: session.user_id,
        user_login: session.user_login,
        role: session.role,
        provider: session.provider,
        group_id: session.group_id,
        expires_at: session.expires_at,
    };


    cache.insert_session(session_id, payload.clone()).await;
    Ok(Some(payload))
}

/// Delete a session by ID.
pub async fn delete_session_by_id(
    sessions: &dyn SessionRepository,
    session_id: &str,
) -> Result<(), crate::auth::error::AuthError> {
    sessions.delete_session(session_id).await?;
    info!(session_id = %session_id, "Deleted session");
    Ok(())
}
