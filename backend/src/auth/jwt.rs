//! Session JWT generation and verification.
//!
//! Distinct from `sandbox/jwt.rs` which handles sandbox task tokens.
//! Uses HS256 signing — same algorithm, separate claims structure.

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use super::models::UserRole;
use crate::config::Config;

/// Issuer stamped on session JWTs. Bumped at the internal-uuid cutover so cookies
/// minted before it (provider-id `sub`, `personal-<provider-id>` teams) fail
/// verification and force a fresh login.
const SESSION_JWT_ISSUER: &str = "moatless-vibe-backend/v2";

/// JWT claims embedded in the session cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    /// Internal user uuid (serialized as a string in the token).
    pub sub: Uuid,
    /// User login name
    pub login: String,
    /// User role ("user" or "admin")
    pub role: String,
    /// Auth provider ("github", "auth0", "api-key")
    pub provider: String,
    /// Optional group ID (deprecated, kept for backwards compat)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// Issued at (unix timestamp)
    pub iat: u64,
    /// Expiration (unix timestamp)
    pub exp: u64,
    /// Issuer
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

/// Resolve the JWT secret from config.
///
/// Prefers `SESSION_JWT_SECRET`, falls back to `TASK_TOKEN_SECRET`. Under the
/// fallback, session cookies are signed with the sandbox secret, so rotating
/// `TASK_TOKEN_SECRET` logs everyone out — see [`warn_on_jwt_secret_fallback`].
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
            "SESSION_JWT_SECRET is unset — signing session cookies with TASK_TOKEN_SECRET. \
             Rotating TASK_TOKEN_SECRET will invalidate every active session. Set \
             SESSION_JWT_SECRET to decouple them."
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
}
