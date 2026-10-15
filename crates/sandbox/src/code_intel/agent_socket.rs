//! One Unix socket per configured server, through which the agent's `LSP` tool
//! reaches the server the UI uses. Claude Code starts `moat lsp-proxy`, which
//! bridges its stdio to the socket, so a checkout runs one rust-analyzer
//! rather than two.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moatless_workspace_types::workspace::LanguageServer;
use serde_json::{Value, json};
use tokio::io::BufReader;
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::lsp_client::{LspError, read_message, uri_to_path, write_message};
use super::servers::{CodeIntel, Server};

/// The agent waits on these itself; this only bounds a server that hangs.
const SESSION_REQUEST_TIMEOUT: Duration = Duration::from_mins(5);

/// Where the socket for `server` lives under the sandbox's runtime directory.
pub fn socket_path(runtime_dir: &Path, server: LanguageServer) -> PathBuf {
    runtime_dir
        .join("lsp")
        .join(format!("{}.sock", server.as_str()))
}

/// Bind a socket for every server `code_intel` runs and serve sessions on them
/// until `cancel` fires.
pub fn spawn(
    code_intel: &CodeIntel,
    runtime_dir: &Path,
    cancel: CancellationToken,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let mut listeners = Vec::new();
    for &server in code_intel.enabled() {
        let path = socket_path(runtime_dir, server);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A socket file left by an earlier run refuses the bind.
        let _ = std::fs::remove_file(&path);
        listeners.push((server, UnixListener::bind(&path)?));
        info!(server = %server, socket = %path.display(), "Serving shared language server");
    }
    let code_intel = code_intel.clone();
    Ok(tokio::spawn(async move {
        let next_session = Arc::new(AtomicU64::new(1));
        let accepts = listeners.into_iter().map(|(server, listener)| {
            accept_loop(
                code_intel.clone(),
                server,
                listener,
                Arc::clone(&next_session),
            )
        });
        tokio::select! {
            _ = futures::future::join_all(accepts) => {}
            () = cancel.cancelled() => {}
        }
    }))
}

async fn accept_loop(
    code_intel: CodeIntel,
    language: LanguageServer,
    listener: UnixListener,
    next_session: Arc<AtomicU64>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let id = next_session.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(session(code_intel.clone(), language, stream, id));
            }
            Err(e) => {
                warn!(server = %language, error = %e, "Shared language server socket failed");
                return;
            }
        }
    }
}

type Writer = Arc<Mutex<OwnedWriteHalf>>;

/// Serve one agent connection: answer its `initialize` from the running server,
/// forward its requests, and fold its document sync into the server's.
async fn session(code_intel: CodeIntel, language: LanguageServer, stream: UnixStream, id: u64) {
    let (read, write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let writer: Writer = Arc::new(Mutex::new(write));

    let Ok(Some(initialize)) = read_message(&mut reader).await else {
        return;
    };
    let request_id = initialize.get("id").cloned().unwrap_or(Value::Null);
    if initialize.get("method").and_then(Value::as_str) != Some("initialize") {
        reply_error(&writer, request_id, -32002, "expected initialize").await;
        return;
    }
    let Some(root) = root_of(initialize.get("params").unwrap_or(&Value::Null)) else {
        reply_error(&writer, request_id, -32602, "initialize names no root").await;
        return;
    };
    let server = match code_intel.server(&root, language).await {
        Ok(server) => server,
        Err(e) => {
            reply_error(&writer, request_id, -32603, &e.to_string()).await;
            return;
        }
    };
    let _held = server.hold();
    let result = advertised(server.client.initialize_result());
    reply(
        &writer,
        json!({ "jsonrpc": "2.0", "id": request_id, "result": result }),
    )
    .await;
    debug!(server = %language, session = id, root = %root.display(), "Agent connected to shared language server");

    let diagnostics = tokio::spawn(forward_diagnostics(
        server.client.subscribe(),
        Arc::clone(&writer),
    ));
    loop {
        let message = match read_message(&mut reader).await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(e) => {
                debug!(error = %e, session = id, "Agent sent an unreadable message");
                break;
            }
        };
        if !server.client.is_alive() {
            break;
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        match message.get("id").cloned() {
            Some(request_id) if !method.is_empty() => {
                if method == "shutdown" {
                    reply(
                        &writer,
                        json!({ "jsonrpc": "2.0", "id": request_id, "result": null }),
                    )
                    .await;
                    continue;
                }
                tokio::spawn(forward_request(
                    Arc::clone(&server),
                    Arc::clone(&writer),
                    request_id,
                    method.to_string(),
                    message.get("params").cloned().unwrap_or(Value::Null),
                ));
            }
            // A response to a server request: the server's requests are
            // answered by `LspClient` and never reach a session.
            Some(_) => {}
            None if method == "exit" => break,
            None => {
                let params = message.get("params").unwrap_or(&Value::Null);
                if let Err(e) = notification(&server, id, method, params).await {
                    debug!(error = %e, method, "Could not pass on an agent notification");
                }
            }
        }
    }
    diagnostics.abort();
    server.release(id).await;
}

async fn forward_request(
    server: Arc<Server>,
    writer: Writer,
    id: Value,
    method: String,
    params: Value,
) {
    server.touch();
    let answer = match server
        .client
        .request(&method, params, SESSION_REQUEST_TIMEOUT)
        .await
    {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(LspError::Rejected { code, message, .. }) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
        Err(e) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": e.to_string() } })
        }
    };
    reply(&writer, answer).await;
}

async fn notification(
    server: &Server,
    session: u64,
    method: &str,
    params: &Value,
) -> Result<(), LspError> {
    let path = || {
        params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .and_then(uri_to_path)
    };
    match method {
        "textDocument/didOpen" => {
            let (Some(path), Some(text)) = (path(), params.pointer("/textDocument/text")) else {
                return Ok(());
            };
            let language_id = params
                .pointer("/textDocument/languageId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let text = text.as_str().unwrap_or_default().to_string();
            server.open(session, &path, language_id, text).await
        }
        "textDocument/didChange" => {
            let (Some(path), Some(changes)) = (
                path(),
                params.get("contentChanges").and_then(Value::as_array),
            ) else {
                return Ok(());
            };
            server.change(&path, changes).await
        }
        "textDocument/didClose" => match path() {
            Some(path) => server.close(session, &path).await,
            None => Ok(()),
        },
        "textDocument/didSave"
        | "workspace/didChangeWatchedFiles"
        | "workspace/didChangeConfiguration" => server.client.notify(method, params.clone()).await,
        // `initialized` was the session's own, and the server sent its
        // own long ago; a cancel names an id the server never saw.
        _ => Ok(()),
    }
}

/// Diagnostics are the one notification the agent acts on; progress and log
/// messages are for the sandbox.
async fn forward_diagnostics(mut notifications: broadcast::Receiver<Value>, writer: Writer) {
    loop {
        match notifications.recv().await {
            Ok(message)
                if message.get("method").and_then(Value::as_str)
                    == Some("textDocument/publishDiagnostics") =>
            {
                reply(&writer, message).await;
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// The server's `initialize` result with whole-text sync, so a session's
/// changes never need the server's view of the text to apply.
fn advertised(initialize_result: &Value) -> Value {
    let mut result = initialize_result.clone();
    let save = result
        .pointer("/capabilities/textDocumentSync/save")
        .cloned()
        .unwrap_or(json!({ "includeText": false }));
    if let Some(capabilities) = result
        .get_mut("capabilities")
        .and_then(Value::as_object_mut)
    {
        capabilities.insert(
            "textDocumentSync".to_string(),
            json!({ "openClose": true, "change": 1, "save": save }),
        );
    }
    result
}

/// The directory an `initialize` names: `rootUri`, then `rootPath`, then the
/// first workspace folder.
fn root_of(params: &Value) -> Option<PathBuf> {
    params
        .get("rootUri")
        .and_then(Value::as_str)
        .and_then(uri_to_path)
        .or_else(|| {
            params
                .get("rootPath")
                .and_then(Value::as_str)
                .map(PathBuf::from)
        })
        .or_else(|| {
            params
                .pointer("/workspaceFolders/0/uri")
                .and_then(Value::as_str)
                .and_then(uri_to_path)
        })
}

async fn reply(writer: &Writer, message: Value) {
    if let Err(e) = write_message(&mut *writer.lock().await, &message).await {
        debug!(error = %e, "Agent session closed before a reply");
    }
}

async fn reply_error(writer: &Writer, id: Value, code: i64, message: &str) {
    reply(
        writer,
        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    )
    .await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_offered_whole_text_sync_and_keep_the_rest() {
        let result = json!({
            "capabilities": {
                "textDocumentSync": { "openClose": true, "change": 2, "save": {} },
                "definitionProvider": true,
            },
            "serverInfo": { "name": "rust-analyzer" },
        });
        let offered = advertised(&result);
        assert_eq!(offered["capabilities"]["textDocumentSync"]["change"], 1);
        assert_eq!(
            offered["capabilities"]["textDocumentSync"]["save"],
            json!({})
        );
        assert_eq!(offered["capabilities"]["definitionProvider"], true);
        assert_eq!(offered["serverInfo"]["name"], "rust-analyzer");
    }

    #[test]
    fn the_root_comes_from_the_uri_then_the_path_then_a_folder() {
        assert_eq!(
            root_of(&json!({ "rootUri": "file:///w/a%20b", "rootPath": "/x" })),
            Some(PathBuf::from("/w/a b"))
        );
        assert_eq!(
            root_of(&json!({ "rootPath": "/x" })),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(
            root_of(&json!({ "workspaceFolders": [{ "uri": "file:///y" }] })),
            Some(PathBuf::from("/y"))
        );
        assert_eq!(root_of(&json!({})), None);
    }

    /// An agent session against the real `tsc --lsp`, when installed: the
    /// unsaved text it opens is what the shared server resolves against.
    #[tokio::test]
    async fn a_session_navigates_its_own_unsaved_text_on_the_shared_server() {
        use super::super::lsp_client::path_to_uri;
        let Some(path) = std::env::var_os("PATH") else {
            return;
        };
        if !std::env::split_paths(&path).any(|dir| dir.join("tsc").is_file()) {
            warn!("tsc not on PATH; skipping");
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("w");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("tsconfig.json"), "{}").unwrap();
        std::fs::write(root.join("a.ts"), "export class Greeter {}\n").unwrap();
        std::fs::write(root.join("b.ts"), "").unwrap();
        let intel = CodeIntel::new(&[LanguageServer::Typescript], CancellationToken::new());
        let runtime = dir.path().join("run");
        let _serving = spawn(&intel, &runtime, CancellationToken::new()).unwrap();

        let stream = UnixStream::connect(socket_path(&runtime, LanguageServer::Typescript))
            .await
            .unwrap();
        let (read, mut write) = stream.into_split();
        let mut read = BufReader::new(read);
        let b = path_to_uri(&root.join("b.ts"));
        for message in [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "rootUri": path_to_uri(&root), "capabilities": {} } }),
            json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
                "textDocument": { "uri": b, "languageId": "typescript", "version": 1,
                    "text": "import { Greeter } from \"./a\";\nnew Greeter();\n" } } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition", "params": {
                "textDocument": { "uri": b }, "position": { "line": 1, "character": 5 } } }),
        ] {
            write_message(&mut write, &message).await.unwrap();
        }

        let mut answers = Vec::new();
        while answers.len() < 2 {
            let message = read_message(&mut read).await.unwrap().unwrap();
            if message.get("id").is_some() {
                answers.push(message);
            }
        }
        let change = &answers[0]["result"]["capabilities"]["textDocumentSync"]["change"];
        assert_eq!(*change, 1);
        let found = answers[1]["result"].to_string();
        assert!(found.contains(&path_to_uri(&root.join("a.ts"))), "{found}");
    }
}
