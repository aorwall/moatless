//! A minimal LSP client over a child's stdio: framing, request ids, and the
//! replies a server blocks on when it asks the client something.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, broadcast, oneshot};
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
    Rejected {
        method: String,
        code: i64,
        message: String,
    },
}

/// The client capabilities Claude Code's `LSP` tool declares, plus
/// `workDoneProgress` for [`LspClient::is_busy`]. A server shapes its answers to
/// these, so the agent reading through `shared` gets what it would from its own.
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "window": { "workDoneProgress": true },
        "workspace": { "workspaceFolders": true, "didChangeWatchedFiles": { "dynamicRegistration": false } },
        "textDocument": {
            "synchronization": { "dynamicRegistration": false, "didSave": true },
            "publishDiagnostics": {
                "relatedInformation": true,
                "tagSupport": { "valueSet": [1, 2] },
                "codeDescriptionSupport": true,
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": true },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "callHierarchy": {},
        },
    })
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>;

/// What `read_loop` observes besides responses.
#[derive(Clone)]
struct Observed {
    notifications: broadcast::Sender<Value>,
    /// Work-done progress tokens begun and not yet ended.
    progress: Arc<std::sync::Mutex<HashSet<String>>>,
    alive: Arc<AtomicBool>,
}

/// One running server. Dropping it kills the child.
pub(crate) struct LspClient {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: AtomicI64,
    observed: Observed,
    initialize_result: Value,
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
        let observed = Observed {
            notifications: broadcast::channel(256).0,
            progress: Arc::default(),
            alive: Arc::new(AtomicBool::new(true)),
        };
        tokio::spawn(read_loop(
            BufReader::new(stdout),
            Arc::clone(&stdin),
            Arc::clone(&pending),
            observed.clone(),
        ));

        let mut client = Self {
            stdin,
            pending,
            next_id: AtomicI64::new(1),
            observed,
            initialize_result: Value::Null,
            _child: child,
        };
        let root_uri = path_to_uri(root);
        client.initialize_result = client
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "workspaceFolders": [{ "uri": root_uri, "name": "workspace" }],
                    "capabilities": client_capabilities(),
                }),
                timeout,
            )
            .await?;
        client.notify("initialized", json!({})).await?;
        Ok(client)
    }

    /// The server's answer to `initialize`.
    pub(crate) fn initialize_result(&self) -> &Value {
        &self.initialize_result
    }

    /// Every notification the server sends from now on.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.observed.notifications.subscribe()
    }

    /// Whether the server reports work in progress, such as indexing, during
    /// which it may answer a lookup with nothing.
    pub(crate) fn is_busy(&self) -> bool {
        self.observed
            .progress
            .lock()
            .is_ok_and(|tokens| !tokens.is_empty())
    }

    /// False once the server closed its stdout.
    pub(crate) fn is_alive(&self) -> bool {
        self.observed.alive.load(Ordering::Relaxed)
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
                LspError::Rejected { code, message, .. } => LspError::Rejected {
                    method: method.to_string(),
                    code,
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

pub(super) async fn write_message(
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
pub(super) async fn read_message(
    reader: &mut (impl AsyncBufRead + Unpin),
) -> std::io::Result<Option<Value>> {
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

/// Route responses to their waiting request, answer the server's own requests,
/// and publish its notifications. Ends when the server closes stdout, failing
/// every waiter.
async fn read_loop(
    mut stdout: impl AsyncBufRead + Unpin,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    observed: Observed,
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
            (Some(method), None) => {
                if method == "$/progress" {
                    track_progress(&observed.progress, &message);
                }
                // No receiver is the usual case; nothing is lost that anyone wanted.
                let _ = observed.notifications.send(message);
            }
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else { continue };
                let Some(waiter) = pending.lock().await.remove(&id) else {
                    continue;
                };
                let result = match message.get("error") {
                    Some(error) => Err(LspError::Rejected {
                        method: String::new(),
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
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
    observed.alive.store(false, Ordering::Relaxed);
    for (_, waiter) in pending.lock().await.drain() {
        let _ = waiter.send(Err(LspError::Exited));
    }
}

fn track_progress(progress: &std::sync::Mutex<HashSet<String>>, message: &Value) {
    let Some(params) = message.get("params") else {
        return;
    };
    let token = params
        .get("token")
        .map(Value::to_string)
        .unwrap_or_default();
    let Ok(mut tokens) = progress.lock() else {
        return;
    };
    match params.pointer("/value/kind").and_then(Value::as_str) {
        Some("begin") => {
            tokens.insert(token);
        }
        Some("end") => {
            tokens.remove(&token);
        }
        _ => {}
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
    fn progress_is_busy_from_begin_until_end() {
        let progress = std::sync::Mutex::new(HashSet::new());
        let report = |kind: &str| json!({ "method": "$/progress", "params": { "token": "rustAnalyzer/Indexing", "value": { "kind": kind } } });
        track_progress(&progress, &report("begin"));
        track_progress(&progress, &report("report"));
        assert_eq!(progress.lock().unwrap().len(), 1);
        track_progress(&progress, &report("end"));
        assert!(progress.lock().unwrap().is_empty());
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
