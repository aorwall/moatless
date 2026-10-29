//! Generates HS256 JWTs that sandbox pods use to authenticate back to the
//! backend. Two flavours share the same claim shape and signing secret.
//!
//! The long-lived user token (`aud` unset) is the credential for a sandbox's
//! gRPC calls; see [`generate_user_auth_token`] for its TTL behavior.
//!
//! The bootstrap token ([`AUDIENCE_BOOTSTRAP`]) is short-lived and scoped to
//! bootstrapping; see [`generate_bootstrap_token`] for its lifecycle.
//!
//! Both carry their Sandbox's token generation; [`check_generation`] is how
//! every backend route refuses a token from an older one.

use std::sync::Arc;

use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use moatless_types::store::StoreError;
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
    /// The task this token is minted for; a sandbox is provisioned per task, so
    /// its token is bound to exactly one task id.
    ///
    /// The verification layer rejects any RPC whose in-band `task_id`
    /// disagrees; `None` is accepted only when the grace flag is on (see
    /// `SandboxAuthorizer`).
    #[serde(rename = "taskId", default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// The sandbox this token is minted for. When present, backend gRPC
    /// authorization admits any requested task that is a member of this
    /// sandbox.
    #[serde(rename = "sandboxId", default, skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
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
    /// The Sandbox's token generation this token was minted at. A token minted
    /// before the claim existed decodes as 0, the column's default, so pods
    /// running across the upgrade keep working until the first bump.
    #[serde(rename = "gen", default)]
    pub generation: i64,
}

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
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
/// `ttl_secs` controls expiry — the token is re-minted on every
/// `GetCredentials` response, so a short TTL is fine (the 15-min credential
/// refresher keeps a running sandbox's copy fresh); `aud` is left unset so the
/// backend's [`verify_user_auth_token`] accepts it everywhere.
pub fn generate_user_auth_token(
    user_id: &str,
    login: &str,
    task_id: &str,
    generation: i64,
    secret: &str,
    ttl_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = now_secs();
    let claims = UserTokenClaims {
        user_id: user_id.to_string(),
        login: login.to_string(),
        task_id: None,
        sandbox_id: Some(task_id.to_string()),
        iat: now,
        exp: now + ttl_secs,
        iss: "moatless-vibe-backend".to_string(),
        aud: None,
        generation,
    };

    let token = encode_claims(&claims, secret)?;
    debug!(user_id, task_id, ttl_secs, "Generated user auth token");
    Ok(token)
}

/// Generate a short-lived bootstrap token (audience [`AUDIENCE_BOOTSTRAP`]).
///
/// Placed on the pod spec in lieu of the long-lived user token. It
/// authenticates only the initial `GetCredentials` call; the sandbox then swaps
/// to the long-lived token delivered in that response.
pub fn generate_bootstrap_token(
    user_id: &str,
    login: &str,
    task_id: &str,
    generation: i64,
    secret: &str,
    ttl_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = now_secs();
    let claims = UserTokenClaims {
        user_id: user_id.to_string(),
        login: login.to_string(),
        task_id: None,
        sandbox_id: Some(task_id.to_string()),
        iat: now,
        exp: now + ttl_secs,
        iss: "moatless-vibe-backend".to_string(),
        aud: Some(AUDIENCE_BOOTSTRAP.to_string()),
        generation,
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
#[expect(
    clippy::too_many_arguments,
    reason = "the two tokens' shared claims plus each one's TTL"
)]
pub fn mint_sandbox_tokens(
    user_id: &str,
    login: &str,
    task_id: &str,
    generation: i64,
    secret: &str,
    bootstrap_ttl_secs: u64,
    user_ttl_secs: u64,
    emit_legacy: bool,
) -> Result<MintedSandboxTokens, jsonwebtoken::errors::Error> {
    let bootstrap = generate_bootstrap_token(
        user_id,
        login,
        task_id,
        generation,
        secret,
        bootstrap_ttl_secs,
    )?;
    let legacy_user_token = if emit_legacy {
        Some(generate_user_auth_token(
            user_id,
            login,
            task_id,
            generation,
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
/// Used only by the `GetCredentials` handler. Both bootstrap and long-lived
/// user tokens are accepted here.
pub fn verify_bootstrap_or_user_token(
    token: &str,
    secret: &str,
) -> Result<UserTokenClaims, jsonwebtoken::errors::Error> {
    decode_claims(token, secret)
}

/// Check if a user auth token is near expiry.
///
/// Returns `true` if the token expires within `threshold_secs` seconds, or if
/// the token is invalid / cannot be decoded.
pub fn is_token_near_expiry(token: &str, secret: &str, threshold_secs: u64) -> bool {
    match verify_user_auth_token(token, secret) {
        Ok(claims) => claims.exp <= now_secs() + threshold_secs,
        Err(_) => true, // Invalid/expired token should be regenerated
    }
}

/// The current generation of each Sandbox's task tokens, which a bump
/// advances to revoke every token minted before it.
#[async_trait]
pub trait TokenGenerations: Send + Sync {
    /// `sandbox_id`'s current generation, absent when the Sandbox has no row.
    async fn current(&self, sandbox_id: &str) -> Result<Option<i64>, StoreError>;

    /// The user `sandbox_id`'s current-generation tokens are minted for: its
    /// driver, else its owner. Absent when the Sandbox has no row or neither.
    async fn token_user(&self, sandbox_id: &str) -> Result<Option<String>, StoreError>;
}

/// Why [`check_generation`] refused a token.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    #[error("task token generation {presented} is not current ({current})")]
    Stale { presented: i64, current: i64 },
    /// The row goes only with its Task, so its tokens go with it too.
    #[error("the task token names a sandbox that no longer exists")]
    NoSandbox,
    #[error("failed to read the sandbox's token generation")]
    Lookup(#[from] StoreError),
}

/// Refuse a token minted at `presented` unless that is `sandbox_id`'s current
/// generation. Fails closed when the generation cannot be read.
pub async fn check_generation(
    generations: &dyn TokenGenerations,
    sandbox_id: &str,
    presented: i64,
) -> Result<(), GenerationError> {
    let current = generations
        .current(sandbox_id)
        .await?
        .ok_or(GenerationError::NoSandbox)?;
    if presented == current {
        Ok(())
    } else {
        Err(GenerationError::Stale { presented, current })
    }
}

/// Fixed generations per Sandbox, with no row for any other, for tests that
/// mint tokens without a database.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct StaticTokenGenerations {
    pub generations: std::collections::HashMap<String, i64>,
    /// Each Sandbox's [`TokenGenerations::token_user`].
    pub users: std::collections::HashMap<String, String>,
}

#[cfg(any(test, feature = "test-support"))]
impl StaticTokenGenerations {
    /// One Sandbox at `generation`, its tokens minted for `user_id`.
    pub fn one(sandbox_id: &str, generation: i64, user_id: &str) -> Self {
        Self {
            generations: [(sandbox_id.to_string(), generation)].into(),
            users: [(sandbox_id.to_string(), user_id.to_string())].into(),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl TokenGenerations for StaticTokenGenerations {
    async fn current(&self, sandbox_id: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.generations.get(sandbox_id).copied())
    }

    async fn token_user(&self, sandbox_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self.users.get(sandbox_id).cloned())
    }
}

/// Verifies `X-User-Token` JWTs against the process token secret and their
/// Sandbox's current generation.
///
/// Satisfies the consumer-declared [`crate::http_auth::UserTokenVerifier`]
/// port, keeping the `sandbox::jwt` dependency behind it so the auth extractors
/// do not name this module.
pub struct JwtUserTokenVerifier {
    secret: Option<String>,
    generations: Arc<dyn TokenGenerations>,
}

impl JwtUserTokenVerifier {
    pub fn new(secret: Option<String>, generations: Arc<dyn TokenGenerations>) -> Self {
        Self {
            secret,
            generations,
        }
    }
}

#[async_trait]
impl crate::http_auth::UserTokenVerifier for JwtUserTokenVerifier {
    async fn verify_user_token(
        &self,
        token: &str,
    ) -> Result<crate::http_auth::VerifiedUserToken, crate::http_auth::TokenVerifyError> {
        use crate::http_auth::TokenVerifyError;

        let secret = self
            .secret
            .as_deref()
            .ok_or(TokenVerifyError::NotConfigured)?;
        let claims = verify_user_auth_token(token, secret)
            .map_err(|e| TokenVerifyError::Invalid(e.to_string()))?;
        // A token naming no Sandbox has no generation to check, so it could
        // never be revoked; every minted token names one.
        let sandbox_id = claims
            .sandbox_id
            .as_deref()
            .ok_or_else(|| TokenVerifyError::Invalid("token names no sandbox".to_string()))?;
        check_generation(self.generations.as_ref(), sandbox_id, claims.generation)
            .await
            .map_err(|e| TokenVerifyError::Invalid(e.to_string()))?;
        Ok(crate::http_auth::VerifiedUserToken {
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
            generate_user_auth_token("user-456", "john", "task-1", 0, "test-secret", TTL).unwrap();
        assert!(!token.is_empty());
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn test_verify_user_auth_token() {
        let token =
            generate_user_auth_token("user-456", "john", "task-1", 0, "test-secret", TTL).unwrap();
        let claims = verify_user_auth_token(&token, "test-secret").unwrap();
        assert_eq!(claims.user_id, "user-456");
        assert_eq!(claims.login, "john");
        assert_eq!(claims.task_id, None);
        assert_eq!(claims.sandbox_id.as_deref(), Some("task-1"));
        assert_eq!(claims.iss, "moatless-vibe-backend");
        assert_eq!(claims.aud, None);
    }

    #[test]
    fn test_verify_user_auth_token_wrong_secret() {
        let token =
            generate_user_auth_token("user-456", "john", "task-1", 0, "test-secret", TTL).unwrap();
        let result = verify_user_auth_token(&token, "wrong-secret");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_token_near_expiry_fresh_token() {
        let token =
            generate_user_auth_token("user-456", "john", "task-1", 0, "test-secret", TTL).unwrap();
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
            generate_bootstrap_token("user-456", "john", "task-1", 0, "test-secret", 600).unwrap();
        let claims = verify_bootstrap_or_user_token(&token, "test-secret").unwrap();
        assert_eq!(claims.aud.as_deref(), Some(AUDIENCE_BOOTSTRAP));
        assert_eq!(claims.user_id, "user-456");
        assert_eq!(claims.task_id, None);
        assert_eq!(claims.sandbox_id.as_deref(), Some("task-1"));
    }

    #[test]
    fn bootstrap_token_rejected_by_user_verifier() {
        // A bootstrap token must not authenticate the long-lived RPCs.
        let token =
            generate_bootstrap_token("user-456", "john", "task-1", 0, "test-secret", 600).unwrap();
        assert!(verify_user_auth_token(&token, "test-secret").is_err());
    }

    #[test]
    fn user_token_accepted_by_bootstrap_verifier() {
        // GetCredentials accepts the long-lived user token.
        let token =
            generate_user_auth_token("user-456", "john", "task-1", 0, "test-secret", TTL).unwrap();
        let claims = verify_bootstrap_or_user_token(&token, "test-secret").unwrap();
        assert_eq!(claims.user_id, "user-456");
        assert_eq!(claims.aud, None);
    }

    fn generations(sandbox_id: &str, generation: i64) -> Arc<dyn TokenGenerations> {
        Arc::new(StaticTokenGenerations::one(
            sandbox_id, generation, "user-456",
        ))
    }

    #[tokio::test]
    async fn port_maps_outcomes_to_the_extractor_error() {
        use crate::http_auth::{TokenVerifyError, UserTokenVerifier};

        let unconfigured = JwtUserTokenVerifier::new(None, generations("task-1", 0));
        assert!(matches!(
            unconfigured.verify_user_token("anything").await,
            Err(TokenVerifyError::NotConfigured)
        ));

        let verifier =
            JwtUserTokenVerifier::new(Some("test-secret".to_string()), generations("task-1", 0));

        let token =
            generate_user_auth_token("user-456", "john", "task-1", 0, "test-secret", TTL).unwrap();
        let verified = verifier
            .verify_user_token(&token)
            .await
            .expect("valid token");
        assert_eq!(verified.user_id, "user-456");
        assert_eq!(verified.login, "john");

        // Garbage -> Invalid, not a panic.
        assert!(matches!(
            verifier.verify_user_token("not-a-jwt").await,
            Err(TokenVerifyError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn verifier_refuses_a_token_from_an_older_generation() {
        use crate::http_auth::{TokenVerifyError, UserTokenVerifier};

        let verifier =
            JwtUserTokenVerifier::new(Some("test-secret".to_string()), generations("task-1", 2));
        let old =
            generate_user_auth_token("user-456", "john", "task-1", 1, "test-secret", TTL).unwrap();
        assert!(matches!(
            verifier.verify_user_token(&old).await,
            Err(TokenVerifyError::Invalid(_))
        ));

        let current =
            generate_user_auth_token("user-456", "john", "task-1", 2, "test-secret", TTL).unwrap();
        assert!(verifier.verify_user_token(&current).await.is_ok());
    }

    #[test]
    fn token_without_a_generation_claim_decodes_as_generation_zero() {
        let claims = UserTokenClaims {
            user_id: "user-456".to_string(),
            login: "john".to_string(),
            task_id: None,
            sandbox_id: Some("task-1".to_string()),
            iat: now_secs(),
            exp: now_secs() + TTL,
            iss: "moatless-vibe-backend".to_string(),
            aud: None,
            generation: 7,
        };
        let mut json = serde_json::to_value(&claims).unwrap();
        json.as_object_mut().unwrap().remove("gen");
        let token = encode(
            &Header::default(),
            &json,
            &EncodingKey::from_secret(b"test-secret"),
        )
        .unwrap();
        let decoded = verify_user_auth_token(&token, "test-secret").unwrap();
        assert_eq!(decoded.generation, 0);
    }

    #[tokio::test]
    async fn check_generation_fails_closed_on_a_lookup_error() {
        struct Broken;
        #[async_trait]
        impl TokenGenerations for Broken {
            async fn current(&self, _sandbox_id: &str) -> Result<Option<i64>, StoreError> {
                Err(StoreError::new("db down"))
            }
            async fn token_user(&self, _sandbox_id: &str) -> Result<Option<String>, StoreError> {
                Err(StoreError::new("db down"))
            }
        }
        assert!(matches!(
            check_generation(&Broken, "task-1", 0).await,
            Err(GenerationError::Lookup(_))
        ));
        // A Sandbox with no row refuses even a generation-0 token.
        assert!(matches!(
            check_generation(&StaticTokenGenerations::default(), "task-1", 0).await,
            Err(GenerationError::NoSandbox)
        ));
    }
}
