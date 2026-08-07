//! Consumer-owned port for waking a task's sandbox.
//!
//! Declared in the tasks band and implemented in sandbox-control
//! ([`crate::sandbox::notify::SandboxWakeService`]), so a tasks-band consumer —
//! message delivery, chiefly — can trigger a wake without naming the sandbox
//! layer above it.

use async_trait::async_trait;

/// Ensure a task's sandbox is running and notify it of new work.
///
/// A closed task is a no-op. On failure the wake returns a human-readable
/// string so the caller can decide whether to surface it to the user.
#[async_trait]
pub trait SandboxWaker: Send + Sync {
    async fn wake(&self, task_id: &str) -> Result<(), String>;
}

/// Wake `task_id`'s sandbox, returning the failure text (if any) for the caller
/// to persist or surface.
///
/// The one seam a message-send path uses, so its wake-failure handling is
/// reachable in a test through a [`SandboxWaker`] fake. It proves the seam — the
/// task id reaches the port unaltered and a failure is surfaced rather than
/// swallowed — not the wake behaviour itself, which lives in the sandbox-control
/// implementation and is recorded as still-unproven in `docs/deferred-changes.md`.
pub async fn wake_for_message(waker: &dyn SandboxWaker, task_id: &str) -> Option<String> {
    waker.wake(task_id).await.err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records the ids it is woken with and returns a canned outcome.
    struct RecordingSandboxWaker {
        calls: Mutex<Vec<String>>,
        outcome: Result<(), String>,
    }

    #[async_trait]
    impl SandboxWaker for RecordingSandboxWaker {
        async fn wake(&self, task_id: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(task_id.to_string());
            self.outcome.clone()
        }
    }

    /// The load-bearing assertion: the *task* id — not a message id or a turn id
    /// threaded nearby on the send path — is what reaches the port. That is the
    /// bug class this seam exists to catch; the success/None result is secondary.
    #[tokio::test]
    async fn wake_for_message_wakes_the_task_it_was_given() {
        let waker = RecordingSandboxWaker {
            calls: Mutex::new(Vec::new()),
            outcome: Ok(()),
        };

        let surfaced = wake_for_message(&waker, "task-42").await;

        assert_eq!(*waker.calls.lock().unwrap(), vec!["task-42".to_string()]);
        assert_eq!(surfaced, None, "a successful wake surfaces no error");
    }

    /// Thinner — this is `Result::err` plumbing, kept for intent, not the
    /// assertion doing the work: a wake failure is handed back for the caller to
    /// persist rather than dropped.
    #[tokio::test]
    async fn wake_for_message_surfaces_the_failure_text() {
        let waker = RecordingSandboxWaker {
            calls: Mutex::new(Vec::new()),
            outcome: Err("sandbox unreachable".to_string()),
        };

        let surfaced = wake_for_message(&waker, "task-7").await;

        assert_eq!(*waker.calls.lock().unwrap(), vec!["task-7".to_string()]);
        assert_eq!(surfaced.as_deref(), Some("sandbox unreachable"));
    }
}
