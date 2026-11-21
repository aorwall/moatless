//! Newline-delimited JSON-RPC 2.0 over an ACP agent's stdio.
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// The longest line the agent may write; a longer one ends the stream.
const MAX_LINE: usize = 8 * 1024 * 1024;
/// Parsed messages buffered ahead of the harness. Full stops reading stdout,
/// which backs the agent up rather than dropping a message.
const INCOMING_CAPACITY: usize = 256;

/// One message the agent sent.
#[derive(Debug)]
pub(super) enum Incoming {
    /// The answer to our request `id`: its result or its error object.
    Response {
        id: u64,
        result: Result<Value, Value>,
    },
    /// A notification, such as `session/update`.
    Notification { method: String, params: Value },
    /// A request the agent expects us to answer, such as `session/request_permission`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// A JSON-RPC connection over a child's stdio. Reading and writing run on
/// their own tasks, so [`Rpc::recv`] is cancellation-safe and a send never
/// awaits; both tasks stop when this is dropped.
pub(super) struct Rpc {
    out: mpsc::UnboundedSender<Vec<u8>>,
    incoming: mpsc::Receiver<Incoming>,
    next_id: u64,
    tasks: [JoinHandle<()>; 2],
}

impl Rpc {
    pub(super) fn spawn(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        #[expect(
            clippy::disallowed_methods,
            reason = "each write is our own request or the answer to one the agent awaits, so the agent's own pace bounds the queue"
        )]
        let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let writer = tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(bytes) = out_rx.recv().await {
                if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                    return;
                }
            }
        });
        let (tx, incoming) = mpsc::channel(INCOMING_CAPACITY);
        let reader = tokio::spawn(read_loop(BufReader::new(stdout), tx));
        Self {
            out,
            incoming,
            next_id: 0,
            tasks: [writer, reader],
        }
    }

    fn write(&self, message: &Value) {
        let mut bytes = message.to_string().into_bytes();
        bytes.push(b'\n');
        // A closed writer means the agent is gone; `recv` reports that.
        let _ = self.out.send(bytes);
    }

    /// Sends a request and returns its id.
    pub(super) fn request(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        self.write(
            &json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}),
        );
        self.next_id
    }

    pub(super) fn notify(&self, method: &str, params: Value) {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    pub(super) fn respond(&self, id: Value, result: Value) {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    pub(super) fn respond_error(&self, id: Value, code: i64, message: &str) {
        self.write(
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        );
    }

    /// The next message, or `None` once the agent's stdout has closed or sent
    /// a line that is not JSON-RPC.
    pub(super) async fn recv(&mut self) -> Option<Incoming> {
        self.incoming.recv().await
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn read_loop(mut stdout: BufReader<ChildStdout>, tx: mpsc::Sender<Incoming>) {
    let mut line = Vec::new();
    loop {
        let Ok(bytes) = stdout.fill_buf().await else {
            return;
        };
        if bytes.is_empty() {
            return;
        }
        let end = bytes.iter().position(|b| *b == b'\n');
        let n = end.map_or(bytes.len(), |i| i + 1);
        if line.len() + n > MAX_LINE {
            tracing::warn!("ACP agent wrote a line over 8 MiB; closing the stream");
            return;
        }
        line.extend_from_slice(&bytes[..n]);
        stdout.consume(n);
        if end.is_none() {
            continue;
        }
        let blank = line.iter().all(u8::is_ascii_whitespace);
        let parsed = parse(&line);
        line.clear();
        match parsed {
            _ if blank => {}
            Some(message) => {
                if tx.send(message).await.is_err() {
                    return;
                }
            }
            None => {
                tracing::warn!("ACP agent wrote a line that is not JSON-RPC; closing the stream");
                return;
            }
        }
    }
}

/// `None` for a line that is not JSON-RPC.
fn parse(line: &[u8]) -> Option<Incoming> {
    let mut value: Value = serde_json::from_slice(line).ok()?;
    let method = value["method"].as_str().map(str::to_string);
    let params = value["params"].take();
    Some(match (method, value.get("id").cloned()) {
        (Some(method), Some(id)) => Incoming::Request { id, method, params },
        (Some(method), None) => Incoming::Notification { method, params },
        (None, Some(id)) => Incoming::Response {
            id: id.as_u64()?,
            result: match value.get("error") {
                Some(error) => Err(error.clone()),
                None => Ok(value["result"].take()),
            },
        },
        (None, None) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_notifications_and_responses_are_told_apart() {
        let request = parse(
            br#"{"jsonrpc":"2.0","id":"p1","method":"session/request_permission","params":{"x":1}}"#,
        );
        assert!(
            matches!(request, Some(Incoming::Request { ref method, .. }) if method == "session/request_permission")
        );
        let note = parse(br#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#);
        assert!(matches!(note, Some(Incoming::Notification { .. })));
        let ok = parse(br#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#);
        assert!(matches!(
            ok,
            Some(Incoming::Response {
                id: 3,
                result: Ok(_)
            })
        ));
        let err = parse(br#"{"jsonrpc":"2.0","id":4,"error":{"code":-32603,"message":"x"}}"#);
        assert!(matches!(
            err,
            Some(Incoming::Response {
                id: 4,
                result: Err(_)
            })
        ));
        assert!(parse(b"not json\n").is_none());
    }
}
