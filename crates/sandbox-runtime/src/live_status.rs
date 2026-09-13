//! Canonical live-status vocabulary and projection.
//!
//! Callers provide the durable Task intent plus raw Sandbox and Agent signals;
//! [`derive_live_status`] owns every rule that turns them into display status.

use moatless_ports::task::TurnSignals;
use moatless_types::running::{RunningInputs, Working, derive_running};
use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason};
use moatless_types::task::TaskStatus;

pub use moatless_types::sandbox::{AgentStatus, LiveStatus, SandboxStatus};

/// Combine raw runtime signals and durable Task intent into canonical live
/// status.
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
        // Not derived from anything: the caller reads them from the same
        // agent-status call and attaches them with `with_commands`.
        commands: Vec::new(),
    }
}

/// Reconcile a live status with the durable turn state behind it.
///
/// The second half of [`derive_live_status`]; only ever upgrades a `Running`
/// heartbeat's case, never demotes it. A turn stuck open behind a finished
/// agent is the reaper's to fix, not this function's.
pub fn apply_running_rule(
    status: LiveStatus,
    desired_state: SandboxDesiredState,
    stop_reason: Option<SandboxStopReason>,
    turns: TurnSignals,
) -> LiveStatus {
    let working = derive_running(&RunningInputs {
        desired_state: Some(desired_state),
        stop_reason,
        sandbox_up: Some(status.sandbox_status == SandboxStatus::Ready),
        turn_status: turns.turn_status,
        turn_ended_at: turns.turn_ended_at,
        latest_user_message_at: turns.latest_user_message_at,
        agent: status.agent_status,
    });

    let agent_status = match working {
        Working::Running => Some(AgentStatus::Running),
        Working::Waiting => Some(AgentStatus::Waiting),
        // The rule collapses "idle" and "unreachable" into one answer; this
        // surface has always told them apart, so the raw report is what carries
        // through. `Starting`/`Stopped`/`Error` only arise with the sandbox
        // down, where `derive_live_status` has already made this `None`.
        Working::Idle | Working::Starting | Working::Stopped | Working::Error => {
            status.agent_status
        }
    };

    LiveStatus {
        agent_status,
        ..status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use moatless_types::task::TurnStatus;

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

    // ── The running rule, applied to this surface ────────────────────────────
    //
    // These pin the half that makes the sandbox surface and the T3 conversation
    // list answer "is the agent working" the same way.

    fn ready(agent: Option<AgentStatus>) -> LiveStatus {
        derive(
            TaskStatus::Open,
            SandboxDesiredState::Running,
            SandboxStatus::Ready,
            None,
            agent,
        )
    }

    fn with_turn(status: TurnStatus) -> TurnSignals {
        TurnSignals {
            turn_status: Some(status),
            ..TurnSignals::default()
        }
    }

    fn apply(status: LiveStatus, turns: TurnSignals) -> LiveStatus {
        apply_running_rule(status, SandboxDesiredState::Running, None, turns)
    }

    #[test]
    fn an_open_turn_speaks_for_an_agent_that_cannot() {
        // The gap this closes: the pod is up, the heartbeat is idle or was
        // never reached, and the durable turn says a run is in flight. The
        // conversation list has always called that running; now so does this.
        for agent in [None, Some(AgentStatus::Unknown), Some(AgentStatus::Idle)] {
            let status = apply(ready(agent), with_turn(TurnStatus::Active));
            assert_eq!(
                status.agent_status,
                Some(AgentStatus::Running),
                "{agent:?} with an active turn"
            );
        }
    }

    #[test]
    fn a_user_message_owed_a_turn_reads_as_running() {
        let asked = Utc::now();
        let status = apply(
            ready(Some(AgentStatus::Idle)),
            TurnSignals {
                turn_status: Some(TurnStatus::Completed),
                turn_ended_at: Some(asked - chrono::Duration::seconds(30)),
                latest_user_message_at: Some(asked),
            },
        );

        assert_eq!(status.agent_status, Some(AgentStatus::Running));
    }

    #[test]
    fn a_settled_turn_leaves_a_live_heartbeat_alone() {
        // The rule never downgrades. The agent's own report that it is running
        // outranks a turn row that says the run ended — repairing that row is
        // the reaper's job, not this surface's.
        let status = apply(
            ready(Some(AgentStatus::Running)),
            with_turn(TurnStatus::Completed),
        );

        assert_eq!(status.agent_status, Some(AgentStatus::Running));
    }

    #[test]
    fn idle_and_unreachable_stay_told_apart() {
        // `Working::Idle` collapses both; this surface's three-field contract
        // has always distinguished them, and the rule must not erase that.
        for agent in [AgentStatus::Idle, AgentStatus::Unknown] {
            let status = apply(ready(Some(agent)), with_turn(TurnStatus::Completed));
            assert_eq!(status.agent_status, Some(agent));
        }
    }

    #[test]
    fn a_sandbox_that_is_not_ready_reports_no_agent_whatever_the_turn_says() {
        let down = derive(
            TaskStatus::Open,
            SandboxDesiredState::Stopped,
            SandboxStatus::Stopped,
            None,
            Some(AgentStatus::Running),
        );
        assert_eq!(down.agent_status, None, "precondition");

        let status = apply_running_rule(
            down,
            SandboxDesiredState::Stopped,
            Some(SandboxStopReason::User),
            with_turn(TurnStatus::Active),
        );

        assert_eq!(status.agent_status, None);
    }

    #[test]
    fn the_rule_touches_only_the_agent_field() {
        let before = derive(
            TaskStatus::Open,
            SandboxDesiredState::Running,
            SandboxStatus::Stopped,
            Some("runtime lookup failed"),
            None,
        );
        let after = apply(before.clone(), with_turn(TurnStatus::Active));

        assert_eq!(after.sandbox_status, before.sandbox_status);
        assert_eq!(after.sandbox_error, before.sandbox_error);
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
