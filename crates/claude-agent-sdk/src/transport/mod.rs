pub(crate) mod subprocess;

pub(crate) use subprocess::SubprocessTransport;

use async_trait::async_trait;

use crate::error::Result;
use crate::types::control::RawStdoutMessage;

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

// -- Implementations for real subprocess transport --

#[async_trait]
impl Transport for SubprocessTransport {
    async fn read_message(&mut self) -> Result<Option<RawStdoutMessage>> {
        self.read_message().await
    }

    async fn close(&mut self) {
        self.close().await;
    }
}

/// Wrapper around `Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>` that
/// implements `StdinWriter`.
pub(crate) struct ChildStdinWriter {
    pub(crate) inner: std::sync::Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
}

#[async_trait]
impl StdinWriter for ChildStdinWriter {
    async fn write_json_line(&self, json: &str) -> Result<()> {
        use crate::error::SdkError;
        use tokio::io::AsyncWriteExt;
        let mut stdin = self.inner.lock().await;
        stdin
            .write_all(json.as_bytes())
            .await
            .map_err(SdkError::SpawnError)?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(SdkError::SpawnError)?;
        stdin.flush().await.map_err(SdkError::SpawnError)?;
        Ok(())
    }
}

// -- Mock implementations for testing --

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use tokio::sync::mpsc;

    /// A mock transport that reads from a channel instead of a subprocess.
    pub(crate) struct MockTransport {
        rx: mpsc::Receiver<RawStdoutMessage>,
    }

    impl MockTransport {
        pub(crate) fn new(rx: mpsc::Receiver<RawStdoutMessage>) -> Self {
            Self { rx }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn read_message(&mut self) -> Result<Option<RawStdoutMessage>> {
            Ok(self.rx.recv().await)
        }

        async fn close(&mut self) {
            self.rx.close();
        }
    }

    /// A mock stdin writer that captures all written lines.
    pub(crate) struct MockStdinWriter {
        tx: mpsc::UnboundedSender<String>,
    }

    impl MockStdinWriter {
        pub(crate) fn new() -> (Self, mpsc::UnboundedReceiver<String>) {
            let (tx, rx) = mpsc::unbounded_channel();
            (Self { tx }, rx)
        }
    }

    #[async_trait]
    impl StdinWriter for MockStdinWriter {
        async fn write_json_line(&self, json: &str) -> Result<()> {
            let _ = self.tx.send(json.to_string());
            Ok(())
        }
    }
}
