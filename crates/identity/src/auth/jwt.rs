//! Session JWT generation and verification.
//!
//! Distinct from `sandbox/jwt.rs` which handles sandbox task tokens.
//! Uses HS256 signing — same algorithm, separate claims structure.

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;
use moatless_core::config::Config;

/// Issuer stamped on session JWTs. Bumped at the internal-uuid cutover so cookies
/// minted before it (provider-id `sub`, `personal-<provider-id>` teams) fail
/// verification and force a fresh login.
const SESSION_JWT_ISSUER: &str = "moatless-vibe-backend/v2";

/// Issuer stamped on WebSocket tickets.
///
/// Different from [`SESSION_JWT_ISSUER`] and checked on both sides, so the two
/// cannot stand in for one another: a ticket is not a session cookie, and a
/// session cookie copied into a query string is not a ticket.
const WS_TICKET_JWT_ISSUER: &str = "moatless-vibe-backend/ws-ticket";

/// How long a WebSocket ticket can be presented for.
///
/// A ticket travels in a query string, which is the part of a URL that ends up
/// in access logs and proxy traces. It is therefore issued to be spent
/// immediately — the client asks for one and connects with it in the same
/// breath — and this is the width of the window in which a copy of one taken
/// out of a log is still worth anything.
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
    /// Legacy grouping id, threaded through `AuthIdentity` into
    /// [`crate::extractors::AuthUser`] and `t3code::auth::AuthContext`. No
    /// code branches on its value; kept only so already-issued session
    /// cookies keep deserializing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
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

/// Generate a session JWT.
pub fn generate_session_jwt(
    user_id: Uuid,
    login: &str,
    role: UserRole,
    provider: &str,
    group_id: Option<&str>,
    secret: &str,
    max_age_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let claims = SessionClaims {
        sub: user_id,
        login: login.to_string(),
        role: role.as_str().to_string(),
        provider: provider.to_string(),
        group_id: group_id.map(String::from),
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
    validation.set_issuer(&[SESSION_JWT_ISSUER]);

    let data = decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    Ok(data.claims)
}

/// Mint a WebSocket ticket for an already-authenticated identity.
///
/// A WebSocket upgrade is opened by the browser `WebSocket` constructor, which
/// takes a URL and nothing else — no headers, so neither an `Authorization`
/// header nor anything else a client would normally authenticate with can be
/// attached to it. A client that has no cookie (a native app, say) therefore
/// has one way left to say who it is, and it is the URL. This is the credential
/// that goes there: minted from a request that *did* carry a real credential,
/// good for [`WS_TICKET_TTL_SECS`], and good for nothing but the upgrade.
pub fn generate_ws_ticket_jwt(
    identity: &AuthIdentity,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let claims = SessionClaims {
        sub: identity.user_id,
        login: identity.login.clone(),
        role: identity.role.as_str().to_string(),
        provider: identity.provider.clone(),
        group_id: identity.group_id.clone(),
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

/// Resolve the JWT secret from config.
///
/// Prefers `SESSION_JWT_SECRET`, falling back to `TASK_TOKEN_SECRET` for the
/// short-lived JWTs still used outside browser-session authentication.
pub fn get_jwt_secret(config: &Config) -> Option<&str> {
    config
        .auth
        .session_jwt_secret
        .as_deref()
        .or(config.sandbox.task_token_secret.as_deref())
}

/// Warn once at startup when session cookies are signed with the sandbox
/// secret because `SESSION_JWT_SECRET` is unset.
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
        let token = generate_session_jwt(
            uid,
            "alice",
            UserRole::Admin,
            "github",
            Some("group-1"),
            "test-secret",
            3600,
        )
        .unwrap();

        assert_eq!(token.split('.').count(), 3);

        let claims = verify_session_jwt(&token, "test-secret").unwrap();
        assert_eq!(claims.sub, uid);
        assert_eq!(claims.login, "alice");
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.provider, "github");
        assert_eq!(claims.group_id.as_deref(), Some("group-1"));
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
            None,
            "test-secret",
            3600,
        )
        .unwrap();

        assert!(verify_session_jwt(&token, "wrong-secret").is_err());
    }

    #[test]
    fn expired_token_fails() {
        // Manually create a token with exp in the past (beyond default leeway)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = SessionClaims {
            sub: test_user_id(),
            login: "alice".to_string(),
            role: "user".to_string(),
            provider: "github".to_string(),
            group_id: None,
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
    fn no_group_id_omitted_in_json() {
        let token = generate_session_jwt(
            test_user_id(),
            "alice",
            UserRole::User,
            "github",
            None,
            "test-secret",
            3600,
        )
        .unwrap();

        let claims = verify_session_jwt(&token, "test-secret").unwrap();
        assert!(claims.group_id.is_none());
    }

    fn test_identity() -> AuthIdentity {
        AuthIdentity {
            user_id: test_user_id(),
            login: "alice".to_string(),
            role: UserRole::Admin,
            provider: "api-key".to_string(),
            group_id: Some("group-1".to_string()),
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
        assert_eq!(claims.group_id.as_deref(), Some("group-1"));
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
            None,
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
