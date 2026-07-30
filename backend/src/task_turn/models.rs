//! Task turn domain model.
//!
//! A *turn* is one user-message-and-agent-response cycle. Its lifecycle state
//! lives in the `task_turns` table, written transactionally at the gRPC event
//! ingress so it is authoritative rather than derived from the event log.

/// Lifecycle status of a single task turn — the `task_turns.status` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    /// The agent is processing the turn.
    Active,
    /// The agent finished successfully.
    Completed,
    /// The agent run errored (harness or agent-level).
    Error,
    /// The user stopped the turn.
    Stopped,
    /// The pod was interrupted mid-turn (still non-terminal — it resumes on
    /// reschedule, or the reaper reconciles it as a backstop).
    Interrupted,
}

impl TurnStatus {
    /// The `status` string stored in `task_turns`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Stopped => "stopped",
            Self::Interrupted => "interrupted",
        }
    }

    /// Parse a `task_turns.status` string back into the enum. `None` for a
    /// value this build doesn't know (a row written by a newer backend).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "completed" => Some(Self::Completed),
            "error" => Some(Self::Error),
            "stopped" => Some(Self::Stopped),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    /// Whether a turn that ended in this state left the agent session cut off
    /// mid-flight. Resuming such a session replays a transcript whose last
    /// entry is an unanswered tool call, so the next run must start fresh.
    pub fn cut_off_session(self) -> bool {
        matches!(self, Self::Error | Self::Stopped)
    }
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
    match (event_type, sub_type) {
        ("taskTurn", "active") => Some(TurnTransition::Open),
        ("taskTurn", "turn_interrupted") => Some(TurnTransition::Interrupted),
        ("taskTurn", "completed") | ("agent", "completed") => {
            Some(TurnTransition::Terminal(TurnStatus::Completed))
        }
        ("taskTurn", "error") | ("agent", "error") => {
            Some(TurnTransition::Terminal(TurnStatus::Error))
        }
        ("taskTurn", "stopped") | ("agent", "stopped") => {
            Some(TurnTransition::Terminal(TurnStatus::Stopped))
        }
        _ => None,
    }
}

/// A task whose most recent turn is still open (`active` or `interrupted`) and
/// older than a cutoff — the reaper's stuck-turn backstop. Surfaced by
/// [`crate::task_turn::dao::TaskTurnDao::list_unclosed_active_turns`]
/// (and, for a single task, by the event-log based `active_turn_for_task`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnclosedTurn {
    pub task_id: String,
    pub turn_number: Option<i32>,
    pub agent_type: Option<String>,
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
