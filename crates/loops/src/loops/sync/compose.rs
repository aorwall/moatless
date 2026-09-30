//! Loop synchronization dependencies.

use super::fetcher::GitHubLoopFetcher;
use super::reconcile::LoopSyncService;
use crate::loops::dao::LoopDao;
use moatless_adapter_model::connections::ConnectionRouting;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_ports::task::TagResolver;
use moatless_types::http::HttpTimeouts;
use moatless_workspace_ports::WorkspaceResolver;
use std::sync::Arc;

impl LoopSyncService {
    pub(crate) fn new(
        adapter_connections: Arc<ConnectionRouting>,
        loops: Arc<LoopDao>,
        tags: Arc<dyn TagResolver>,
        workspaces: Arc<dyn WorkspaceResolver>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        git_content: HttpTimeouts,
    ) -> Self {
        Self {
            fetcher: GitHubLoopFetcher::new(git_content),
            adapter_connections,
            loops,
            tags,
            workspaces,
            git_host_tokens,
        }
    }
}
