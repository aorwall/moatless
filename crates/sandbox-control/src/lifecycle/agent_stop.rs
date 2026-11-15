//! [`AgentStopPort`] over the sandbox gRPC client.
//!
//! Graceful stop is lifecycle's ordering rule: `stop`/`remove` issue a
//! best-effort `StopAgent` before the runtime effect so the pod exits promptly
//! instead of riding termination grace into a SIGKILL. Every failure is
//! swallowed — it never blocks the transition.

use std::sync::Arc;

use async_trait::async_trait;
use sandbox_proto::AgentStopSource;

use moatless_ports::sandbox::AgentStopOutcome;
use moatless_sandbox_runtime::client::SandboxConnector;
use moatless_types::sandbox::SandboxStopReason;

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

/// Map lifecycle's stop reason onto the proto stop source the sandbox echoes
/// back on the terminal `agent.stopped` event.
fn stop_source(reason: SandboxStopReason) -> AgentStopSource {
    match reason {
        SandboxStopReason::User => AgentStopSource::User,
        SandboxStopReason::IdleReaper => AgentStopSource::IdleReaper,
        SandboxStopReason::Cleanup => AgentStopSource::Cleanup,
        SandboxStopReason::System => AgentStopSource::System,
        SandboxStopReason::Error => AgentStopSource::Error,
    }
}

#[async_trait]
impl AgentStopPort for GrpcAgentStop {
    async fn stop_agent(&self, task_id: &str, reason: SandboxStopReason) -> AgentStopOutcome {
        match self.connector.sandbox_client(task_id).await {
            Ok(client) => match client
                .stop_agent_for_task(Some(task_id), stop_source(reason))
                .await
            {
                Ok(()) => AgentStopOutcome::Stopped,
                Err(e) => {
                    tracing::debug!(task_id, error = %e, "graceful StopAgent failed (continuing)");
                    outcome_of_failure(e.code())
                }
            },
            Err(e) => {
                tracing::debug!(task_id, error = %e, "no sandbox client for StopAgent (continuing)");
                AgentStopOutcome::Stopped
            }
        }
    }
}

/// A deadline is the one failure that means a sandbox is up with an agent still
/// working: both the client's and the sandbox's own stop wait end in one. Any
/// other code is a sandbox that cannot be reached, which runs no agent.
fn outcome_of_failure(code: tonic::Code) -> AgentStopOutcome {
    match code {
        tonic::Code::DeadlineExceeded => AgentStopOutcome::Unresponsive,
        _ => AgentStopOutcome::Stopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_deadline_reads_as_an_agent_ignoring_its_stop() {
        assert_eq!(
            outcome_of_failure(tonic::Code::DeadlineExceeded),
            AgentStopOutcome::Unresponsive
        );
        for code in [
            tonic::Code::Unavailable,
            tonic::Code::NotFound,
            tonic::Code::Internal,
        ] {
            assert_eq!(
                outcome_of_failure(code),
                AgentStopOutcome::Stopped,
                "{code:?}"
            );
        }
    }
}
