//! JWT generation and verification for sandbox auth tokens.
//!
//! Generates HS256 JWTs that sandbox pods use to authenticate back to the
//! backend. Two flavours share the same claim shape and signing secret:
//!
//! - **Long-lived user token** (`aud` unset): the credential a sandbox uses for
//!   the bulk of its gRPC calls. Historically a 90-day token baked into the pod
//!   spec as `USER_AUTH_TOKEN`; it is now delivered at runtime over the
//!   `GetCredentials` channel and its TTL is configurable (and much shorter).
//! - **Bootstrap token** (`aud = "sandbox-bootstrap"`): a short-lived token set
//!   on the pod spec whose *only* privilege is to authenticate the sandbox's
//!   initial `GetCredentials` call, at which point the sandbox swaps to the
//!   long-lived token it receives back. Minutes-scale TTL bounds the blast
//!   radius of the one secret that still lands in etcd.

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Audience marker for bootstrap tokens. A token carrying this `aud` may only
/// authenticate the initial `GetCredentials` call — never the long-lived RPCs.
pub const AUDIENCE_BOOTSTRAP: &str = "sandbox-bootstrap";

/// JWT claims for a sandbox auth token (long-lived user token or bootstrap).
#[derive(Debug, Serialize, Deserialize)]
pub struct UserTokenClaims {
    /// User ID
    #[serde(rename = "userId")]
    pub user_id: String,
    /// User login name
    pub login: String,
    /// The task this token is minted for. A sandbox is provisioned per task, so
    /// its token is bound to exactly one task id; the verification layer rejects
    /// any RPC whose in-band `task_id` disagrees with this claim. `None` is
    /// accepted only when the claim grace flag is on (see `SandboxAuthorizer`).
    #[serde(rename = "taskId", default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Issued at (unix timestamp)
    pub iat: u64,
    /// Expiration (unix timestamp)
    pub exp: u64,
    /// Issuer
    pub iss: String,
    /// Audience. Unset for the long-lived user token; set to
    /// [`AUDIENCE_BOOTSTRAP`] for the short-lived bootstrap token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn encode_claims(
    claims: &UserTokenClaims,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// Generate the long-lived user auth token a sandbox uses for its gRPC calls.
///
/// `ttl_secs` controls expiry — the token is re-minted on every `GetCredentials`
/// response, so a short TTL is fine (the 15-min credential refresher keeps a
/// running sandbox's copy fresh). `aud` is left unset so the backend's
/// [`verify_user_auth_token`] accepts it everywhere.
pub fn generate_user_auth_token(
    user_id: &str,
    login: &str,
    task_id: &str,
    secret: &str,
    ttl_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = now_secs();
    let claims = UserTokenClaims {
        user_id: user_id.to_string(),
        login: login.to_string(),
        task_id: Some(task_id.to_string()),
        iat: now,
        exp: now + ttl_secs,
        iss: "moatless-vibe-backend".to_string(),
        aud: None,
    };

    let token = encode_claims(&claims, secret)?;
    debug!(user_id, task_id, ttl_secs, "Generated user auth token");
    Ok(token)
}

/// Generate a short-lived bootstrap token (audience [`AUDIENCE_BOOTSTRAP`]).
///
/// Placed on the pod spec in lieu of the long-lived user token. It authenticates
/// only the initial `GetCredentials` call; the sandbox then swaps to the
/// long-lived token delivered in that response.
pub fn generate_bootstrap_token(
    user_id: &str,
    login: &str,
    task_id: &str,
    secret: &str,
    ttl_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = now_secs();
    let claims = UserTokenClaims {
        user_id: user_id.to_string(),
        login: login.to_string(),
        task_id: Some(task_id.to_string()),
        iat: now,
        exp: now + ttl_secs,
        iss: "moatless-vibe-backend".to_string(),
        aud: Some(AUDIENCE_BOOTSTRAP.to_string()),
    };

    let token = encode_claims(&claims, secret)?;
    debug!(
        user_id,
        task_id, ttl_secs, "Generated sandbox bootstrap token"
    );
    Ok(token)
}

/// The auth tokens minted for a sandbox pod: a short-lived bootstrap token plus
/// an optional long-lived user token.
pub struct MintedSandboxTokens {
    /// Short-lived bootstrap token placed on the pod spec.
    pub bootstrap: String,
    /// Long-lived user token, emitted as `USER_AUTH_TOKEN` only when
    /// `emit_legacy` is set.
    pub legacy_user_token: Option<String>,
}

/// Mint the auth tokens a sandbox pod needs.
///
/// Always mints a bootstrap token (`bootstrap_ttl_secs`). When `emit_legacy` is
/// set, also mints a long-lived user token (`user_ttl_secs`) for the
/// `USER_AUTH_TOKEN` env.
pub fn mint_sandbox_tokens(
    user_id: &str,
    login: &str,
    task_id: &str,
    secret: &str,
    bootstrap_ttl_secs: u64,
    user_ttl_secs: u64,
    emit_legacy: bool,
) -> Result<MintedSandboxTokens, jsonwebtoken::errors::Error> {
    let bootstrap = generate_bootstrap_token(user_id, login, task_id, secret, bootstrap_ttl_secs)?;
    let legacy_user_token = if emit_legacy {
        Some(generate_user_auth_token(
            user_id,
            login,
            task_id,
            secret,
            user_ttl_secs,
        )?)
    } else {
        None
    };
    Ok(MintedSandboxTokens {
        bootstrap,
        legacy_user_token,
    })
}

/// Decode + verify a token's signature and issuer without checking audience.
///
/// `aud` is validated by the callers below so a bootstrap token can be accepted
/// on exactly one RPC and rejected everywhere else.
fn decode_claims(
    token: &str,
    secret: &str,
) -> Result<UserTokenClaims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::default();
    validation.set_issuer(&["moatless-vibe-backend"]);
    // Audience is checked manually by the callers; disable library-side aud
    // validation so tokens without an `aud` claim (the long-lived token) don't
    // trip the default `validate_aud = true` behaviour.
    validation.validate_aud = false;

    let data = decode::<UserTokenClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(data.claims)
}

/// Verify a long-lived user auth token and return its claims.
///
/// Rejects bootstrap tokens: those are scoped to the initial `GetCredentials`
/// call only (see [`verify_bootstrap_or_user_token`]).
pub fn verify_user_auth_token(
    token: &str,
    secret: &str,
) -> Result<UserTokenClaims, jsonwebtoken::errors::Error> {
    let claims = decode_claims(token, secret)?;
    if claims.aud.as_deref() == Some(AUDIENCE_BOOTSTRAP) {
        return Err(jsonwebtoken::errors::Error::from(
            jsonwebtoken::errors::ErrorKind::InvalidAudience,
        ));
    }
    Ok(claims)
}

/// Verify a token accepted for the initial `GetCredentials` call: either a
/// short-lived bootstrap token or a long-lived user token.
///
/// Used only by the `GetCredentials` handler. Both bootstrap and long-lived user
/// tokens are accepted here.
pub fn verify_bootstrap_or_user_token(
    token: &str,
    secret: &str,
) -> Result<UserTokenClaims, jsonwebtoken::errors::Error> {
    decode_claims(token, secret)
}

/// Check if a user auth token is near expiry.
///
/// Returns `true` if the token expires within `threshold_secs` seconds,
/// or if the token is invalid / cannot be decoded.
pub fn is_token_near_expiry(token: &str, secret: &str, threshold_secs: u64) -> bool {
    match verify_user_auth_token(token, secret) {
        Ok(claims) => claims.exp <= now_secs() + threshold_secs,
        Err(_) => true, // Invalid/expired token should be regenerated
    }
}

/// Verifies `X-User-Token` JWTs against the process token secret, satisfying
/// the consumer-declared [`moatless_identity::extractors::UserTokenVerifier`] port. Keeps
/// the `sandbox::jwt` dependency behind the port so the auth extractors do not
/// name this module.
pub struct JwtUserTokenVerifier {
    secret: Option<String>,
}

impl JwtUserTokenVerifier {
    pub fn new(secret: Option<String>) -> Self {
        Self { secret }
    }
}

impl moatless_identity::extractors::UserTokenVerifier for JwtUserTokenVerifier {
    fn verify_user_token(
        &self,
        token: &str,
    ) -> Result<moatless_identity::extractors::VerifiedUserToken, moatless_identity::extractors::TokenVerifyError> {
        let secret = self
            .secret
            .as_deref()
            .ok_or(moatless_identity::extractors::TokenVerifyError::NotConfigured)?;
        let claims = verify_user_auth_token(token, secret)
            .map_err(|e| moatless_identity::extractors::TokenVerifyError::Invalid(e.to_string()))?;
        Ok(moatless_identity::extractors::VerifiedUserToken {
            user_id: claims.user_id,
            login: claims.login,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: u64 = 7 * 24 * 3600;

    #[test]
    fn test_generate_user_auth_token() {
        let token =
            generate_user_auth_token("user-456", "john", "task-1", "test-secret", TTL).unwrap();
        assert!(!token.is_empty());
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn test_verify_user_auth_token() {
        let token =
            generate_user_auth_token("user-456", "john", "task-1", "test-secret", TTL).unwrap();
        let claims = verify_user_auth_token(&token, "test-secret").unwrap();
        assert_eq!(claims.user_id, "user-456");
        assert_eq!(claims.login, "john");
        assert_eq!(claims.task_id.as_deref(), Some("task-1"));
        assert_eq!(claims.iss, "moatless-vibe-backend");
        assert_eq!(claims.aud, None);
    }

    #[test]
    fn test_verify_user_auth_token_wrong_secret() {
        let token =
            generate_user_auth_token("user-456", "john", "task-1", "test-secret", TTL).unwrap();
        let result = verify_user_auth_token(&token, "wrong-secret");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_token_near_expiry_fresh_token() {
        let token =
            generate_user_auth_token("user-456", "john", "task-1", "test-secret", TTL).unwrap();
        // Fresh 7-day token should NOT be near expiry with a 1-day threshold.
        assert!(!is_token_near_expiry(&token, "test-secret", 24 * 3600));
    }

    #[test]
    fn test_is_token_near_expiry_invalid_token() {
        assert!(is_token_near_expiry("invalid", "test-secret", 0));
    }

    #[test]
    fn bootstrap_token_carries_audience() {
        let token =
            generate_bootstrap_token("user-456", "john", "task-1", "test-secret", 600).unwrap();
        let claims = verify_bootstrap_or_user_token(&token, "test-secret").unwrap();
        assert_eq!(claims.aud.as_deref(), Some(AUDIENCE_BOOTSTRAP));
        assert_eq!(claims.user_id, "user-456");
        assert_eq!(claims.task_id.as_deref(), Some("task-1"));
    }

    #[test]
    fn bootstrap_token_rejected_by_user_verifier() {
        // A bootstrap token must not authenticate the long-lived RPCs.
        let token =
            generate_bootstrap_token("user-456", "john", "task-1", "test-secret", 600).unwrap();
        assert!(verify_user_auth_token(&token, "test-secret").is_err());
    }

    #[test]
    fn user_token_accepted_by_bootstrap_verifier() {
        // GetCredentials accepts the long-lived user token.
        let token =
            generate_user_auth_token("user-456", "john", "task-1", "test-secret", TTL).unwrap();
        let claims = verify_bootstrap_or_user_token(&token, "test-secret").unwrap();
        assert_eq!(claims.user_id, "user-456");
        assert_eq!(claims.aud, None);
    }

    #[test]
    fn port_maps_outcomes_to_the_extractor_error() {
        use moatless_identity::extractors::{TokenVerifyError, UserTokenVerifier};

        // No configured secret -> NotConfigured.
        let unconfigured = JwtUserTokenVerifier::new(None);
        assert!(matches!(
            unconfigured.verify_user_token("anything"),
            Err(TokenVerifyError::NotConfigured)
        ));

        let verifier = JwtUserTokenVerifier::new(Some("test-secret".to_string()));

        // A valid token -> the two fields the extractor reads.
        let token =
            generate_user_auth_token("user-456", "john", "task-1", "test-secret", TTL).unwrap();
        let verified = verifier.verify_user_token(&token).expect("valid token");
        assert_eq!(verified.user_id, "user-456");
        assert_eq!(verified.login, "john");

        // Garbage -> Invalid, not a panic.
        assert!(matches!(
            verifier.verify_user_token("not-a-jwt"),
            Err(TokenVerifyError::Invalid(_))
        ));
    }
}
