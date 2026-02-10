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
