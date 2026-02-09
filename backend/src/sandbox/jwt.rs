//! JWT generation and verification for user auth tokens.
//!
//! Generates HS256 JWTs that sandbox pods use to authenticate
//! back to the backend.

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// JWT claims for user auth token (per-user, long-lived)
#[derive(Debug, Serialize, Deserialize)]
pub struct UserTokenClaims {
    /// User ID
    #[serde(rename = "userId")]
    pub user_id: String,
    /// User login name
    pub login: String,
    /// Issued at (unix timestamp)
    pub iat: u64,
    /// Expiration (unix timestamp)
    pub exp: u64,
    /// Issuer
    pub iss: String,
}

/// Generate a long-lived user auth token (90-day expiry).
///
/// This token is shared across all tasks for a user, stored in the
/// per-user K8s Secret, and verified by the backend extractors.
pub fn generate_user_auth_token(
    user_id: &str,
    login: &str,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let claims = UserTokenClaims {
        user_id: user_id.to_string(),
        login: login.to_string(),
        iat: now,
        exp: now + 90 * 24 * 3600, // 90 days
        iss: "moatless-vibe-backend".to_string(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    debug!(user_id, "Generated user auth token");
    Ok(token)
}

/// Verify a user auth token and return its claims.
pub fn verify_user_auth_token(
    token: &str,
    secret: &str,
) -> Result<UserTokenClaims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::default();
    validation.set_issuer(&["moatless-vibe-backend"]);

    let data = decode::<UserTokenClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    Ok(data.claims)
}

/// Check if a user auth token is near expiry.
///
/// Returns `true` if the token expires within `threshold_secs` seconds,
/// or if the token is invalid / cannot be decoded.
pub fn is_token_near_expiry(token: &str, secret: &str, threshold_secs: u64) -> bool {
    match verify_user_auth_token(token, secret) {
        Ok(claims) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            claims.exp <= now + threshold_secs
        }
        Err(_) => true, // Invalid/expired token should be regenerated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_user_auth_token() {
        let token = generate_user_auth_token("user-456", "john", "test-secret").unwrap();
        assert!(!token.is_empty());
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn test_verify_user_auth_token() {
        let token = generate_user_auth_token("user-456", "john", "test-secret").unwrap();
        let claims = verify_user_auth_token(&token, "test-secret").unwrap();
        assert_eq!(claims.user_id, "user-456");
        assert_eq!(claims.login, "john");
        assert_eq!(claims.iss, "moatless-vibe-backend");
    }

    #[test]
    fn test_verify_user_auth_token_wrong_secret() {
        let token = generate_user_auth_token("user-456", "john", "test-secret").unwrap();
        let result = verify_user_auth_token(&token, "wrong-secret");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_token_near_expiry_fresh_token() {
        let token = generate_user_auth_token("user-456", "john", "test-secret").unwrap();
        // Fresh token should NOT be near expiry with 7-day threshold
        assert!(!is_token_near_expiry(&token, "test-secret", 7 * 24 * 3600));
    }

    #[test]
    fn test_is_token_near_expiry_invalid_token() {
        assert!(is_token_near_expiry("invalid", "test-secret", 0));
    }
}
