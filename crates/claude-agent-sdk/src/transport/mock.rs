use super::{StdinWriter, Transport};
use crate::api::control::RawStdoutMessage;
use crate::error::Result;
use async_trait::async_trait;
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
