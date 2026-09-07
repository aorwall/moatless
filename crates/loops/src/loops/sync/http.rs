//! Authenticated HTTP caller adapters for Loop synchronization.

use super::{LoopSyncService, error::LoopError, models::ParsedLoop};
use moatless_api_schemas::loops::LoopSyncSummary;
use moatless_core::http_auth::AuthUser;
use moatless_types::repository::Repository;

impl LoopSyncService {
    pub async fn sync_repo_loops(
        &self,
        repo: &Repository,
        auth: &AuthUser,
    ) -> Result<LoopSyncSummary, LoopError> {
        self.sync_repo_loops_for(repo, &auth.scope_context()).await
    }
    pub async fn reset_repo_loop(
        &self,
        repo: &Repository,
        auth: &AuthUser,
        source_key: &str,
    ) -> Result<bool, LoopError> {
        self.reset_repo_loop_for(repo, &auth.scope_context(), source_key)
            .await
    }
    pub async fn sync_loops(
        &self,
        repo: &Repository,
        auth: &AuthUser,
        loops: Vec<ParsedLoop>,
    ) -> Result<LoopSyncSummary, LoopError> {
        self.sync_loops_for(repo, &auth.scope_context(), loops)
            .await
    }
}
