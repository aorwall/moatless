use crate::error::Result;
use crate::query::{query_streaming, QueryHandle};
use crate::types::content::{ApiMessage, ContentBlock};
use crate::types::messages::{MessagePriority, SdkMessage, UserMessage};
use crate::types::options::ClaudeOptions;

/// A multi-turn Claude client that manages a persistent session.
///
/// Unlike `query()` which is a one-shot function, `ClaudeClient` maintains
/// a subprocess and allows sending multiple messages over time. Callers are
/// responsible for extracting `session_id` from `SdkMessage::System` init
/// messages and passing it explicitly to `send()`.
pub struct ClaudeClient {
    handle: QueryHandle,
}

impl ClaudeClient {
    /// Create a new client with the given options.
    /// This spawns the Claude CLI subprocess and initializes the session.
    pub async fn new(options: ClaudeOptions) -> Result<Self> {
        let handle = query_streaming(options).await?;
        Ok(Self { handle })
    }

    /// Send a text message to the Claude CLI.
    ///
    /// The `session_id` should be extracted from the `SdkMessage::System` init
    /// message received during the first turn. For the initial `send()` before
    /// any messages have been received, pass an empty string.
    pub async fn send(&self, message: &str, session_id: &str) -> Result<()> {
        self.send_with_priority(message, session_id, MessagePriority::Next)
            .await
    }

    /// Send a text message with an explicit priority.
    ///
    /// Priority controls when the CLI processes the message:
    /// - `Now` — immediately, interrupting current tool execution
    /// - `Next` — between the current tool result and the next inference call (default)
    /// - `Later` — after the current turn completes
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
