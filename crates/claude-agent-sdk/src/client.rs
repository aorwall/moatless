use crate::error::Result;
use crate::query::{query_streaming, QueryHandle};
use crate::types::content::{ApiMessage, ContentBlock};
use crate::types::messages::{SdkMessage, UserMessage};
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
        let user_msg = UserMessage {
            uuid: None,
            session_id: session_id.to_string(),
            message: ApiMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::Text {
                    text: message.to_string(),
                }],
                usage: None,
            },
            parent_tool_use_id: None,
            is_synthetic: None,
            tool_use_result: None,
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
