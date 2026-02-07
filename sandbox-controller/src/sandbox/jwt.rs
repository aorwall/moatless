//! JWT generation for task auth tokens.
//!
//! Generates HS256 JWTs that sandbox pods use to authenticate
//! back to the backend for MCP communication.

use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;
use tracing::debug;

/// JWT claims for task auth token
#[derive(Debug, Serialize)]
pub struct TaskTokenClaims {
    /// Task ID
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// User ID
    #[serde(rename = "userId")]
    pub user_id: String,
    /// Issued at (unix timestamp)
    pub iat: u64,
    /// Expiration (unix timestamp)
    pub exp: u64,
    /// Issuer
    pub iss: String,
}

/// Generate a task-scoped JWT token.
///
/// The token is signed with HS256 and expires in 24 hours.
pub fn generate_task_auth_token(
    task_id: &str,
    user_id: &str,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let claims = TaskTokenClaims {
        task_id: task_id.to_string(),
        user_id: user_id.to_string(),
        iat: now,
        exp: now + 24 * 3600, // 24 hours
        iss: "sandbox-controller".to_string(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    debug!(task_id, "Generated task auth token");
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_task_auth_token() {
        let token = generate_task_auth_token("task-123", "user-456", "test-secret").unwrap();
        assert!(!token.is_empty());
        // JWT has 3 parts separated by dots
        assert_eq!(token.split('.').count(), 3);
    }
}
