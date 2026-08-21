//! [`WorkspaceSyncPort`] over the sandbox gRPC client.
//!
//! The sandbox's workspace lives on a volume only the sandbox can read, and on
//! `stop`/`remove` that volume is about to be scaled away or deleted. Lifecycle
//! asks for a push to S3 at that moment — the last point where the live copy and
//! a process that can read it both exist.
//!
//! Best-effort, like `agent_stop`: a sandbox that cannot answer must not hold the
//! transition open. The push is bounded by [`SYNC_TIMEOUT`] because the caller is
//! on the stop path; the sandbox itself skips the transfer when nothing changed
//! since its last upload, so the usual case costs a round trip and a fingerprint
//! walk, not a tar.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::client::SandboxConnector;

use super::orchestrator::WorkspaceSyncPort;

/// Deadline for the pre-stop push.
///
/// Sized for a real transfer of a source tree, not for the fingerprint check
/// that usually answers it. Two minutes is well beyond the pod's own 30s
/// termination grace — this runs *before* the runtime effect, so it is not
/// competing with the drain, and a stop that waits a little is better than one
/// that abandons a workspace mid-upload.
const SYNC_TIMEOUT: Duration = Duration::from_mins(2);

/// Issues `SyncWorkspace` over the sandbox gRPC client.
pub struct GrpcWorkspaceSync {
    connector: Arc<dyn SandboxConnector>,
}

impl GrpcWorkspaceSync {
    pub fn new(connector: Arc<dyn SandboxConnector>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl WorkspaceSyncPort for GrpcWorkspaceSync {
    async fn sync_workspace(&self, task_id: &str) {
        let client = match self.connector.sandbox_client(task_id).await {
            Ok(client) => client,
            Err(e) => {
                tracing::debug!(task_id, error = %e, "no sandbox client for SyncWorkspace (continuing)");
                return;
            }
        };
        match client.sync_workspace(SYNC_TIMEOUT).await {
            Ok(response) if response.synced => {
                tracing::info!(
                    task_id,
                    bytes = response.bytes,
                    "pushed workspace to S3 before the runtime effect"
                );
            }
            Ok(_) => {
                tracing::debug!(task_id, "workspace unchanged or sync off; nothing pushed");
            }
            Err(e) => {
                tracing::debug!(task_id, error = %e, "pre-stop SyncWorkspace failed (continuing)");
            }
        }
    }
}
