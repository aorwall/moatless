//! Concrete [`CredentialSink`] implementations.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::info;

use super::CredentialSink;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::fetch::persist_user_auth_token;
use crate::init::git::GitCredentials;

/// Swaps in the task token minted for a new driver: the backend has already
/// revoked the one it replaces, so every holder takes it here at once — the
/// gRPC client, the file the `moat` CLI reads, and proxied git.
pub(crate) struct UserAuthTokenSink {
    pub config: Arc<Config>,
    pub backend_client: Arc<dyn BackendClient>,
    pub git_credentials: Arc<RwLock<GitCredentials>>,
}

#[async_trait]
impl CredentialSink for UserAuthTokenSink {
    async fn apply(&self, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err("USER_AUTH_TOKEN must not be empty".to_string());
        }
        self.backend_client.update_auth_token(value.to_string());
        persist_user_auth_token(&self.config, value).await;
        let mut git_credentials = self.git_credentials.write().await;
        *git_credentials = git_credentials
            .clone()
            .with_task_token(Some(value.to_string()));
        info!("Swapped in the task token of the sandbox's new driver");
        Ok(())
    }
}
