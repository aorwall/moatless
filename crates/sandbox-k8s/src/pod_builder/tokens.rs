//! The auth tokens a pod is handed, and the mint that issues them.

use super::*;

/// Auth tokens delivered to a sandbox pod.
#[derive(Clone, Copy)]
pub struct SandboxAuthTokens<'a> {
    /// Sits on the pod spec (`SANDBOX_BOOTSTRAP_TOKEN`) and authenticates only
    /// the sandbox's initial `GetCredentials` call; the long-lived user token
    /// instead travels over that credential channel (see `secrets.rs`).
    pub bootstrap: &'a str,
    /// When present, additionally emits `USER_AUTH_TOKEN` (gated by
    /// `SANDBOX_EMIT_LEGACY_USER_TOKEN`).
    pub legacy_user_token: Option<&'a str>,
}

impl<'a> SandboxAuthTokens<'a> {
    /// Borrow the owned [`MintedSandboxTokens`] into the pod-spec view.
    pub fn from_minted(minted: &'a MintedSandboxTokens) -> Self {
        Self {
            bootstrap: minted.bootstrap.as_str(),
            legacy_user_token: minted.legacy_user_token.as_deref(),
        }
    }
}

/// Mint the auth tokens a sandbox pod needs from config.
///
/// Produces a short-lived bootstrap token plus, while
/// `SANDBOX_EMIT_LEGACY_USER_TOKEN` is set, the long-lived user token as
/// `USER_AUTH_TOKEN`; callers borrow the result into [`SandboxAuthTokens`] via
/// [`SandboxAuthTokens::from_minted`].
pub fn mint_sandbox_tokens(
    config: &Config,
    task_id: &str,
    user_id: &str,
) -> Result<MintedSandboxTokens, SandboxError> {
    let secret = config
        .sandbox
        .task_token_secret
        .as_deref()
        .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?;
    jwt::mint_sandbox_tokens(
        user_id,
        user_id,
        task_id,
        secret,
        config.sandbox.bootstrap_token_ttl_secs,
        config.sandbox.user_token_ttl_secs,
        config.sandbox.emit_legacy_user_token_env,
    )
    .map_err(|source| SandboxError::JwtSigning {
        purpose: "the sandbox task token",
        source,
    })
}
