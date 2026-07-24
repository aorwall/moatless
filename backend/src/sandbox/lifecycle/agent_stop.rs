//! [`AgentStopPort`] over the sandbox gRPC client.
//!
//! Graceful stop is lifecycle's ordering rule: `stop`/`remove` issue a
//! best-effort `StopAgent` before the runtime effect so the pod exits promptly
//! instead of riding termination grace into a SIGKILL. Every failure is
//! swallowed — it never blocks the transition.

use std::sync::Arc;

use async_trait::async_trait;

use crate::sandbox::access::SandboxConnector;

use super::orchestrator::AgentStopPort;

/// Issues `StopAgent` over the sandbox gRPC client.
pub struct GrpcAgentStop {
    connector: Arc<dyn SandboxConnector>,
}

impl GrpcAgentStop {
    pub fn new(connector: Arc<dyn SandboxConnector>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl AgentStopPort for GrpcAgentStop {
    async fn stop_agent(&self, task_id: &str) {
        match self.connector.sandbox_client(task_id).await {
            Ok(client) => {
                if let Err(e) = client.stop_agent().await {
                    tracing::debug!(task_id, error = %e, "graceful StopAgent failed (continuing)");
                }
            }
            Err(e) => {
                tracing::debug!(task_id, error = %e, "no sandbox client for StopAgent (continuing)");
            }
        }
    }
}
