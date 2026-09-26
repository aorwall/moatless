//! Task turn domain model.
//!
//! A *turn* is one user-message-and-agent-response cycle. Its lifecycle state
//! lives in the `task_turns` table, written transactionally at the gRPC event
//! ingress so it is authoritative rather than derived from the event log.

/// Lifecycle status of a single task turn — the `task_turns.status` column.
///
/// Defined in `moatless_types` so `moatless_types::running` can name it, since
/// that rule sits below this crate.
pub use moatless_types::task::TurnStatus;

use chrono::{DateTime, Utc};

/// A task's newest turn, closed, with activity recorded after it closed.
///
/// The row says the run finished yet something happened since; only the agent
/// itself can say whether that means it's still in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledTurn {
    pub task_id: String,
    pub turn_number: i32,
    pub agent_profile: Option<String>,
    /// The run recorded on this closed turn, where one was. A heartbeat naming
    /// a different run describes work this row never covered.
    pub run_id: Option<String>,
}

/// The Task band's half of the running rule, for one task.
///
/// The [`moatless_types::running::RunningInputs`] fields this band owns;
/// optional means "this reader cannot say", so an empty answer is legal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnSignals {
    pub turn_status: Option<TurnStatus>,
    pub turn_ended_at: Option<DateTime<Utc>>,
    pub latest_user_message_at: Option<DateTime<Utc>>,
}

/// A turn state transition derived from an inbound event's `(type, sub_type)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnTransition {
    /// A turn started — upsert an `active` row (`taskTurn.active`).
    Open,
    /// The pod was interrupted mid-turn — mark an existing active turn
    /// `interrupted` (`taskTurn.turn_interrupted`).
    Interrupted,
    /// The turn reached a terminal state — `agent`/`taskTurn`
    /// `.completed`/`.error`/`.stopped`.
    Terminal(TurnStatus),
}

impl TurnTransition {
    /// Whether this transition closes the turn (and returns the task to Open).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal(_))
    }
}

/// Classify an event's `(event_type, sub_type)` into a turn transition.
///
/// `None` for non-state-bearing events; the single source of truth for that
/// question, shared by the transactional gRPC ingress and the idempotent
/// dispatcher fallback.
pub fn classify_turn_transition(event_type: &str, sub_type: &str) -> Option<TurnTransition> {
    // The `(type, sub_type)` table itself lives on `TurnStatus::from_event`,
    // because the event publisher reads the same one to say whether an event
    // reports a working agent. What is added here is what each state *does* to
    // the row, which is this band's alone.
    match TurnStatus::from_event(event_type, sub_type)? {
        TurnStatus::Active => Some(TurnTransition::Open),
        TurnStatus::Interrupted => Some(TurnTransition::Interrupted),
        terminal => Some(TurnTransition::Terminal(terminal)),
    }
}

/// Re-exported from [`crate::task_event::models`], which owns the type.
/// [`crate::task_turn::operations::TaskTurns::list_unclosed_active_turns`] is
/// the authoritative fleet-wide producer.
pub use crate::task_event::models::UnclosedTurn;

/// A state-bearing event applied transactionally at the gRPC ingress.
///
/// `event_id` is generated before the publish so the written row and published
/// event share one id, which the dispatcher's `ON CONFLICT DO NOTHING` dedups.
#[derive(Debug, Clone)]
pub struct TurnStateEvent {
    pub event_id: String,
    pub task_id: String,
    pub event_type: String,
    pub sub_type: String,
    pub is_error: bool,
    pub agent_profile: Option<String>,
    pub turn_number: i32,
    pub payload: serde_json::Value,
    pub timestamp: String,
    pub transition: TurnTransition,
}

/// What [`crate::task_turn::operations::TaskTurns::fork_point`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForkPoint {
    /// The Task's newest turn, when it hasn't reached a terminal status.
    ///
    /// A cut reaching it is refused — it's still writing rows the copy would
    /// race — a cut below it is allowed.
    pub open_turn: Option<i32>,
    /// The turn the cut keeps — the requested one, or the newest terminal one.
    /// `None` means the requested turn has no row, or the Task has no terminal
    /// turn at all; the caller tells the two apart by whether it asked for one.
    pub turn: Option<i32>,
    /// The harness's own id for that turn — Codex's `lastTurnId`. `None` for
    /// Claude, and for a turn that ran before the column existed.
    pub agent_turn_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_state_bearing_events() {
        assert_eq!(
            classify_turn_transition("taskTurn", "active"),
            Some(TurnTransition::Open)
        );
        assert_eq!(
            classify_turn_transition("taskTurn", "turn_interrupted"),
            Some(TurnTransition::Interrupted)
        );
        assert_eq!(
            classify_turn_transition("agent", "completed"),
            Some(TurnTransition::Terminal(TurnStatus::Completed))
        );
        assert_eq!(
            classify_turn_transition("agent", "error"),
            Some(TurnTransition::Terminal(TurnStatus::Error))
        );
        assert_eq!(
            classify_turn_transition("agent", "stopped"),
            Some(TurnTransition::Terminal(TurnStatus::Stopped))
        );
        // Backend-published taskTurn terminals are classified too (idempotent
        // no-ops against the sandbox's agent.* terminal for the same turn).
        assert_eq!(
            classify_turn_transition("taskTurn", "completed"),
            Some(TurnTransition::Terminal(TurnStatus::Completed))
        );
    }

    #[test]
    fn classify_ignores_non_state_events() {
        assert_eq!(classify_turn_transition("agent", "message"), None);
        assert_eq!(classify_turn_transition("agent", "started"), None);
        assert_eq!(classify_turn_transition("agent", "session_init"), None);
        assert_eq!(classify_turn_transition("sandbox", "ready"), None);
        assert_eq!(classify_turn_transition("task", "metadata.updated"), None);
    }

    #[test]
    fn terminal_classification_matches_is_terminal() {
        assert!(TurnTransition::Terminal(TurnStatus::Completed).is_terminal());
        assert!(!TurnTransition::Open.is_terminal());
        assert!(!TurnTransition::Interrupted.is_terminal());
    }

    #[test]
    fn status_parses_back_from_its_stored_string() {
        for status in [
            TurnStatus::Active,
            TurnStatus::Completed,
            TurnStatus::Error,
            TurnStatus::Stopped,
            TurnStatus::Interrupted,
        ] {
            assert_eq!(TurnStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TurnStatus::parse("abandoned"), None);
    }

    #[test]
    fn stopped_and_errored_turns_cut_the_session_off() {
        assert!(TurnStatus::Stopped.cut_off_session());
        assert!(TurnStatus::Error.cut_off_session());
        assert!(!TurnStatus::Completed.cut_off_session());
        // An interrupted turn resumes on reschedule — it is not cut off.
        assert!(!TurnStatus::Interrupted.cut_off_session());
        assert!(!TurnStatus::Active.cut_off_session());
    }
}
