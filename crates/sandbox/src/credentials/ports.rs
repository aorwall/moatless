//! Declares the credential application capability.

use async_trait::async_trait;

/// Apply a single credential value. Implementations decide *how* a credential
/// becomes effective inside the sandbox (file write, env var update, etc.).
#[async_trait]
pub trait CredentialSink: Send + Sync {
    async fn apply(&self, value: &str) -> Result<(), String>;
}
