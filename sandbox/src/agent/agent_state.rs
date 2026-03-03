//! Generic agent state machine — tracks status, session, cancel token, and pending prompts.
//!
//! Extracted from `executor.rs` so it can be reused by any agent type.

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::error::SandboxError;

use super::prompt::PendingPrompt;
use super::types::AgentStatus;

/// Shared agent execution state, protected by RwLock for concurrent reads.
pub struct AgentState {
    inner: RwLock<AgentInner>,
}

struct AgentInner {
    status: AgentStatus,
    last_session_id: Option<String>,
    last_error: Option<String>,
    cancel_token: Option<CancellationToken>,
    pending_prompt: Option<PendingPrompt>,
    consecutive_errors: u32,
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
                last_error: None,
                cancel_token: None,
                pending_prompt: None,
                consecutive_errors: 0,
            }),
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
    }

    pub async fn set_stopped(&self) {
        let mut inner = self.inner.write().await;
        inner.status = AgentStatus::Stopped;
        // Drop the pending prompt sender, which will cause the receiver to error
        inner.pending_prompt = None;
    }

    pub async fn set_error(&self, error: String) {
        let mut inner = self.inner.write().await;
        inner.status = AgentStatus::Idle;
        inner.last_error = Some(error);
        inner.cancel_token = None;
        inner.pending_prompt = None;
        inner.consecutive_errors += 1;
    }

    /// Number of consecutive errors without a successful idle transition.
    pub async fn consecutive_errors(&self) -> u32 {
        self.inner.read().await.consecutive_errors
    }

    pub async fn set_session_id(&self, session_id: String) {
        self.inner.write().await.last_session_id = Some(session_id);
    }

    pub async fn set_pending_prompt(&self, prompt: PendingPrompt) {
        self.inner.write().await.pending_prompt = Some(prompt);
    }

    /// Take the pending prompt (removes it from state).
    pub async fn take_pending_prompt(&self) -> Option<PendingPrompt> {
        self.inner.write().await.pending_prompt.take()
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
    async fn set_running_clears_pending_prompt() {
        let state = AgentState::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let prompt = PendingPrompt {
            tool_name: super::super::prompt::PromptToolName::ExitPlanMode,
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
