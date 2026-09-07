//! Loop synchronization dependencies and HTTP client configuration.

use super::fetcher::GitHubLoopFetcher;
use super::reconcile::LoopSyncService;
use crate::loops::dao::LoopDao;
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_ports::task::TagResolver;
use std::sync::Arc;
use tracing::warn;

impl LoopSyncService {
    pub fn new(
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<LoopDao>,
        tags: Arc<dyn TagResolver>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    ) -> Self {
        Self {
            fetcher: GitHubLoopFetcher::new(loop_http_client()),
            adapter_connections,
            loops,
            tags,
            git_host_tokens,
        }
    }
}

fn loop_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("moatless-vibe-backend")
        .build()
        .unwrap_or_else(|e| {
            warn!(error = %e, "Loop sync: failed to build reqwest client, using default");
            reqwest::Client::new()
        })
}
