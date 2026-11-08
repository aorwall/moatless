use crate::config::OrchestrationProtocol;
use crate::rpc::dispatch::methods;
use crate::rpc::metrics::CONNECTION_BLOCKED;
use crate::rpc::outbound::{self, QueuedFrame};
use crate::rpc::session::ProtocolSession;
use crate::state::UiRpcState;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use futures::{SinkExt, StreamExt};
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use tracing::debug;
use tracing::warn;

pub(crate) use crate::rpc::outbound::MAX_FRAME_BYTES;

/// Frame slots per connection; `outbound` also bounds their encoded bytes.
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
    let (frames, mut pushed) = outbound::channel(OUTBOUND_FRAMES);
    let disconnect = frames.disconnected();
    let mut session = ProtocolSession::new(frames, methods());
    session.shared.protocol = protocol;

    let serving = async {
        'connection: loop {
            // Set the moment a client frame is in hand and cleared once it is
            // answered: everything in between is time this connection cannot read
            // its socket, so a second request from the same client waits it out.
            let (frames, started) = tokio::select! {
                message = incoming.next() => {
                    let Some(message) = message else { break };
                    let (output, started) = match message {
                        Ok(Message::Text(text)) => {
                            let started = std::time::Instant::now();
                            (session.handle_json(text.as_bytes(), &state, &identity).await, started)
                        }
                        Ok(Message::Binary(bytes)) => {
                            let started = std::time::Instant::now();
                            (session.handle_json(&bytes, &state, &identity).await, started)
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
                    (output.frames, started)
                }
                pushed = pushed.recv() => {
                    // The sender is held by the session, which outlives this loop,
                    // so `None` cannot arrive while the connection is open.
                    match pushed {
                        Some(frame) => {
                            if !send_queued_frame(&mut sink, frame).await {
                                break;
                            }
                            continue;
                        }
                        None => break,
                    }
                }
            };

            let sent = send_frames(&mut sink, frames).await;
            metrics::histogram!(CONNECTION_BLOCKED).record(started.elapsed().as_secs_f64());
            if !sent {
                break 'connection;
            }
        }
    };
    tokio::select! {
        () = serving => {},
        () = disconnect.cancelled() => {},
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
        let encoded = match outbound::encode(frame) {
            Ok(encoded) => encoded,
            Err(error) => {
                warn!(%error, "Failed to encode T3 UI RPC response");
                return false;
            }
        };
        if sink
            .send(Message::Text(String::from(encoded).into()))
            .await
            .is_err()
        {
            return false;
        }
    }
    true
}

async fn send_queued_frame(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    frame: QueuedFrame,
) -> bool {
    let QueuedFrame { encoded, permit } = frame;
    let sent = sink
        .send(Message::Text(String::from(encoded).into()))
        .await
        .is_ok();
    drop(permit);
    sent
}

#[cfg(test)]
#[path = "socket_tests.rs"]
mod tests;
