//! [`WorkspaceSyncPort`] over the sandbox gRPC client.
//!
//! On `stop`/`remove` the sandbox's volume is about to be scaled away or
//! deleted, so lifecycle asks for a push to S3 at the last point where the live
//! copy and a process that can read it both exist.
//!
//! Best-effort, like `agent_stop`, and bounded by [`SYNC_TIMEOUT`]; the sandbox
//! itself skips the transfer when nothing changed, so the usual case costs a
//! round trip and a fingerprint walk, not a tar.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use moatless_sandbox_runtime::client::SandboxConnector;

use super::orchestrator::WorkspaceSyncPort;

/// Deadline for the pre-stop push.
///
/// Sized for a real transfer, not just the fingerprint check that usually
/// answers it; well beyond the pod's own termination grace, since this runs
/// before the runtime effect and is not competing with the drain.
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
