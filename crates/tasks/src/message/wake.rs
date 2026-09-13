//! Wake failure handling for incoming messages.

#[cfg(test)]
use async_trait::async_trait;
use moatless_ports::sandbox::SandboxWaker;

/// Wake a message recipient and return any failure text.
pub async fn wake_for_message(waker: &dyn SandboxWaker, task_id: &str) -> Option<String> {
    waker.wake(task_id).await.err()
}

#[cfg(test)]
mod waker_tests {
    #![allow(clippy::expect_used, reason = "test recorder locks must succeed")]
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
            self.calls.lock().expect("lock").push(task_id.to_string());
            self.outcome.clone()
        }
    }

    /// The load-bearing assertion: the *task* id — not a message id or a turn
    /// id threaded nearby on the send path — is what reaches the port. That is
    /// the bug class this seam exists to catch; the success/None result is
    /// secondary.
    #[tokio::test]
    async fn wake_for_message_wakes_the_task_it_was_given() {
        let waker = RecordingSandboxWaker {
            calls: Mutex::new(Vec::new()),
            outcome: Ok(()),
        };

        let surfaced = wake_for_message(&waker, "task-42").await;

        assert_eq!(
            *waker.calls.lock().expect("lock"),
            vec!["task-42".to_string()]
        );
        assert_eq!(surfaced, None, "a successful wake surfaces no error");
    }

    /// Thinner — this is `Result::err` plumbing, kept for intent, not the
    /// assertion doing the work: a wake failure is handed back for the caller
    /// to persist rather than dropped.
    #[tokio::test]
    async fn wake_for_message_surfaces_the_failure_text() {
        let waker = RecordingSandboxWaker {
            calls: Mutex::new(Vec::new()),
            outcome: Err("sandbox unreachable".to_string()),
        };

        let surfaced = wake_for_message(&waker, "task-7").await;

        assert_eq!(
            *waker.calls.lock().expect("lock"),
            vec!["task-7".to_string()]
        );
        assert_eq!(surfaced.as_deref(), Some("sandbox unreachable"));
    }
}
