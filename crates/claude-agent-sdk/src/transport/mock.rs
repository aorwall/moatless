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
    tx: mpsc::Sender<String>,
}

impl MockStdinWriter {
    /// Holds 64 undrained lines; a test that writes more fails on the write.
    pub(crate) fn new() -> (Self, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, rx)
    }
}

#[async_trait]
impl StdinWriter for MockStdinWriter {
    async fn write_json_line(&self, json: &str) -> Result<()> {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.tx.try_send(json.to_string()) {
            panic!("MockStdinWriter holds 64 undrained lines");
        }
        Ok(())
    }
}
