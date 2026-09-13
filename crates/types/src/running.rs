//! Whether a Task's agent is working.
//!
//! Every surface that shows live activity projects [`Working`] into its own
//! vocabulary; none re-derives it.

use chrono::{DateTime, Utc};

use crate::sandbox::{AgentStatus, SandboxDesiredState, SandboxStopReason};
use crate::task::TurnStatus;

/// What a Task's agent is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Working {
    /// A run is in flight.
    Running,
    /// A run is in flight and parked on a person's answer.
    Waiting,
    /// The Sandbox has not come up yet.
    Starting,
    /// The Sandbox is up and nothing is in flight.
    Idle,
    /// The Sandbox is not running, and was not stood down by a failure.
    Stopped,
    /// The Sandbox stood down after a failure.
    Error,
}

impl Working {
    /// Whether a run is in flight. `Waiting` counts.
    pub fn is_working(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
}

/// The signals [`derive_running`] reads.
///
/// `None` means *not known to this caller*, never *absent*: callers see
/// different halves of the picture, so an unknown field skips its clause rather
/// than reading as a negative answer.
#[derive(Debug, Clone, Default)]
pub struct RunningInputs {
    /// Durable Sandbox intent.
    pub desired_state: Option<SandboxDesiredState>,
    /// Why the intent moved away from `Running`.
    pub stop_reason: Option<SandboxStopReason>,
    /// Whether the Sandbox has come up.
    pub sandbox_up: Option<bool>,
    /// The newest turn's status.
    pub turn_status: Option<TurnStatus>,
    /// When the newest turn ended.
    pub turn_ended_at: Option<DateTime<Utc>>,
    /// When a person last asked for something.
    pub latest_user_message_at: Option<DateTime<Utc>>,
    /// The live agent heartbeat. It can only upgrade the answer to working.
    pub agent: Option<AgentStatus>,
}

/// Whether this Task's agent is working.
pub fn derive_running(inputs: &RunningInputs) -> Working {
    // Intent that is not Running settles it, without reading anything else.
    match inputs.desired_state {
        Some(SandboxDesiredState::Stopped | SandboxDesiredState::Removed) => {
            return match inputs.stop_reason {
                Some(SandboxStopReason::Error) => Working::Error,
                _ => Working::Stopped,
            };
        }
        Some(SandboxDesiredState::Running) | None => {}
    }

    // A heartbeat outranks a turn row that has not caught up, but says nothing
    // before the Sandbox is up.
    if inputs.sandbox_up != Some(false) {
        match inputs.agent {
            Some(AgentStatus::Running) => return Working::Running,
            Some(AgentStatus::Waiting) => return Working::Waiting,
            Some(AgentStatus::Idle | AgentStatus::Unknown) | None => {}
        }
    }

    if inputs.turn_status.is_some_and(TurnStatus::is_open) || turn_is_owed(inputs) {
        return Working::Running;
    }

    // Nothing is in flight, so a Sandbox still coming up reads as connecting.
    if inputs.sandbox_up == Some(false) {
        return Working::Starting;
    }

    Working::Idle
}

/// Whether a person is waiting for a turn that has not opened yet.
///
/// A turn opens on `taskTurn.active`, published from inside the Sandbox —
/// leaving a window between a person sending and the agent starting with no
/// open turn and work in flight.
fn turn_is_owed(inputs: &RunningInputs) -> bool {
    let Some(asked) = inputs.latest_user_message_at else {
        return false;
    };
    match inputs.turn_status {
        // Open already, which the caller asks about separately.
        Some(status) if status.is_open() => false,
        Some(_) => inputs.turn_ended_at.is_some_and(|ended| asked > ended),
        // Nothing has ever run here, so the first turn is still owed.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0)
            .single()
            .expect("valid timestamp")
    }

    /// A Sandbox that is up and running, with no turn and no message.
    fn up() -> RunningInputs {
        RunningInputs {
            desired_state: Some(SandboxDesiredState::Running),
            sandbox_up: Some(true),
            ..RunningInputs::default()
        }
    }

    #[test]
    fn an_open_turn_is_working() {
        for status in [TurnStatus::Active, TurnStatus::Interrupted] {
            let inputs = RunningInputs {
                turn_status: Some(status),
                ..up()
            };
            assert_eq!(derive_running(&inputs), Working::Running, "{status:?}");
        }
    }

    #[test]
    fn a_settled_turn_with_a_newer_message_is_owed() {
        let inputs = RunningInputs {
            turn_status: Some(TurnStatus::Completed),
            turn_ended_at: Some(at(100)),
            latest_user_message_at: Some(at(200)),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Running);
    }

    #[test]
    fn a_settled_turn_answered_after_the_message_is_idle() {
        let inputs = RunningInputs {
            turn_status: Some(TurnStatus::Completed),
            turn_ended_at: Some(at(200)),
            latest_user_message_at: Some(at(100)),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Idle);
    }

    #[test]
    fn a_turn_that_ended_without_recording_when_is_settled() {
        let inputs = RunningInputs {
            turn_status: Some(TurnStatus::Completed),
            turn_ended_at: None,
            latest_user_message_at: Some(at(200)),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Idle);
    }

    #[test]
    fn a_task_that_has_never_run_owes_its_first_turn() {
        let inputs = RunningInputs {
            latest_user_message_at: Some(at(200)),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Running);
    }

    #[test]
    fn a_sandbox_that_has_not_come_up_is_starting_not_idle() {
        let inputs = RunningInputs {
            sandbox_up: Some(false),
            turn_status: Some(TurnStatus::Completed),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Starting);
    }

    #[test]
    fn an_unknown_sandbox_up_never_reports_starting() {
        let inputs = RunningInputs {
            sandbox_up: None,
            turn_status: Some(TurnStatus::Completed),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Idle);
    }

    #[test]
    fn a_running_heartbeat_outranks_a_settled_turn() {
        let inputs = RunningInputs {
            turn_status: Some(TurnStatus::Completed),
            agent: Some(AgentStatus::Running),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Running);
    }

    #[test]
    fn a_waiting_heartbeat_is_working() {
        let inputs = RunningInputs {
            turn_status: Some(TurnStatus::Completed),
            agent: Some(AgentStatus::Waiting),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Waiting);
    }

    #[test]
    fn an_idle_or_unknown_heartbeat_never_closes_an_open_turn() {
        for agent in [AgentStatus::Idle, AgentStatus::Unknown] {
            let inputs = RunningInputs {
                turn_status: Some(TurnStatus::Active),
                agent: Some(agent),
                ..up()
            };
            assert_eq!(derive_running(&inputs), Working::Running, "{agent:?}");
        }
    }

    #[test]
    fn a_heartbeat_is_not_read_before_the_sandbox_is_up() {
        let inputs = RunningInputs {
            sandbox_up: Some(false),
            turn_status: Some(TurnStatus::Completed),
            agent: Some(AgentStatus::Running),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Starting);
    }

    #[test]
    fn intent_away_from_running_settles_the_answer() {
        for desired in [SandboxDesiredState::Stopped, SandboxDesiredState::Removed] {
            let inputs = RunningInputs {
                desired_state: Some(desired),
                // Every signal below says working, and none of them is read.
                turn_status: Some(TurnStatus::Active),
                agent: Some(AgentStatus::Running),
                ..up()
            };
            assert_eq!(derive_running(&inputs), Working::Stopped, "{desired:?}");
        }
    }

    #[test]
    fn a_sandbox_stood_down_by_a_failure_is_an_error() {
        let inputs = RunningInputs {
            desired_state: Some(SandboxDesiredState::Stopped),
            stop_reason: Some(SandboxStopReason::Error),
            ..up()
        };
        assert_eq!(derive_running(&inputs), Working::Error);
    }

    #[test]
    fn an_ordinary_stop_reason_is_not_an_error() {
        for reason in [
            SandboxStopReason::User,
            SandboxStopReason::IdleReaper,
            SandboxStopReason::Cleanup,
            SandboxStopReason::System,
        ] {
            let inputs = RunningInputs {
                desired_state: Some(SandboxDesiredState::Stopped),
                stop_reason: Some(reason),
                ..up()
            };
            assert_eq!(derive_running(&inputs), Working::Stopped, "{reason:?}");
        }
    }

    /// The live-status route reads the runtime and no turn rows. Its unknowns
    /// travel together, so an idle agent reads as idle rather than as owed.
    #[test]
    fn the_runtime_only_caller_resolves_from_its_heartbeat_alone() {
        let idle = RunningInputs {
            agent: Some(AgentStatus::Idle),
            ..up()
        };
        assert_eq!(derive_running(&idle), Working::Idle);

        let running = RunningInputs {
            agent: Some(AgentStatus::Running),
            ..up()
        };
        assert_eq!(derive_running(&running), Working::Running);
    }

    #[test]
    fn waiting_and_running_both_count_as_working() {
        assert!(Working::Running.is_working());
        assert!(Working::Waiting.is_working());
        for not_working in [
            Working::Starting,
            Working::Idle,
            Working::Stopped,
            Working::Error,
        ] {
            assert!(!not_working.is_working(), "{not_working:?}");
        }
    }
}
