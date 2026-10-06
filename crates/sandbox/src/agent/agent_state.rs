//! Generic agent state machine — tracks status, session, cancel token, and
//! pending prompts.
//!
//! Owned by the coordinator service (`coordinator.rs`); harness-neutral so any
//! agent implementation can reuse it.

use tokio::sync::{Notify, RwLock};
use tokio_util::sync::CancellationToken;

use super::prompt::PendingPrompt;

use super::types::AgentStatus;
use crate::error::SandboxError;

/// Shared agent execution state, protected by `RwLock` for concurrent reads.
pub struct AgentState {
    inner: RwLock<AgentInner>,
    /// Woken on every transition out of a turn: idle, error or stop.
    turn_ended: Notify,
}

struct AgentInner {
    status: AgentStatus,
    last_session_id: Option<String>,
    /// The agent run currently in flight (see the coordinator's `run_id`). Read
    /// by paths that must attribute a message to the running turn without going
    /// through the lifecycle task — e.g. acknowledging an interactive prompt
    /// response.
    current_run_id: Option<String>,
    last_error: Option<String>,
    cancel_token: Option<CancellationToken>,
    pending_prompt: Option<PendingPrompt>,
    consecutive_errors: u32,
    messages_pending: bool,
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentState {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(AgentInner {
                status: AgentStatus::Idle,
                last_session_id: None,
                current_run_id: None,
                last_error: None,
                cancel_token: None,
                pending_prompt: None,
                consecutive_errors: 0,
                messages_pending: false,
            }),
            turn_ended: Notify::new(),
        }
    }

    pub async fn status(&self) -> AgentStatus {
        self.inner.read().await.status
    }

    pub async fn last_session_id(&self) -> Option<String> {
        self.inner.read().await.last_session_id.clone()
    }

    pub async fn last_error(&self) -> Option<String> {
        self.inner.read().await.last_error.clone()
    }

    pub async fn has_pending_prompt(&self) -> bool {
        self.inner.read().await.pending_prompt.is_some()
    }

    #[allow(dead_code)]
    pub async fn pending_prompt_tool_name(&self) -> Option<String> {
        self.inner
            .read()
            .await
            .pending_prompt
            .as_ref()
            .map(|p| serde_json::to_string(&p.tool_name).unwrap_or_default())
    }

    /// Report that the run is parked on a person, for as long as it is.
    ///
    /// Only transitions `Running` -> `Waiting`; any other status is left alone,
    /// since it already reflects a lifecycle transition past the prompt.
    pub async fn begin_waiting(&self) {
        let mut inner = self.inner.write().await;
        if inner.status == AgentStatus::Running {
            inner.status = AgentStatus::Waiting;
        }
    }

    pub async fn end_waiting(&self) {
        let mut inner = self.inner.write().await;
        if inner.status == AgentStatus::Waiting {
            inner.status = AgentStatus::Running;
        }
    }

    pub async fn set_running(&self, cancel_token: CancellationToken) {
        let mut inner = self.inner.write().await;
        inner.status = AgentStatus::Running;
        inner.cancel_token = Some(cancel_token);
        inner.last_error = None;
        inner.pending_prompt = None;
    }

    pub async fn set_idle(&self) {
        let mut inner = self.inner.write().await;
        inner.status = AgentStatus::Idle;
        inner.cancel_token = None;
        inner.pending_prompt = None;
        inner.consecutive_errors = 0;
        drop(inner);
        self.turn_ended.notify_waiters();
    }

    pub async fn set_stopped(&self) {
        let mut inner = self.inner.write().await;
        inner.status = AgentStatus::Stopped;
        // Drop the pending prompt sender, which will cause the receiver to error
        inner.pending_prompt = None;
        drop(inner);
        self.turn_ended.notify_waiters();
    }

    pub async fn set_error(&self, error: String) {
        let mut inner = self.inner.write().await;
        inner.status = AgentStatus::Idle;
        inner.last_error = Some(error);
        inner.cancel_token = None;
        inner.pending_prompt = None;
        inner.consecutive_errors += 1;
        drop(inner);
        self.turn_ended.notify_waiters();
    }

    /// Undo one error increment (saturating). Called when a retryable error
    /// will be retried, so it doesn't count toward the circuit breaker until
    /// retries are exhausted.
    pub async fn undo_error_increment(&self) {
        let mut inner = self.inner.write().await;
        inner.consecutive_errors = inner.consecutive_errors.saturating_sub(1);
    }

    /// Number of consecutive errors without a successful idle transition.
    pub async fn consecutive_errors(&self) -> u32 {
        self.inner.read().await.consecutive_errors
    }

    pub async fn set_session_id(&self, session_id: String) {
        self.inner.write().await.last_session_id = Some(session_id);
    }

    /// Forget the last session, which a rebuilt harness of another family
    /// cannot resume.
    pub async fn clear_session_id(&self) {
        self.inner.write().await.last_session_id = None;
    }

    pub async fn current_run_id(&self) -> Option<String> {
        self.inner.read().await.current_run_id.clone()
    }

    pub async fn set_run_id(&self, run_id: String) {
        self.inner.write().await.current_run_id = Some(run_id);
    }

    /// Park a prompt, unless one is already parked. Returns whether it was.
    ///
    /// Only one question stands at a time: replacing a standing prompt would
    /// drop its sender, which the waiting run reads as a cancellation, so a
    /// second concurrent call is refused and the model is told to ask again.
    pub async fn set_pending_prompt(&self, prompt: PendingPrompt) -> bool {
        let mut inner = self.inner.write().await;
        if inner.pending_prompt.is_some() {
            return false;
        }
        inner.pending_prompt = Some(prompt);
        true
    }

    /// Take the pending prompt (removes it from state).
    pub async fn take_pending_prompt(&self) -> Option<PendingPrompt> {
        self.inner.write().await.pending_prompt.take()
    }

    /// Flag that messages arrived while the agent was busy. Returns whether
    /// the flag was already set.
    pub async fn set_messages_pending(&self) -> bool {
        std::mem::replace(&mut self.inner.write().await.messages_pending, true)
    }

    /// Resolve once no turn is open: at once when the agent is neither running
    /// nor parked on a prompt, otherwise on the next transition out of the turn.
    pub async fn turn_ended(&self) {
        let notified = self.turn_ended.notified();
        let mut notified = std::pin::pin!(notified);
        // Registered before the read, so a transition between the two wakes it.
        notified.as_mut().enable();
        if !self.status().await.in_turn() {
            return;
        }
        notified.await;
    }

    /// Returns whether messages were pending and resets the flag.
    pub async fn take_messages_pending(&self) -> bool {
        let mut inner = self.inner.write().await;
        let pending = inner.messages_pending;
        inner.messages_pending = false;
        pending
    }

    /// Stop the running agent by cancelling its token.
    pub async fn stop(&self) -> Result<(), SandboxError> {
        let inner = self.inner.read().await;
        if let Some(ref token) = inner.cancel_token {
            token.cancel();
            Ok(())
        } else {
            Err(SandboxError::AgentNotRunning)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_state_is_idle() {
        let state = AgentState::new();
        assert_eq!(state.status().await, AgentStatus::Idle);
        assert!(state.last_session_id().await.is_none());
        assert!(state.last_error().await.is_none());
        assert!(!state.has_pending_prompt().await);
    }

    #[tokio::test]
    async fn default_same_as_new() {
        let state = AgentState::default();
        assert_eq!(state.status().await, AgentStatus::Idle);
    }

    #[tokio::test]
    async fn set_running_transitions_status() {
        let state = AgentState::new();
        let token = CancellationToken::new();
        state.set_running(token).await;
        assert_eq!(state.status().await, AgentStatus::Running);
    }

    #[tokio::test]
    async fn set_running_clears_error() {
        let state = AgentState::new();
        state.set_error("previous failure".to_string()).await;
        assert!(state.last_error().await.is_some());

        let token = CancellationToken::new();
        state.set_running(token).await;
        assert!(state.last_error().await.is_none());
    }

    #[tokio::test]
    async fn set_idle_clears_cancel_token() {
        let state = AgentState::new();
        let token = CancellationToken::new();
        state.set_running(token).await;
        assert_eq!(state.status().await, AgentStatus::Running);

        state.set_idle().await;
        assert_eq!(state.status().await, AgentStatus::Idle);
    }

    #[tokio::test]
    async fn set_stopped_transitions_status() {
        let state = AgentState::new();
        let token = CancellationToken::new();
        state.set_running(token).await;

        state.set_stopped().await;
        assert_eq!(state.status().await, AgentStatus::Stopped);
    }

    #[tokio::test]
    async fn set_error_records_message_and_goes_idle() {
        let state = AgentState::new();
        state.set_error("something broke".to_string()).await;

        assert_eq!(state.status().await, AgentStatus::Idle);
        assert_eq!(state.last_error().await.as_deref(), Some("something broke"));
    }

    #[tokio::test]
    async fn set_session_id_persists() {
        let state = AgentState::new();
        assert!(state.last_session_id().await.is_none());

        state.set_session_id("sess-123".to_string()).await;
        assert_eq!(state.last_session_id().await.as_deref(), Some("sess-123"));
    }

    #[tokio::test]
    async fn pending_prompt_lifecycle() {
        let state = AgentState::new();
        assert!(!state.has_pending_prompt().await);

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let prompt = PendingPrompt {
            tool_name: super::super::prompt::PromptToolName::AskUserQuestion,
            tool_use_id: "toolu_test".to_string(),
            input: serde_json::json!({"question": "ok?"}),
            sender: tx,
        };
        state.set_pending_prompt(prompt).await;
        assert!(state.has_pending_prompt().await);

        let taken = state.take_pending_prompt().await;
        assert!(taken.is_some());
        assert!(!state.has_pending_prompt().await);

        // Taking again returns None
        assert!(state.take_pending_prompt().await.is_none());
    }

    #[tokio::test]
    async fn stop_running_agent_cancels_token() {
        let state = AgentState::new();
        let token = CancellationToken::new();
        let child = token.clone();

        state.set_running(token).await;
        assert!(!child.is_cancelled());

        state.stop().await.unwrap();
        assert!(child.is_cancelled());
    }

    #[tokio::test]
    async fn stop_idle_agent_errors() {
        let state = AgentState::new();
        let result = state.stop().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn messages_pending_lifecycle() {
        let state = AgentState::new();
        // Initially no messages pending
        assert!(!state.take_messages_pending().await);

        // Set flag and take it
        state.set_messages_pending().await;
        assert!(state.take_messages_pending().await);

        // Flag is reset after take
        assert!(!state.take_messages_pending().await);

        // Multiple sets before take still returns true once
        state.set_messages_pending().await;
        state.set_messages_pending().await;
        assert!(state.take_messages_pending().await);
        assert!(!state.take_messages_pending().await);
    }

    /// Parked on a person is its own status: the run is open and nothing is
    /// running, which is what the reaper reads to decide whether the sandbox is
    /// holding work.
    #[tokio::test]
    async fn waiting_is_entered_and_left_from_running() {
        let state = AgentState::new();
        state.set_running(CancellationToken::new()).await;

        state.begin_waiting().await;
        assert_eq!(state.status().await, AgentStatus::Waiting);

        state.end_waiting().await;
        assert_eq!(state.status().await, AgentStatus::Running);
    }

    /// Every other status is a lifecycle transition that has already cleared
    /// the prompt. Moving one of those to waiting would report a stopped agent
    /// as mid-turn, and moving it back would restart a run that ended.
    #[tokio::test]
    async fn waiting_never_overwrites_a_lifecycle_status() {
        for (transition, expected) in [
            (AgentStatus::Idle, AgentStatus::Idle),
            (AgentStatus::Stopped, AgentStatus::Stopped),
        ] {
            let state = AgentState::new();
            state.set_running(CancellationToken::new()).await;
            match transition {
                AgentStatus::Idle => state.set_idle().await,
                _ => state.set_stopped().await,
            }

            state.begin_waiting().await;
            assert_eq!(state.status().await, expected);
            state.end_waiting().await;
            assert_eq!(state.status().await, expected);
        }
    }

    /// A run cancelled while parked is stopped, and the status the coordinator
    /// set stands — the handler's way out must not put it back to running.
    #[tokio::test]
    async fn leaving_a_cancelled_wait_leaves_the_status_alone() {
        let state = AgentState::new();
        state.set_running(CancellationToken::new()).await;
        state.begin_waiting().await;

        state.set_stopped().await;
        state.end_waiting().await;
        assert_eq!(state.status().await, AgentStatus::Stopped);
    }

    #[tokio::test]
    async fn set_running_clears_pending_prompt() {
        let state = AgentState::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let prompt = PendingPrompt {
            tool_name: super::super::prompt::PromptToolName::ExitPlanMode,
            tool_use_id: "toolu_test".to_string(),
            input: serde_json::json!({}),
            sender: tx,
        };
        state.set_pending_prompt(prompt).await;
        assert!(state.has_pending_prompt().await);

        let token = CancellationToken::new();
        state.set_running(token).await;
        assert!(!state.has_pending_prompt().await);
    }
}
