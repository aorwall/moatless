use tokio::sync::mpsc;

use crate::error::Result;
use crate::query::{query_streaming, QueryHandle};
use crate::types::content::{ApiMessage, ContentBlock};
use crate::types::messages::{SdkMessage, UserMessage};
use crate::types::options::ClaudeOptions;

/// A multi-turn Claude client that manages a persistent session.
///
/// Unlike `query()` which is a one-shot function, `ClaudeClient` maintains
/// a session and allows sending multiple messages over time.
pub struct ClaudeClient {
    handle: QueryHandle,
    session_id: Option<String>,
}

impl ClaudeClient {
    /// Create a new client with the given options.
    /// This spawns the Claude CLI subprocess and initializes the session.
    pub async fn new(options: ClaudeOptions) -> Result<Self> {
        let handle = query_streaming(options).await?;
        Ok(Self {
            handle,
            session_id: None,
        })
    }

    /// Send a text message to the Claude CLI.
    pub async fn send(&self, message: &str) -> Result<()> {
        let user_msg = UserMessage {
            uuid: None,
            session_id: self.session_id.clone().unwrap_or_default(),
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

    /// Access the message receiver for consuming SDK messages.
    pub fn messages(&mut self) -> &mut mpsc::Receiver<Result<SdkMessage>> {
        &mut self.handle.messages
    }

    /// Get the session ID (available after the init message is received).
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Update the session ID from a system init message.
    pub fn update_session_id(&mut self, session_id: String) {
        self.session_id = Some(session_id);
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
