//! The typed error surface of every lifecycle operation.
//!
//! Errors are **classifiable, not stringly**: the reconciler's attempt cap and
//! the schedule runner's failure metrics need error classes, and HTTP callers
//! map strict failures to a response while background callers log best-effort.
//! The class vocabulary and the runtime seam's own failure live with that seam,
//! in `moatless_sandbox_runtime::runtime`.

use thiserror::Error;

use moatless_types::task::TaskStatus;

use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::runtime::{ErrorClass, RuntimeError};
use moatless_sandbox_runtime::types::OperationKind;

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

/// How a lifecycle failure reaches an HTTP caller. Three map to a client-visible
/// status; everything else keeps the whole chain behind a 500.
impl From<LifecycleError> for SandboxError {
    fn from(value: LifecycleError) -> Self {
        match &value {
            LifecycleError::TaskNotFound(id) => Self::NotFound(id.clone()),
            // The Closed guard rejected the operation — a client state error.
            LifecycleError::NotAllowed { .. } => Self::BadRequest(value.to_string()),
            // Another operation holds the per-Task lock — a client-visible
            // conflict (409), matching the task API.
            LifecycleError::OperationInProgress { .. } => Self::Conflict(value.to_string()),
            // Everything else is an internal failure: keep the lifecycle error
            // itself so the response boundary can log the whole chain,
            // including the `RuntimeError` a failed effect carries.
            _ => Self::Lifecycle(Box::new(value)),
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