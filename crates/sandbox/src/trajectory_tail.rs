//! Tails one harness session file into the backend's trajectory, one event per
//! complete line, in file order.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sandbox_proto::{AppendTrajectoryRequest, TrajectoryLine};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::agent::harness::signal::Input;
use crate::agent::harness::trajectory::{
    SessionFormat, WriteTarget, Written, latest_session, locate_session, write_session,
};
use crate::agent::types::ResumeSession;
use crate::backend_client::BackendClient;
use crate::config::Config;

/// How often a live session's new lines are shipped.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// Kept under the backend's 4 MiB gRPC decode limit; a single line over that
/// limit fails its append and stops recording of that session.
const MAX_BATCH_BYTES: usize = 1024 * 1024;

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
            stopped: false,
        }
    }

    /// Sends every complete line past the cursor; returns the acknowledged
    /// `last_seq`.
    pub async fn flush(&mut self) -> Result<i64, RecordError> {
        if self.stopped || self.recording.is_off() {
            return Err(RecordError::Off);
        }
        let result = self.flush_inner().await;
        if let Err(error) = &result {
            if error.is_final() {
                self.stopped = true;
            }
        }
        result
    }

    async fn flush_inner(&mut self) -> Result<i64, RecordError> {
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
                match skip_lines(&path, self.offset, skip).await {
                    Ok(Ok((offset, last))) => {
                        if let Some(tip) = self.held_tip.take() {
                            let tip = self.format.retarget(&tip, &self.target.harness_session_id);
                            if String::from_utf8_lossy(&last) != tip {
                                return Err(RecordError::Diverged { seq: skip - 1 });
                            }
                        }
                        self.offset = offset;
                        self.next_seq = skip;
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
            match self.send(&lines).await? {
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

    /// Sends `lines` from `next_seq` in batches. `Some(last_seq)` when the
    /// backend rejected one as out of place.
    async fn send(&mut self, lines: &[&[u8]]) -> Result<Option<i64>, RecordError> {
        let mut rest = lines;
        while !rest.is_empty() {
            let mut size = 0;
            let take = rest
                .iter()
                .take_while(|l| {
                    size += l.len();
                    size <= MAX_BATCH_BYTES
                })
                .count()
                .max(1);
            let (batch, tail) = rest.split_at(take);
            let wire = batch.iter().map(|l| self.wire_line(l)).collect();
            let reply = self.append(self.next_seq, wire).await?;
            if !reply.accepted {
                return Ok(Some(reply.last_seq));
            }
            self.next_seq += batch.len() as i64;
            self.offset += batch.iter().map(|l| l.len() as u64 + 1).sum::<u64>();
            if reply.last_seq != self.next_seq - 1 {
                return Ok(Some(reply.last_seq));
            }
            rest = tail;
        }
        Ok(None)
    }

    fn wire_line(&self, line: &[u8]) -> TrajectoryLine {
        let payload = String::from_utf8_lossy(line).into_owned();
        let read = self.format.read_line(&payload);
        TrajectoryLine {
            kind: read.kind,
            harness_item_id: read.harness_item_id,
            harness_turn_id: read.harness_turn_id,
            content_json: read.content.to_json(),
            payload,
        }
    }

    async fn append(
        &self,
        first_seq: i64,
        lines: Vec<TrajectoryLine>,
    ) -> Result<sandbox_proto::AppendTrajectoryResponse, RecordError> {
        let request = AppendTrajectoryRequest {
            task_id: self.target.task_id.clone(),
            harness: self.format.harness().to_string(),
            harness_session_id: self.target.harness_session_id.clone(),
            first_seq,
            lines,
            turn_number: None,
        };
        match self.recording.backend.append_trajectory(request).await {
            Ok(reply) => Ok(reply),
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

/// A [`SessionRecorder`] flushed every [`FLUSH_INTERVAL`] until dropped.
pub struct LiveRecorder {
    recorder: Arc<Mutex<SessionRecorder>>,
    ticker: JoinHandle<()>,
}

impl LiveRecorder {
    pub fn spawn(recorder: SessionRecorder) -> Self {
        let recorder = Arc::new(Mutex::new(recorder));
        let ticking = Arc::clone(&recorder);
        let ticker = tokio::spawn(async move {
            let mut interval = tokio::time::interval(FLUSH_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut warned = false;
            loop {
                interval.tick().await;
                let mut recorder = ticking.lock().await;
                if let Err(error) = recorder.flush().await {
                    log_failure(&recorder, &error, &mut warned);
                    if recorder.stopped {
                        return;
                    }
                }
            }
        });
        Self { recorder, ticker }
    }

    /// Flush now; called at Turn end, before the Turn is reported finished.
    pub async fn flush(&self) {
        let mut recorder = self.recorder.lock().await;
        let mut warned = false;
        if let Err(error) = recorder.flush().await {
            log_failure(&recorder, &error, &mut warned);
        }
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

/// The Workspace env that makes a Turn write its session file out of the
/// trajectory before the harness opens it.
pub const RESUME_FROM_TRAJECTORY: &str = "MOATLESS_RESUME_FROM_TRAJECTORY";

/// One Turn's trajectory work: write the session out before the harness opens
/// it, record it once its id is known, and flush before the Turn is reported.
pub(crate) struct TurnTrajectory {
    recording: Arc<TrajectoryRecording>,
    format: Arc<dyn SessionFormat>,
    task_id: String,
    home: PathBuf,
    cwd: PathBuf,
    resume_from_trajectory: bool,
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
            resume_from_trajectory: var(RESUME_FROM_TRAJECTORY).as_deref() == Some("1"),
            written: None,
            live: None,
        })
    }

    /// Writes the session `input` resumes or forks out of the trajectory, when
    /// the Workspace asks for it. Returns the session a fork resumes; `None`
    /// leaves the harness to its own fork, resume or fresh start.
    pub(crate) async fn write_out(&mut self, input: &Input) -> Option<String> {
        if !self.resume_from_trajectory || self.recording.is_off() {
            return None;
        }
        let target = match (&input.fork, &input.resume) {
            (Some(_), _) => WriteTarget::Fork,
            (None, Some(ResumeSession::WithId(id))) if !input.start_fresh => {
                WriteTarget::Resume(id.clone())
            }
            // After a cut-off Turn the backend sends no id, and Claude takes
            // `--continue` on a file a replaced pod restored from the archive.
            (None, Some(ResumeSession::Last)) if !input.start_fresh => {
                WriteTarget::Resume(latest_session(&self.format, &self.home, &self.cwd).await?)
            }
            _ => return None,
        };
        let fork = target == WriteTarget::Fork;
        let written = write_session(
            self.recording.backend(),
            &self.format,
            &self.task_id,
            &self.home,
            &self.cwd,
            target,
        )
        .await?;
        let id = written.harness_session_id.clone();
        self.written = Some(written);
        fork.then_some(id)
    }

    /// Starts recording `harness_session_id`; later calls do nothing.
    pub(crate) fn record(&mut self, harness_session_id: &str) {
        if self.live.is_some() || self.recording.is_off() {
            return;
        }
        let start_seq = self
            .written
            .as_ref()
            .filter(|w| w.rewritten && w.harness_session_id == harness_session_id)
            .map(|w| w.line_count);
        let recorder = SessionRecorder::start(
            Arc::clone(&self.format),
            Arc::clone(&self.recording),
            RecordTarget {
                task_id: self.task_id.clone(),
                harness_session_id: harness_session_id.to_string(),
                home: self.home.clone(),
                cwd: self.cwd.clone(),
            },
            start_seq,
        );
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
