//! [`AgentStopPort`] over the sandbox gRPC client.
//!
//! Graceful stop is lifecycle's ordering rule: `stop`/`remove` issue a
//! best-effort `StopAgent` before the runtime effect so the pod exits promptly
//! instead of riding termination grace into a SIGKILL. Every failure is
//! swallowed — it never blocks the transition.

use std::sync::Arc;

use async_trait::async_trait;

use crate::sandbox::provider::SandboxProvider;

use super::orchestrator::AgentStopPort;

/// Issues `StopAgent` over the sandbox gRPC client.
pub struct GrpcAgentStop {
    provider: Arc<dyn SandboxProvider>,
}

impl GrpcAgentStop {
    pub fn new(provider: Arc<dyn SandboxProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl AgentStopPort for GrpcAgentStop {
    async fn stop_agent(&self, task_id: &str) {
        match self.provider.sandbox_client(task_id).await {
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
