//! Message-wake causal precedence.
//!
//! Message-wake durability does not rest on in-memory requeues: messages are
//! already durable rows with delivery acknowledgement, and **undelivered
//! messages are reconciler input**. The reconciler compares DB-owned Message
//! `id`/`created_at` with the DB-owned intent timestamp
//! (`sandbox_desired_state_updated_at`), avoiding process clocks and
//! user-supplied timestamps.

use chrono::{DateTime, Utc};

use moatless_backend_core::models::{SandboxDesiredState, TaskStatus};

use super::types::ArchiveRetention;

/// The persisted intent a pending message is evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct WakeIntent {
    pub task_status: TaskStatus,
    pub desired_state: SandboxDesiredState,
    /// The archive retention persisted with a `Removed` intent. `None` for
    /// non-Removed intents.
    pub retention: Option<ArchiveRetention>,
    /// DB-owned `sandbox_desired_state_updated_at`.
    pub desired_state_updated_at: DateTime<Utc>,
}

/// Does a pending user message supersede the persisted intent — i.e. should the
/// reconciler converge the Task to Running and (re-)notify?
///
/// Causal precedence is explicit:
/// - only Open/Error Tasks participate; **Closed is never resurrected**;
/// - a pending message **older** than `sandbox_desired_state_updated_at` cannot
///   supersede a later stop/remove;
/// - a newer message may supersede Stopped;
/// - a newer message may supersede Removed **only** when the removal retention
///   is `RetainArchives` (the durable explicit wake after a soft removal) — an
///   in-progress `DeleteArchives` removal is never superseded, deletion wins.
pub fn message_supersedes_intent(intent: WakeIntent, message_created_at: DateTime<Utc>) -> bool {
    // Closed is never resurrected by a pending message.
    if intent.task_status == TaskStatus::Closed {
        return false;
    }

    match intent.desired_state {
        // Already toward Running — the message is causally fine; the reconciler
        // converges to Running and notifies.
        SandboxDesiredState::Running => true,
        // A newer message supersedes Stopped; an older one cannot undo a later
        // explicit stop.
        SandboxDesiredState::Stopped => message_created_at > intent.desired_state_updated_at,
        // Removed is superseded only for a soft (RetainArchives) removal, and
        // only by a strictly newer message.
        SandboxDesiredState::Removed => {
            intent.retention == Some(ArchiveRetention::RetainArchives)
                && message_created_at > intent.desired_state_updated_at
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("valid ts")
    }

    fn intent(
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        retention: Option<ArchiveRetention>,
    ) -> WakeIntent {
        WakeIntent {
            task_status,
            desired_state,
            retention,
            desired_state_updated_at: at(1_000),
        }
    }

    // ---- Closed is never resurrected ----

    #[test]
    fn closed_task_is_never_resurrected() {
        for desired in [
            SandboxDesiredState::Running,
            SandboxDesiredState::Stopped,
            SandboxDesiredState::Removed,
        ] {
            let i = intent(
                TaskStatus::Closed,
                desired,
                Some(ArchiveRetention::RetainArchives),
            );
            // Even a message far newer than the intent cannot wake a Closed task.
            assert!(!message_supersedes_intent(i, at(9_999)), "{desired:?}");
        }
    }

    // ---- Stopped ----

    #[test]
    fn newer_message_supersedes_stopped() {
        let i = intent(TaskStatus::Open, SandboxDesiredState::Stopped, None);
        assert!(message_supersedes_intent(i, at(1_001)));
    }

    #[test]
    fn older_message_cannot_supersede_a_later_stop() {
        // The load-bearing case: a later explicit stop is never undone by an
        // older pending message.
        let i = intent(TaskStatus::Open, SandboxDesiredState::Stopped, None);
        assert!(!message_supersedes_intent(i, at(999)));
        // Exactly equal is not strictly newer → does not supersede.
        assert!(!message_supersedes_intent(i, at(1_000)));
    }

    #[test]
    fn error_task_participates_like_open() {
        let i = intent(TaskStatus::Error, SandboxDesiredState::Stopped, None);
        assert!(message_supersedes_intent(i, at(1_001)));
    }

    // ---- Removed ----

    #[test]
    fn newer_message_supersedes_soft_removed() {
        // RetainArchives removal + newer message → durable explicit wake.
        let i = intent(
            TaskStatus::Open,
            SandboxDesiredState::Removed,
            Some(ArchiveRetention::RetainArchives),
        );
        assert!(message_supersedes_intent(i, at(1_001)));
    }

    #[test]
    fn delete_archives_removal_is_never_superseded() {
        // Deletion wins — a newer message cannot undo a hard removal.
        let i = intent(
            TaskStatus::Open,
            SandboxDesiredState::Removed,
            Some(ArchiveRetention::DeleteArchives),
        );
        assert!(!message_supersedes_intent(i, at(9_999)));
    }

    #[test]
    fn older_message_cannot_supersede_soft_removed() {
        let i = intent(
            TaskStatus::Open,
            SandboxDesiredState::Removed,
            Some(ArchiveRetention::RetainArchives),
        );
        assert!(!message_supersedes_intent(i, at(999)));
    }

    #[test]
    fn removed_without_retention_is_not_superseded() {
        // Absent retention is treated as not-RetainArchives → never superseded.
        let i = intent(TaskStatus::Open, SandboxDesiredState::Removed, None);
        assert!(!message_supersedes_intent(i, at(9_999)));
    }

    // ---- Running ----

    #[test]
    fn running_intent_is_always_eligible_for_open_error() {
        // Nothing to supersede — the reconciler converges to Running and
        // notifies regardless of message age.
        let i = intent(TaskStatus::Open, SandboxDesiredState::Running, None);
        assert!(message_supersedes_intent(i, at(1)));
        let i = intent(TaskStatus::Error, SandboxDesiredState::Running, None);
        assert!(message_supersedes_intent(i, at(1)));
    }
}
