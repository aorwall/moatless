//! Dependencies of the Linear workspace installation flow.
use super::client::LinearOAuthProvider;
use moatless_adapter_model::connections::ConnectionInstaller;
use moatless_core::config::Config;
use moatless_ports::identity::AppCredentials;
use std::sync::Arc;

/// Configured workspace installation operation.
pub struct Installation {
    pub(super) config: Arc<Config>,
    pub(super) provider: Option<LinearOAuthProvider>,
    pub(super) connections: Arc<dyn ConnectionInstaller>,
    pub(super) credentials: Arc<dyn AppCredentials>,
}

impl Installation {
    /// Assemble the configured provider and its persistence capabilities.
    pub fn new(
        config: Arc<Config>,
        connections: Arc<dyn ConnectionInstaller>,
        credentials: Arc<dyn AppCredentials>,
    ) -> Self {
        Self {
            provider: LinearOAuthProvider::from_config(&config),
            config,
            connections,
            credentials,
        }
    }
}
