//! Classifiable lifecycle and runtime errors.
//!
//! Errors are **classifiable, not stringly**: the reconciler's attempt cap and
//! the schedule runner's failure metrics need error classes, and HTTP callers
//! map strict failures to a response while background callers log best-effort.

use thiserror::Error;

use moatless_tasks::task::models::TaskStatus;

use super::types::OperationKind;

/// Coarse error class driving retry policy.
///
/// The reconciler retries only *transient* classes (runtime/infra) under its
/// attempt cap and never *permanent* ones (the Closed guard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient infrastructure/runtime failure — safe to retry with backoff.
    Transient,
    /// Permanent failure — retrying cannot help (bad plan, Closed guard,
    /// missing task). The reconciler must not spin on these.
    Permanent,
    /// Per-Task lock contention — not a failure of the operation itself;
    /// resolved by serialization, not by the attempt cap.
    Contention,
}

impl ErrorClass {
    pub fn is_transient(self) -> bool {
        matches!(self, Self::Transient)
    }
}

/// Failure raised by a [`super::runtime::SandboxRuntime`] mutation.
///
/// **Failure diagnostics belong here**, never on the success
/// [`super::runtime::RuntimeOutcome`]. Adapters have no event publisher; they
/// surface everything through this type and lifecycle republishes it.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// A transient infrastructure failure (API timeout, conflict, network).
    #[error("runtime infrastructure error: {0}")]
    Infrastructure(String),

    /// The runtime rejected the request permanently (e.g. an unbuildable
    /// plan). Retrying cannot help.
    #[error("runtime rejected the request: {0}")]
    Rejected(String),

    /// A delete was accepted but the resource did not reach observed absence
    /// within the wait bound. Strict removal treats accepted-but-not-absent as
    /// **not** success, so this is the failure a stuck deletion surfaces.
    #[error("resource still present after deletion was accepted: {0}")]
    NotAbsent(String),
}

impl RuntimeError {
    pub fn class(&self) -> ErrorClass {
        match self {
            // A stuck-but-accepted deletion is retriable — the reconciler
            // re-drives remove and re-awaits absence.
            Self::Infrastructure(_) | Self::NotAbsent(_) => ErrorClass::Transient,
            Self::Rejected(_) => ErrorClass::Permanent,
        }
    }

    /// Bounded cause slug published as the `Error` fact's `lifecycle_event`.
    /// Bounded because it becomes a UI label key and a log correlation field.
    pub fn cause(&self) -> &'static str {
        match self {
            Self::Infrastructure(_) => "runtime_infrastructure",
            Self::Rejected(_) => "runtime_rejected",
            Self::NotAbsent(_) => "runtime_not_absent",
        }
    }
}

/// The typed error surface of every [`super::Lifecycle`] operation.
#[derive(Debug, Error)]
pub enum LifecycleError {
    /// The task row does not exist. Permanent.
    #[error("task not found: {0}")]
    TaskNotFound(String),

    /// The operation is not permitted for the task's current status — a
    /// `start`/`restart`/`redeploy` against a Closed task. Permanent (the
    /// Closed guard is per-operation; stop/remove stay permitted). The
    /// [`TaskStatus`] is retained as a domain value and formatted only at the
    /// response/logging boundary.
    #[error("operation {kind} not allowed for task {task_id} in status {status}")]
    NotAllowed {
        kind: OperationKind,
        task_id: String,
        status: TaskStatus,
    },

    /// Loading the persisted intent failed — a *read*, so no write occurred.
    /// Transient.
    #[error("failed to load intent for task {task_id}: {reason}")]
    IntentLoad { task_id: String, reason: String },

    /// Allocating the operation generation failed. Transient — no runtime
    /// effect (allocation precedes the intent write).
    #[error("failed to allocate generation for task {task_id}: {reason}")]
    GenerationUnavailable { task_id: String, reason: String },

    /// The persisted intent write failed. Transient — the operation left no
    /// runtime effect (intent is written before effect).
    #[error("failed to persist intent for task {task_id}: {reason}")]
    IntentWrite { task_id: String, reason: String },

    /// The runtime mutation failed after intent was persisted. Intent remains
    /// the requested target; the reconciler (level ops) or the caller (edges)
    /// retries. Class is inherited from the underlying [`RuntimeError`].
    #[error("runtime effect failed for task {task_id}: {source}")]
    Runtime {
        task_id: String,
        #[source]
        source: RuntimeError,
    },

    /// The completion transaction (persist the deterministic completion fact +
    /// clear the marker) failed. Transient — intent and marker survive for the
    /// reconciler, and the completion event was **not** published.
    #[error("failed to commit completion for task {task_id}: {reason}")]
    CompletionCommit { task_id: String, reason: String },

    /// Another lifecycle operation holds the per-Task lock. Carries the
    /// conflicting operation's `kind` so callers can react without a marker
    /// read — HTTP maps to 409, background to a bounded retry or a return to
    /// the reconciler. Deliberately **not** "Busy" (which already means
    /// agent/process activity here). The `kind` is best-effort: in the brief
    /// window between lock acquisition and the intent write, the holder's marker
    /// is not yet persisted, so `kind` reports the requester's own kind.
    #[error("operation in progress on task: conflicting {kind}")]
    OperationInProgress { kind: OperationKind },

    /// The cold transition could not establish that the workspace archive is
    /// current — the head read failed, or the rescue upload could not be
    /// presigned. Transient, and deliberately *not* a decision to delete: a
    /// storage backend that cannot answer "is the archive there?" has not
    /// answered "no". The volume survives for the next cycle to retry.
    #[error("cannot prove workspace archive for task {task_id}: {reason}")]
    ArchiveUnproven { task_id: String, reason: String },

    /// The detached transition task failed to join (panicked). A bug; permanent.
    #[error("lifecycle transition failed for task {task_id}: {reason}")]
    Interrupted { task_id: String, reason: String },
}

impl LifecycleError {
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::TaskNotFound(_) | Self::NotAllowed { .. } | Self::Interrupted { .. } => {
                ErrorClass::Permanent
            }
            Self::IntentLoad { .. }
            | Self::GenerationUnavailable { .. }
            | Self::IntentWrite { .. }
            | Self::ArchiveUnproven { .. }
            | Self::CompletionCommit { .. } => ErrorClass::Transient,
            Self::Runtime { source, .. } => source.class(),
            Self::OperationInProgress { .. } => ErrorClass::Contention,
        }
    }

    /// Whether the reconciler's attempt cap should retry this failure.
    pub fn is_retriable(&self) -> bool {
        self.class().is_transient()
    }

    /// Bounded cause slug published as the `Error` fact's `lifecycle_event`.
    /// A runtime failure defers to its [`RuntimeError`] so the adapter's own
    /// vocabulary survives the wrap.
    pub fn cause(&self) -> &'static str {
        match self {
            Self::TaskNotFound(_) => "task_not_found",
            Self::NotAllowed { .. } => "not_allowed",
            Self::IntentLoad { .. } => "intent_load",
            Self::GenerationUnavailable { .. } => "generation_unavailable",
            Self::IntentWrite { .. } => "intent_write",
            Self::Runtime { source, .. } => source.cause(),
            Self::ArchiveUnproven { .. } => "archive_unproven",
            Self::CompletionCommit { .. } => "completion_commit",
            Self::OperationInProgress { .. } => "operation_in_progress",
            Self::Interrupted { .. } => "interrupted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_errors_are_not_retriable() {
        assert!(!LifecycleError::TaskNotFound("t".into()).is_retriable());
        assert!(
            !LifecycleError::NotAllowed {
                kind: OperationKind::Start,
                task_id: "t".into(),
                status: TaskStatus::Closed,
            }
            .is_retriable()
        );
        assert!(
            !LifecycleError::Interrupted {
                task_id: "t".into(),
                reason: "panic".into(),
            }
            .is_retriable()
        );
    }

    #[test]
    fn transient_errors_are_retriable() {
        // Reads and writes to the store are transient — none left a runtime
        // effect, and a completion-commit failure keeps intent+marker.
        assert!(
            LifecycleError::IntentLoad {
                task_id: "t".into(),
                reason: "conn reset".into(),
            }
            .is_retriable()
        );
        assert!(
            LifecycleError::GenerationUnavailable {
                task_id: "t".into(),
                reason: "seq locked".into(),
            }
            .is_retriable()
        );
        assert!(
            LifecycleError::CompletionCommit {
                task_id: "t".into(),
                reason: "deadlock".into(),
            }
            .is_retriable()
        );
        assert!(
            LifecycleError::IntentWrite {
                task_id: "t".into(),
                reason: "deadlock".into(),
            }
            .is_retriable()
        );
        // An unreadable archive must come back next cycle: the volume it would
        // have replaced is still there.
        assert!(
            LifecycleError::ArchiveUnproven {
                task_id: "t".into(),
                reason: "s3 timeout".into(),
            }
            .is_retriable()
        );
        assert!(
            LifecycleError::Runtime {
                task_id: "t".into(),
                source: RuntimeError::Infrastructure("timeout".into()),
            }
            .is_retriable()
        );
        // A stuck-but-accepted deletion is transient so the reconciler keeps
        // re-driving remove until observed absence.
        assert!(
            LifecycleError::Runtime {
                task_id: "t".into(),
                source: RuntimeError::NotAbsent("pvc".into()),
            }
            .is_retriable()
        );
    }

    #[test]
    fn rejected_runtime_is_permanent() {
        assert_eq!(
            LifecycleError::Runtime {
                task_id: "t".into(),
                source: RuntimeError::Rejected("bad image".into()),
            }
            .class(),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn contention_is_its_own_class() {
        let e = LifecycleError::OperationInProgress {
            kind: OperationKind::Stop,
        };
        assert_eq!(e.class(), ErrorClass::Contention);
        // Contention is not "transient" — it is resolved by serialization, not
        // by consuming an attempt-cap retry.
        assert!(!e.is_retriable());
    }
}
