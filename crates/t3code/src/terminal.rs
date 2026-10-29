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
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use tonic::Streaming;
use tracing::warn;

use super::dao::ConversationDao;
use super::workspace::{self, WorkspaceAccessError};
use crate::subscription::{StreamFailure, StreamResult, StreamSink};
use crate::terminal_relay::TerminalRelay;
use moatless_ports::sandbox::SandboxError;
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::SandboxDesiredState;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};

const TERMINAL_EVENT_BUFFER: usize = 512;

#[derive(Clone)]
pub struct TerminalHub {
    inner: Arc<Mutex<HubInner>>,
    events: broadcast::Sender<Value>,
    metadata: broadcast::Sender<Value>,
    /// Identifies this hub on the relay feed, so its own records are skipped.
    /// New per process: the summaries it names live only in this one's memory.
    epoch: Arc<str>,
    relay: TerminalRelay,
}

struct HubInner {
    summaries: HashMap<TerminalKey, Value>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TerminalKey {
    thread_id: String,
    terminal_id: String,
}

/// A hub whose starts reach no other replica, which is what a unit test with no
/// database wants. Production builds one through [`TerminalHub::new`].
#[cfg(test)]
impl Default for TerminalHub {
    fn default() -> Self {
        Self::with_relay(TerminalRelay::disabled())
    }
}

impl TerminalHub {
    pub fn new(relay: TerminalRelay) -> Self {
        Self::with_relay(relay)
    }

    fn with_relay(relay: TerminalRelay) -> Self {
        let (events, _) = broadcast::channel(TERMINAL_EVENT_BUFFER);
        let (metadata, _) = broadcast::channel(TERMINAL_EVENT_BUFFER);
        Self {
            inner: Arc::new(Mutex::new(HubInner {
                summaries: HashMap::new(),
            })),
            events,
            metadata,
            epoch: Arc::from(uuid::Uuid::new_v4().to_string()),
            relay,
        }
    }

    pub(crate) fn epoch(&self) -> &str {
        &self.epoch
    }

    pub(crate) fn relay(&self) -> &TerminalRelay {
        &self.relay
    }

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

    /// Raise a session the environment just started, here and on the other
    /// replicas.
    ///
    /// The summary is already stored by [`Self::observe_snapshot`]; this is the
    /// `announced` upsert that tells a client to open its drawer, plus the feed
    /// record that gets the same to a viewer socketed elsewhere.
    async fn announce_start(&self, task_id: &str, terminal_id: &str, summary: &Value) {
        self.publish_announced(summary);
        self.relay
            .announce(&self.epoch, task_id, terminal_id, summary)
            .await;
    }

    /// Store and raise a session another replica started.
    ///
    /// No re-announcement, or two replicas would relay each other's starts
    /// without end. No authorization either: this hub holds nothing back, and
    /// what reaches a viewer is decided per subscription by
    /// [`watch_terminal_metadata`](crate::rpc::watch::watch_terminal_metadata).
    pub(crate) async fn apply_relayed_start(
        &self,
        thread_id: &str,
        terminal_id: &str,
        summary: Value,
    ) {
        {
            let mut inner = self.inner.lock().await;
            inner.summaries.insert(
                key(thread_id.to_string(), terminal_id.to_string()),
                summary.clone(),
            );
        }
        self.publish_announced(&summary);
    }

    /// An upsert carrying `announced`, which is what separates a session worth
    /// raising a pane for from the upserts every status change produces.
    fn publish_announced(&self, summary: &Value) {
        let _ = self.metadata.send(json!({
            "type": "upsert",
            "terminal": summary,
            "announced": true
        }));
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

/// Every field the contract's `Terminal*Input` payloads carry, each optional.
///
/// Which fields a method requires is that method's check, so a missing one is
/// answered with the `TerminalCwdStatError` [`typed_error`] gives
/// [`TerminalError::Malformed`] rather than refused by the decode.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInput {
    pub thread_id: Option<String>,
    pub terminal_id: Option<String>,
    pub cwd: Option<String>,
    pub worktree_path: Option<String>,
    pub cols: Option<u64>,
    pub rows: Option<u64>,
    pub env: Option<HashMap<String, String>>,
    pub data: Option<String>,
    pub restart_if_not_running: Option<bool>,
    pub delete_history: Option<bool>,
}

/// Open a session for the client that asked for it.
///
/// Announces nothing: the reply travels back down the asking client's own
/// socket, so the replica serving it is already current and no other replica
/// has a viewer waiting on this session. [`open_from_environment`] is the path
/// that does announce.
pub async fn open(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    input: &TerminalInput,
) -> Result<Value, TerminalError> {
    let cwd = required_string(&input.cwd, "cwd")?;
    let task_id = workspace::owned_conversation(conversations, viewer, cwd).await?;
    open_session(sandboxes, hub, &task_id, input)
        .await
        .map(|(snapshot, _)| terminal_snapshot(&snapshot))
}

/// Open a session for the environment, the only path that announces.
///
/// Gated on write: a Script's session runs only its own command. Also reports
/// whether this call started the process, since `scripts.run` must not type
/// into a reused session; the sandbox answers, else a pid heuristic.
pub async fn open_from_environment(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    input: &TerminalInput,
) -> Result<(Value, bool), TerminalError> {
    let cwd = required_string(&input.cwd, "cwd")?;
    let task_id = workspace::written_conversation(conversations, viewer, cwd).await?;
    let (snapshot, is_new_process) = open_session(sandboxes, hub, &task_id, input).await?;
    if is_new_process {
        // `task_id` and the snapshot's `thread_id` are the same Task under two
        // names; the relayed record is keyed by the one the hub stores under.
        hub.announce_start(
            &snapshot.thread_id,
            &snapshot.terminal_id,
            &terminal_summary(&snapshot),
        )
        .await;
    }
    Ok((terminal_snapshot(&snapshot), is_new_process))
}

/// The open both entry points share once each has gated `task_id`, up to the
/// announcement neither of them does here.
async fn open_session(
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    task_id: &str,
    input: &TerminalInput,
) -> Result<(TerminalSessionSnapshot, bool), TerminalError> {
    let mut request = TerminalOpenRequest {
        thread_id: required_string(&input.thread_id, "threadId")?.to_string(),
        terminal_id: required_string(&input.terminal_id, "terminalId")?.to_string(),
        cwd: required_string(&input.cwd, "cwd")?.to_string(),
        worktree_path: optional_string(&input.worktree_path, "worktreePath")?,
        cols: optional_u32(input.cols, "cols")?.unwrap_or(0),
        rows: optional_u32(input.rows, "rows")?.unwrap_or(0),
        env: input.env.clone().unwrap_or_default(),
    };
    stamp_task_id(&mut request.env, task_id);
    let client = sandboxes
        .sandbox_client(task_id)
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
    Ok((snapshot, is_new_process))
}

pub async fn attach(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    input: &TerminalInput,
) -> Result<Streaming<TerminalAttachStreamEvent>, TerminalError> {
    let thread_id = required_string(&input.thread_id, "threadId")?.to_string();
    let task_id = match optional_string(&input.cwd, "cwd")? {
        Some(cwd) => workspace::owned_conversation(conversations, viewer, &cwd).await?,
        None => owned_thread(conversations, viewer, &thread_id).await?,
    };
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    let mut request = TerminalAttachRequest {
        thread_id,
        terminal_id: required_string(&input.terminal_id, "terminalId")?.to_string(),
        cwd: optional_string(&input.cwd, "cwd")?,
        worktree_path: optional_string(&input.worktree_path, "worktreePath")?,
        cols: optional_u32(input.cols, "cols")?.unwrap_or(0),
        rows: optional_u32(input.rows, "rows")?.unwrap_or(0),
        env: input.env.clone().unwrap_or_default(),
        restart_if_not_running: input.restart_if_not_running.unwrap_or(false),
    };
    stamp_task_id(&mut request.env, &task_id);
    client
        .terminal_attach(request)
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_attach")))
}

pub async fn write(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    input: &TerminalInput,
) -> Result<(), TerminalError> {
    let thread_id = required_string(&input.thread_id, "threadId")?;
    let task_id = owned_thread(conversations, viewer, thread_id).await?;
    write_session(sandboxes, &task_id, thread_id, input).await
}

/// [`write`] for the environment: types a Script's own command into the
/// session [`open_from_environment`] started, under the same write gate.
pub async fn write_from_environment(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    input: &TerminalInput,
) -> Result<(), TerminalError> {
    let thread_id = required_string(&input.thread_id, "threadId")?;
    let task_id = written_thread(conversations, viewer, thread_id).await?;
    write_session(sandboxes, &task_id, thread_id, input).await
}

async fn write_session(
    sandboxes: &dyn SandboxConnector,
    task_id: &str,
    thread_id: &str,
    input: &TerminalInput,
) -> Result<(), TerminalError> {
    let client = sandboxes
        .sandbox_client(task_id)
        .await
        .map_err(as_not_running)?;
    client
        .terminal_write(TerminalWriteRequest {
            thread_id: thread_id.to_string(),
            terminal_id: required_string(&input.terminal_id, "terminalId")?.to_string(),
            data: required_raw_string(&input.data, "data")?.to_string(),
        })
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_write")))
}

pub async fn resize(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    input: &TerminalInput,
) -> Result<(), TerminalError> {
    let thread_id = required_string(&input.thread_id, "threadId")?.to_string();
    let task_id = owned_thread(conversations, viewer, &thread_id).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    client
        .terminal_resize(TerminalResizeRequest {
            thread_id,
            terminal_id: required_string(&input.terminal_id, "terminalId")?.to_string(),
            cols: required_u32(input.cols, "cols")?,
            rows: required_u32(input.rows, "rows")?,
        })
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_resize")))
}

pub async fn clear(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    input: &TerminalInput,
) -> Result<(), TerminalError> {
    let thread_id = required_string(&input.thread_id, "threadId")?.to_string();
    let terminal_id = required_string(&input.terminal_id, "terminalId")?.to_string();
    let task_id = owned_thread(conversations, viewer, &thread_id).await?;
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
    input: &TerminalInput,
) -> Result<Value, TerminalError> {
    let mut request = TerminalRestartRequest {
        thread_id: required_string(&input.thread_id, "threadId")?.to_string(),
        terminal_id: required_string(&input.terminal_id, "terminalId")?.to_string(),
        cwd: required_string(&input.cwd, "cwd")?.to_string(),
        worktree_path: optional_string(&input.worktree_path, "worktreePath")?,
        cols: required_u32(input.cols, "cols")?,
        rows: required_u32(input.rows, "rows")?,
        env: input.env.clone().unwrap_or_default(),
    };
    let task_id = workspace::owned_conversation(conversations, viewer, &request.cwd).await?;
    stamp_task_id(&mut request.env, &task_id);
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
    input: &TerminalInput,
) -> Result<(), TerminalError> {
    let thread_id = required_string(&input.thread_id, "threadId")?.to_string();
    let terminal_id = optional_string(&input.terminal_id, "terminalId")?;
    let task_id = owned_thread(conversations, viewer, &thread_id).await?;
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(as_not_running)?;
    client
        .terminal_close(TerminalCloseRequest {
            thread_id: thread_id.clone(),
            terminal_id: terminal_id.clone(),
            delete_history: input.delete_history.unwrap_or(false),
        })
        .await
        .map_err(|error| TerminalError::sandbox(classify_grpc_error(error, "terminal_close")))?;
    hub.close(&thread_id, terminal_id.as_deref()).await;
    Ok(())
}

pub async fn forward_attach_stream(
    mut stream: Streaming<TerminalAttachStreamEvent>,
    hub: TerminalHub,
    sink: StreamSink,
) -> StreamResult {
    while let Some(next) = stream.next().await {
        let event = match next {
            Ok(event) => event,
            Err(error) => {
                warn!(%error, "Terminal attach stream ended with a sandbox error");
                return Err(StreamFailure::message(
                    "The terminal stream ended unexpectedly",
                ));
            }
        };
        let Some(value) = hub.observe_event(&event).await.map(attach_event) else {
            continue;
        };
        if !sink.send(value).await {
            return Ok(());
        }
    }
    Ok(())
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

/// Whether the failure means there was no Sandbox to run in.
///
/// [`typed_error`] and `scripts.run` each map this to their own contract's
/// not-running error; both read this rather than the tag, so the two cannot
/// drift.
pub fn means_not_running(error: &TerminalError) -> bool {
    match error {
        TerminalError::Workspace(WorkspaceAccessError::NotRunning(_))
        | TerminalError::Sandbox(SandboxError::NotReady(_)) => true,
        TerminalError::Workspace(WorkspaceAccessError::Sandbox(SandboxError::BadRequest(
            detail,
        )))
        | TerminalError::Sandbox(SandboxError::BadRequest(detail)) => {
            detail.contains("not running")
        }
        _ => false,
    }
}

pub fn typed_error(error: &TerminalError, input: &TerminalInput) -> Option<Value> {
    let thread_id = input.thread_id.as_deref().unwrap_or_default();
    let terminal_id = input.terminal_id.as_deref().unwrap_or_default();
    let cwd = input.cwd.as_deref().unwrap_or_default();
    if means_not_running(error) {
        return Some(json!({
            "_tag": "TerminalNotRunningError",
            "threadId": thread_id,
            "terminalId": terminal_id
        }));
    }
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
        | TerminalError::Sandbox(SandboxError::BadRequest(detail)) => Some(cwd_error(cwd, detail)),
        TerminalError::Workspace(WorkspaceAccessError::NoWorktree)
        | TerminalError::Workspace(WorkspaceAccessError::Unaddressable) => Some(json!({
            "_tag": "TerminalCwdStatError",
            "cwd": cwd,
            "cause": "This conversation's workspace is not available."
        })),
        TerminalError::Malformed(field) => Some(json!({
            "_tag": "TerminalCwdStatError",
            "cwd": cwd,
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

fn cwd_error(cwd: &str, detail: &str) -> Value {
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

/// The same gate [`workspace::owned_conversation`] applies, for calls that
/// name a thread directly rather than through a `cwd`: [`written_thread`],
/// then owned — a public Task's visitors do not get its shell.
async fn owned_thread(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    thread_id: &str,
) -> Result<String, TerminalError> {
    let task_id = written_thread(conversations, viewer, thread_id).await?;
    if !conversations.owns_conversation(viewer, &task_id).await? {
        return Err(WorkspaceAccessError::NotOwner.into());
    }
    Ok(task_id)
}

/// The same gate [`workspace::written_conversation`] applies, for calls that
/// name a thread directly rather than through a `cwd`: readable, then
/// writable, then holding a worktree.
pub(crate) async fn written_thread(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    thread_id: &str,
) -> Result<String, TerminalError> {
    let conversation = conversations
        .readable_by_id(viewer, thread_id)
        .await?
        .ok_or(WorkspaceAccessError::Unaddressable)?;
    if !conversations
        .writes_conversation(viewer, &conversation.task_id)
        .await?
    {
        return Err(WorkspaceAccessError::NoWrite.into());
    }
    match conversation.sandbox_desired_state() {
        Some(SandboxDesiredState::Running | SandboxDesiredState::Stopped) => {
            Ok(conversation.task_id)
        }
        Some(SandboxDesiredState::Removed) | None => Err(WorkspaceAccessError::NoWorktree.into()),
    }
}

fn as_not_running(error: SandboxError) -> TerminalError {
    TerminalError::Workspace(WorkspaceAccessError::NotRunning(error))
}

fn required_string<'a>(
    value: &'a Option<String>,
    field: &'static str,
) -> Result<&'a str, TerminalError> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(TerminalError::Malformed(field))
}

fn required_raw_string<'a>(
    value: &'a Option<String>,
    field: &'static str,
) -> Result<&'a str, TerminalError> {
    value
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(TerminalError::Malformed(field))
}

/// `None` when absent; a value that trims to nothing is malformed.
fn optional_string(
    value: &Option<String>,
    field: &'static str,
) -> Result<Option<String>, TerminalError> {
    match value {
        None => Ok(None),
        Some(value) => Some(value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| Some(value.to_string()))
            .ok_or(TerminalError::Malformed(field)),
    }
}

fn required_u32(value: Option<u64>, field: &'static str) -> Result<u32, TerminalError> {
    optional_u32(value, field)?.ok_or(TerminalError::Malformed(field))
}

fn optional_u32(value: Option<u64>, field: &'static str) -> Result<Option<u32>, TerminalError> {
    value
        .map(|number| u32::try_from(number).map_err(|_| TerminalError::Malformed(field)))
        .transpose()
}

/// Export the Task this terminal was authorized against as `TASK_ID`.
///
/// Stamped per Member; see `crates/sandbox/AGENTS.md` "Spawning a child
/// process".
fn stamp_task_id(env: &mut HashMap<String, String>, task_id: &str) {
    env.insert("TASK_ID".to_string(), task_id.to_string());
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

    /// A relayed start has to reach a subscriber that was already listening,
    /// carrying `announced` — that flag is the whole difference between a
    /// session worth raising the drawer for and the upserts every status change
    /// produces.
    #[tokio::test]
    async fn a_relayed_start_reaches_the_metadata_subscribers_as_announced() {
        let hub = TerminalHub::default();
        let mut metadata = hub.subscribe_metadata();

        hub.apply_relayed_start("thread-1", "term-1", terminal_summary(&running_snapshot(7)))
            .await;

        let event = metadata.try_recv().expect("an upsert was published");
        assert_eq!(event["type"], "upsert");
        assert_eq!(event["announced"], true);
        assert_eq!(event["terminal"]["terminalId"], "term-1");
    }

    /// The relayed session must also be listed, or a client that subscribes
    /// after the relay applied it would never learn the terminal exists.
    #[tokio::test]
    async fn a_relayed_start_is_listed_in_the_next_snapshot() {
        let hub = TerminalHub::default();

        hub.apply_relayed_start("thread-1", "term-1", terminal_summary(&running_snapshot(7)))
            .await;

        let snapshot = hub.metadata_snapshot().await;
        let terminals = snapshot["terminals"].as_array().expect("a terminal list");
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0]["terminalId"], "term-1");
    }

    /// Applying the same session twice is what a replica reading its own record
    /// back would do; it must leave one terminal, not two.
    #[tokio::test]
    async fn applying_the_same_start_twice_leaves_one_terminal() {
        let hub = TerminalHub::default();
        let summary = terminal_summary(&running_snapshot(7));

        hub.apply_relayed_start("thread-1", "term-1", summary.clone())
            .await;
        hub.apply_relayed_start("thread-1", "term-1", summary).await;

        let snapshot = hub.metadata_snapshot().await;
        assert_eq!(snapshot["terminals"].as_array().expect("a list").len(), 1);
    }

    /// A hub with no database still raises the session for the viewers on its
    /// own replica: the relay is what the *other* replicas need, and a
    /// deployment of one must not depend on it.
    #[tokio::test]
    async fn a_start_is_announced_locally_even_with_the_relay_disabled() {
        let hub = TerminalHub::default();
        let mut metadata = hub.subscribe_metadata();

        hub.announce_start(
            "thread-1",
            "term-1",
            &terminal_summary(&running_snapshot(7)),
        )
        .await;

        let event = metadata.try_recv().expect("an upsert was published");
        assert_eq!(event["announced"], true);
    }

    /// Observing a snapshot is all a client-served `terminal.open` does, and it
    /// must not raise anyone's drawer: only the environment's path announces,
    /// which is the rule `0170_terminal_session_relay.sql` records.
    #[tokio::test]
    async fn observing_a_snapshot_publishes_an_upsert_that_is_not_announced() {
        let hub = TerminalHub::default();
        let mut metadata = hub.subscribe_metadata();

        hub.observe_snapshot(&running_snapshot(7)).await;

        let event = metadata.try_recv().expect("an upsert was published");
        assert_eq!(event["type"], "upsert");
        assert_ne!(event["announced"], true);
    }

    #[test]
    fn stamp_task_id_replaces_a_client_supplied_task_id() {
        let mut env = HashMap::from([
            ("TASK_ID".to_string(), "parent-task".to_string()),
            ("EDITOR".to_string(), "vim".to_string()),
        ]);

        stamp_task_id(&mut env, "fork-task");

        assert_eq!(env.get("TASK_ID").map(String::as_str), Some("fork-task"));
        assert_eq!(env.get("EDITOR").map(String::as_str), Some("vim"));
    }

    #[test]
    fn required_raw_string_preserves_terminal_control_input() {
        let data = Some("\r".to_string());

        assert_eq!(required_raw_string(&data, "data").unwrap(), "\r");
    }

    #[test]
    fn required_raw_string_accepts_whitespace_terminal_input() {
        let data = Some(" ".to_string());

        assert_eq!(required_raw_string(&data, "data").unwrap(), " ");
    }

    #[test]
    fn required_raw_string_rejects_empty_terminal_input() {
        let data = Some(String::new());

        assert!(matches!(
            required_raw_string(&data, "data"),
            Err(TerminalError::Malformed("data"))
        ));
    }

    /// A public Task's visitor may write to it but not reach its shell.
    #[tokio::test]
    async fn the_terminal_needs_ownership_beyond_write() {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_readable_by_id()
            .returning(|_, id| Ok(Some(crate::dao::running_conversation(id))));
        conversations
            .expect_writes_conversation()
            .returning(|_, _| Ok(true));
        conversations
            .expect_owns_conversation()
            .returning(|_, _| Ok(false));

        let error = owned_thread(&conversations, &ScopeContext::user("visitor"), "task-1")
            .await
            .expect_err("a visitor is refused");

        assert!(matches!(
            error,
            TerminalError::Workspace(WorkspaceAccessError::NotOwner)
        ));
    }

    /// A connector with no Sandbox: a call that reaches it has passed its gate.
    struct NoSandbox;

    #[async_trait::async_trait]
    impl SandboxConnector for NoSandbox {
        async fn sandbox_client(
            &self,
            task_id: &str,
        ) -> Result<sandbox_proto::client::SandboxClient, SandboxError> {
            Err(SandboxError::NotReady(task_id.to_string()))
        }
    }

    /// Conversations where the viewer reads `task-1` and writes to it when
    /// `writes`, never owning it: a public Task's visitor, or a stranger on a
    /// private Task.
    fn visited(writes: bool) -> crate::dao::MockConversationDao {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_readable_by_id()
            .returning(|_, id| Ok(Some(crate::dao::running_conversation(id))));
        conversations
            .expect_writes_conversation()
            .returning(move |_, _| Ok(writes));
        conversations
            .expect_owns_conversation()
            .returning(|_, _| Ok(false));
        conversations
    }

    fn script_session(data: Option<&str>) -> TerminalInput {
        TerminalInput {
            thread_id: Some("task-1".to_string()),
            terminal_id: Some("script-dev".to_string()),
            cwd: Some(crate::nominal_path::nominal_worktree("task-1")),
            data: data.map(str::to_string),
            ..Default::default()
        }
    }

    /// A Script's session needs write alone, so a public Task's visitor gets
    /// as far as the Sandbox; a stranger on a private Task is refused first.
    #[tokio::test]
    async fn a_script_session_needs_write_not_ownership() {
        let viewer = ScopeContext::user("visitor");
        let hub = TerminalHub::default();
        let open = script_session(None);
        let write = script_session(Some("npm run dev\n"));

        let visitor = visited(true);
        let opened = open_from_environment(&visitor, &NoSandbox, &hub, &viewer, &open).await;
        assert!(matches!(
            opened,
            Err(TerminalError::Workspace(WorkspaceAccessError::NotRunning(
                _
            )))
        ));
        let typed = write_from_environment(&visitor, &NoSandbox, &viewer, &write).await;
        assert!(matches!(
            typed,
            Err(TerminalError::Workspace(WorkspaceAccessError::NotRunning(
                _
            )))
        ));

        let stranger = visited(false);
        let opened = open_from_environment(&stranger, &NoSandbox, &hub, &viewer, &open).await;
        assert!(matches!(
            opened,
            Err(TerminalError::Workspace(WorkspaceAccessError::NoWrite))
        ));
        let typed = write_from_environment(&stranger, &NoSandbox, &viewer, &write).await;
        assert!(matches!(
            typed,
            Err(TerminalError::Workspace(WorkspaceAccessError::NoWrite))
        ));
    }

    /// The interactive paths over the same session stay with the owner.
    #[tokio::test]
    async fn the_interactive_terminal_still_refuses_a_visitor() {
        let viewer = ScopeContext::user("visitor");
        let visitor = visited(true);
        let hub = TerminalHub::default();

        let opened = open(&visitor, &NoSandbox, &hub, &viewer, &script_session(None)).await;
        assert!(matches!(
            opened,
            Err(TerminalError::Workspace(WorkspaceAccessError::NotOwner))
        ));
        let typed = write(&visitor, &NoSandbox, &viewer, &script_session(Some("ls\n"))).await;
        assert!(matches!(
            typed,
            Err(TerminalError::Workspace(WorkspaceAccessError::NotOwner))
        ));
    }
}
