//! Session service — create, validate, delete sessions.

use chrono::Utc;
use rand::RngExt;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use super::super::github::user_credential;
use super::super::models::{OAuthTokens, ProviderUserInfo};
use super::super::provider_tokens::ProviderTokenAuthDao;
use super::super::users::{User, UserDao};
use super::dao::{AuthenticatedSession, NewAuthSession, SessionDao};
use moatless_types::user::UserRole;

/// Generate a cryptographically secure session ID (64 hex chars).
fn generate_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}

/// Raw bearer and CSRF values returned exactly once when a session is issued.
pub(in crate::auth) struct IssuedAuthSession {
    pub token: String,
    pub csrf_token: String,
}

pub(in crate::auth) fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

async fn issue_auth_session(
    sessions: &dyn SessionDao,
    user_id: Uuid,
    group_id: Option<&str>,
    authenticated_via: &str,
    idle_timeout_secs: u64,
    absolute_lifetime_secs: u64,
) -> Result<IssuedAuthSession, crate::auth::error::AuthError> {
    let now = Utc::now();
    let idle_expires_at = now + chrono::Duration::seconds(idle_timeout_secs as i64);
    let absolute_expires_at = now + chrono::Duration::seconds(absolute_lifetime_secs as i64);
    let token = generate_token();
    let csrf_token = generate_token();
    sessions
        .insert_session(&NewAuthSession {
            id: Uuid::new_v4(),
            user_id,
            token_hash: hash_token(&token),
            csrf_token_hash: hash_token(&csrf_token),
            authenticated_via: authenticated_via.to_string(),
            group_id: group_id.map(String::from),
            created_at: now,
            last_seen_at: now,
            idle_expires_at: idle_expires_at.min(absolute_expires_at),
            absolute_expires_at,
        })
        .await?;
    Ok(IssuedAuthSession { token, csrf_token })
}

/// Create a new session after OAuth login.
///
/// Upserts the user, creates a session row (audit trail + provider token persistence),
/// and returns an opaque token for the auth-session cookie.
#[allow(clippy::too_many_arguments)]
pub(in crate::auth) async fn create_session(
    users: &dyn UserDao,
    sessions: &dyn SessionDao,
    provider_tokens: &dyn ProviderTokenAuthDao,
    secrets: &crate::secret::service::SecretService,
    user_id: Uuid,
    provider_id: &str,
    tokens: &OAuthTokens,
    user_info: &ProviderUserInfo,
    group_id: Option<&str>,
    idle_timeout_secs: u64,
    absolute_lifetime_secs: u64,
) -> Result<IssuedAuthSession, crate::auth::error::AuthError> {
    let now = Utc::now();
    let user_id_str = user_id.to_string();

    // GitHub App login uses the stable `github` identity key shared with
    // repository credentials.
    let provider_key = if provider_id == "github_app" {
        "github"
    } else {
        provider_id
    };

    // Upsert user (keyed on the internal uuid).
    let existing_user = users.get_user(&user_id).await?;
    if existing_user.is_none()
        && !user_info
            .email
            .as_deref()
            .is_some_and(|email| email.contains('@'))
    {
        return Err(crate::auth::error::AuthError::OAuth(
            "provider did not supply the email required for a human account".into(),
        ));
    }
    let role = existing_user.as_ref().map_or(UserRole::User, |u| u.role);

    let user_row = User {
        id: user_id,
        login: user_info.login.clone(),
        email: user_info.email.clone(),
        name: user_info.name.clone(),
        avatar_url: user_info.avatar_url.clone(),
        provider: provider_id.to_string(),
        provider_user_id: Some(user_info.id.clone()),
        role,
        created_at: existing_user.as_ref().map_or(now, |u| u.created_at),
        last_login_at: now,
        settings: existing_user
            .as_ref()
            .map_or_else(|| serde_json::json!({}), |u| u.settings.clone()),
        is_bot: existing_user.as_ref().is_some_and(|u| u.is_bot),
    };
    users.upsert_user(&user_row).await?;

    // Record the provider identity so future logins resolve by
    // (provider, provider_user_id) to this internal uuid.
    if let Err(e) = users
        .upsert_provider_id(&user_id, provider_key, &user_info.id)
        .await
    {
        warn!(error = %e, user_id = %user_id, "Failed to record provider identity");
    }

    let action = if existing_user.is_some() {
        "Updated"
    } else {
        "Created"
    };
    info!(
        user_login = %user_info.login,
        user_id = %user_id,
        provider_user_id = %user_info.id,
        provider = %provider_id,
        "{} user record",
        action,
    );

    // A credential the person can still fix from Settings is not worth failing
    // a login over, so this warns where `connect` — whose whole purpose is the
    // write — propagates.
    if provider_id == "github_app" {
        if let Err(e) =
            user_credential::persist(provider_tokens, secrets, &user_id_str, tokens, user_info)
                .await
        {
            warn!(
                error = %e,
                user_id = %user_id,
                "Failed to persist the GitHub App user token"
            );
        }
    }

    let token = issue_auth_session(
        sessions,
        user_id,
        group_id,
        provider_id,
        idle_timeout_secs,
        absolute_lifetime_secs,
    )
    .await?;

    info!(
        user_login = %user_info.login,
        provider = %provider_id,
        group_id = ?group_id,
        "Created session"
    );

    Ok(token)
}

/// Create a new session from a user row (for API key → session exchange).
///
/// Unlike `create_session`, this doesn't require OAuth tokens — it creates
/// a session with `provider = "api-key"` and no token fields.
/// Returns an opaque token for the auth-session cookie.
pub(in crate::auth) async fn create_session_from_user(
    sessions: &dyn SessionDao,
    user: &User,
    group_id: Option<&str>,
    idle_timeout_secs: u64,
    absolute_lifetime_secs: u64,
) -> Result<IssuedAuthSession, crate::auth::error::AuthError> {
    create_session_from_user_with_provider(
        sessions,
        user,
        group_id,
        idle_timeout_secs,
        absolute_lifetime_secs,
        "api-key",
    )
    .await
}

/// Create a session from a user row, stamping an explicit provider.
///
/// `provider` records how this auth session was established and flows through
/// to `AuthUser.provider`; it is not an application identity key.
pub(in crate::auth) async fn create_session_from_user_with_provider(
    sessions: &dyn SessionDao,
    user: &User,
    group_id: Option<&str>,
    idle_timeout_secs: u64,
    absolute_lifetime_secs: u64,
    provider: &str,
) -> Result<IssuedAuthSession, crate::auth::error::AuthError> {
    let token = issue_auth_session(
        sessions,
        user.id,
        group_id,
        provider,
        idle_timeout_secs,
        absolute_lifetime_secs,
    )
    .await?;

    info!(
        user_login = %user.login,
        user_id = %user.id,
        provider = %provider,
        "Created session from user row"
    );

    Ok(token)
}

/// Resolve an opaque auth-session token. Revoked, expired, disabled-account,
/// and unknown tokens all produce `None`.
pub(in crate::auth) async fn validate_session(
    sessions: &dyn SessionDao,
    token: &str,
    idle_timeout_secs: u64,
) -> Result<Option<AuthenticatedSession>, crate::auth::error::AuthError> {
    let now = Utc::now();
    let session = sessions
        .find_active_by_token_hash(&hash_token(token), now)
        .await?;
    let Some(mut session) = session else {
        return Ok(None);
    };

    if now - session.last_seen_at >= chrono::Duration::minutes(5) {
        let idle_expires_at = now + chrono::Duration::seconds(idle_timeout_secs as i64);
        sessions.touch(session.id, now, idle_expires_at).await?;
        session.last_seen_at = now;
        session.idle_expires_at = idle_expires_at.min(session.absolute_expires_at);
    }
    Ok(Some(session))
}

/// Revoke the auth session represented by a raw browser token.
pub(in crate::auth) async fn revoke_session(
    sessions: &dyn SessionDao,
    token: &str,
) -> Result<bool, crate::auth::error::AuthError> {
    Ok(sessions
        .revoke_by_token_hash(&hash_token(token), Utc::now())
        .await?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::Duration;

    use super::*;
    use crate::auth::sessions::dao::MockSessionDao;

    fn authenticated_session(last_seen_at: chrono::DateTime<Utc>) -> AuthenticatedSession {
        AuthenticatedSession {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            login: "alice".into(),
            role: UserRole::User,
            authenticated_via: "local".into(),
            csrf_token_hash: hash_token("csrf"),
            group_id: None,
            last_seen_at,
            idle_expires_at: Utc::now() + Duration::minutes(1),
            absolute_expires_at: Utc::now() + Duration::minutes(2),
        }
    }

    #[tokio::test]
    async fn issuance_persists_only_hashes_and_caps_the_idle_lifetime() {
        let hashes = Arc::new(std::sync::Mutex::new(None));
        let captured = hashes.clone();
        let user_id = Uuid::new_v4();
        let mut sessions = MockSessionDao::new();
        sessions
            .expect_insert_session()
            .once()
            .returning(move |row| {
                assert_eq!(row.user_id, user_id);
                assert_eq!(row.group_id.as_deref(), Some("group"));
                assert_eq!(row.authenticated_via, "local");
                assert_eq!(row.idle_expires_at, row.absolute_expires_at);
                assert_eq!(
                    row.absolute_expires_at - row.created_at,
                    Duration::seconds(60)
                );
                *captured.lock().unwrap() =
                    Some((row.token_hash.clone(), row.csrf_token_hash.clone()));
                Ok(())
            });
        let issued = issue_auth_session(&sessions, user_id, Some("group"), "local", 3600, 60)
            .await
            .unwrap();
        assert_eq!(issued.token.len(), 64);
        assert_eq!(issued.csrf_token.len(), 64);
        assert_ne!(issued.token, issued.csrf_token);
        assert_eq!(
            hashes.lock().unwrap().as_ref().unwrap(),
            &(hash_token(&issued.token), hash_token(&issued.csrf_token))
        );
    }

    #[tokio::test]
    async fn validation_refreshes_only_stale_sessions_and_caps_returned_expiry() {
        for stale in [false, true] {
            let now = Utc::now();
            let row = authenticated_session(now - Duration::minutes(if stale { 6 } else { 4 }));
            let expected = row.clone();
            let mut sessions = MockSessionDao::new();
            sessions
                .expect_find_active_by_token_hash()
                .withf(|hash, _| hash == hash_token("cookie"))
                .once()
                .returning(move |_, _| Ok(Some(row.clone())));
            if stale {
                let session_id = expected.id;
                sessions
                    .expect_touch()
                    .withf(move |id, at, expiry| {
                        *id == session_id && *expiry - *at == Duration::seconds(3600)
                    })
                    .once()
                    .returning(|_, _, _| Ok(()));
            } else {
                sessions.expect_touch().never();
            }
            let actual = validate_session(&sessions, "cookie", 3600)
                .await
                .unwrap()
                .unwrap();
            if stale {
                assert!(actual.last_seen_at >= now);
                assert_eq!(actual.idle_expires_at, expected.absolute_expires_at);
            } else {
                assert_eq!(actual.last_seen_at, expected.last_seen_at);
                assert_eq!(actual.idle_expires_at, expected.idle_expires_at);
            }
        }
    }

    #[tokio::test]
    async fn missing_session_does_not_touch_the_database() {
        let mut sessions = MockSessionDao::new();
        sessions
            .expect_find_active_by_token_hash()
            .once()
            .returning(|_, _| Ok(None));
        sessions.expect_touch().never();
        assert!(
            validate_session(&sessions, "unknown", 3600)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn database_errors_keep_their_cause_through_session_operations() {
        use crate::auth::error::AuthError;
        let mut sessions = MockSessionDao::new();
        sessions
            .expect_insert_session()
            .once()
            .returning(|_| Err(sqlx::Error::PoolClosed));
        sessions
            .expect_find_active_by_token_hash()
            .once()
            .returning(|_, _| Err(sqlx::Error::PoolClosed));
        sessions
            .expect_revoke_by_token_hash()
            .once()
            .returning(|_, _| Err(sqlx::Error::PoolClosed));
        assert!(matches!(
            issue_auth_session(&sessions, Uuid::new_v4(), None, "local", 60, 60).await,
            Err(AuthError::Database(sqlx::Error::PoolClosed))
        ));
        assert!(matches!(
            validate_session(&sessions, "cookie", 60).await,
            Err(AuthError::Database(sqlx::Error::PoolClosed))
        ));
        assert!(matches!(
            revoke_session(&sessions, "cookie").await,
            Err(AuthError::Database(sqlx::Error::PoolClosed))
        ));
    }

    #[tokio::test]
    async fn refresh_failure_does_not_return_an_authenticated_session() {
        let row = authenticated_session(Utc::now() - Duration::minutes(6));
        let mut sessions = MockSessionDao::new();
        sessions
            .expect_find_active_by_token_hash()
            .once()
            .returning(move |_, _| Ok(Some(row.clone())));
        sessions
            .expect_touch()
            .once()
            .returning(|_, _, _| Err(sqlx::Error::PoolClosed));
        assert!(matches!(
            validate_session(&sessions, "cookie", 60).await,
            Err(crate::auth::error::AuthError::Database(
                sqlx::Error::PoolClosed
            ))
        ));
    }

    #[tokio::test]
    async fn revoking_the_row_immediately_invalidates_the_cookie_token() {
        let revoked = Arc::new(AtomicBool::new(false));
        let session_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let now = Utc::now();

        let lookup_revoked = revoked.clone();
        let mut sessions = MockSessionDao::new();
        sessions
            .expect_find_active_by_token_hash()
            .times(2)
            .returning(move |_, _| {
                if lookup_revoked.load(Ordering::SeqCst) {
                    Ok(None)
                } else {
                    Ok(Some(AuthenticatedSession {
                        id: session_id,
                        user_id,
                        login: "alice".into(),
                        role: UserRole::User,
                        authenticated_via: "oidc:company".into(),
                        csrf_token_hash: hash_token("csrf"),
                        group_id: None,
                        last_seen_at: now,
                        idle_expires_at: now + Duration::hours(1),
                        absolute_expires_at: now + Duration::days(1),
                    }))
                }
            });
        let revoke_flag = revoked.clone();
        sessions
            .expect_revoke_by_token_hash()
            .withf(|hash, _| hash == hash_token("browser-cookie"))
            .once()
            .returning(move |_, _| {
                revoke_flag.store(true, Ordering::SeqCst);
                Ok(true)
            });

        assert!(
            validate_session(&sessions, "browser-cookie", 3600)
                .await
                .unwrap()
                .is_some()
        );
        assert!(revoke_session(&sessions, "browser-cookie").await.unwrap());
        assert!(
            validate_session(&sessions, "browser-cookie", 3600)
                .await
                .unwrap()
                .is_none()
        );
    }
}
