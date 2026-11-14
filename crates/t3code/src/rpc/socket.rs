use crate::config::OrchestrationProtocol;
use crate::rpc::dispatch::methods;
use crate::rpc::metrics::CONNECTION_BLOCKED;
use crate::rpc::outbound::{self, QueuedFrame};
use crate::rpc::session::{ProtocolOutput, ProtocolSession};
use crate::state::UiRpcState;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use futures::{Sink, SinkExt, Stream, StreamExt};
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde_json::Value;
use std::collections::VecDeque;
use std::future::Future;
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

    // Messages that arrived while a frame was being answered, in arrival order.
    let mut held = VecDeque::new();
    let serving = async {
        'connection: loop {
            let next = if let Some(message) = held.pop_front() {
                Next::Client(message)
            } else {
                tokio::select! {
                    message = incoming.next() => Next::Client(message),
                    pushed = pushed.recv() => Next::Pushed(pushed),
                }
            };
            // Set the moment a client frame is in hand and cleared once it is
            // answered: everything in between is time this connection answers
            // only heartbeats, so a second request from the same client waits it out.
            let (frames, started) = match next {
                Next::Client(message) => {
                    let Some(message) = message else { break };
                    let (output, started) = match message {
                        Ok(Message::Text(text)) => {
                            let started = std::time::Instant::now();
                            let handling = session.handle_json(text.as_bytes(), &state, &identity);
                            (
                                keeping_alive(handling, &mut incoming, &mut sink, &mut held).await,
                                started,
                            )
                        }
                        Ok(Message::Binary(bytes)) => {
                            let started = std::time::Instant::now();
                            let handling = session.handle_json(&bytes, &state, &identity);
                            (
                                keeping_alive(handling, &mut incoming, &mut sink, &mut held).await,
                                started,
                            )
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
                Next::Pushed(pushed) => {
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

/// What the connection loop takes next.
enum Next {
    Client(Option<Result<Message, axum::Error>>),
    Pushed(Option<QueuedFrame>),
}

/// Await `handling` while answering pings, holding every other message in `held`
/// in order. A [`crate::rpc::table::Handler::Session`] method holds the frame
/// loop, and a T3 client that hears nothing for 15 s drops the socket and the
/// request with it.
async fn keeping_alive<S, K>(
    handling: impl Future<Output = ProtocolOutput>,
    incoming: &mut S,
    sink: &mut K,
    held: &mut VecDeque<Option<Result<Message, axum::Error>>>,
) -> ProtocolOutput
where
    S: Stream<Item = Result<Message, axum::Error>> + Unpin,
    K: Sink<Message> + Unpin,
{
    let mut handling = std::pin::pin!(handling);
    // Nothing after the end of the stream, a close or an error is read.
    let mut reading = true;
    loop {
        tokio::select! {
            output = &mut handling => return output,
            message = incoming.next(), if reading => {
                let reply = match &message {
                    Some(Ok(Message::Ping(payload))) => Some(Message::Pong(payload.clone())),
                    Some(Ok(Message::Text(text))) if only_pings(text.as_bytes()) => {
                        Some(Message::Text(PONG.into()))
                    }
                    Some(Ok(Message::Binary(bytes))) if only_pings(bytes) => {
                        Some(Message::Text(PONG.into()))
                    }
                    _ => None,
                };
                // A failed send is the dead socket the loop's own next send
                // reports, once `handling` is answered.
                if let Some(reply) = reply {
                    drop(sink.send(reply).await);
                } else {
                    reading = matches!(&message, Some(Ok(message)) if !matches!(message, Message::Close(_)));
                    held.push_back(message);
                    // Past the cap the socket backs up, as it did before heartbeats.
                    reading &= held.len() < MAX_HELD;
                }
            }
        }
    }
}

const PONG: &str = r#"{"_tag":"Pong"}"#;

/// Messages [`keeping_alive`] holds before it stops reading the socket.
const MAX_HELD: usize = 256;

/// Whether a client frame holds nothing but `Ping` envelopes, so answering it
/// out of order with the frame being handled changes nothing else.
fn only_pings(bytes: &[u8]) -> bool {
    let is_ping = |value: &Value| value.get("_tag").and_then(Value::as_str) == Some("Ping");
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Array(values)) => !values.is_empty() && values.iter().all(is_ping),
        Ok(value) => is_ping(&value),
        Err(_) => false,
    }
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
