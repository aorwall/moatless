//! A JSON-RPC 2.0 client over one `codex app-server --stdio` subprocess.
//!
//! This is the transport half of moving the Codex harness off `codex exec`. It
//! owns the process and the framing — request-id correlation, notification
//! dispatch, and a reply path for server-initiated requests — and nothing above
//! that. Mapping items to canonical blocks belongs to the harness.
//!
//! [`crate::agent::harness::codex`] is the only consumer: it owns one session per
//! Turn and maps the events below onto canonical blocks.
//!
//! ## The steerability gate
//!
//! A Turn is **not** steerable when `turn/start` returns. Steering with the
//! correct turn id in that window is rejected with `-32600 "no active turn to
//! steer"`. It becomes steerable when the `turn/started` *notification* arrives.
//! Callers must hold input until then; this module does not queue for them.
//!
//! ## Errors arrive on three channels
//!
//! 1. `turn/completed` carries the authoritative [`TurnStatus`] and, when failed,
//!    a [`TurnError`] with a named `codexErrorInfo`.
//! 2. The `error` notification carries the same [`TurnError`] earlier, plus
//!    `willRetry`.
//! 3. A JSON-RPC request error carries only `{code, message, data}`. Every
//!    `turn/steer` rejection measured used `-32600` with a different sentence, so
//!    never branch on the sentence — see [`protocol::JsonRpcErrorInfo`].

// The consumer is the Codex harness, a sibling in this crate: nothing here is
// meant to leave it. Warning keeps that true as the harness grows into it.
#![warn(unreachable_pub)]

pub(crate) mod protocol;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;

use self::protocol::{
    AskForApproval, ClientInfo, ErrorNotification, IncomingMessage, InitializeParams,
    InitializeResponse, ItemNotification, JsonRpcErrorInfo, RequestId, SandboxMode, ThreadEnvelope,
    ThreadForkParams, ThreadResumeParams, ThreadStartParams, ThreadTokenUsageUpdatedNotification,
    TurnCompletedNotification, TurnInterruptParams, TurnPlanUpdatedNotification, TurnStartParams,
    TurnStartResponse, TurnStartedNotification, TurnSteerParams, TurnSteerResponse, UserInput,
};
use super::options::build_prompt_append;
use super::types::AgentRunRequest;

/// Hardcoded final fallback model for Codex sessions.
///
/// Used when neither the request, the repo `agent_config.model`, nor the
/// `DEFAULT_CODEX_MODEL` env var supplies a model. Keep this aligned with the
/// backend's default so direct sandbox runs and orchestrated pods behave alike.
pub(crate) const DEFAULT_CODEX_MODEL: &str = "gpt-5.6-sol";

/// Default Codex reasoning effort, sent as `TurnStartParams.effort`.
pub(crate) const DEFAULT_CODEX_REASONING_EFFORT: &str = "high";

/// Resolve the Codex model with the same priority as Claude:
/// request > repo `agent_config.model` > `DEFAULT_CODEX_MODEL` env > hardcoded default.
pub(crate) fn resolve_codex_model(config: &Config, request: &AgentRunRequest) -> String {
    request
        .model
        .clone()
        .or_else(|| config.agent_config.model.clone())
        .or_else(|| config.default_codex_model.clone())
        .unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string())
}

/// The `params` of one server notification the harness consumes. Every other
/// notification method is logged and dropped.
#[derive(Debug, Clone)]
pub(crate) enum ServerEvent {
    /// The Session id, for `ProcessEvent::SessionReady`.
    ThreadStarted(ThreadEnvelope),
    /// The Turn is now steerable.
    TurnStarted(TurnStartedNotification),
    /// An item the Turn has begun but not finished. Nothing is rendered until it
    /// completes; the harness logs it so a Turn that hangs names what it hung on.
    ItemStarted(ItemNotification),
    ItemCompleted(ItemNotification),
    TokenUsage(ThreadTokenUsageUpdatedNotification),
    /// The running plan tracker — the app-server equivalent of the exec
    /// transport's `todo_list` item.
    PlanUpdated(TurnPlanUpdatedNotification),
    Error(ErrorNotification),
    /// The terminal. Measured to carry `itemsView: "notLoaded"` and no items, so
    /// content must come from the streamed `item/completed` notifications.
    TurnCompleted(TurnCompletedNotification),
}

impl ServerEvent {
    /// Parse a notification, or `Ok(None)` for a method this client ignores.
    fn parse(method: &str, params: serde_json::Value) -> Result<Option<Self>, serde_json::Error> {
        let event = match method {
            "thread/started" => Self::ThreadStarted(serde_json::from_value(params)?),
            "turn/started" => Self::TurnStarted(serde_json::from_value(params)?),
            "item/started" => Self::ItemStarted(serde_json::from_value(params)?),
            "item/completed" => Self::ItemCompleted(serde_json::from_value(params)?),
            "thread/tokenUsage/updated" => Self::TokenUsage(serde_json::from_value(params)?),
            "turn/plan/updated" => Self::PlanUpdated(serde_json::from_value(params)?),
            "error" => Self::Error(serde_json::from_value(params)?),
            "turn/completed" => Self::TurnCompleted(serde_json::from_value(params)?),
            _ => return Ok(None),
        };
        Ok(Some(event))
    }
}

/// Why a request did not produce a result.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RpcError {
    /// The server answered with a JSON-RPC error object.
    #[error("{0}")]
    Rpc(JsonRpcErrorInfo),
    /// The pipe is gone: the process exited, or the reader stopped.
    #[error("app-server transport failed: {0}")]
    Transport(String),
    /// The response arrived but did not match the expected shape.
    #[error("app-server response could not be decoded: {0}")]
    Decode(#[from] serde_json::Error),
}

/// One `codex app-server` process and the JSON-RPC conversation with it.
///
/// `notifications` is the only stream of server-pushed events; requests are made
/// through the methods, which are `&self` so a running Turn can be steered while
/// the caller still owns the receiver.
pub(crate) struct AppServerSession {
    pub(crate) notifications: mpsc::UnboundedReceiver<Result<ServerEvent, String>>,
    rpc: Arc<Rpc>,
    cancel_token: CancellationToken,
}

/// The session owns the process: losing the handle kills the server rather than
/// leaving the supervisor waiting on a child nobody can reach. This also covers
/// [`start`], which drops a half-built session when the handshake fails.
impl Drop for AppServerSession {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

impl AppServerSession {
    /// Tear the process tree down without waiting for the handle to go out of
    /// scope. The Turn is *not* interrupted gracefully — callers that want a
    /// clean stop send `turn/interrupt` first.
    pub(crate) fn cancel(&self) {
        self.cancel_token.cancel();
    }

    /// `thread/start` → the new Session id.
    pub(crate) async fn start_thread(&self, params: ThreadStartParams) -> Result<String, RpcError> {
        let value = self.rpc.request("thread/start", params).await?;
        Ok(serde_json::from_value::<ThreadEnvelope>(value)?.thread.id)
    }

    /// `thread/resume` → the reopened Session id, which equals the one asked for.
    ///
    /// Measured: a resume emits **no** `thread/started` notification, so the
    /// response is the only place the Session id is announced. A caller that
    /// waits for the notification to learn it waits forever on every Turn after
    /// the first.
    pub(crate) async fn resume_thread(
        &self,
        params: ThreadResumeParams,
    ) -> Result<String, RpcError> {
        let value = self.rpc.request("thread/resume", params).await?;
        Ok(serde_json::from_value::<ThreadEnvelope>(value)?.thread.id)
    }

    /// `thread/fork` → the new, forked Session id.
    ///
    /// Like `thread/resume`, a fork announces its id only in the response — no
    /// `thread/started` notification — so the return value is the sole place the
    /// forked Session id appears.
    pub(crate) async fn fork_thread(&self, params: ThreadForkParams) -> Result<String, RpcError> {
        let value = self.rpc.request("thread/fork", params).await?;
        Ok(serde_json::from_value::<ThreadEnvelope>(value)?.thread.id)
    }

    /// `turn/start` → the new Turn id. The Turn is not steerable yet; wait for
    /// [`ServerEvent::TurnStarted`].
    pub(crate) async fn start_turn(&self, params: TurnStartParams) -> Result<String, RpcError> {
        let value = self.rpc.request("turn/start", params).await?;
        Ok(serde_json::from_value::<TurnStartResponse>(value)?.turn.id)
    }

    /// `turn/steer` → the Turn the input joined, which is `turn_id`.
    ///
    /// Any failure means one thing — this Turn cannot take the input — whatever
    /// the message says.
    pub(crate) async fn steer(
        &self,
        thread_id: &str,
        turn_id: &str,
        text: &str,
    ) -> Result<String, RpcError> {
        let params = TurnSteerParams {
            thread_id: thread_id.to_string(),
            expected_turn_id: turn_id.to_string(),
            input: vec![UserInput::text(text)],
        };
        let value = self.rpc.request("turn/steer", params).await?;
        Ok(serde_json::from_value::<TurnSteerResponse>(value)?.turn_id)
    }

    /// `turn/interrupt`. The Turn terminates with
    /// [`protocol::TurnStatus::Interrupted`]; the Session stays resumable.
    pub(crate) async fn interrupt(&self, thread_id: &str, turn_id: &str) -> Result<(), RpcError> {
        let params = TurnInterruptParams {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
        };
        self.rpc.request("turn/interrupt", params).await?;
        Ok(())
    }
}

/// Waiters keyed by the request id, one entry per in-flight [`Rpc::request`].
///
/// The handoff is unbounded and untimed by design. An entry is admitted the
/// moment the id is minted, and it is released by exactly two things: the
/// matching response ([`Rpc::resolve`]), or the stdout stream ending —
/// [`Rpc::fail_all`], which covers EOF, a read error and cancellation. Nothing
/// else drops a waiter, so a server that stays alive and simply never answers
/// leaves its entry in the map and its caller parked forever.
///
/// That is deliberate: the deadline belongs to the Turn, not to the frame. A
/// request is only ever in flight because the harness is waiting on a Turn, and
/// the harness already owns that Turn's cancellation — cancelling it kills the
/// child, which ends the stream, which fails every waiter with a transport
/// error. A per-request timeout here would either duplicate that deadline or
/// contradict it, and it would have to guess a bound for `turn/start`, which
/// legitimately takes as long as the model does.
///
/// This rests entirely on the harness being able to cancel *whenever* a request
/// is in flight, including the opening `initialize` / `thread/*` / `turn/start`
/// before it holds a session — which is why [`start`] takes the token rather
/// than returning one, and why the driver races cancellation against the whole
/// of `submit`.
///
/// Capacity is the number of concurrent requests one Session can have in
/// flight, which is bounded by the harness: a Turn issues them one at a time,
/// plus at most one `turn/steer` or `turn/interrupt` alongside.
type PendingRequests = HashMap<i64, oneshot::Sender<Result<serde_json::Value, JsonRpcErrorInfo>>>;

/// Request-id correlation and the write half of the pipe.
struct Rpc {
    stdin: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    /// Waiters keyed by the id we minted. The guard never crosses an `.await`.
    pending: Mutex<PendingRequests>,
    next_id: AtomicI64,
}

impl Rpc {
    async fn request<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> Result<serde_json::Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        // Register before writing: a fast server can answer before this returns.
        let (tx, rx) = oneshot::channel();
        self.pending().insert(id, tx);

        if let Err(error) = self.write(&message).await {
            self.pending().remove(&id);
            return Err(RpcError::Transport(error.to_string()));
        }

        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(info)) => Err(RpcError::Rpc(info)),
            // The reader dropped every waiter: stdout hit EOF or a read error.
            Err(_) => Err(RpcError::Transport(format!(
                "no response to {method}: the app-server stream closed"
            ))),
        }
    }

    async fn notify<P: Serialize>(&self, method: &str, params: P) -> std::io::Result<()> {
        self.write(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    /// Answer a server-initiated request. An unanswered one can stall the Turn,
    /// so every request gets a reply even though none is recognised.
    async fn reply_error(&self, id: &RequestId, code: i64, message: &str) {
        let reply = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        });
        if let Err(error) = self.write(&reply).await {
            warn!(%error, "Failed to answer app-server request");
        }
    }

    async fn write(&self, message: &serde_json::Value) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&line).await?;
        stdin.flush().await
    }

    /// Hand a response to its waiter. An id with no waiter is a server bug or a
    /// duplicate; log it rather than failing the stream.
    fn resolve(&self, id: i64, outcome: Result<serde_json::Value, JsonRpcErrorInfo>) {
        if let Some(tx) = self.pending().remove(&id) {
            let _ = tx.send(outcome);
        } else {
            warn!(id, "app-server response with no pending request");
        }
    }

    /// Drop every waiter, so a request in flight when the stream ends fails
    /// instead of hanging for the life of the process.
    fn fail_all(&self) {
        self.pending().clear();
    }

    /// A poisoned waiter map is still usable: the data behind it is a plain
    /// `HashMap` and no invariant spans a panic.
    fn pending(&self) -> MutexGuard<'_, PendingRequests> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Spawn the stdout reader and return the request half.
///
/// Split out from [`start`] so the framing can be exercised over an in-memory
/// pipe. The read loop ends at EOF, at a read error, or when `cancel` fires;
/// dropping the pending map then fails every outstanding request.
fn connect(
    reader: impl AsyncRead + Send + Unpin + 'static,
    writer: impl AsyncWrite + Send + Unpin + 'static,
    cancel: CancellationToken,
    events: mpsc::UnboundedSender<Result<ServerEvent, String>>,
) -> Arc<Rpc> {
    let rpc = Arc::new(Rpc {
        stdin: tokio::sync::Mutex::new(Box::new(writer)),
        pending: Mutex::new(HashMap::new()),
        next_id: AtomicI64::new(1),
    });

    let reader_rpc = Arc::clone(&rpc);
    tokio::spawn(async move {
        tokio::select! {
            () = cancel.cancelled() => {}
            () = read_loop(reader, &reader_rpc, &events) => {}
        }
        reader_rpc.fail_all();
    });

    rpc
}

/// Read framed messages until the stream ends, dispatching each one.
async fn read_loop(
    reader: impl AsyncRead + Unpin,
    rpc: &Arc<Rpc>,
    events: &mpsc::UnboundedSender<Result<ServerEvent, String>>,
) {
    let mut lines = BufReader::new(reader).lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                info!("Codex app-server stdout EOF");
                break;
            }
            Err(error) => {
                let _ = events.send(Err(format!(
                    "Failed to read Codex app-server stdout: {error}"
                )));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let message = match serde_json::from_str::<IncomingMessage>(&line) {
            Ok(message) => message,
            Err(error) => {
                // A frame we cannot parse means the stream is no longer
                // trustworthy: stop rather than resynchronize on guesswork.
                let _ = events.send(Err(format!(
                    "Failed to parse Codex app-server message: {error}"
                )));
                break;
            }
        };

        match (message.method, message.id) {
            (Some(method), Some(id)) => {
                warn!(%method, "Unhandled Codex app-server request, replying with an error");
                rpc.reply_error(&id, -32601, "method not supported by this client")
                    .await;
            }
            (Some(method), None) => {
                let params = message.params.unwrap_or(serde_json::Value::Null);
                match ServerEvent::parse(&method, params) {
                    Ok(Some(event)) => {
                        if events.send(Ok(event)).is_err() {
                            info!("Codex app-server event channel closed by receiver");
                            break;
                        }
                    }
                    Ok(None) => info!(%method, "Ignoring Codex app-server notification"),
                    Err(error) => {
                        let _ = events.send(Err(format!(
                            "Failed to parse Codex app-server {method} notification: {error}"
                        )));
                        break;
                    }
                }
            }
            (None, Some(RequestId::Number(id))) => {
                let outcome = match message.error {
                    Some(info) => Err(info),
                    None => Ok(message.result.unwrap_or(serde_json::Value::Null)),
                };
                rpc.resolve(id, outcome);
            }
            (None, Some(RequestId::String(id))) => {
                warn!(id, "app-server response with a non-numeric id");
            }
            (None, None) => warn!(line, "Unrecognized Codex app-server message"),
        }
    }
}

/// Spawn `codex app-server --stdio`, complete the `initialize` → `initialized`
/// handshake, and return before any thread work.
///
/// Spawn parity with `codex exec` (see the spec's parity table): the binary,
/// `CODEX_HOME`, `HOME` and the coordinator-supplied `env` are identical. The
/// difference is stdin, which is a pipe here rather than `Stdio::null()` — the
/// hang the exec transport guarded against with `Stdio::null()` was `codex exec`
/// blocking on an *inherited empty* stdin, and this one is a real pipe that is
/// written to.
///
/// `cancel_token` comes from the caller rather than being minted here, because
/// the handshake this function awaits is itself cancellable work: a caller that
/// only received a token back could not stop a server that never answers
/// `initialize`. Cancelling it kills the child at any point, including before
/// the spawn.
pub(crate) async fn start(
    config: &Config,
    env: Option<HashMap<String, String>>,
    cancel_token: CancellationToken,
) -> Result<AppServerSession, std::io::Error> {
    let mut command = Command::new(&config.codex_cli_path);
    command.kill_on_drop(true);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.current_dir(&config.workspace_dir);
    command.arg("app-server");
    command.arg("--stdio");
    command.env("CODEX_HOME", &config.codex_home_dir);
    command.env("HOME", &config.home_dir);
    if let Some(vars) = env {
        command.envs(vars);
    }

    info!(
        cli = %config.codex_cli_path,
        cwd = %config.workspace_dir,
        home = %config.home_dir,
        codex_home = %config.codex_home_dir,
        "Spawning Codex app-server"
    );
    let mut child = command.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("Codex app-server did not expose stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("Codex app-server did not expose stdout"))?;

    // stderr is logged and nothing more. Credential failures are recognised from
    // the named `codexErrorInfo` on the failed Turn, which is scoped to a
    // `turnId`; background model-refresh stderr is not a turn failure signal
    // (see `credentials::recovery`).
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                info!(line, "Codex app-server stderr");
            }
            info!("Codex app-server stderr stream ended");
        });
    }

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let rpc = connect(stdout, stdin, cancel_token.clone(), events_tx.clone());

    // The client owns the process for the whole Turn, so an exit that was not
    // asked for is a failure of the Turn, not a normal end of stream.
    let supervisor_cancel = cancel_token.clone();
    tokio::spawn(async move {
        tokio::select! {
            () = supervisor_cancel.cancelled() => {
                let _ = child.kill().await;
            }
            status = child.wait() => match status {
                Ok(status) => {
                    info!(%status, "Codex app-server exited");
                    let _ = events_tx.send(Err(format!(
                        "Codex app-server exited with status {status}"
                    )));
                }
                Err(error) => {
                    let _ = events_tx.send(Err(format!(
                        "Failed to wait for the Codex app-server process: {error}"
                    )));
                }
            },
        }
    });

    let session = AppServerSession {
        notifications: events_rx,
        rpc,
        cancel_token,
    };

    let params = InitializeParams {
        client_info: ClientInfo {
            name: "moatless-sandbox".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        // Measured on codex-cli 0.145.0: `turn/steer` works without opting into
        // the experimental API, which matches `codex features list` reporting
        // `steer removed true`.
        capabilities: None,
    };
    let response = session
        .rpc
        .request("initialize", params)
        .await
        .map_err(|error| std::io::Error::other(format!("Codex app-server initialize: {error}")))?;
    let response: InitializeResponse = serde_json::from_value(response)
        .map_err(|error| std::io::Error::other(format!("Codex app-server initialize: {error}")))?;
    info!(
        user_agent = %response.user_agent,
        codex_home = %response.codex_home,
        "Codex app-server initialized"
    );
    session
        .rpc
        .notify("initialized", serde_json::json!({}))
        .await?;

    Ok(session)
}

/// `thread/start` params carrying the `codex exec` spawn parity set.
pub(crate) fn thread_start_params(config: &Config, request: &AgentRunRequest) -> ThreadStartParams {
    ThreadStartParams {
        cwd: Some(config.workspace_dir.clone()),
        model: Some(resolve_codex_model(config, request)),
        sandbox: Some(SandboxMode::DangerFullAccess),
        approval_policy: Some(AskForApproval::Never),
        developer_instructions: Some(build_prompt_append(config, request)),
    }
}

/// `thread/resume` params for `resume <id>`. There is no equivalent of
/// `resume --last`, and none is needed: `build_resume` (`coordinator.rs`) only
/// ever yields `ResumeSession::Last` for the Claude family.
pub(crate) fn thread_resume_params(
    config: &Config,
    request: &AgentRunRequest,
    thread_id: &str,
) -> ThreadResumeParams {
    ThreadResumeParams {
        thread_id: thread_id.to_string(),
        cwd: Some(config.workspace_dir.clone()),
        model: Some(resolve_codex_model(config, request)),
        sandbox: Some(SandboxMode::DangerFullAccess),
        approval_policy: Some(AskForApproval::Never),
        developer_instructions: Some(build_prompt_append(config, request)),
    }
}

/// `thread/fork` params for branching a fork's first Turn off the parent's
/// thread at `last_turn_id` (`None` = the parent's head).
pub(crate) fn thread_fork_params(
    config: &Config,
    source_session_id: &str,
    last_turn_id: Option<&str>,
) -> ThreadForkParams {
    ThreadForkParams {
        thread_id: source_session_id.to_string(),
        last_turn_id: last_turn_id.map(str::to_string),
        cwd: Some(config.workspace_dir.clone()),
        exclude_turns: true,
    }
}

/// `turn/start` params for one Turn's prompt.
pub(crate) fn turn_start_params(
    thread_id: &str,
    prompt: &str,
    config: &Config,
    request: &AgentRunRequest,
) -> TurnStartParams {
    TurnStartParams {
        thread_id: thread_id.to_string(),
        input: vec![UserInput::text(prompt)],
        model: Some(resolve_codex_model(config, request)),
        effort: Some(
            request
                .effort
                .clone()
                .unwrap_or_else(|| DEFAULT_CODEX_REASONING_EFFORT.to_string()),
        ),
    }
}

/// An in-memory server standing in for `codex app-server`, so both the framing
/// tests here and the harness's driver tests can drive a session without a
/// subprocess.
#[cfg(test)]
pub(crate) struct ScriptedServer {
    reader: tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    writer: tokio::io::DuplexStream,
}

#[cfg(test)]
impl ScriptedServer {
    /// The next frame the client wrote.
    pub(crate) async fn next_request(&mut self) -> serde_json::Value {
        let line = self
            .reader
            .next_line()
            .await
            .expect("read client frame")
            .expect("client frame");
        serde_json::from_str(&line).expect("client frame is JSON")
    }

    pub(crate) async fn write(&mut self, message: serde_json::Value) {
        let mut line = serde_json::to_vec(&message).expect("serialize");
        line.push(b'\n');
        self.writer.write_all(&line).await.expect("write");
        self.writer.flush().await.expect("flush");
    }

    /// Answer the next client request with `result`, whatever it asks.
    pub(crate) async fn answer_next(&mut self, result: serde_json::Value) -> serde_json::Value {
        let request = self.next_request().await;
        self.write(serde_json::json!({"id": request["id"], "result": result}))
            .await;
        request
    }
}

/// The client half of a scripted connection: the request plumbing, both ends of
/// the notification stream, the server's end of the pipe, and the token that
/// tears it all down.
#[cfg(test)]
type ScriptedPipe = (
    Arc<Rpc>,
    mpsc::UnboundedSender<Result<ServerEvent, String>>,
    mpsc::UnboundedReceiver<Result<ServerEvent, String>>,
    ScriptedServer,
    CancellationToken,
);

/// A connected client and server with no process behind them.
#[cfg(test)]
fn scripted_pipe() -> ScriptedPipe {
    let (client_read, server_write) = tokio::io::duplex(64 * 1024);
    let (server_read, client_write) = tokio::io::duplex(64 * 1024);
    let (tx, rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let rpc = connect(client_read, client_write, cancel.clone(), tx.clone());
    (
        rpc,
        tx,
        rx,
        ScriptedServer {
            reader: BufReader::new(server_read).lines(),
            writer: server_write,
        },
        cancel,
    )
}

#[cfg(test)]
impl AppServerSession {
    /// A session backed by an in-memory pipe instead of a real process, for
    /// deterministic harness lifecycle tests.
    ///
    /// The returned sender feeds `notifications` directly, so a test can script
    /// server events without framing them; dropping it closes the stream (EOF).
    /// The [`ScriptedServer`] is the other end of the request pipe, for the
    /// requests the harness makes (`turn/interrupt` on cancel).
    pub(crate) fn scripted() -> (
        mpsc::UnboundedSender<Result<ServerEvent, String>>,
        ScriptedServer,
        Self,
    ) {
        let (rpc, tx, rx, server, cancel_token) = scripted_pipe();
        (
            tx,
            server,
            Self {
                notifications: rx,
                rpc,
                cancel_token,
            },
        )
    }

    /// Whether cancellation has been requested — the scripted analog of the real
    /// process tree being torn down.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }
}
