//! Canonical live-status vocabulary and projection.
//!
//! Callers provide the durable Task intent plus raw Sandbox and Agent signals;
//! [`derive_live_status`] owns every rule that turns them into display status.

use moatless_backend_core::models::{SandboxDesiredState, TaskStatus};

pub use moatless_events::{AgentStatus, LiveStatus, SandboxStatus};

/// Combine raw runtime signals and durable Task intent into canonical live status.
pub fn derive_live_status(
    task_status: TaskStatus,
    desired_state: SandboxDesiredState,
    sandbox_status: SandboxStatus,
    resolver_error: Option<String>,
    agent: Option<AgentStatus>,
) -> LiveStatus {
    let sandbox_status =
        if task_status == TaskStatus::Closed && sandbox_status == SandboxStatus::NotCreated {
            SandboxStatus::Removed
        } else {
            sandbox_status
        };

    let agent_status = if sandbox_status == SandboxStatus::Ready {
        Some(agent.unwrap_or(AgentStatus::Unknown))
    } else {
        None
    };

    let sandbox_error = resolver_error.or_else(|| {
        (desired_state == SandboxDesiredState::Running
            && matches!(
                sandbox_status,
                SandboxStatus::Stopped | SandboxStatus::NotCreated
            ))
        .then(|| {
            "Sandbox is not running but its desired state is 'running' \
             (reconcile pending or failed)"
                .to_string()
        })
    });

    LiveStatus {
        sandbox_status,
        agent_status,
        sandbox_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn derive(
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        sandbox_status: SandboxStatus,
        resolver_error: Option<&str>,
        agent: Option<AgentStatus>,
    ) -> LiveStatus {
        derive_live_status(
            task_status,
            desired_state,
            sandbox_status,
            resolver_error.map(str::to_owned),
            agent,
        )
    }

    #[test]
    fn closed_task_with_no_sandbox_is_removed() {
        let status = derive(
            TaskStatus::Closed,
            SandboxDesiredState::Removed,
            SandboxStatus::NotCreated,
            None,
            None,
        );

        assert_eq!(status.sandbox_status, SandboxStatus::Removed);
        assert_eq!(status.agent_status, None);
        assert_eq!(status.sandbox_error, None);
    }

    #[test]
    fn non_ready_sandbox_makes_agent_status_not_applicable() {
        for sandbox_status in [
            SandboxStatus::NotCreated,
            SandboxStatus::Initializing,
            SandboxStatus::Stopped,
            SandboxStatus::Removing,
            SandboxStatus::Removed,
            SandboxStatus::Error,
        ] {
            let status = derive(
                TaskStatus::Open,
                SandboxDesiredState::Stopped,
                sandbox_status,
                None,
                Some(AgentStatus::Running),
            );

            assert_eq!(status.agent_status, None, "{sandbox_status:?}");
        }
    }

    #[test]
    fn ready_sandbox_preserves_resolved_agent_status() {
        for agent_status in [
            AgentStatus::Running,
            AgentStatus::Waiting,
            AgentStatus::Idle,
            AgentStatus::Unknown,
        ] {
            let status = derive(
                TaskStatus::Open,
                SandboxDesiredState::Running,
                SandboxStatus::Ready,
                None,
                Some(agent_status),
            );

            assert_eq!(status.agent_status, Some(agent_status));
        }
    }

    #[test]
    fn ready_sandbox_with_unreachable_agent_is_unknown() {
        let status = derive(
            TaskStatus::Open,
            SandboxDesiredState::Running,
            SandboxStatus::Ready,
            None,
            None,
        );

        assert_eq!(status.agent_status, Some(AgentStatus::Unknown));
    }

    #[test]
    fn running_intent_with_stopped_or_missing_sandbox_reports_reconcile_miss() {
        for sandbox_status in [SandboxStatus::Stopped, SandboxStatus::NotCreated] {
            let status = derive(
                TaskStatus::Open,
                SandboxDesiredState::Running,
                sandbox_status,
                None,
                None,
            );

            assert_eq!(
                status.sandbox_error.as_deref(),
                Some(
                    "Sandbox is not running but its desired state is 'running' \
                     (reconcile pending or failed)"
                ),
                "{sandbox_status:?}"
            );
        }
    }

    #[test]
    fn resolver_error_takes_precedence_over_reconcile_miss() {
        let status = derive(
            TaskStatus::Open,
            SandboxDesiredState::Running,
            SandboxStatus::Stopped,
            Some("runtime lookup failed"),
            None,
        );

        assert_eq!(
            status.sandbox_error.as_deref(),
            Some("runtime lookup failed")
        );
    }

    #[test]
    fn live_status_serializes_as_exact_three_field_contract() {
        let status = derive(
            TaskStatus::Open,
            SandboxDesiredState::Running,
            SandboxStatus::Ready,
            None,
            Some(AgentStatus::Idle),
        );

        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!({
                "sandboxStatus": "ready",
                "agentStatus": "idle",
                "sandboxError": null,
            })
        );
    }
}
