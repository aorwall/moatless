//! Tails one harness session file into the backend's trajectory, one event per
//! complete line, in file order.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use moatless_types::agent::AgentSubType;
use sandbox_proto::{
    AppendTrajectoryRequest, EventType, PublishEventRequest, SubagentParent, TrajectoryLine,
};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::agent::harness::signal::Input;
use crate::agent::harness::trajectory::{
    LineContext, RecordedLine, ReplyHold, SessionFormat, SubagentFile, WriteTarget, Written,
    latest_session, locate_session, reply_of, reseed_session, write_session,
};
use crate::agent::harness::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use crate::agent::types::ResumeSession;
use crate::backend_client::BackendClient;
#[cfg(test)]
use crate::backend_client::TrajectoryRead;
use crate::config::Config;

/// How often a live session is read when no file event arrives, as the tmux
/// tailer polls.
pub const FALLBACK_POLL: Duration = Duration::from_millis(100);

/// Kept under the backend's 4 MiB gRPC decode limit; a single line over that
/// limit fails its append and stops recording of that session.
const MAX_BATCH_BYTES: usize = 1024 * 1024;

/// Lines per `AppendTrajectory`.
pub const MAX_BATCH_LINES: usize = 256;

/// The backend every recorder in this sandbox ships to, and whether recording
/// is off for the sandbox's life.
pub struct TrajectoryRecording {
    backend: Arc<dyn BackendClient>,
    off: AtomicBool,
}

impl TrajectoryRecording {
    pub fn new(backend: Arc<dyn BackendClient>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            off: AtomicBool::new(false),
        })
    }

    pub fn backend(&self) -> &dyn BackendClient {
        self.backend.as_ref()
    }

    /// Whether an `AppendTrajectory` has answered `Unimplemented`.
    pub fn is_off(&self) -> bool {
        self.off.load(Ordering::Relaxed)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("trajectory recording is off")]
    Off,
    #[error("the session file holds {held} lines, fewer than the trajectory's {expected}")]
    Behind { held: i64, expected: i64 },
    #[error("the session file's line {seq} differs from the trajectory's")]
    Diverged { seq: i64 },
    #[error("reading the session file: {0}")]
    Io(#[from] std::io::Error),
    #[error("AppendTrajectory failed: {0}")]
    Rpc(#[from] tonic::Status),
}

impl RecordError {
    /// Whether retrying on the next flush cannot help.
    fn is_final(&self) -> bool {
        match self {
            Self::Off | Self::Behind { .. } | Self::Diverged { .. } => true,
            Self::Io(_) => false,
            Self::Rpc(status) => status.code() == tonic::Code::ResourceExhausted,
        }
    }
}

/// Where the lines go: one harness session in one Task.
pub struct RecordTarget {
    pub task_id: String,
    pub harness_session_id: String,
    /// The harness's home (`CLAUDE_CONFIG_DIR` or `CODEX_HOME`).
    pub home: PathBuf,
    pub cwd: PathBuf,
    /// A subagent's file, which `SessionFormat::locate` does not find, and
    /// the call that spawned it.
    pub subagent: Option<(PathBuf, SubagentParent)>,
}

impl RecordTarget {
    fn subagent(&self, file: SubagentFile) -> Self {
        Self {
            task_id: self.task_id.clone(),
            harness_session_id: file.harness_session_id,
            home: self.home.clone(),
            cwd: self.cwd.clone(),
            subagent: Some((
                file.path,
                SubagentParent {
                    parent_harness_session_id: self.harness_session_id.clone(),
                    tool_use_id: file.tool_use_id,
                },
            )),
        }
    }
}

/// Ships the complete lines of one session file past a cursor.
pub struct SessionRecorder {
    format: Arc<dyn SessionFormat>,
    recording: Arc<TrajectoryRecording>,
    target: RecordTarget,
    path: Option<PathBuf>,
    /// Byte offset just past line `next_seq - 1`.
    offset: u64,
    next_seq: i64,
    /// Lines from byte 0 the trajectory already holds, while `offset` is not
    /// yet placed after them. `None` with `probe` set: ask the backend first.
    skip: Option<i64>,
    probe: bool,
    /// The probe's payload at `skip - 1`, which the file's line there must
    /// match: a file restored from an older archive and then resumed holds
    /// the right line count with different lines.
    held_tip: Option<String>,
    /// The item id of the line before `next_seq`: a later line of the same
    /// model response ships without the usage Claude repeats on it.
    last_item_id: Option<String>,
    /// What the lines before `next_seq` tell the next one.
    context: LineContext,
    hold: ReplyHold,
    /// Whether a reply has shown the session exists, so `subagent_parent` is
    /// no longer sent.
    created: bool,
    /// A switch session's [`Written::own_from_seq`], sent until an append
    /// with lines is accepted.
    own_from_seq: Option<i64>,
    reseeded: bool,
    stopped: bool,
}

impl SessionRecorder {
    /// Starts after the first `start_seq` lines, which the trajectory already
    /// holds; `None` asks the backend how many that is on the first flush.
    pub fn start(
        format: Arc<dyn SessionFormat>,
        recording: Arc<TrajectoryRecording>,
        target: RecordTarget,
        start_seq: Option<i64>,
    ) -> Self {
        Self {
            format,
            recording,
            target,
            path: None,
            offset: 0,
            next_seq: 0,
            skip: start_seq,
            probe: start_seq.is_none(),
            held_tip: None,
            last_item_id: None,
            context: LineContext::default(),
            hold: ReplyHold::default(),
            created: false,
            own_from_seq: None,
            reseeded: false,
            stopped: false,
        }
    }

    /// Sends `own_from_seq` on the appends that may create a switch session.
    #[must_use]
    pub fn own_from(mut self, own_from_seq: Option<i64>) -> Self {
        self.own_from_seq = own_from_seq;
        self
    }

    /// The session file this recorder reads, once found.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Sends the complete lines past the cursor that [`ReplyHold`] lets
    /// through; returns the acknowledged `last_seq`.
    pub async fn flush(&mut self) -> Result<i64, RecordError> {
        self.flush_with(false).await
    }

    /// Sends every complete line past the cursor, a held reply included.
    pub async fn flush_all(&mut self) -> Result<i64, RecordError> {
        self.flush_with(true).await
    }

    async fn flush_with(&mut self, release: bool) -> Result<i64, RecordError> {
        if self.stopped || self.recording.is_off() {
            return Err(RecordError::Off);
        }
        let mut result = self.flush_inner(release).await;
        if matches!(
            result,
            Err(RecordError::Behind { .. } | RecordError::Diverged { .. })
        ) && self.reseed().await
        {
            result = self.flush_inner(release).await;
        }
        if let Err(error) = &result {
            if error.is_final() {
                self.stopped = true;
                if !matches!(error, RecordError::Off) {
                    self.report(error).await;
                }
            }
        }
        result
    }

    /// Rewrites a file that is behind or diverged from the trajectory with
    /// the trajectory's lines (S10) and places the cursor after them. Once per
    /// recorder; a subagent file is never reseeded.
    async fn reseed(&mut self) -> bool {
        if self.reseeded || self.target.subagent.is_some() {
            return false;
        }
        self.reseeded = true;
        let Some(path) = self.path.clone() else {
            return false;
        };
        let Some(line_count) = reseed_session(
            self.recording.backend(),
            &self.format,
            &self.target.task_id,
            &self.target.harness_session_id,
            &path,
        )
        .await
        else {
            return false;
        };
        self.offset = 0;
        self.skip = Some(line_count);
        self.held_tip = None;
        true
    }

    /// Records a recorder that stopped in `task_events`.
    async fn report(&self, error: &RecordError) {
        let payload = serde_json::json!({
            "harnessSessionId": self.target.harness_session_id,
            "error": error.to_string(),
        });
        let request = PublishEventRequest {
            task_id: self.target.task_id.clone(),
            event_type: EventType::Agent as i32,
            sub_type: AgentSubType::RecordingStopped.as_str().to_string(),
            payload_json: payload.to_string(),
            ..PublishEventRequest::default()
        };
        if let Err(status) = self.recording.backend.publish_event(request).await {
            warn!(%status, "Failed to report the stopped trajectory recorder");
        }
    }

    async fn flush_inner(&mut self, release: bool) -> Result<i64, RecordError> {
        if self.probe {
            let reply = self.append(0, Vec::new()).await?;
            self.skip = Some(reply.last_seq + 1);
            self.held_tip = reply.last_payload;
            self.probe = false;
        }
        let Some(path) = self.located().await else {
            return Ok(self.next_seq - 1);
        };
        // Bounded: a rejected batch re-places the cursor once from the reply.
        let mut replaced = 0;
        loop {
            if let Some(skip) = self.skip.take() {
                let format = Arc::clone(&self.format);
                let mut context = match self.offset {
                    0 => LineContext::default(),
                    _ => self.context.clone(),
                };
                let observe = |line: &[u8]| {
                    context.observe(&mut format.read_line(&String::from_utf8_lossy(line)));
                };
                match skip_lines(&path, self.offset, skip, observe).await {
                    Ok(Ok((offset, last))) => {
                        let last = String::from_utf8_lossy(&last).into_owned();
                        if let Some(tip) = self.held_tip.take() {
                            // A subagent's lines name the parent's session id.
                            let tip = match self.target.subagent {
                                Some(_) => tip,
                                None => self.format.retarget(&tip, &self.target.harness_session_id),
                            };
                            if last != tip {
                                return Err(RecordError::Diverged { seq: skip - 1 });
                            }
                        }
                        self.offset = offset;
                        self.next_seq = skip;
                        self.context = context;
                        self.last_item_id = (skip > 0)
                            .then(|| self.format.read_line(&last).harness_item_id)
                            .flatten();
                    }
                    Ok(Err(held)) => {
                        return Err(RecordError::Behind {
                            held,
                            expected: skip,
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        self.skip = Some(skip);
                        return Ok(self.next_seq - 1);
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            let (bytes, more) = match read_batch(&path, self.offset).await {
                Ok(read) => read,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(self.next_seq - 1);
                }
                Err(e) => return Err(e.into()),
            };
            let lines = complete_lines(&bytes);
            // A batch cut at the budget ships whole: the next read starts at
            // a held line, and one over the budget alone would never ship.
            match self.send(&lines, release || more).await? {
                None if more => {}
                None => return Ok(self.next_seq - 1),
                Some(last_seq) => {
                    replaced += 1;
                    if replaced == 3 {
                        return Ok(self.next_seq - 1);
                    }
                    info!(
                        harness_session_id = %self.target.harness_session_id,
                        last_seq,
                        next_seq = self.next_seq,
                        "Trajectory holds a different line count; re-placing the cursor"
                    );
                    self.offset = 0;
                    self.skip = Some(last_seq + 1);
                }
            }
        }
    }

    /// Sends `lines` from `next_seq` in batches, all of them on `release`.
    /// `Some(last_seq)` when the backend rejected one as out of place.
    async fn send(&mut self, lines: &[&[u8]], release: bool) -> Result<Option<i64>, RecordError> {
        let mut payloads: Vec<String> = lines
            .iter()
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect();
        let reads: Vec<RecordedLine> = payloads.iter().map(|p| self.format.read_line(p)).collect();
        let ready = if release {
            self.hold = ReplyHold::default();
            reads.len()
        } else {
            self.hold
                .ready(&*self.format, self.next_seq, &reads, Instant::now())
        };
        let mut start = 0;
        while start < ready {
            let mut size = 0;
            let take = lines[start..ready]
                .iter()
                .take(MAX_BATCH_LINES)
                .take_while(|l| {
                    size += l.len();
                    size <= MAX_BATCH_BYTES
                })
                .count()
                .max(1);
            // A reply goes in the append of the line after it, which may
            // supersede it.
            let end = match start + take {
                end if take > 1
                    && end < ready
                    && self.format.quotes_replies()
                    && reply_of(&reads[end - 1]).is_some() =>
                {
                    end - 1
                }
                end => end,
            };
            // `wire_line` advances it; a batch the backend did not take rereads
            // its first line against the id before it.
            let before = (self.last_item_id.clone(), self.context.clone());
            let wire = (start..end)
                .map(|i| self.wire_line(std::mem::take(&mut payloads[i]), reads[i].clone()))
                .collect();
            let reply = match self.append(self.next_seq, wire).await {
                Ok(reply) if reply.accepted => reply,
                other => {
                    (self.last_item_id, self.context) = before;
                    return other.map(|reply| Some(reply.last_seq));
                }
            };
            self.next_seq += (end - start) as i64;
            self.offset += lines[start..end]
                .iter()
                .map(|l| l.len() as u64 + 1)
                .sum::<u64>();
            if reply.last_seq != self.next_seq - 1 {
                return Ok(Some(reply.last_seq));
            }
            start = end;
        }
        Ok(None)
    }

    fn wire_line(&mut self, payload: String, mut read: RecordedLine) -> TrajectoryLine {
        if read.harness_item_id.is_some() && read.harness_item_id == self.last_item_id {
            read.content.usage = None;
        }
        self.last_item_id.clone_from(&read.harness_item_id);
        self.context.observe(&mut read);
        TrajectoryLine {
            kind: read.kind,
            harness_item_id: read.harness_item_id,
            harness_turn_id: read.harness_turn_id,
            content_json: read.content.to_json(),
            payload,
        }
    }

    async fn append(
        &mut self,
        first_seq: i64,
        lines: Vec<TrajectoryLine>,
    ) -> Result<sandbox_proto::AppendTrajectoryResponse, RecordError> {
        let has_lines = !lines.is_empty();
        let request = AppendTrajectoryRequest {
            task_id: self.target.task_id.clone(),
            harness: self.format.harness().to_string(),
            harness_session_id: self.target.harness_session_id.clone(),
            first_seq,
            lines,
            turn_number: None,
            subagent_parent: (!self.created)
                .then(|| {
                    self.target
                        .subagent
                        .as_ref()
                        .map(|(_, parent)| parent.clone())
                })
                .flatten(),
            own_from_seq: self.own_from_seq,
        };
        match self.recording.backend.append_trajectory(request).await {
            Ok(reply) => {
                // A probe creates no session, so its reply may not show one.
                self.created |= reply.last_seq >= 0;
                if reply.accepted && has_lines {
                    self.own_from_seq = None;
                }
                Ok(reply)
            }
            Err(status) if status.code() == tonic::Code::Unimplemented => {
                if !self.recording.off.swap(true, Ordering::Relaxed) {
                    info!("Trajectory recording is off; stopping every recorder in this sandbox");
                }
                Err(RecordError::Off)
            }
            Err(status) => Err(status.into()),
        }
    }

    async fn located(&mut self) -> Option<PathBuf> {
        if let Some((path, _)) = &self.target.subagent {
            self.path = Some(path.clone());
        }
        if self.path.is_none() {
            self.path = locate_session(
                &self.format,
                &self.target.home,
                &self.target.cwd,
                &self.target.harness_session_id,
            )
            .await;
        }
        self.path.clone()
    }
}

/// The complete lines from `offset` up to [`MAX_BATCH_BYTES`], or the one
/// line past it; `true` when the batch stopped at the budget, not the end.
async fn read_batch(path: &Path, offset: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut reader = open_at(path, offset).await?;
    let mut bytes = Vec::new();
    loop {
        let start = bytes.len();
        if reader.read_until(b'\n', &mut bytes).await? == 0 {
            return Ok((bytes, false));
        }
        if bytes.last() != Some(&b'\n') {
            bytes.truncate(start);
            return Ok((bytes, false));
        }
        if bytes.len() >= MAX_BATCH_BYTES {
            return Ok((bytes, true));
        }
    }
}

/// The offset past `lines` complete lines from `offset`, or `Err(held)` with
/// how many the file holds when it holds fewer.
/// The offset past `lines` more lines and the last of them without its
/// newline, or how many complete lines the file holds.
async fn skip_lines(
    path: &Path,
    offset: u64,
    lines: i64,
    mut observe: impl FnMut(&[u8]),
) -> std::io::Result<Result<(u64, Vec<u8>), i64>> {
    let mut reader = open_at(path, offset).await?;
    let mut line = Vec::new();
    let mut offset = offset;
    for held in 0..lines {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).await?;
        if line.pop() != Some(b'\n') {
            return Ok(Err(held));
        }
        observe(&line);
        offset += read as u64;
    }
    Ok(Ok((offset, line)))
}

async fn open_at(path: &Path, offset: u64) -> std::io::Result<BufReader<tokio::fs::File>> {
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    Ok(BufReader::new(file))
}

/// Every line that ends in a newline, without it. A trailing partial line is
/// left for a later flush.
fn complete_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
        return Vec::new();
    };
    bytes[..end].split(|b| *b == b'\n').collect()
}

/// A [`SessionRecorder`] and the subagent sessions its file spawns, flushed
/// on every change to the file's directory and every [`FALLBACK_POLL`] until
/// dropped.
pub struct LiveRecorder {
    sessions: Arc<Mutex<LiveSessions>>,
    ticker: JoinHandle<()>,
}

struct LiveSessions {
    main: SessionRecorder,
    subagents: Vec<SessionRecorder>,
    /// Whether each recorder has logged a failure at warn already.
    warned: Vec<bool>,
    watcher: Option<notify::RecommendedWatcher>,
}

impl LiveSessions {
    async fn flush(&mut self, changes: &mpsc::Sender<()>, release: bool) {
        self.warned.resize(1 + self.subagents.len(), false);
        if let Err(error) = self.main.flush_with(release).await {
            log_failure(&self.main, &error, &mut self.warned[0]);
        }
        let Some(path) = self.main.path().map(Path::to_path_buf) else {
            return;
        };
        if self.watcher.is_none() {
            self.watcher = watch(&path, changes.clone());
        }
        self.adopt_subagents(&path).await;
        self.warned.resize(1 + self.subagents.len(), false);
        for (i, recorder) in self.subagents.iter_mut().enumerate() {
            if let Err(error) = recorder.flush_with(release).await {
                log_failure(recorder, &error, &mut self.warned[i + 1]);
            }
        }
    }

    async fn adopt_subagents(&mut self, path: &Path) {
        let format = Arc::clone(&self.main.format);
        let file = path.to_path_buf();
        let found = tokio::task::spawn_blocking(move || format.subagents(&file))
            .await
            .unwrap_or_default();
        for file in found {
            let known = self
                .subagents
                .iter()
                .any(|r| r.target.harness_session_id == file.harness_session_id);
            if !known {
                self.subagents.push(SessionRecorder::start(
                    Arc::clone(&self.main.format),
                    Arc::clone(&self.main.recording),
                    self.main.target.subagent(file),
                    None,
                ));
            }
        }
    }
}

/// Signals `changes` on any change under the session file's directory,
/// which holds Claude's subagent files too. `None` leaves the poll alone.
fn watch(path: &Path, changes: mpsc::Sender<()>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher as _;
    let dir = path.parent()?;
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = changes.try_send(());
        }
    })
    .map_err(|error| debug!(%error, "No file watch; polling only"))
    .ok()?;
    watcher
        .watch(dir, notify::RecursiveMode::Recursive)
        .map_err(|error| debug!(%error, dir = %dir.display(), "No file watch; polling only"))
        .ok()?;
    Some(watcher)
}

impl LiveRecorder {
    pub fn spawn(recorder: SessionRecorder) -> Self {
        let sessions = Arc::new(Mutex::new(LiveSessions {
            main: recorder,
            subagents: Vec::new(),
            warned: Vec::new(),
            watcher: None,
        }));
        let ticking = Arc::clone(&sessions);
        let ticker = tokio::spawn(async move {
            let (changes, mut changed) = mpsc::channel(1);
            loop {
                let mut sessions = ticking.lock().await;
                sessions.flush(&changes, false).await;
                if sessions.main.stopped && sessions.subagents.iter().all(|r| r.stopped) {
                    return;
                }
                drop(sessions);
                let _ = tokio::time::timeout(FALLBACK_POLL, changed.recv()).await;
            }
        });
        Self { sessions, ticker }
    }

    /// Flush every line now, a held reply included; called at Turn end,
    /// before the Turn is reported finished.
    pub async fn flush(&self) {
        let (changes, _) = mpsc::channel(1);
        self.sessions.lock().await.flush(&changes, true).await;
    }
}

fn log_failure(recorder: &SessionRecorder, error: &RecordError, warned: &mut bool) {
    let session = &recorder.target.harness_session_id;
    match error {
        RecordError::Off => debug!(%session, "Trajectory recording is off"),
        _ if !*warned => {
            *warned = true;
            warn!(%session, %error, "Trajectory flush failed");
        }
        _ => debug!(%session, %error, "Trajectory flush failed again"),
    }
}

impl Drop for LiveRecorder {
    fn drop(&mut self) {
        self.ticker.abort();
    }
}

/// The turn error a switch that cannot write its session out ends in.
fn switch_failed() -> HarnessError {
    HarnessError::new(
        FailureStage::Startup,
        ErrorClass::UnavailableSession,
        SessionImpact::Intact,
        "Could not carry the conversation over to the new agent: its recorded history is \
         unreadable or incomplete",
    )
}

/// One Turn's trajectory work: write the session out before the harness opens
/// it, record it once its id is known, and flush before the Turn is reported.
pub(crate) struct TurnTrajectory {
    recording: Arc<TrajectoryRecording>,
    format: Arc<dyn SessionFormat>,
    task_id: String,
    home: PathBuf,
    cwd: PathBuf,
    written: Option<Written>,
    live: Option<LiveRecorder>,
}

impl TurnTrajectory {
    /// `None` when the harness was built without a backend to record to.
    pub(crate) fn new(
        recording: Option<&Arc<TrajectoryRecording>>,
        format: Arc<dyn SessionFormat>,
        config: &Config,
        home: &str,
        input: &Input,
    ) -> Option<Self> {
        let env = input.env.as_ref();
        let var = |key: &str| {
            env.and_then(|e| e.get(key).cloned())
                .or_else(|| std::env::var(key).ok())
        };
        Some(Self {
            recording: Arc::clone(recording?),
            format,
            task_id: var("TASK_ID").unwrap_or_else(|| config.task_id.clone()),
            home: PathBuf::from(home),
            cwd: PathBuf::from(&config.workspace_dir),
            written: None,
            live: None,
        })
    }

    /// Writes the session `input` resumes, forks or switches into out of the
    /// trajectory. Returns the session a fork or switch resumes; `None` leaves
    /// the harness to its own fork, resume or fresh start. Fails only when
    /// `switch_harness` is set, since then the harness has no other way to
    /// carry the conversation over (S6).
    pub(crate) async fn write_out(
        &mut self,
        input: &Input,
    ) -> Result<Option<String>, HarnessError> {
        let unwritten = || {
            if input.switch_harness {
                Err(switch_failed())
            } else {
                Ok(None)
            }
        };
        if self.recording.is_off() {
            return unwritten();
        }
        let target = match (&input.fork, &input.resume) {
            (Some(_), _) => WriteTarget::Fork,
            (None, _) if input.switch_harness => WriteTarget::Switch,
            (None, Some(ResumeSession::WithId(id))) if !input.start_fresh => {
                WriteTarget::Resume(id.clone())
            }
            // After a cut-off Turn the backend sends no id, and Claude takes
            // `--continue` on a file a replaced pod restored from the archive.
            (None, Some(ResumeSession::Last)) if !input.start_fresh => {
                match latest_session(&self.format, &self.home, &self.cwd).await {
                    Some(id) => WriteTarget::Resume(id),
                    None => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        let opens = matches!(target, WriteTarget::Fork | WriteTarget::Switch);
        let Some(written) = write_session(
            self.recording.backend(),
            &self.format,
            &self.task_id,
            &self.home,
            &self.cwd,
            target,
            input.model.as_deref(),
        )
        .await
        else {
            return unwritten();
        };
        let id = written.harness_session_id.clone();
        self.written = Some(written);
        Ok(opens.then_some(id))
    }

    /// Starts recording `harness_session_id`; later calls do nothing.
    pub(crate) fn record(&mut self, harness_session_id: &str) {
        if self.live.is_some() || self.recording.is_off() {
            return;
        }
        let written = self
            .written
            .as_ref()
            .filter(|w| w.harness_session_id == harness_session_id);
        let start_seq = written.filter(|w| w.rewritten).map(|w| w.line_count);
        let own_from_seq = written.and_then(|w| w.own_from_seq);
        let recorder = SessionRecorder::start(
            Arc::clone(&self.format),
            Arc::clone(&self.recording),
            RecordTarget {
                task_id: self.task_id.clone(),
                harness_session_id: harness_session_id.to_string(),
                home: self.home.clone(),
                cwd: self.cwd.clone(),
                subagent: None,
            },
            start_seq,
        )
        .own_from(own_from_seq);
        self.live = Some(LiveRecorder::spawn(recorder));
    }

    pub(crate) async fn flush(&self) {
        if let Some(live) = &self.live {
            live.flush().await;
        }
    }
}

#[cfg(test)]
#[path = "trajectory_tail_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/support/memory_trajectory.rs"]
#[allow(dead_code, reason = "shared with trajectory_e2e, which uses the rest")]
pub(crate) mod memory;
