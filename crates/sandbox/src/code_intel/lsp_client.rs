//! A minimal LSP client over a child's stdio: framing, request ids, and the
//! replies a server blocks on when it asks the client something.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

#[derive(Debug, thiserror::Error)]
pub enum LspError {
    #[error("failed to start language server: {0}")]
    Spawn(std::io::Error),
    #[error("language server pipe failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("language server exited")]
    Exited,
    #[error("language server did not answer {0} within {1:?}")]
    Timeout(String, Duration),
    #[error("language server rejected {method}: {message}")]
    Rejected { method: String, message: String },
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>;

/// One running server. Dropping it kills the child.
pub(crate) struct LspClient {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: AtomicI64,
    _child: Child,
}

impl LspClient {
    /// Start `command` in `root` and complete the `initialize` handshake.
    pub(crate) async fn start(
        command: &str,
        args: &[&str],
        root: &Path,
        timeout: Duration,
    ) -> Result<Self, LspError> {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(root)
            // One server answers every Member sharing this checkout, so no
            // Member's `TASK_ID` is right for it; the pod's would name the wrong one.
            .env_remove("TASK_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(LspError::Spawn)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(LspError::Exited);
        };
        let stdin = Arc::new(Mutex::new(stdin));
        let pending: Pending = Arc::default();
        tokio::spawn(read_loop(
            BufReader::new(stdout),
            Arc::clone(&stdin),
            Arc::clone(&pending),
        ));

        let client = Self {
            stdin,
            pending,
            next_id: AtomicI64::new(1),
            _child: child,
        };
        let root_uri = path_to_uri(root);
        client
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "workspaceFolders": [{ "uri": root_uri, "name": "workspace" }],
                    "capabilities": {
                        "general": { "positionEncodings": ["utf-16"] },
                        "textDocument": {
                            "synchronization": { "dynamicRegistration": false },
                            "definition": { "linkSupport": true },
                        },
                    },
                }),
                timeout,
            )
            .await?;
        client.notify("initialized", json!({})).await?;
        Ok(client)
    }

    pub(crate) async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let sent = self
            .send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        if let Err(e) = sent {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result.map_err(|e| match e {
                LspError::Rejected { message, .. } => LspError::Rejected {
                    method: method.to_string(),
                    message,
                },
                other => other,
            }),
            Ok(Err(_)) => Err(LspError::Exited),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(LspError::Timeout(method.to_string(), timeout))
            }
        }
    }

    pub(crate) async fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn send(&self, message: &Value) -> Result<(), LspError> {
        write_message(&mut *self.stdin.lock().await, message).await
    }
}

async fn write_message(
    stdin: &mut (impl AsyncWriteExt + Unpin),
    message: &Value,
) -> Result<(), LspError> {
    let body = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    stdin
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    stdin.write_all(&body).await?;
    stdin.flush().await?;
    Ok(())
}

/// Read one framed message; `None` at end of stream.
async fn read_message(reader: &mut (impl AsyncBufRead + Unpin)) -> std::io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(length) = length else {
        return Err(std::io::Error::other("LSP header without Content-Length"));
    };
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(std::io::Error::other)
}

/// Route responses to their waiting request and answer the server's own
/// requests. Ends when the server closes stdout, failing every waiter.
async fn read_loop(
    mut stdout: impl AsyncBufRead + Unpin,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
) {
    loop {
        let message = match read_message(&mut stdout).await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(e) => {
                warn!(error = %e, "Language server sent an unreadable message");
                break;
            }
        };
        let id = message.get("id").cloned();
        match (message.get("method").and_then(Value::as_str), id) {
            // A server request left unanswered can stall the server's own work.
            (Some(method), Some(id)) => {
                let reply = json!({ "jsonrpc": "2.0", "id": id, "result": server_request_reply(method, &message) });
                if let Err(e) = write_message(&mut *stdin.lock().await, &reply).await {
                    debug!(error = %e, method, "Could not answer language server request");
                }
            }
            (Some(_), None) => {}
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else { continue };
                let Some(waiter) = pending.lock().await.remove(&id) else {
                    continue;
                };
                let result = match message.get("error") {
                    Some(error) => Err(LspError::Rejected {
                        method: String::new(),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                            .to_string(),
                    }),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(result);
            }
            (None, None) => {}
        }
    }
    for (_, waiter) in pending.lock().await.drain() {
        let _ = waiter.send(Err(LspError::Exited));
    }
}

/// `workspace/configuration` expects one entry per requested item; every other
/// request this client receives is satisfied by `null`.
fn server_request_reply(method: &str, message: &Value) -> Value {
    match method {
        "workspace/configuration" => {
            let items = message
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        }
        _ => Value::Null,
    }
}

/// A `file://` URI for an absolute path, each segment percent-encoded.
pub(crate) fn path_to_uri(path: &Path) -> String {
    let encoded: Vec<String> = path
        .to_string_lossy()
        .split('/')
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect();
    format!("file://{}", encoded.join("/"))
}

/// The absolute path a `file://` URI names; `None` for any other scheme.
pub(crate) fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let path = uri.strip_prefix("file://")?;
    urlencoding::decode(path)
        .ok()
        .map(|decoded| std::path::PathBuf::from(decoded.into_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip_paths_with_spaces_and_unicode() {
        let path = Path::new("/workspace/my repo/src/åäö.ts");
        let uri = path_to_uri(path);
        assert_eq!(uri, "file:///workspace/my%20repo/src/%C3%A5%C3%A4%C3%B6.ts");
        assert_eq!(uri_to_path(&uri).unwrap(), path);
        assert!(uri_to_path("untitled:Untitled-1").is_none());
    }

    #[tokio::test]
    async fn reads_framed_messages_until_end_of_stream() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let raw = format!(
            "Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc\r\n\r\n{body}",
            body.len()
        );
        let mut reader = BufReader::new(raw.as_bytes());
        let message = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(message["id"], 1);
        assert!(read_message(&mut reader).await.unwrap().is_none());
    }

    #[test]
    fn configuration_requests_get_one_entry_per_item() {
        let request = json!({ "params": { "items": [{}, {}] } });
        assert_eq!(
            server_request_reply("workspace/configuration", &request),
            json!([null, null])
        );
        assert_eq!(
            server_request_reply("client/registerCapability", &request),
            Value::Null
        );
    }
}
