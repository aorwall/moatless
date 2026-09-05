use std::collections::HashMap;
use std::future::Future;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::{MAX_ACTIVE_STREAMS, RequestId, failure_exit, success_exit};

/// Sends values from one live subscription.
#[derive(Clone, Debug)]
pub(crate) struct StreamSink {
    request_id: RequestId,
    frames: mpsc::Sender<Value>,
}

impl StreamSink {
    pub(crate) async fn send(&self, value: Value) -> bool {
        self.send_values(vec![value]).await
    }

    pub(crate) async fn send_values(&self, values: Vec<Value>) -> bool {
        self.frames
            .send(json!({
                "_tag": "Chunk",
                "requestId": self.request_id.wire(),
                "values": values,
            }))
            .await
            .is_ok()
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
    frames: mpsc::Sender<Value>,
}

impl StreamSet {
    pub(crate) fn new(frames: mpsc::Sender<Value>) -> Self {
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

fn chunk(id: &RequestId, values: Vec<Value>) -> Value {
    json!({
        "_tag": "Chunk",
        "requestId": id.wire(),
        "values": values
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_id(id: &str) -> RequestId {
        serde_json::from_value(json!(id)).expect("a string is a request id")
    }

    #[tokio::test]
    async fn a_finished_producer_gives_its_slot_back() {
        let (frames, mut pushed) = mpsc::channel(8);
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
        let (frames, mut pushed) = mpsc::channel(8);
        let mut streams = StreamSet::new(frames);
        streams.start_produced(&request_id("1"), None, |_| async move {
            Err(StreamFailure::message("stream failed"))
        });

        let terminal = pushed.recv().await.expect("a terminal frame");

        assert_eq!(terminal["_tag"], "Exit");
        assert_eq!(terminal["requestId"], "1");
        assert_eq!(terminal["exit"]["_tag"], "Failure");
    }
}
