//! T3 terminal RPC adapter.
//!
//! The PTY lives in the Sandbox because that is where the checkout and
//! credentials live. This module only authorizes T3's thread/cwd-shaped
//! requests, calls the sandbox gRPC terminal surface, and projects protobuf
//! snapshots/events into T3's JSON contract. A small in-process hub feeds the
//! global terminal event and metadata subscriptions for clients attached to
//! this backend instance.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use sandbox_proto::terminal_attach_stream_event::Event as ProtoEvent;
use sandbox_proto::{
    TerminalAttachRequest, TerminalAttachStreamEvent, TerminalClearRequest, TerminalCloseRequest,
    TerminalOpenRequest, TerminalResizeRequest, TerminalRestartRequest, TerminalSessionSnapshot,
    TerminalSessionStatus, TerminalWriteRequest,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use tonic::Streaming;
use tracing::{debug, warn};

use super::dao::ConversationDao;
use super::workspace::{self, WorkspaceAccessError};
use moatless_backend_core::models::SandboxDesiredState;
use moatless_backend_core::sandbox::{SandboxConnector, SandboxError, classify_grpc_error};
use moatless_backend_core::scope::ScopeContext;

const TERMINAL_EVENT_BUFFER: usize = 512;

#[derive(Clone)]
pub struct TerminalHub {
    inner: Arc<Mutex<HubInner>>,
    events: broadcast::Sender<Value>,
    metadata: broadcast::Sender<Value>,
}

struct HubInner {
    summaries: HashMap<TerminalKey, Value>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TerminalKey {
    thread_id: String,
    terminal_id: String,
}

impl Default for TerminalHub {
    fn default() -> Self {
        let (events, _) = broadcast::channel(TERMINAL_EVENT_BUFFER);
        let (metadata, _) = broadcast::channel(TERMINAL_EVENT_BUFFER);
        Self {
            inner: Arc::new(Mutex::new(HubInner {
                summaries: HashMap::new(),
            })),
            events,
            metadata,
        }
    }
}

impl TerminalHub {
    pub fn subscribe_events(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    pub async fn metadata_snapshot(&self) -> Value {
        let inner = self.inner.lock().await;
        json!({
            "type": "snapshot",
            "terminals": inner.summaries.values().cloned().collect::<Vec<_>>()
        })
    }

    pub fn subscribe_metadata(&self) -> broadcast::Receiver<Value> {
        self.metadata.subscribe()
    }

    async fn observe_snapshot(&self, snapshot: &TerminalSessionSnapshot) -> Option<Value> {
        let summary = terminal_summary(snapshot);
        let key = key(snapshot.thread_id.clone(), snapshot.terminal_id.clone());
        let previous = {
            let mut inner = self.inner.lock().await;
            inner.summaries.insert(key, summary.clone())
        };
        let _ = self.metadata.send(json!({
            "type": "upsert",
            "terminal": summary
        }));
        previous
    }

    async fn observe_event(&self, event: &TerminalAttachStreamEvent) -> Option<Value> {
        let value = terminal_event(event)?;
        match event.event.as_ref()? {
            ProtoEvent::Snapshot(snapshot) => {
                self.observe_snapshot(snapshot).await;
            }
            ProtoEvent::Started(started) => {
                if let Some(snapshot) = &started.snapshot {
                    self.observe_snapshot(snapshot).await;
                }
            }
            ProtoEvent::Restarted(restarted) => {
                if let Some(snapshot) = &restarted.snapshot {
                    self.observe_snapshot(snapshot).await;
                }
            }
            ProtoEvent::Exited(_) | ProtoEvent::Error(_) => {
                let key = key(event.thread_id.clone(), event.terminal_id.clone());
                let maybe_summary = {
                    let mut inner = self.inner.lock().await;
                    inner.summaries.get_mut(&key).map(|summary| {
                        summary["status"] = if matches!(event.event, Some(ProtoEvent::Exited(_))) {
                            json!("exited")
                        } else {
                            json!("error")
                        };
                        summary["pid"] = Value::Null;
                        summary["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
                        summary.clone()
                    })
                };
                if let Some(summary) = maybe_summary {
                    let _ = self.metadata.send(json!({
                        "type": "upsert",
                        "terminal": summary
                    }));
                }
            }
            ProtoEvent::Closed(_) => {
                self.remove(&event.thread_id, &event.terminal_id).await;
            }
            ProtoEvent::Output(_) | ProtoEvent::Cleared(_) | ProtoEvent::Activity(_) => {}
        }
        if !matches!(event.event.as_ref(), Some(ProtoEvent::Snapshot(_))) {
            let _ = self.events.send(value.clone());
        }
        Some(value)
    }

    pub async fn publish_clear(&self, thread_id: &str, terminal_id: &str) {
        let _ = self.events.send(json!({
            "type": "cleared",
            "threadId": thread_id,
            "terminalId": terminal_id
        }));
    }

    pub async fn close(&self, thread_id: &str, terminal_id: Option<&str>) {
        if let Some(terminal_id) = terminal_id {
            self.remove(thread_id, terminal_id).await;
            let _ = self.events.send(json!({
                "type": "closed",
                "threadId": thread_id,
                "terminalId": terminal_id
            }));
            return;
        }

        let removed = {
            let mut inner = self.inner.lock().await;
            let keys: Vec<_> = inner
                .summaries
                .keys()
                .filter(|key| key.thread_id == thread_id)
                .cloned()
                .collect();
            for key in &keys {
                inner.summaries.remove(key);
            }
            keys
        };
        for key in removed {
            let _ = self.metadata.send(json!({
                "type": "remove",
                "threadId": key.thread_id,
                "terminalId": key.terminal_id
            }));
            let _ = self.events.send(json!({
                "type": "closed",
                "threadId": key.thread_id,
                "terminalId": key.terminal_id
            }));
        }
    }

    async fn remove(&self, thread_id: &str, terminal_id: &str) {
        {
            let mut inner = self.inner.lock().await;
            inner
                .summaries
                .remove(&key(thread_id.to_string(), terminal_id.to_string()));
        }
        let _ = self.metadata.send(json!({
            "type": "remove",
            "threadId": thread_id,
            "terminalId": terminal_id
        }));
    }
}

pub async fn open(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, TerminalError> {
    open_reporting_new_process(conversations, sandboxes, hub, viewer, payload)
        .await
        .map(|(snapshot, _)| snapshot)
}

/// `open`, plus whether this call is what started the session's current process.
///
/// `terminal.open` starts a shell or reuses the one already under that id, and
/// a running snapshot looks the same either way. A caller that opens a terminal
/// in order to *put a command in it* needs the difference: writing into a
/// reused session types into whatever is already running there. `scripts.run`
/// is that caller.
///
/// The answer comes from the sandbox, which owns the session and so can tell a
/// fresh spawn from a reuse no matter which backend replica asks. Only a sandbox
/// too old to report it falls back to the replica-local pid heuristic.
pub async fn open_reporting_new_process(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(Value, bool), TerminalError> {
    let request = TerminalOpenRequest {
        thread_id: required_string(payload, "threadId")?.to_string(),
        terminal_id: required_string(payload, "terminalId")?.to_string(),
        cwd: required_string(payload, "cwd")?.to_string(),
        worktree_path: optional_string(payload, "worktreePath")?,
        cols: optional_u32(payload, "cols")?.unwrap_or(0),
        rows: optional_u32(payload, "rows")?.unwrap_or(0),
        env: optional_env(payload)?,
    };
    let task_id = workspace::addressed_conversation(conversations, viewer, &request.cwd).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    let snapshot = client
        .terminal_open(request)
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_open")))?;
    let previous = hub.observe_snapshot(&snapshot).await;
    // The sandbox owns the session and answers this authoritatively: its reply
    // holds across replicas and restarts, where `previous` — this replica's
    // memory of past opens — does not. The pid heuristic is the fallback for a
    // sandbox too old to report the flag.
    let is_new_process = snapshot
        .started_new_process
        .unwrap_or_else(|| snapshot_is_new_process(previous.as_ref(), &snapshot));
    if is_new_process {
        let event = json!({
            "type": "started",
            "threadId": snapshot.thread_id,
            "terminalId": snapshot.terminal_id,
            "sequence": snapshot.sequence,
            "snapshot": terminal_snapshot(&snapshot)
        });
        let _ = hub.events.send(event);
    }
    Ok((terminal_snapshot(&snapshot), is_new_process))
}

pub async fn attach(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Streaming<TerminalAttachStreamEvent>, TerminalError> {
    let thread_id = required_string(payload, "threadId")?.to_string();
    let task_id = match optional_string(payload, "cwd")? {
        Some(cwd) => workspace::addressed_conversation(conversations, viewer, &cwd).await?,
        None => authorized_thread(conversations, viewer, &thread_id).await?,
    };
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    let request = TerminalAttachRequest {
        thread_id,
        terminal_id: required_string(payload, "terminalId")?.to_string(),
        cwd: optional_string(payload, "cwd")?,
        worktree_path: optional_string(payload, "worktreePath")?,
        cols: optional_u32(payload, "cols")?.unwrap_or(0),
        rows: optional_u32(payload, "rows")?.unwrap_or(0),
        env: optional_env(payload)?,
        restart_if_not_running: payload
            .get("restartIfNotRunning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    client
        .terminal_attach(request)
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_attach")))
}

pub async fn write(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), TerminalError> {
    let thread_id = required_string(payload, "threadId")?.to_string();
    let task_id = authorized_thread(conversations, viewer, &thread_id).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    client
        .terminal_write(TerminalWriteRequest {
            thread_id,
            terminal_id: required_string(payload, "terminalId")?.to_string(),
            data: required_raw_string(payload, "data")?.to_string(),
        })
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_write")))
}

pub async fn resize(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), TerminalError> {
    let thread_id = required_string(payload, "threadId")?.to_string();
    let task_id = authorized_thread(conversations, viewer, &thread_id).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    client
        .terminal_resize(TerminalResizeRequest {
            thread_id,
            terminal_id: required_string(payload, "terminalId")?.to_string(),
            cols: required_u32(payload, "cols")?,
            rows: required_u32(payload, "rows")?,
        })
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_resize")))
}

pub async fn clear(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), TerminalError> {
    let thread_id = required_string(payload, "threadId")?.to_string();
    let terminal_id = required_string(payload, "terminalId")?.to_string();
    let task_id = authorized_thread(conversations, viewer, &thread_id).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    client
        .terminal_clear(TerminalClearRequest {
            thread_id: thread_id.clone(),
            terminal_id: terminal_id.clone(),
        })
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_clear")))?;
    hub.publish_clear(&thread_id, &terminal_id).await;
    Ok(())
}

pub async fn restart(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, TerminalError> {
    let request = TerminalRestartRequest {
        thread_id: required_string(payload, "threadId")?.to_string(),
        terminal_id: required_string(payload, "terminalId")?.to_string(),
        cwd: required_string(payload, "cwd")?.to_string(),
        worktree_path: optional_string(payload, "worktreePath")?,
        cols: required_u32(payload, "cols")?,
        rows: required_u32(payload, "rows")?,
        env: optional_env(payload)?,
    };
    let task_id = workspace::addressed_conversation(conversations, viewer, &request.cwd).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    let snapshot = client
        .terminal_restart(request)
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_restart")))?;
    hub.observe_snapshot(&snapshot).await;
    let event = json!({
        "type": "restarted",
        "threadId": snapshot.thread_id,
        "terminalId": snapshot.terminal_id,
        "sequence": snapshot.sequence,
        "snapshot": terminal_snapshot(&snapshot)
    });
    let _ = hub.events.send(event);
    Ok(terminal_snapshot(&snapshot))
}

pub async fn close(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<(), TerminalError> {
    let thread_id = required_string(payload, "threadId")?.to_string();
    let terminal_id = optional_string(payload, "terminalId")?;
    let task_id = authorized_thread(conversations, viewer, &thread_id).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    client
        .terminal_close(TerminalCloseRequest {
            thread_id: thread_id.clone(),
            terminal_id: terminal_id.clone(),
            delete_history: payload
                .get("deleteHistory")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_close")))?;
    hub.close(&thread_id, terminal_id.as_deref()).await;
    Ok(())
}

pub async fn forward_attach_stream(
    mut stream: Streaming<TerminalAttachStreamEvent>,
    hub: TerminalHub,
    request_id: super::RequestId,
    frames: tokio::sync::mpsc::Sender<Value>,
) {
    while let Some(next) = stream.next().await {
        let event = match next {
            Ok(event) => event,
            Err(error) => {
                warn!(%error, "Terminal attach stream ended with a sandbox error");
                let _ = frames
                    .send(super::failure_exit(
                        &request_id,
                        "The terminal stream ended unexpectedly",
                    ))
                    .await;
                return;
            }
        };
        let Some(value) = hub.observe_event(&event).await.map(attach_event) else {
            continue;
        };
        let chunk = json!({
            "_tag": "Chunk",
            "requestId": request_id.wire(),
            "values": [value]
        });
        if frames.send(chunk).await.is_err() {
            return;
        }
    }
}

pub async fn forward_value_subscription(
    mut rx: broadcast::Receiver<Value>,
    request_id: super::RequestId,
    frames: tokio::sync::mpsc::Sender<Value>,
) {
    loop {
        match rx.recv().await {
            Ok(value) => {
                let chunk = json!({
                    "_tag": "Chunk",
                    "requestId": request_id.wire(),
                    "values": [value]
                });
                if frames.send(chunk).await.is_err() {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                debug!(missed, "A T3 terminal subscriber lagged");
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[derive(Debug)]
pub enum TerminalError {
    Workspace(WorkspaceAccessError),
    Sandbox(SandboxError),
    Malformed(&'static str),
}

impl TerminalError {
    fn sandbox(error: SandboxError) -> Self {
        Self::Sandbox(error)
    }
}

impl From<WorkspaceAccessError> for TerminalError {
    fn from(value: WorkspaceAccessError) -> Self {
        Self::Workspace(value)
    }
}

impl From<sqlx::Error> for TerminalError {
    fn from(value: sqlx::Error) -> Self {
        Self::Workspace(WorkspaceAccessError::Database(value))
    }
}

pub fn typed_error(error: &TerminalError, payload: &Value) -> Option<Value> {
    let thread_id = payload
        .get("threadId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let terminal_id = payload
        .get("terminalId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match error {
        TerminalError::Workspace(WorkspaceAccessError::Sandbox(SandboxError::NotFound(_)))
        | TerminalError::Sandbox(SandboxError::NotFound(_)) => Some(json!({
            "_tag": "TerminalSessionLookupError",
            "threadId": thread_id,
            "terminalId": terminal_id
        })),
        TerminalError::Workspace(WorkspaceAccessError::Sandbox(SandboxError::BadRequest(
            detail,
        )))
        | TerminalError::Sandbox(SandboxError::BadRequest(detail))
            if detail.contains("not running") =>
        {
            Some(json!({
                "_tag": "TerminalNotRunningError",
                "threadId": thread_id,
                "terminalId": terminal_id
            }))
        }
        TerminalError::Workspace(WorkspaceAccessError::Sandbox(SandboxError::BadRequest(
            detail,
        )))
        | TerminalError::Sandbox(SandboxError::BadRequest(detail)) => {
            Some(cwd_error(payload, detail))
        }
        TerminalError::Workspace(WorkspaceAccessError::NotRunning(_))
        | TerminalError::Sandbox(SandboxError::NotReady(_)) => Some(json!({
            "_tag": "TerminalNotRunningError",
            "threadId": thread_id,
            "terminalId": terminal_id
        })),
        TerminalError::Workspace(WorkspaceAccessError::NoWorktree)
        | TerminalError::Workspace(WorkspaceAccessError::Unaddressable) => Some(json!({
            "_tag": "TerminalCwdStatError",
            "cwd": payload.get("cwd").and_then(Value::as_str).unwrap_or_default(),
            "cause": "This conversation's workspace is not available."
        })),
        TerminalError::Malformed(field) => Some(json!({
            "_tag": "TerminalCwdStatError",
            "cwd": payload.get("cwd").and_then(Value::as_str).unwrap_or_default(),
            "cause": format!("terminal request is missing {field}")
        })),
        _ => None,
    }
}

pub fn message(error: &TerminalError) -> String {
    match error {
        TerminalError::Workspace(error) => error.message(),
        TerminalError::Sandbox(error) => error.to_string(),
        TerminalError::Malformed(field) => format!("terminal request is missing {field}"),
    }
}

fn cwd_error(payload: &Value, detail: &str) -> Value {
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(path) = detail.strip_prefix("terminal cwd does not exist: ") {
        return json!({
            "_tag": "TerminalCwdNotFoundError",
            "cwd": path
        });
    }
    if let Some(path) = detail.strip_prefix("terminal cwd is not a directory: ") {
        return json!({
            "_tag": "TerminalCwdNotDirectoryError",
            "cwd": path
        });
    }
    json!({
        "_tag": "TerminalCwdStatError",
        "cwd": cwd,
        "cause": detail
    })
}

async fn authorized_thread(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    thread_id: &str,
) -> Result<String, TerminalError> {
    let named = [thread_id.to_string()];
    let conversation = conversations
        .list_followed_by_ids(viewer, &named)
        .await?
        .into_iter()
        .next()
        .ok_or(WorkspaceAccessError::Unaddressable)?;
    match conversation.sandbox_desired_state {
        Some(SandboxDesiredState::Running | SandboxDesiredState::Stopped) => {
            Ok(conversation.task_id)
        }
        Some(SandboxDesiredState::Removed) | None => Err(WorkspaceAccessError::NoWorktree.into()),
    }
}

fn as_not_running(error: SandboxError) -> TerminalError {
    TerminalError::Workspace(WorkspaceAccessError::NotRunning(error))
}

fn required_string<'a>(payload: &'a Value, field: &'static str) -> Result<&'a str, TerminalError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(TerminalError::Malformed(field))
}

fn required_raw_string<'a>(
    payload: &'a Value,
    field: &'static str,
) -> Result<&'a str, TerminalError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(TerminalError::Malformed(field))
}

fn optional_string(payload: &Value, field: &'static str) -> Result<Option<String>, TerminalError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .map(Some)
            .ok_or(TerminalError::Malformed(field)),
    }
}

fn required_u32(payload: &Value, field: &'static str) -> Result<u32, TerminalError> {
    optional_u32(payload, field)?.ok_or(TerminalError::Malformed(field))
}

fn optional_u32(payload: &Value, field: &'static str) -> Result<Option<u32>, TerminalError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .map(Some)
            .ok_or(TerminalError::Malformed(field)),
    }
}

fn optional_env(payload: &Value) -> Result<HashMap<String, String>, TerminalError> {
    let Some(value) = payload.get("env") else {
        return Ok(HashMap::new());
    };
    let Some(object) = value.as_object() else {
        return Err(TerminalError::Malformed("env"));
    };
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_string()))
                .ok_or(TerminalError::Malformed("env"))
        })
        .collect()
}

fn terminal_snapshot(snapshot: &TerminalSessionSnapshot) -> Value {
    json!({
        "threadId": snapshot.thread_id,
        "terminalId": snapshot.terminal_id,
        "cwd": snapshot.cwd,
        "worktreePath": snapshot.worktree_path,
        "status": status(snapshot.status),
        "pid": snapshot.pid,
        "history": snapshot.history,
        "exitCode": snapshot.exit_code,
        "exitSignal": snapshot.exit_signal,
        "label": snapshot.label,
        "updatedAt": snapshot.updated_at,
        "sequence": snapshot.sequence
    })
}

fn terminal_summary(snapshot: &TerminalSessionSnapshot) -> Value {
    json!({
        "threadId": snapshot.thread_id,
        "terminalId": snapshot.terminal_id,
        "cwd": snapshot.cwd,
        "worktreePath": snapshot.worktree_path,
        "status": status(snapshot.status),
        "pid": snapshot.pid,
        "exitCode": snapshot.exit_code,
        "exitSignal": snapshot.exit_signal,
        "hasRunningSubprocess": false,
        "label": snapshot.label,
        "updatedAt": snapshot.updated_at
    })
}

fn snapshot_is_new_process(previous: Option<&Value>, snapshot: &TerminalSessionSnapshot) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    previous.get("status").and_then(Value::as_str) != Some("running")
        || previous.get("pid").and_then(Value::as_u64) != snapshot.pid.map(u64::from)
}

fn terminal_event(event: &TerminalAttachStreamEvent) -> Option<Value> {
    let base = |event_type: &str| {
        json!({
            "type": event_type,
            "threadId": event.thread_id,
            "terminalId": event.terminal_id,
            "sequence": event.sequence
        })
    };
    let proto = event.event.as_ref()?;
    Some(match proto {
        ProtoEvent::Snapshot(snapshot) => json!({
            "type": "snapshot",
            "snapshot": terminal_snapshot(snapshot)
        }),
        ProtoEvent::Started(started) => json!({
            "type": "started",
            "threadId": event.thread_id,
            "terminalId": event.terminal_id,
            "sequence": event.sequence,
            "snapshot": started.snapshot.as_ref().map_or(Value::Null, terminal_snapshot)
        }),
        ProtoEvent::Output(output) => {
            let mut value = base("output");
            value["data"] = json!(output.data);
            value
        }
        ProtoEvent::Exited(exited) => {
            let mut value = base("exited");
            value["exitCode"] = json!(exited.exit_code);
            value["exitSignal"] = json!(exited.exit_signal);
            value
        }
        ProtoEvent::Closed(_) => base("closed"),
        ProtoEvent::Error(error) => {
            let mut value = base("error");
            value["message"] = json!(error.message);
            value
        }
        ProtoEvent::Cleared(_) => base("cleared"),
        ProtoEvent::Restarted(restarted) => json!({
            "type": "restarted",
            "threadId": event.thread_id,
            "terminalId": event.terminal_id,
            "sequence": event.sequence,
            "snapshot": restarted.snapshot.as_ref().map_or(Value::Null, terminal_snapshot)
        }),
        ProtoEvent::Activity(activity) => {
            let mut value = base("activity");
            value["hasRunningSubprocess"] = json!(activity.has_running_subprocess);
            value["label"] = json!(activity.label);
            value
        }
    })
}

fn attach_event(value: Value) -> Value {
    if value.get("type").and_then(Value::as_str) == Some("started") {
        json!({
            "type": "snapshot",
            "snapshot": value.get("snapshot").cloned().unwrap_or(Value::Null)
        })
    } else {
        value
    }
}

fn status(status: i32) -> &'static str {
    match TerminalSessionStatus::try_from(status).unwrap_or(TerminalSessionStatus::Error) {
        TerminalSessionStatus::Starting => "starting",
        TerminalSessionStatus::Running => "running",
        TerminalSessionStatus::Exited => "exited",
        TerminalSessionStatus::Error => "error",
    }
}

fn key(thread_id: String, terminal_id: String) -> TerminalKey {
    TerminalKey {
        thread_id,
        terminal_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_snapshot(pid: u32) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            thread_id: "thread-1".to_string(),
            terminal_id: "term-1".to_string(),
            cwd: "/moatless/tasks/task-1".to_string(),
            worktree_path: None,
            status: TerminalSessionStatus::Running.into(),
            pid: Some(pid),
            history: String::new(),
            exit_code: None,
            exit_signal: None,
            label: "Terminal 1".to_string(),
            updated_at: "2026-08-02T00:00:00.000Z".to_string(),
            sequence: 1,
            started_new_process: None,
        }
    }

    #[test]
    fn running_snapshot_with_same_pid_is_not_a_new_process() {
        let snapshot = running_snapshot(4242);
        let previous = terminal_summary(&snapshot);

        assert!(!snapshot_is_new_process(Some(&previous), &snapshot));
    }

    #[test]
    fn running_snapshot_with_changed_pid_is_a_new_process() {
        let snapshot = running_snapshot(4242);
        let previous = terminal_summary(&running_snapshot(1111));

        assert!(snapshot_is_new_process(Some(&previous), &snapshot));
    }

    #[test]
    fn required_raw_string_preserves_terminal_control_input() {
        let payload = json!({ "data": "\r" });

        assert_eq!(required_raw_string(&payload, "data").unwrap(), "\r");
    }

    #[test]
    fn required_raw_string_accepts_whitespace_terminal_input() {
        let payload = json!({ "data": " " });

        assert_eq!(required_raw_string(&payload, "data").unwrap(), " ");
    }

    #[test]
    fn required_raw_string_rejects_empty_terminal_input() {
        let payload = json!({ "data": "" });

        assert!(matches!(
            required_raw_string(&payload, "data"),
            Err(TerminalError::Malformed("data"))
        ));
    }
}
