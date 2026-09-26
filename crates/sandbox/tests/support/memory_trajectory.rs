//! `MemoryTrajectoryBackend`: a `BackendClient` holding trajectories in memory
//! under the append, chain, fork-cut and switch rules of `PgTrajectoryDao`.
//!
//! Included by path from `trajectory_tail` and `trajectory_e2e.rs`; each
//! includer brings `BackendClient` into its parent scope.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use sandbox_proto::{
    AppendTrajectoryRequest, AppendTrajectoryResponse, DownloadUploadResponse,
    GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery, PublishEventRequest,
    ReadTrajectoryRequest, SessionHeader, TranscodeItem, read_trajectory_request::Target,
};
use serde_json::Value;
use tonic::Status;

use super::{BackendClient, TrajectoryRead};

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
    /// A subagent session's parent session and its spawning event's seq.
    parent: Option<(String, i64)>,
    /// A switch session's other-harness tip, as a session and seq.
    transcoded_from: Option<(String, i64)>,
    own_from_seq: Option<i64>,
    /// The append counter when this session last took a line: the head is
    /// the latest.
    appended_at: usize,
    own: Vec<MemoryEvent>,
    last_seq: i64,
}

/// One event on a lineage, as `LineageRow`.
struct Row {
    session: String,
    harness: String,
    event: MemoryEvent,
    prefix: bool,
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
    widest_append: usize,
    published: Vec<PublishEventRequest>,
    /// Appends with lines to answer `Unavailable` before storing any.
    failing_appends: usize,
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

    /// A subagent session's parent session and its spawning event's seq.
    pub fn parent_of(&self, harness_session_id: &str) -> Option<(String, i64)> {
        let state = self.state.lock().unwrap();
        state.sessions.get(harness_session_id)?.parent.clone()
    }

    /// Every event published, as `(sub_type, payload_json)`.
    pub fn published(&self) -> Vec<(String, String)> {
        let state = self.state.lock().unwrap();
        state
            .published
            .iter()
            .map(|e| (e.sub_type.clone(), e.payload_json.clone()))
            .collect()
    }

    /// The most lines any one append carried.
    pub fn widest_append(&self) -> usize {
        self.state.lock().unwrap().widest_append
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

    /// Answer the next `n` appends that carry lines `Unavailable`.
    pub fn fail_appends(&self, n: usize) {
        self.state.lock().unwrap().failing_appends = n;
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
                parent: None,
                transcoded_from: None,
                own_from_seq: None,
                appended_at: 0,
                own: Vec::new(),
                last_seq: cut_seq,
            },
        );
        state.fork_sessions += 1;
        Some(id)
    }
}

/// The switch session `task_id` runs in `harness`, and the items it renders.
fn switch_session(
    state: &mut State,
    task_id: &str,
    harness: &str,
) -> Option<(String, Vec<TranscodeItem>)> {
    let (head, head_session) = state
        .sessions
        .iter()
        .filter(|(_, s)| s.task_id == task_id && s.parent.is_none() && !s.own.is_empty())
        .max_by_key(|(_, s)| s.appended_at)?;
    let (head, head_harness, head_seq) = (
        head.clone(),
        head_session.harness.clone(),
        head_session.last_seq,
    );
    if head_harness == harness {
        return Some((head, Vec::new()));
    }
    let rows = lineage(state, &head, head_seq);
    let since = rows.iter().rposition(|r| r.harness == harness);
    let mut items = Vec::new();
    for row in &rows[since.map_or(0, |i| i + 1)..] {
        if row.prefix || row.harness == harness {
            continue;
        }
        let content = &row.event.content;
        if content.get("compaction").and_then(Value::as_bool) == Some(true) {
            items.clear();
        }
        let spoken = content.get("role").is_some_and(|r| !r.is_null());
        let injected = content.get("injected").and_then(Value::as_bool) == Some(true);
        if !spoken || injected || blocks(&row.event).is_empty() {
            continue;
        }
        items.push(TranscodeItem {
            content_json: content.to_string(),
            turn_number: None,
            source_harness: row.harness.clone(),
        });
    }
    let forked_from = since.map(|i| (rows[i].session.clone(), rows[i].event.seq));
    let transcoded_from = Some((head, head_seq));
    let reuse = state.sessions.iter().find(|(_, s)| {
        s.task_id == task_id
            && s.own.is_empty()
            && s.harness == harness
            && s.forked_from == forked_from
            && s.transcoded_from == transcoded_from
    });
    if let Some((id, _)) = reuse {
        return Some((id.clone(), items));
    }
    let id = uuid::Uuid::now_v7().to_string();
    let last_seq = forked_from.as_ref().map_or(-1, |(_, seq)| *seq);
    state.sessions.insert(
        id.clone(),
        MemorySession {
            task_id: task_id.to_string(),
            harness: harness.to_string(),
            forked_from,
            parent: None,
            transcoded_from,
            own_from_seq: None,
            appended_at: 0,
            own: Vec::new(),
            last_seq,
        },
    );
    Some((id, items))
}

/// Every event `harness_session_id` descends from through seq `at`, oldest
/// first: across a switch session's prefix, the other harness's chain it was
/// rendered from.
fn lineage(state: &State, harness_session_id: &str, at: i64) -> Vec<Row> {
    let Some(session) = state.sessions.get(harness_session_id) else {
        return Vec::new();
    };
    let mut rows = match (&session.transcoded_from, &session.forked_from) {
        (Some((source, seq)), _) | (None, Some((source, seq))) => lineage(state, source, *seq),
        (None, None) => Vec::new(),
    };
    let own_from = session.own_from_seq.unwrap_or(0);
    rows.extend(session.own.iter().filter(|e| e.seq <= at).map(|e| Row {
        session: harness_session_id.to_string(),
        harness: session.harness.clone(),
        event: e.clone(),
        prefix: session.transcoded_from.is_some() && e.seq < own_from,
    }));
    rows
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
        state.widest_append = state.widest_append.max(req.lines.len());
        let probe = req.lines.is_empty();
        if !probe && state.failing_appends > 0 {
            state.failing_appends -= 1;
            return Err(Status::unavailable("backend unavailable"));
        }
        let parent = match &req.subagent_parent {
            Some(p) if !state.sessions.contains_key(&req.harness_session_id) => {
                let spawning = state
                    .sessions
                    .get(&p.parent_harness_session_id)
                    .filter(|s| s.task_id == req.task_id)
                    .and_then(|s| {
                        s.own.iter().find(|e| {
                            blocks(e).iter().any(|b| {
                                b["type"] == "tool_use" && b["id"] == p.tool_use_id.as_str()
                            })
                        })
                    })
                    .map(|e| (p.parent_harness_session_id.clone(), e.seq));
                if spawning.is_none() {
                    return Err(Status::failed_precondition(
                        "spawning tool call not recorded",
                    ));
                }
                spawning
            }
            _ => None,
        };
        // As the dao, a probe creates no session.
        if probe && !state.sessions.contains_key(&req.harness_session_id) {
            return Ok(AppendTrajectoryResponse {
                accepted: true,
                last_seq: -1,
                last_payload: None,
            });
        }
        let appended_at = state.appends;
        let session = state
            .sessions
            .entry(req.harness_session_id.clone())
            .or_insert_with(|| MemorySession {
                task_id: req.task_id.clone(),
                harness: req.harness.clone(),
                forked_from: None,
                parent,
                transcoded_from: None,
                own_from_seq: None,
                appended_at,
                own: Vec::new(),
                last_seq: -1,
            });
        if session.task_id != req.task_id || session.harness != req.harness {
            return Err(Status::permission_denied("another task's session"));
        }
        if !probe && session.transcoded_from.is_some() && session.own_from_seq.is_none() {
            let Some(own_from_seq) = req.own_from_seq else {
                return Err(Status::invalid_argument("switch session: no own_from_seq"));
            };
            session.own_from_seq = Some(own_from_seq);
        }
        if !probe {
            session.appended_at = appended_at;
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

    async fn read_trajectory(&self, req: ReadTrajectoryRequest) -> Result<TrajectoryRead, Status> {
        if self.off {
            return Err(Status::unimplemented("trajectory recording is off"));
        }
        let mut state = self.state.lock().unwrap();
        let mut transcode = Vec::new();
        let id = match req.target {
            Some(Target::HarnessSessionId(id)) => state
                .sessions
                .get(&id)
                .filter(|s| s.task_id == req.task_id && s.harness == req.harness)
                .map(|_| id),
            Some(Target::Fork(true)) => Self::fork_session(&mut state, &req.task_id, &req.harness),
            Some(Target::Switch(true)) => switch_session(&mut state, &req.task_id, &req.harness)
                .map(|(id, items)| {
                    transcode = items;
                    id
                }),
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
        Ok(TrajectoryRead {
            header,
            lines: events.into_iter().map(|e| e.payload).collect(),
            transcode,
        })
    }

    async fn publish_event(&self, req: PublishEventRequest) -> Result<(), Status> {
        self.state.lock().unwrap().published.push(req);
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
