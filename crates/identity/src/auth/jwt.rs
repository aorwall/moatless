//! Session JWT generation and verification.
//!
//! Distinct from `sandbox/jwt.rs` which handles sandbox task tokens. Uses HS256
//! signing — same algorithm, separate claims structure.

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use moatless_core::config::Config;
use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;

/// Issuer stamped on session JWTs. Bumped at the internal-uuid cutover so
/// cookies minted before it (provider-id `sub`) fail verification and force a fresh login.
const SESSION_JWT_ISSUER: &str = "moatless-backend/v2";

/// Session issuer before the rename to [`SESSION_JWT_ISSUER`]. Still accepted,
/// so the rename signs nobody out.
const LEGACY_SESSION_JWT_ISSUER: &str = "moatless-vibe-backend/v2";

/// Issuer stamped on WebSocket tickets.
///
/// Different from [`SESSION_JWT_ISSUER`] and checked on both sides, so a ticket
/// can't stand in for a session cookie, or vice versa, even if one lands in the
/// other's slot.
const WS_TICKET_JWT_ISSUER: &str = "moatless-backend/ws-ticket";

/// How long a WebSocket ticket can be presented for.
///
/// A ticket travels in a query string, which ends up in access logs and proxy
/// traces; issued to be spent immediately, this is the window in which a copy
/// taken from a log is still worth anything.
pub const WS_TICKET_TTL_SECS: u64 = 60;

/// JWT claims embedded in the session cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    /// Internal user uuid (serialized as a string in the token).
    pub sub: Uuid,
    pub login: String,
    /// User role: "user" or "admin".
    pub role: String,
    /// Auth provider: "github", "auth0", or "api-key".
    pub provider: String,
    /// Issued at, Unix seconds.
    pub iat: u64,
    /// Expiration, Unix seconds.
    pub exp: u64,
    pub iss: String,
}

impl SessionClaims {
    /// Parse the `role` claim into a `UserRole`.
    pub fn user_role(&self) -> UserRole {
        UserRole::try_from(self.role.as_str()).unwrap_or(UserRole::User)
    }
}

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

/// Generate a session JWT.
pub fn generate_session_jwt(
    user_id: Uuid,
    login: &str,
    role: UserRole,
    provider: &str,
    secret: &str,
    max_age_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = now_secs();

    let claims = SessionClaims {
        sub: user_id,
        login: login.to_string(),
        role: role.as_str().to_string(),
        provider: provider.to_string(),
        iat: now,
        exp: now + max_age_secs,
        iss: SESSION_JWT_ISSUER.to_string(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    debug!(user_id = %user_id, "Generated session JWT");
    Ok(token)
}

/// Verify a session JWT and return its claims.
pub fn verify_session_jwt(
    token: &str,
    secret: &str,
) -> Result<SessionClaims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::default();
    validation.set_issuer(&[SESSION_JWT_ISSUER, LEGACY_SESSION_JWT_ISSUER]);

    let data = decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    Ok(data.claims)
}

/// Mints a WebSocket ticket for an already-authenticated identity.
///
/// A `WebSocket` upgrade carries no headers, only a URL, so a client with no
/// cookie has only the URL left to say who it is; this ticket is what goes
/// there — minted from a request with a real credential, good for
/// [`WS_TICKET_TTL_SECS`], and good for nothing but the upgrade.
pub fn generate_ws_ticket_jwt(
    identity: &AuthIdentity,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = now_secs();

    let claims = SessionClaims {
        sub: identity.user_id,
        login: identity.login.clone(),
        role: identity.role.as_str().to_string(),
        provider: identity.provider.clone(),
        iat: now,
        exp: now + WS_TICKET_TTL_SECS,
        iss: WS_TICKET_JWT_ISSUER.to_string(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// Verify a WebSocket ticket and return its claims.
pub fn verify_ws_ticket_jwt(
    token: &str,
    secret: &str,
) -> Result<SessionClaims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::default();
    validation.set_issuer(&[WS_TICKET_JWT_ISSUER]);

    let data = decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    Ok(data.claims)
}

/// Resolves the JWT secret from config.
///
/// Prefers `SESSION_JWT_SECRET`, falling back to `TASK_TOKEN_SECRET` for
/// short-lived JWTs still used outside browser-session authentication.
pub fn get_jwt_secret(config: &Config) -> Option<&str> {
    config
        .auth
        .session_jwt_secret
        .as_deref()
        .or(config.sandbox.task_token_secret.as_deref())
}

/// Warn once at startup when session cookies are signed with the sandbox secret
/// because `SESSION_JWT_SECRET` is unset.
pub fn warn_on_jwt_secret_fallback(config: &Config) {
    if config.auth.session_jwt_secret.is_none() && config.sandbox.task_token_secret.is_some() {
        tracing::warn!(
            "SESSION_JWT_SECRET is unset — signing short-lived authentication tickets with \
             TASK_TOKEN_SECRET. Set SESSION_JWT_SECRET to decouple those keys."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_user_id() -> Uuid {
        Uuid::from_u128(0x1234_5678_90ab_cdef)
    }

    #[test]
    fn round_trip() {
        let uid = test_user_id();
        let token =
            generate_session_jwt(uid, "alice", UserRole::Admin, "github", "test-secret", 3600)
                .unwrap();

        assert_eq!(token.split('.').count(), 3);

        let claims = verify_session_jwt(&token, "test-secret").unwrap();
        assert_eq!(claims.sub, uid);
        assert_eq!(claims.login, "alice");
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.provider, "github");
        assert_eq!(claims.iss, SESSION_JWT_ISSUER);
        assert_eq!(claims.user_role(), UserRole::Admin);
    }

    #[test]
    fn wrong_secret_fails() {
        let token = generate_session_jwt(
            test_user_id(),
            "alice",
            UserRole::User,
            "github",
            "test-secret",
            3600,
        )
        .unwrap();

        assert!(verify_session_jwt(&token, "wrong-secret").is_err());
    }

    #[test]
    fn expired_token_fails() {
        // Manually create a token with exp in the past (beyond default leeway)
        let now = now_secs();

        let claims = SessionClaims {
            sub: test_user_id(),
            login: "alice".to_string(),
            role: "user".to_string(),
            provider: "github".to_string(),
            iat: now - 7200,
            exp: now - 3600, // expired 1 hour ago (well past default 60s leeway)
            iss: SESSION_JWT_ISSUER.to_string(),
        };

        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"test-secret"),
        )
        .unwrap();

        assert!(verify_session_jwt(&token, "test-secret").is_err());
    }

    #[test]
    fn a_cookie_issued_with_a_group_id_still_verifies() {
        let now = now_secs();
        let claims = serde_json::json!({
            "sub": test_user_id(),
            "login": "alice",
            "role": "user",
            "provider": "github",
            "group_id": "group-1",
            "iat": now,
            "exp": now + 3600,
            "iss": SESSION_JWT_ISSUER,
        });
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"test-secret"),
        )
        .unwrap();

        let claims = verify_session_jwt(&token, "test-secret").unwrap();
        assert_eq!(claims.login, "alice");
    }

    fn test_identity() -> AuthIdentity {
        AuthIdentity {
            user_id: test_user_id(),
            login: "alice".to_string(),
            role: UserRole::Admin,
            provider: "api-key".to_string(),
        }
    }

    #[test]
    fn a_ticket_carries_the_identity_that_asked_for_it() {
        let token = generate_ws_ticket_jwt(&test_identity(), "test-secret").unwrap();

        let claims = verify_ws_ticket_jwt(&token, "test-secret").unwrap();
        assert_eq!(claims.sub, test_user_id());
        assert_eq!(claims.login, "alice");
        assert_eq!(claims.user_role(), UserRole::Admin);
        assert_eq!(claims.provider, "api-key");
        assert_eq!(claims.exp - claims.iat, WS_TICKET_TTL_SECS);
    }

    /// The whole point of the separate issuer: a ticket is not a session, and a
    /// session is not a ticket, even though both are signed with one secret.
    #[test]
    fn a_ticket_and_a_session_cookie_are_not_interchangeable() {
        let ticket = generate_ws_ticket_jwt(&test_identity(), "test-secret").unwrap();
        assert!(verify_session_jwt(&ticket, "test-secret").is_err());

        let session = generate_session_jwt(
            test_user_id(),
            "alice",
            UserRole::Admin,
            "github",
            "test-secret",
            3600,
        )
        .unwrap();
        assert!(verify_ws_ticket_jwt(&session, "test-secret").is_err());
    }

    #[test]
    fn a_ticket_signed_with_another_secret_fails() {
        let token = generate_ws_ticket_jwt(&test_identity(), "test-secret").unwrap();
        assert!(verify_ws_ticket_jwt(&token, "wrong-secret").is_err());
    }
}
