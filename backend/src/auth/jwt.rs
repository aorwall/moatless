//! Session JWT generation and verification.
//!
//! Distinct from `sandbox/jwt.rs` which handles sandbox task tokens.
//! Uses HS256 signing — same algorithm, separate claims structure.

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::models::UserRole;
use crate::config::Config;

/// JWT claims embedded in the session cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    /// User ID
    pub sub: String,
    /// User login name
    pub login: String,
    /// User role ("user" or "admin")
    pub role: String,
    /// Auth provider ("github", "auth0", "api-key")
    pub provider: String,
    /// Optional group ID (deprecated, kept for backwards compat)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// Team IDs the user belongs to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub team_ids: Vec<String>,
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
#[allow(clippy::too_many_arguments)]
pub fn generate_session_jwt(
    user_id: &str,
    login: &str,
    role: UserRole,
    provider: &str,
    group_id: Option<&str>,
    team_ids: &[String],
    secret: &str,
    max_age_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let claims = SessionClaims {
        sub: user_id.to_string(),
        login: login.to_string(),
        role: role.as_str().to_string(),
        provider: provider.to_string(),
        group_id: group_id.map(String::from),
        team_ids: team_ids.to_vec(),
        iat: now,
        exp: now + max_age_secs,
        iss: "moatless-vibe-backend".to_string(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    debug!(user_id, "Generated session JWT");
    Ok(token)
}

/// Verify a session JWT and return its claims.
pub fn verify_session_jwt(
    token: &str,
    secret: &str,
) -> Result<SessionClaims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::default();
    validation.set_issuer(&["moatless-vibe-backend"]);

    let data = decode::<SessionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    Ok(data.claims)
}

/// Resolve the JWT secret from config.
///
/// Prefers `SESSION_JWT_SECRET`, falls back to `TASK_TOKEN_SECRET`.
pub fn get_jwt_secret(config: &Config) -> Option<&str> {
    config
        .auth
        .session_jwt_secret
        .as_deref()
        .or(config.sandbox.task_token_secret.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let token = generate_session_jwt(
            "user-123",
            "alice",
            UserRole::Admin,
            "github",
            Some("group-1"),
            &["team-a".to_string(), "team-b".to_string()],
            "test-secret",
            3600,
        )
        .unwrap();

        assert_eq!(token.split('.').count(), 3);

        let claims = verify_session_jwt(&token, "test-secret").unwrap();
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.login, "alice");
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.provider, "github");
        assert_eq!(claims.group_id.as_deref(), Some("group-1"));
        assert_eq!(claims.team_ids, vec!["team-a", "team-b"]);
        assert_eq!(claims.iss, "moatless-vibe-backend");
        assert_eq!(claims.user_role(), UserRole::Admin);
    }

    #[test]
    fn wrong_secret_fails() {
        let token = generate_session_jwt(
            "user-123",
            "alice",
            UserRole::User,
            "github",
            None,
            &[],
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
            sub: "user-123".to_string(),
            login: "alice".to_string(),
            role: "user".to_string(),
            provider: "github".to_string(),
            group_id: None,
            team_ids: vec![],
            iat: now - 7200,
            exp: now - 3600, // expired 1 hour ago (well past default 60s leeway)
            iss: "moatless-vibe-backend".to_string(),
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
            "user-123",
            "alice",
            UserRole::User,
            "github",
            None,
            &[],
            "test-secret",
            3600,
        )
        .unwrap();

        let claims = verify_session_jwt(&token, "test-secret").unwrap();
        assert!(claims.group_id.is_none());
        assert!(claims.team_ids.is_empty());
    }
}
