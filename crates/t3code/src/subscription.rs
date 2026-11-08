use std::collections::HashMap;
use std::future::Future;

use crate::rpc::outbound::OutboundSender;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tracing::warn;

use crate::{MAX_ACTIVE_STREAMS, MAX_DETACHED_REQUESTS, RequestId, failure_exit, success_exit};

/// Sends values from one live subscription.
#[derive(Clone, Debug)]
pub(crate) struct StreamSink {
    request_id: RequestId,
    frames: OutboundSender,
}

impl StreamSink {
    pub(crate) async fn send(&self, value: Value) -> bool {
        self.send_values(vec![value]).await
    }

    pub(crate) async fn send_values(&self, values: Vec<Value>) -> bool {
        self.frames.send(chunk(&self.request_id, values)).await
    }

    pub(crate) fn request_id(&self) -> &RequestId {
        &self.request_id
    }
}

pub(crate) type StreamResult = Result<(), StreamFailure>;

#[derive(Debug)]
pub(crate) enum StreamFailure {
    Message(String),
    Frame(Value),
}

impl StreamFailure {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    pub(crate) fn frame(frame: Value) -> Self {
        Self::Frame(frame)
    }
}

#[derive(Debug)]
struct StreamState {
    awaiting_ack: bool,
    producer: Option<JoinHandle<()>>,
}

impl Drop for StreamState {
    fn drop(&mut self) {
        if let Some(producer) = self.producer.take() {
            producer.abort();
        }
    }
}

/// Owns the active subscriptions for one Effect-RPC connection.
#[derive(Debug)]
pub(crate) struct StreamSet {
    streams: HashMap<String, StreamState>,
    frames: OutboundSender,
}

impl StreamSet {
    pub(crate) fn new(frames: OutboundSender) -> Self {
        Self {
            streams: HashMap::new(),
            frames,
        }
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.streams.contains_key(id)
    }

    pub(crate) fn contains_active(&mut self, id: &RequestId) -> bool {
        self.reap_finished_streams();
        self.streams.contains_key(id.as_str())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.streams.len()
    }

    #[cfg(test)]
    pub(crate) async fn wait_until_finished(&self, id: &str) {
        for _ in 0..100 {
            let finished = self
                .streams
                .get(id)
                .and_then(|stream| stream.producer.as_ref())
                .is_none_or(JoinHandle::is_finished);
            if finished {
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    #[cfg(test)]
    pub(crate) fn awaiting_ack(&self, id: &str) -> Option<bool> {
        self.streams.get(id).map(|stream| stream.awaiting_ack)
    }

    pub(crate) fn ack(&mut self, id: &RequestId) {
        if let Some(stream) = self.streams.get_mut(id.as_str()) {
            stream.awaiting_ack = false;
        }
    }

    pub(crate) fn interrupt(&mut self, id: &RequestId) {
        self.streams.remove(id.as_str());
    }

    pub(crate) fn start(&mut self, id: &RequestId, values: Option<Vec<Value>>) -> Vec<Value> {
        self.reap_finished_streams();
        if self.streams.len() >= MAX_ACTIVE_STREAMS {
            warn!(
                active_streams = self.streams.len(),
                "T3 UI stream refused; this connection is at its subscription cap"
            );
            crate::rpc::metrics::capped();
            return vec![failure_exit(
                id,
                format!("At most {MAX_ACTIVE_STREAMS} RPC streams may be active"),
            )];
        }

        let awaiting_ack = values.is_some();
        self.streams.insert(
            id.as_str().to_string(),
            StreamState {
                awaiting_ack,
                producer: None,
            },
        );
        values.map_or_else(Vec::new, |values| vec![chunk(id, values)])
    }

    pub(crate) fn start_produced<F, Fut>(
        &mut self,
        id: &RequestId,
        values: Option<Vec<Value>>,
        producer: F,
    ) -> Vec<Value>
    where
        F: FnOnce(StreamSink) -> Fut + Send + 'static,
        Fut: Future<Output = StreamResult> + Send + 'static,
    {
        let started = self.start(id, values);
        if let Some(stream) = self.streams.get_mut(id.as_str()) {
            let sink = StreamSink {
                request_id: id.clone(),
                frames: self.frames.clone(),
            };
            let request_id = id.clone();
            let frames = self.frames.clone();
            stream.producer = Some(tokio::spawn(async move {
                let frame = match producer(sink).await {
                    Ok(()) => success_exit(&request_id, Value::Null),
                    Err(StreamFailure::Message(message)) => failure_exit(&request_id, message),
                    Err(StreamFailure::Frame(frame)) => frame,
                };
                let _ = frames.send(frame).await;
            }));
        }
        started
    }

    fn reap_finished_streams(&mut self) {
        self.streams.retain(|_, stream| {
            stream
                .producer
                .as_ref()
                .is_none_or(|producer| !producer.is_finished())
        });
    }
}

/// One request running off the frame loop, aborted when it leaves the set: an
/// interrupted request stops, and so does one whose connection is gone.
#[derive(Debug)]
struct DetachedRequest(JoinHandle<()>);

impl Drop for DetachedRequest {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Owns the requests one connection is answering off its frame loop.
///
/// A detached request shares the frame channel a subscription's producer uses,
/// so `serve_websocket` writes both without knowing which produced a frame.
/// Nothing here is ordered; a request needing the client's ordering is answered
/// on the loop instead.
#[derive(Debug)]
pub(crate) struct DetachedSet {
    running: HashMap<String, DetachedRequest>,
    frames: OutboundSender,
}

impl DetachedSet {
    pub(crate) fn new(frames: OutboundSender) -> Self {
        Self {
            running: HashMap::new(),
            frames,
        }
    }

    /// Answer `request` off the frame loop, so this connection reads its next
    /// frame while it runs.
    ///
    /// Hands the request back unspawned at [`MAX_DETACHED_REQUESTS`], for the
    /// caller to await inline — the behaviour every request had before any
    /// detached.
    pub(crate) fn spawn<F>(&mut self, id: &RequestId, request: F) -> Option<F>
    where
        F: Future<Output = Vec<Value>> + Send + 'static,
    {
        self.reap_finished_requests();
        if self.running.len() >= MAX_DETACHED_REQUESTS {
            warn!(
                detached_requests = self.running.len(),
                "T3 UI request answered on the frame loop; this connection is at its detached cap"
            );
            return Some(request);
        }

        let frames = self.frames.clone();
        self.running.insert(
            id.as_str().to_string(),
            DetachedRequest(tokio::spawn(async move {
                for frame in request.await {
                    if !frames.send(frame).await {
                        return;
                    }
                }
            })),
        );
        None
    }

    pub(crate) fn contains_active(&mut self, id: &RequestId) -> bool {
        self.reap_finished_requests();
        self.running.contains_key(id.as_str())
    }

    pub(crate) fn interrupt(&mut self, id: &RequestId) {
        self.running.remove(id.as_str());
    }

    /// Wait for every detached request to have sent its frames.
    #[cfg(test)]
    pub(crate) async fn wait_until_idle(&self) {
        for _ in 0..1000 {
            if self.running.values().all(|request| request.0.is_finished()) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("a detached request did not finish within 1000 yields");
    }

    fn reap_finished_requests(&mut self) {
        self.running.retain(|_, request| !request.0.is_finished());
    }
}

fn chunk(id: &RequestId, values: Vec<Value>) -> Value {
    let mut frame = json!({
        "_tag": "Chunk",
        "requestId": id.wire(),
    });
    frame["values"] = Value::Array(values);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_id(id: &str) -> RequestId {
        serde_json::from_value(json!(id)).expect("a string is a request id")
    }

    #[tokio::test]
    async fn a_stalled_reader_cannot_queue_unlimited_snapshot_bytes() {
        let (frames, _reader) = crate::rpc::outbound::channel(crate::rpc::socket::OUTBOUND_FRAMES);
        let sink = StreamSink {
            request_id: request_id("1"),
            frames,
        };
        let mut refused = false;
        for _ in 0..8 {
            if !sink
                .send(json!({ "snapshot": "x".repeat(8 * 1024 * 1024) }))
                .await
            {
                refused = true;
                break;
            }
        }
        assert!(
            refused,
            "a stalled reader retained over 32 MiB of snapshots"
        );
    }

    #[tokio::test]
    async fn a_finished_producer_gives_its_slot_back() {
        let (frames, mut pushed) = crate::rpc::outbound::channel(8);
        let mut streams = StreamSet::new(frames);
        streams.start_produced(&request_id("1"), None, |_| async { Ok(()) });
        while streams.contains("1") && pushed.recv().await.is_none() {}

        let opened = streams.start(&request_id("2"), None);

        assert!(opened.is_empty());
        assert!(!streams.contains("1"));
        assert!(streams.contains("2"));
    }

    #[tokio::test]
    async fn a_failed_producer_sends_one_terminal_failure() {
        let (frames, mut pushed) = crate::rpc::outbound::channel(8);
        let mut streams = StreamSet::new(frames);
        streams.start_produced(&request_id("1"), None, |_| async move {
            Err(StreamFailure::message("stream failed"))
        });

        let terminal = pushed.recv().await.expect("a terminal frame").into_value();

        assert_eq!(terminal["_tag"], "Exit");
        assert_eq!(terminal["requestId"], "1");
        assert_eq!(terminal["exit"]["_tag"], "Failure");
    }
}
