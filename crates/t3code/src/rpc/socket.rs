use crate::config::OrchestrationProtocol;
use crate::rpc::dispatch::methods;
use crate::rpc::metrics::CONNECTION_BLOCKED;
use crate::rpc::session::ProtocolSession;
use crate::state::UiRpcState;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use futures::{SinkExt, StreamExt};
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::debug;
use tracing::warn;

/// The largest WebSocket message this socket reads or writes.
///
/// Sized for T3's one large frame: a base64 attachment inside
/// `orchestration.dispatchCommand`, up to 14,000,000 characters. A frame
/// over this drops the socket, not a clean refusal — [`commands`] already
/// refuses oversized attachments by name.
pub(crate) const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// Frames a subscription may have queued for the socket before it is dropped.
///
/// A shell subscription produces at most one chunk per poll, so this is deep
/// enough to absorb several seconds of a stalled writer; past that the poller
/// waits — correct backpressure, since the next poll coalesces whatever
/// accumulated into one row.
pub(crate) const OUTBOUND_FRAMES: usize = 64;

/// Serve one connection: answer what the client asks, and send what a
/// subscription produces on its own.
///
/// The socket is split because those two are independent — a subscription's
/// chunk must not wait for the client to say something first.
pub(crate) async fn serve_websocket(
    socket: WebSocket,
    state: UiRpcState,
    identity: UiRpcAuthIdentity,
    protocol: OrchestrationProtocol,
) {
    debug!(user_id = %identity.user_id, ?protocol, "T3 UI WebSocket connected");
    let (mut sink, mut incoming) = socket.split();
    let (frames, mut pushed) = mpsc::channel::<Value>(OUTBOUND_FRAMES);
    let mut session = ProtocolSession::new(frames, methods());
    session.shared.protocol = protocol;

    'connection: loop {
        // Set the moment a client frame is in hand and cleared once it is
        // answered: everything in between is time this connection cannot read
        // its socket, so a second request from the same client waits it out.
        let mut blocked_since = None;
        let frames = tokio::select! {
            message = incoming.next() => {
                let Some(message) = message else { break };
                let output = match message {
                    Ok(Message::Text(text)) => {
                        blocked_since = Some(std::time::Instant::now());
                        session.handle_json(text.as_bytes(), &state, &identity).await
                    }
                    Ok(Message::Binary(bytes)) => {
                        blocked_since = Some(std::time::Instant::now());
                        session.handle_json(&bytes, &state, &identity).await
                    }
                    Ok(Message::Ping(payload)) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    Ok(Message::Pong(_)) => continue,
                    Ok(Message::Close(_)) => break,
                    Err(error) => {
                        debug!(%error, user_id = %identity.user_id, "T3 UI WebSocket receive failed");
                        break;
                    }
                };
                if output.eof {
                    // No `CONNECTION_BLOCKED` sample: this breaks the loop, so
                    // nothing can be queued behind the frame that ended it.
                    send_frames(&mut sink, output.frames).await;
                    break;
                }
                output.frames
            }
            pushed = pushed.recv() => {
                // The sender is held by the session, which outlives this loop,
                // so `None` cannot arrive while the connection is open.
                match pushed {
                    Some(frame) => vec![frame],
                    None => break,
                }
            }
        };

        let sent = send_frames(&mut sink, frames).await;
        if let Some(started) = blocked_since {
            metrics::histogram!(CONNECTION_BLOCKED).record(started.elapsed().as_secs_f64());
        }
        if !sent {
            break 'connection;
        }
    }

    debug!(user_id = %identity.user_id, "T3 UI WebSocket disconnected");
}

/// Write frames to the socket, reporting whether the connection is still
/// usable.
pub(crate) async fn send_frames(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    frames: Vec<Value>,
) -> bool {
    for frame in frames {
        let encoded = match serde_json::to_string(&frame) {
            Ok(encoded) => encoded,
            Err(error) => {
                warn!(%error, "Failed to encode T3 UI RPC response");
                return false;
            }
        };
        if sink.send(Message::Text(encoded.into())).await.is_err() {
            return false;
        }
    }
    true
}
