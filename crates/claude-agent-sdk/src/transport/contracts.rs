use crate::api::control::RawStdoutMessage;
use crate::error::Result;
use async_trait::async_trait;

/// Trait for reading messages from the CLI transport.
#[async_trait]
pub(crate) trait Transport: Send {
    /// Read the next message. Returns `None` on EOF.
    async fn read_message(&mut self) -> Result<Option<RawStdoutMessage>>;
    /// Close the transport and clean up resources.
    async fn close(&mut self);
}

/// Trait for writing JSON lines to the CLI's stdin.
#[async_trait]
pub(crate) trait StdinWriter: Send + Sync {
    /// Write a JSON line followed by newline and flush.
    async fn write_json_line(&self, json: &str) -> Result<()>;
}
