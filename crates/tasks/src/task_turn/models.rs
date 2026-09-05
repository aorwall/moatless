//! Task turn domain model.
//!
//! A *turn* is one user-message-and-agent-response cycle. Its lifecycle state
//! lives in the `task_turns` table, written transactionally at the gRPC event
//! ingress so it is authoritative rather than derived from the event log.

/// Lifecycle status of a single task turn — the `task_turns.status` column.
///
/// Defined in `moatless_types` so `moatless_types::running` can name it:
/// the rule that decides whether an agent is working reads a turn's state, and
/// that rule sits below this crate.
pub use moatless_types::task::TurnStatus;

use chrono::{DateTime, Utc};

/// A task's newest turn, closed, with activity recorded after it closed.
///
/// The candidate shape for the converse of an orphaned turn: the row says the
/// run finished, and something has happened since. Only the agent itself can
/// say whether that means a run is still in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledTurn {
    pub task_id: String,
    pub turn_number: i32,
    pub agent_type: Option<String>,
    /// The run recorded on this closed turn, where one was. A heartbeat naming
    /// a different run describes work this row never covered.
    pub run_id: Option<String>,
}

/// The Task band's half of the running rule, for one task.
///
/// The fields of [`moatless_types::running::RunningInputs`] this band
/// owns. Optional in the same sense the rule means — "this reader cannot say" —
/// so a task with no turns and no messages is a legal, empty answer.
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
/// Returns `None` for events that don't bear turn state. State-bearing events
/// are `taskTurn.*` and the sandbox's `agent.completed`/`error`/`stopped`. This
/// is the single source of truth for "is this event state-bearing", shared by
/// the gRPC ingress (which writes transactionally) and the Redis consumer
/// (which re-applies it as an idempotent fallback).
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
/// [`crate::task_turn::dao::TaskTurnDao::list_unclosed_active_turns`] is the
/// authoritative fleet-wide producer.
pub use crate::task_event::models::UnclosedTurn;

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
