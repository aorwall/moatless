use crate::error::Result;
use crate::messages::CodexMessage;
use crate::options::CodexOptions;
use crate::query::{self, QueryHandle};

/// High-level multi-turn conversation client.
///
/// This is the Codex equivalent of the Claude SDK's `ClaudeClient`.
/// It wraps a `QueryHandle` and provides a simpler `send()`/`recv()` interface.
pub struct CodexClient {
    handle: QueryHandle,
}

impl CodexClient {
    /// Create a new client and initialize a Codex session.
    /// No initial prompt is sent; use `send()` to start a turn.
    pub async fn new(options: CodexOptions) -> Result<Self> {
        let handle = query::query_streaming(options).await?;
        Ok(Self { handle })
    }

    /// Start a new turn with the given prompt text.
    pub async fn send(&self, message: &str) -> Result<()> {
        self.handle.send(message).await
    }

    /// Receive the next message from the agent.
    /// Returns `None` when the stream is exhausted.
    pub async fn recv(&mut self) -> Option<Result<CodexMessage>> {
        self.handle.messages.recv().await
    }

    /// Interrupt the currently running turn.
    pub async fn interrupt(&self) -> Result<()> {
        self.handle.interrupt().await
    }

    /// Inject additional context into the currently running turn.
    pub async fn steer(&self, input: &str) -> Result<()> {
        self.handle.steer(input).await
    }

    /// Returns the thread ID for this session.
    pub fn thread_id(&self) -> &str {
        self.handle.thread_id()
    }

    /// Signal cancellation and stop the session.
    pub fn cancel(&self) {
        self.handle.cancel();
    }

    /// Cancel and wait for the runtime to fully shut down.
    pub async fn shutdown(&mut self) {
        self.handle.shutdown().await;
    }
}
