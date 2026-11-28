use std::io::{self, Write};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// The largest WebSocket message accepted by T3, including inline attachments.
pub(crate) const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// Encoded bytes held by one connection's queued and pending frames.
const OUTBOUND_BYTES: usize = MAX_FRAME_BYTES;

/// One encoded frame, charged until its socket write finishes.
pub(crate) struct QueuedFrame {
    pub(crate) encoded: Box<str>,
    pub(crate) permit: OwnedSemaphorePermit,
}

/// Producers sharing a connection's byte budget and overload cancellation.
#[derive(Clone, Debug)]
pub(crate) struct OutboundSender {
    frames: mpsc::Sender<QueuedFrame>,
    bytes: Arc<Semaphore>,
    disconnect: CancellationToken,
    /// The connection's user, for the overload warning.
    user_id: Option<Arc<str>>,
}

/// One connection's outbound queue; a receiver must observe `disconnected()`.
pub(crate) fn channel(capacity: usize) -> (OutboundSender, mpsc::Receiver<QueuedFrame>) {
    let (frames, receiver) = mpsc::channel(capacity);
    (
        OutboundSender {
            frames,
            bytes: Arc::new(Semaphore::new(OUTBOUND_BYTES)),
            disconnect: CancellationToken::new(),
            user_id: None,
        },
        receiver,
    )
}

impl OutboundSender {
    /// Name the connection's user in the overload warning.
    pub(crate) fn owned_by(mut self, user_id: &str) -> Self {
        self.user_id = Some(Arc::from(user_id));
        self
    }

    pub(crate) fn disconnected(&self) -> CancellationToken {
        self.disconnect.clone()
    }

    /// Queue a frame; exceeding the byte budget disconnects the connection.
    pub(crate) async fn send(&self, frame: Value) -> bool {
        if self.disconnect.is_cancelled() {
            return false;
        }
        let request_id = frame
            .get("requestId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let frame_tag = frame.get("_tag").and_then(Value::as_str).map(str::to_owned);
        let encoded = match encode(frame) {
            Ok(encoded) => encoded,
            Err(error) => {
                warn!(%error, "Failed to encode T3 UI RPC response");
                self.disconnect.cancel();
                return false;
            }
        };
        // Deliberately refuse rather than wait for bytes: a waiting producer
        // would retain its snapshot outside the queue's budget.
        let Some(permit) = u32::try_from(encoded.len())
            .ok()
            .and_then(|bytes| self.bytes.clone().try_acquire_many_owned(bytes).ok())
        else {
            warn!(
                user_id = self.user_id.as_deref().unwrap_or("(unknown)"),
                request_id = request_id.as_deref().unwrap_or("(none)"),
                frame_tag = frame_tag.as_deref().unwrap_or("(none)"),
                frame_bytes = encoded.len(),
                queued_bytes = OUTBOUND_BYTES.saturating_sub(self.bytes.available_permits()),
                queued_frames = self.frames.max_capacity() - self.frames.capacity(),
                budget_bytes = OUTBOUND_BYTES,
                "T3 UI connection exceeded its outbound byte budget; disconnecting it \
                 and every request it was answering"
            );
            self.disconnect.cancel();
            return false;
        };
        tokio::select! {
            () = self.disconnect.cancelled() => false,
            sent = self.frames.send(QueuedFrame { encoded, permit }) => sent.is_ok(),
        }
    }
}

/// A bounded encoding that consumes the JSON tree before any socket wait.
pub(crate) fn encode(frame: Value) -> Result<Box<str>, EncodingError> {
    let mut buffer = FrameBuffer(Vec::new());
    serde_json::to_writer(&mut buffer, &frame)?;
    Ok(String::from_utf8(buffer.0)?.into_boxed_str())
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EncodingError {
    #[error("RPC frame serialization failed")]
    Json(#[from] serde_json::Error),
    #[error("RPC frame was not UTF-8")]
    Utf8(#[from] std::string::FromUtf8Error),
}

struct FrameBuffer(Vec<u8>);

impl Write for FrameBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FRAME_BYTES - self.0.len() {
            return Err(io::Error::other(
                "RPC frame exceeds the WebSocket message limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
impl QueuedFrame {
    pub(crate) fn into_value(self) -> Value {
        serde_json::from_str(&self.encoded).expect("an encoded RPC frame")
    }
}

#[cfg(test)]
mod tests;
