use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use std::collections::HashMap;

/// The secrets one identity's runtime materializes.
///
/// Env-style values reach the agent as environment variables; ssh keys are
/// written to `~/.ssh` and never exposed as env. Each read walks `user >
/// global`, marking records used so the audit trail is part of the read.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SecretResolver: Send + Sync {
    /// Env-style secrets `user_id` can reach, keyed by secret name.
    async fn plaintext(&self, user_id: &str) -> Result<HashMap<String, String>, StoreError>;

    /// SSH private keys `user_id` can reach, keyed by secret name.
    async fn ssh_keys(&self, user_id: &str) -> Result<HashMap<String, String>, StoreError>;
}

/// Which secrets exist for a caller, by name alone.
///
/// Metadata only: no value is decrypted or returned; a caller that needs a
/// value reads it through [`SecretResolver`].
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait SecretCatalog: Send + Sync {
    /// Every secret key `ctx` may see — the global ones plus its own.
    async fn list(&self, ctx: &ScopeContext) -> Result<Vec<String>, StoreError>;
}
