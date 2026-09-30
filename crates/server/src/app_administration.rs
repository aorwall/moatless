//! The provider half of GitHub App administration: proving a signing key.

use async_trait::async_trait;
use moatless_identity::adapter_apps::administration::{AppAdministrationError, GitHubAppProver};
use moatless_identity::auth::github::service::{GitHubAppService, ProvedApp};

/// Proves a signing key by asking GitHub to confirm the App it signs for.
pub struct ProviderGitHubAppProver {
    /// GitHub REST API root, from `Config::github_api_base`.
    pub github_api_base: String,
    /// From `Config::http`.
    pub github: moatless_types::http::HttpTimeouts,
}

#[async_trait]
impl GitHubAppProver for ProviderGitHubAppProver {
    async fn prove(
        &self,
        app_key: &str,
        app_id: u64,
        private_key_pem: &str,
        default_installation_id: Option<u64>,
    ) -> Result<ProvedApp, AppAdministrationError> {
        let entry = moatless_types::auth::GitHubAppConfig {
            key: app_key.to_string(),
            app_id,
            private_key: private_key_pem.to_string(),
            client_id: None,
            client_secret: None,
            default_installation_id,
        };
        let service = GitHubAppService::from_app_config(&entry, &self.github_api_base, self.github)
            .ok_or_else(|| {
                AppAdministrationError::BadRequest(
                    "The signing key could not be read as a private key. Nothing was stored."
                        .to_string(),
                )
            })?;
        Ok(service.prove().await?)
    }
}
