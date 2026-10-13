//! Authenticated HTTP caller adapters for Loop synchronization.

use super::{LoopSyncService, error::LoopError, models::ParsedLoop};
use moatless_api_schemas::loops::LoopSyncSummary;
use moatless_core::http_auth::AuthUser;
use moatless_workspace_types::repository::Repository;

impl LoopSyncService {
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
