//! `MemoryTrajectoryBackend`: a `BackendClient` holding trajectories in memory
//! under the append, chain and fork-cut rules of `PgTrajectoryDao`.
//!
//! Included by path from `trajectory_tail` and `trajectory_e2e.rs`; each
//! includer brings `BackendClient` into its parent scope.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use sandbox_proto::{
    AppendTrajectoryRequest, AppendTrajectoryResponse, DownloadUploadResponse,
    GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery, PublishEventRequest,
    ReadTrajectoryRequest, SessionHeader, read_trajectory_request::Target,
};
use serde_json::Value;
use tonic::Status;

use super::BackendClient;

#[derive(Debug, Clone)]
pub struct MemoryEvent {
    pub seq: i64,
    pub kind: String,
    pub harness_item_id: Option<String>,
    pub harness_turn_id: Option<String>,
    pub payload: String,
    pub content: Value,
}

#[derive(Debug, Clone)]
struct MemorySession {
    task_id: String,
    harness: String,
    /// The source session and the seq this one continues from.
    forked_from: Option<(String, i64)>,
    own: Vec<MemoryEvent>,
    last_seq: i64,
}

/// A `task_forks` row, as the backend would read it.
#[derive(Debug, Clone)]
#[expect(clippy::struct_field_names, reason = "mirrors `ForkSource`")]
pub struct MemoryFork {
    pub source_session_id: String,
    pub source_message_uuid: Option<String>,
    pub source_agent_turn_id: Option<String>,
}

#[derive(Default)]
struct State {
    sessions: HashMap<String, MemorySession>,
    forks: HashMap<String, MemoryFork>,
    lost: HashSet<(String, i64)>,
    appends: usize,
    fork_sessions: usize,
}

#[derive(Default)]
pub struct MemoryTrajectoryBackend {
    state: Mutex<State>,
    /// Answer both RPCs `Unimplemented`, as the backend does with the flag off.
    pub off: bool,
}

impl MemoryTrajectoryBackend {
    pub fn off() -> Self {
        Self {
            off: true,
            ..Self::default()
        }
    }

    /// Every event `harness_session_id` holds, root-first across fork points.
    pub fn events_on(&self, harness_session_id: &str) -> Vec<MemoryEvent> {
        let state = self.state.lock().unwrap();
        chain(&state, harness_session_id)
    }

    /// Only the events recorded under `harness_session_id` itself.
    pub fn own_events(&self, harness_session_id: &str) -> Vec<MemoryEvent> {
        let state = self.state.lock().unwrap();
        state
            .sessions
            .get(harness_session_id)
            .map(|s| s.own.clone())
            .unwrap_or_default()
    }

    /// Every event stored under any session, counting each once.
    pub fn stored_event_count(&self) -> usize {
        let state = self.state.lock().unwrap();
        state.sessions.values().map(|s| s.own.len()).sum()
    }

    pub fn session_ids(&self) -> Vec<String> {
        let state = self.state.lock().unwrap();
        state.sessions.keys().cloned().collect()
    }

    pub fn append_calls(&self) -> usize {
        self.state.lock().unwrap().appends
    }

    pub fn fork_sessions_made(&self) -> usize {
        self.state.lock().unwrap().fork_sessions
    }

    /// Record `task_id` as a fork, as `task_forks` would.
    pub fn set_fork(&self, task_id: &str, fork: MemoryFork) {
        self.state
            .lock()
            .unwrap()
            .forks
            .insert(task_id.to_string(), fork);
    }

    /// Drop one stored event, leaving a gap in its session's chain.
    pub fn lose(&self, harness_session_id: &str, seq: i64) {
        let mut state = self.state.lock().unwrap();
        state.lost.insert((harness_session_id.to_string(), seq));
    }

    fn fork_session(state: &mut State, task_id: &str, harness: &str) -> Option<String> {
        let fork = state.forks.get(task_id)?.clone();
        let source = state.sessions.get(&fork.source_session_id)?;
        if source.harness != harness {
            return None;
        }
        // Reused while it has recorded nothing, as `insert_fork_session` does.
        let events = chain(state, &fork.source_session_id);
        let at = match (&fork.source_message_uuid, &fork.source_agent_turn_id) {
            (Some(m), _) if harness == "claude-code" => events
                .iter()
                .rposition(|e| e.harness_item_id.as_deref() == Some(m.as_str()))?,
            (_, Some(t)) if harness == "codex" => events.iter().rposition(|e| {
                e.harness_turn_id.as_deref() == Some(t.as_str())
                    && e.kind == "event_msg/task_complete"
            })?,
            _ => events.len().checked_sub(1)?,
        };
        let at = include_pending_results(&events, at);
        let cut_seq = events[at].seq;
        let reuse = state.sessions.iter().find(|(_, s)| {
            s.task_id == task_id
                && s.own.is_empty()
                && s.forked_from.as_ref() == Some(&(fork.source_session_id.clone(), cut_seq))
        });
        if let Some((id, _)) = reuse {
            return Some(id.clone());
        }
        let id = uuid::Uuid::now_v7().to_string();
        state.sessions.insert(
            id.clone(),
            MemorySession {
                task_id: task_id.to_string(),
                harness: harness.to_string(),
                forked_from: Some((fork.source_session_id, cut_seq)),
                own: Vec::new(),
                last_seq: cut_seq,
            },
        );
        state.fork_sessions += 1;
        Some(id)
    }
}

fn chain(state: &State, harness_session_id: &str) -> Vec<MemoryEvent> {
    let Some(session) = state.sessions.get(harness_session_id) else {
        return Vec::new();
    };
    let mut events = match &session.forked_from {
        Some((source, seq)) => chain(state, source)
            .into_iter()
            .filter(|e| e.seq <= *seq)
            .collect(),
        None => Vec::new(),
    };
    events.extend(
        session
            .own
            .iter()
            .filter(|e| {
                !state
                    .lost
                    .contains(&(harness_session_id.to_string(), e.seq))
            })
            .cloned(),
    );
    events
}

fn blocks(event: &MemoryEvent) -> Vec<Value> {
    event
        .content
        .get("blocks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn include_pending_results(events: &[MemoryEvent], at: usize) -> usize {
    let mut pending = HashSet::new();
    for event in &events[..=at] {
        for block in blocks(event) {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        pending.insert(id.to_string());
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                        pending.remove(id);
                    }
                }
                _ => {}
            }
        }
    }
    let mut cut = at;
    for (i, event) in events.iter().enumerate().skip(at + 1) {
        if pending.is_empty() {
            break;
        }
        for block in blocks(event) {
            let id = block.get("tool_use_id").and_then(Value::as_str);
            if block.get("type").and_then(Value::as_str) == Some("tool_result")
                && id.is_some_and(|id| pending.remove(id))
            {
                cut = i;
            }
        }
    }
    cut
}

#[async_trait::async_trait]
impl BackendClient for MemoryTrajectoryBackend {
    async fn append_trajectory(
        &self,
        req: AppendTrajectoryRequest,
    ) -> Result<AppendTrajectoryResponse, Status> {
        if self.off {
            return Err(Status::unimplemented("trajectory recording is off"));
        }
        let mut state = self.state.lock().unwrap();
        state.appends += 1;
        let probe = req.lines.is_empty();
        let session = state
            .sessions
            .entry(req.harness_session_id.clone())
            .or_insert_with(|| MemorySession {
                task_id: req.task_id.clone(),
                harness: req.harness.clone(),
                forked_from: None,
                own: Vec::new(),
                last_seq: -1,
            });
        if session.task_id != req.task_id || session.harness != req.harness {
            return Err(Status::permission_denied("another task's session"));
        }
        if req.first_seq > session.last_seq + 1 {
            return Ok(AppendTrajectoryResponse {
                accepted: false,
                last_seq: session.last_seq,
                last_payload: None,
            });
        }
        for (i, line) in req.lines.into_iter().enumerate() {
            let seq = req.first_seq + i as i64;
            if seq <= session.last_seq {
                continue;
            }
            session.own.push(MemoryEvent {
                seq,
                kind: line.kind,
                harness_item_id: line.harness_item_id,
                harness_turn_id: line.harness_turn_id,
                payload: line.payload,
                content: serde_json::from_str(&line.content_json).unwrap_or(Value::Null),
            });
            session.last_seq = seq;
        }
        let last_seq = session.last_seq;
        let last_payload = if probe {
            chain(&state, &req.harness_session_id)
                .into_iter()
                .find(|e| e.seq == last_seq)
                .map(|e| e.payload)
        } else {
            None
        };
        Ok(AppendTrajectoryResponse {
            accepted: true,
            last_seq,
            last_payload,
        })
    }

    async fn read_trajectory(
        &self,
        req: ReadTrajectoryRequest,
    ) -> Result<(SessionHeader, Vec<String>), Status> {
        if self.off {
            return Err(Status::unimplemented("trajectory recording is off"));
        }
        let mut state = self.state.lock().unwrap();
        let id = match req.target {
            Some(Target::HarnessSessionId(id)) => state
                .sessions
                .get(&id)
                .filter(|s| s.task_id == req.task_id && s.harness == req.harness)
                .map(|_| id),
            Some(Target::Fork(true)) => Self::fork_session(&mut state, &req.task_id, &req.harness),
            _ => return Err(Status::invalid_argument("a target is required")),
        };
        let Some(id) = id else {
            return Err(Status::not_found("no trajectory session"));
        };
        let events = chain(&state, &id);
        let last_seq = state.sessions[&id].last_seq;
        for (expected, event) in events.iter().enumerate() {
            if event.seq != expected as i64 {
                return Err(Status::data_loss(format!("missing seq {expected}")));
            }
        }
        if events.len() as i64 != last_seq + 1 {
            return Err(Status::data_loss(format!("missing seq {}", events.len())));
        }
        let header = SessionHeader {
            harness_session_id: id,
            line_count: events.len() as i64,
        };
        Ok((header, events.into_iter().map(|e| e.payload).collect()))
    }

    async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), Status> {
        Ok(())
    }

    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingAgentDelivery>, Status> {
        Ok(Vec::new())
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
        _answered_prompt: Option<&str>,
    ) -> Result<(), Status> {
        Ok(())
    }

    async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
        Ok(GetCredentialsResponse::default())
    }

    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), Status> {
        Ok(())
    }

    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
        Ok(String::new())
    }

    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadUploadResponse, Status> {
        Err(Status::unimplemented("not used"))
    }

    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, Status> {
        Ok(None)
    }

    async fn upload_transcript(&self, _task_id: &str, _tar_data: Vec<u8>) -> Result<(), Status> {
        Ok(())
    }

    async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, Status> {
        Ok(Vec::new())
    }
}
