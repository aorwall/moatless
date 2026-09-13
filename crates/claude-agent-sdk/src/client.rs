use crate::error::Result;
use crate::query::{QueryHandle, query_streaming};
use crate::tmux::query_tmux_streaming;
use crate::api::content::{ApiMessage, ContentBlock};
use crate::api::messages::{MessagePriority, SdkMessage, UserMessage};
use crate::api::options::ClaudeOptions;

/// A multi-turn Claude client that manages a persistent session.
///
/// Unlike one-shot `query()`, it keeps a subprocess open for multiple messages;
/// callers must extract `session_id` from the `SdkMessage::System` init message
/// and pass it to `send()`.
pub struct ClaudeClient {
    handle: QueryHandle,
}

impl ClaudeClient {
    /// Create a new client with the given options. This spawns the Claude CLI
    /// subprocess and initializes the session.
    pub async fn new(options: ClaudeOptions) -> Result<Self> {
        let handle = query_streaming(options).await?;
        Ok(Self { handle })
    }

    /// Create a new client backed by an interactive `claude` TUI in a detached
    /// tmux session. Most notably, `options.can_use_tool` is ignored and
    /// permissions are always bypassed — see [`crate::tmux`] for other caveats.
    pub async fn new_tmux(options: ClaudeOptions) -> Result<Self> {
        let handle = query_tmux_streaming(options).await?;
        Ok(Self { handle })
    }

    /// Send a text message to the Claude CLI.
    ///
    /// `session_id` comes from the `SdkMessage::System` init message from the
    /// first turn; pass an empty string for the initial call before any message
    /// is received.
    pub async fn send(&self, message: &str, session_id: &str) -> Result<()> {
        self.send_with_priority(message, session_id, MessagePriority::Next)
            .await
    }

    /// Send a text message with an explicit priority.
    ///
    /// See [`MessagePriority`] for when each variant is processed by the CLI.
    pub async fn send_with_priority(
        &self,
        message: &str,
        session_id: &str,
        priority: MessagePriority,
    ) -> Result<()> {
        let user_msg = UserMessage {
            uuid: None,
            session_id: session_id.to_string(),
            message: ApiMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::Text {
                    text: message.to_string(),
                }],
                usage: None,
                model: None,
                id: None,
                stop_reason: None,
                stop_sequence: None,
            },
            parent_tool_use_id: None,
            is_synthetic: None,
            tool_use_result: None,
            is_meta: None,
            is_compact_summary: None,
            is_visible_in_transcript_only: None,
            plan_content: None,
            thinking_metadata: None,
            todos: None,
            summarize_metadata: None,
            source_tool_assistant_uuid: None,
            source_tool_use_id: None,
            permission_mode: None,
            priority,
        };
        self.handle.send_user_message(&user_msg).await
    }

    /// Receive the next SDK message.
    pub async fn recv(&mut self) -> Option<Result<SdkMessage>> {
        self.handle.messages.recv().await
    }

    /// Interrupt the current turn.
    pub async fn interrupt(&self) -> Result<()> {
        self.handle.interrupt().await
    }

    /// Close the client and terminate the subprocess.
    pub fn close(&self) {
        self.handle.close();
    }

    /// Check if the client is still running.
    pub fn is_running(&self) -> bool {
        self.handle.is_running()
    }
}
