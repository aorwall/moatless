//! Interactive terminal sessions owned by a sandbox.
//!
//! The T3 compatibility surface addresses terminals by `thread_id` and
//! `terminal_id`, but the process has to live here: this process owns the real
//! workspace directory, credentials, and PTY handles. The manager keeps session
//! metadata and capped history in memory, and fans PTY output/exit events to
//! any attached gRPC streams.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::models::*;
use chrono::{SecondsFormat, Utc};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;
use tracing::debug;

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MAX_COLS: u32 = 1000;
const MAX_ROWS: u32 = 500;
const MAX_HISTORY_BYTES: usize = 512 * 1024;
const TERMINAL_EVENT_BUFFER: usize = 256;

type TerminalResult<T> = Result<T, TerminalError>;

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{0} is outside the allowed terminal size")]
    InvalidDimension(&'static str),
    #[error("terminal cwd is outside the sandbox workspace")]
    CwdOutsideWorkspace,
    #[error("terminal cwd does not exist: {0}")]
    CwdNotFound(String),
    #[error("terminal cwd is not a directory: {0}")]
    CwdNotDirectory(String),
    #[error("terminal session not found")]
    NotFound,
    #[error("terminal is not running")]
    NotRunning,
    #[error("terminal operation failed: {0}")]
    Operation(String),
}

#[derive(Clone)]
pub struct TerminalManager {
    inner: Arc<Mutex<Inner>>,
    /// The containment boundary: a resolved cwd outside this is refused, which
    /// is what keeps a sibling repo reachable but the rest of the container
    /// not.
    workspace_dir: Arc<PathBuf>,
    /// Where a nominal task path lands: the primary checkout that git
    /// status/diff, the file API, setup commands and dev servers all target, so
    /// a terminal opens where the code is.
    repo_dir: Arc<PathBuf>,
    /// The program each terminal runs; `/bin/bash` unless [`Self::with_shell`].
    shell: Arc<str>,
}

struct Inner {
    sessions: HashMap<SessionKey, Session>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SessionKey {
    thread_id: String,
    terminal_id: String,
}

struct Session {
    thread_id: String,
    terminal_id: String,
    requested_cwd: String,
    worktree_path: Option<String>,
    status: TerminalSessionStatus,
    pid: Option<u32>,
    history: String,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    updated_at: String,
    sequence: u64,
    cols: u16,
    rows: u16,
    env: HashMap<String, String>,
    writer: Option<Box<dyn Write + Send>>,
    master: Option<Box<dyn MasterPty + Send>>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    events: broadcast::Sender<TerminalAttachStreamEvent>,
    reader_task: Option<JoinHandle<()>>,
    wait_task: Option<JoinHandle<()>>,
}

struct SpawnedTerminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    reader: Box<dyn Read + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
}

pub struct AttachedTerminal {
    pub snapshot: TerminalSessionSnapshot,
    pub events: broadcast::Receiver<TerminalAttachStreamEvent>,
}

impl TerminalManager {
    /// A manager whose nominal paths land at the workspace root, for a sandbox
    /// with no repo to clone.
    ///
    /// [`Self::with_repo_dir`] is what a cloned sandbox uses.
    pub fn new(workspace_dir: impl Into<PathBuf>) -> Self {
        let workspace_dir = workspace_dir.into();
        Self::with_repo_dir(workspace_dir.clone(), workspace_dir)
    }

    /// A manager that lands nominal task paths in the primary repo's checkout.
    pub fn with_repo_dir(workspace_dir: impl Into<PathBuf>, repo_dir: impl Into<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                sessions: HashMap::new(),
            })),
            workspace_dir: Arc::new(workspace_dir.into()),
            repo_dir: Arc::new(repo_dir.into()),
            shell: Arc::from("/bin/bash"),
        }
    }

    /// Run `shell` in each terminal this manager opens.
    #[must_use]
    pub fn with_shell(mut self, shell: &str) -> Self {
        self.shell = Arc::from(shell);
        self
    }

    /// Whether any session still holds a shell that can write the workspace;
    /// `workspace_sync::sync` reads this on shutdown.
    ///
    /// A session stays in the map with no `writer` once it exits, so status is
    /// what distinguishes a live shell from a record of one. `Starting` counts:
    /// its shell exists by the time an answer here is acted on.
    pub async fn has_live_sessions(&self) -> bool {
        self.inner.lock().await.sessions.values().any(|session| {
            matches!(
                session.status,
                TerminalSessionStatus::Starting | TerminalSessionStatus::Running
            )
        })
    }

    pub async fn open(
        &self,
        request: TerminalOpenRequest,
    ) -> TerminalResult<TerminalSessionSnapshot> {
        let input = StartInput::from_open(request)?;
        self.open_existing_or_start(input).await
    }

    pub async fn restart(
        &self,
        request: TerminalRestartRequest,
    ) -> TerminalResult<TerminalSessionSnapshot> {
        let input = StartInput::from_restart(request)?;
        self.open_or_restart(input, TerminalStartKind::Restarted)
            .await
    }

    pub async fn attach(&self, request: TerminalAttachRequest) -> TerminalResult<AttachedTerminal> {
        let key = session_key(&request.thread_id, &request.terminal_id)?;
        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(&key) {
            if let (Some(cols), Some(rows)) = (cols(request.cols)?, rows(request.rows)?) {
                resize_session(session, cols, rows)?;
            }
            let events = session.events.subscribe();
            return Ok(AttachedTerminal {
                snapshot: snapshot(session),
                events,
            });
        }
        drop(inner);

        if request.restart_if_not_running {
            let cwd = request.cwd.ok_or(TerminalError::Missing("cwd"))?;
            let input = StartInput {
                thread_id: request.thread_id,
                terminal_id: request.terminal_id,
                cwd,
                worktree_path: request.worktree_path,
                cols: cols(request.cols)?.unwrap_or(DEFAULT_COLS),
                rows: rows(request.rows)?.unwrap_or(DEFAULT_ROWS),
                env: request.env,
            };
            let snapshot = self
                .open_or_restart(input.clone(), TerminalStartKind::Started)
                .await?;
            let mut inner = self.inner.lock().await;
            let session = inner
                .sessions
                .get_mut(&SessionKey {
                    thread_id: input.thread_id,
                    terminal_id: input.terminal_id,
                })
                .ok_or(TerminalError::NotFound)?;
            return Ok(AttachedTerminal {
                snapshot,
                events: session.events.subscribe(),
            });
        }

        Err(TerminalError::NotFound)
    }

    pub async fn write(&self, request: TerminalWriteRequest) -> TerminalResult<()> {
        if request.data.is_empty() {
            return Err(TerminalError::Missing("data"));
        }
        let key = session_key(&request.thread_id, &request.terminal_id)?;
        let mut inner = self.inner.lock().await;
        let session = inner
            .sessions
            .get_mut(&key)
            .ok_or(TerminalError::NotFound)?;
        if session.status != TerminalSessionStatus::Running {
            return Err(TerminalError::NotRunning);
        }
        let writer = session.writer.as_mut().ok_or(TerminalError::NotRunning)?;
        writer
            .write_all(request.data.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|error| TerminalError::Operation(error.to_string()))
    }

    pub async fn resize(&self, request: TerminalResizeRequest) -> TerminalResult<()> {
        let key = session_key(&request.thread_id, &request.terminal_id)?;
        let cols = cols(request.cols)?.ok_or(TerminalError::Missing("cols"))?;
        let rows = rows(request.rows)?.ok_or(TerminalError::Missing("rows"))?;
        let mut inner = self.inner.lock().await;
        let session = inner
            .sessions
            .get_mut(&key)
            .ok_or(TerminalError::NotFound)?;
        resize_session(session, cols, rows)
    }

    pub async fn clear(&self, request: TerminalClearRequest) -> TerminalResult<()> {
        let key = session_key(&request.thread_id, &request.terminal_id)?;
        let mut inner = self.inner.lock().await;
        let session = inner
            .sessions
            .get_mut(&key)
            .ok_or(TerminalError::NotFound)?;
        session.history.clear();
        let sequence = advance(session);
        publish(session, TerminalEvent::Cleared(true), sequence);
        Ok(())
    }

    pub async fn close(&self, request: TerminalCloseRequest) -> TerminalResult<()> {
        if let Some(terminal_id) = request.terminal_id {
            let key = session_key(&request.thread_id, &terminal_id)?;
            let mut inner = self.inner.lock().await;
            if let Some(session) = inner.sessions.remove(&key) {
                close_session(session);
            }
            return Ok(());
        }

        let mut inner = self.inner.lock().await;
        let keys: Vec<SessionKey> = inner
            .sessions
            .keys()
            .filter(|key| key.thread_id == request.thread_id)
            .cloned()
            .collect();
        for key in keys {
            if let Some(session) = inner.sessions.remove(&key) {
                close_session(session);
            }
        }
        Ok(())
    }

    async fn open_or_restart(
        &self,
        input: StartInput,
        kind: TerminalStartKind,
    ) -> TerminalResult<TerminalSessionSnapshot> {
        let key = session_key(&input.thread_id, &input.terminal_id)?;
        let real_cwd = self.resolve_cwd(&input.cwd)?;

        {
            let mut inner = self.inner.lock().await;
            if let Some(existing) = inner.sessions.get_mut(&key) {
                stop_session_process(existing);
                existing.status = TerminalSessionStatus::Starting;
            }
        }

        let spawned = spawn_terminal(
            Arc::clone(&self.shell),
            real_cwd.clone(),
            input.cols,
            input.rows,
            &input.env,
        )
        .await
        .map_err(|error| TerminalError::Operation(error.to_string()))?;
        let SpawnedTerminal {
            master,
            writer,
            reader,
            child,
            killer,
            pid,
        } = spawned;

        let reader_manager = self.clone();
        let reader_key = key.clone();
        let reader_handle = tokio::runtime::Handle::current();
        let reader_task = tokio::task::spawn_blocking(move || {
            read_output_loop(reader_handle, reader_manager, reader_key, reader);
        });

        let wait_manager = self.clone();
        let wait_key = key.clone();
        let wait_handle = tokio::runtime::Handle::current();
        let wait_task = tokio::task::spawn_blocking(move || {
            let mut child = child;
            match child.wait() {
                Ok(status) => {
                    let code = i32::try_from(status.exit_code()).ok();
                    wait_handle.block_on(async move {
                        wait_manager.finish_session(&wait_key, code, None).await;
                    });
                }
                Err(error) => {
                    wait_handle.block_on(async move {
                        wait_manager
                            .fail_session(&wait_key, error.to_string())
                            .await;
                    });
                }
            }
        });

        let mut inner = self.inner.lock().await;
        let session = inner.sessions.entry(key).or_insert_with(|| {
            let (events, _receiver) = broadcast::channel(TERMINAL_EVENT_BUFFER);
            Session {
                thread_id: input.thread_id.clone(),
                terminal_id: input.terminal_id.clone(),
                requested_cwd: input.cwd.clone(),
                worktree_path: input.worktree_path.clone(),
                status: TerminalSessionStatus::Starting,
                pid: None,
                history: String::new(),
                exit_code: None,
                exit_signal: None,
                updated_at: now(),
                sequence: 0,
                cols: input.cols,
                rows: input.rows,
                env: input.env.clone(),
                writer: None,
                master: None,
                killer: None,
                events,
                reader_task: None,
                wait_task: None,
            }
        });
        session.thread_id = input.thread_id.clone();
        session.terminal_id = input.terminal_id.clone();
        session.requested_cwd = input.cwd;
        session.worktree_path = input.worktree_path;
        session.status = TerminalSessionStatus::Running;
        session.pid = pid;
        session.history.clear();
        session.exit_code = None;
        session.exit_signal = None;
        session.cols = input.cols;
        session.rows = input.rows;
        session.env = input.env;
        session.writer = Some(writer);
        session.master = Some(master);
        session.killer = Some(killer);
        session.reader_task = Some(reader_task);
        session.wait_task = Some(wait_task);

        let sequence = advance(session);
        let snapshot_value = snapshot(session);
        let event = match kind {
            TerminalStartKind::Started => TerminalEvent::Started(TerminalStartedEvent {
                snapshot: Some(snapshot_value.clone()),
            }),
            TerminalStartKind::Restarted => TerminalEvent::Restarted(TerminalRestartedEvent {
                snapshot: Some(snapshot_value.clone()),
            }),
        };
        publish(session, event, sequence);
        Ok(snapshot_value)
    }

    async fn open_existing_or_start(
        &self,
        input: StartInput,
    ) -> TerminalResult<TerminalSessionSnapshot> {
        let key = session_key(&input.thread_id, &input.terminal_id)?;
        self.resolve_cwd(&input.cwd)?;

        let mut inner = self.inner.lock().await;
        if let Some(session) = inner.sessions.get_mut(&key) {
            let launch_context_matches = session.requested_cwd == input.cwd
                && session.worktree_path == input.worktree_path
                && session.env == input.env;
            if launch_context_matches
                && session.status == TerminalSessionStatus::Running
                && session.writer.is_some()
            {
                if session.cols != input.cols || session.rows != input.rows {
                    resize_session(session, input.cols, input.rows)?;
                }
                let mut reused = snapshot(session);
                reused.started_new_process = Some(false);
                return Ok(reused);
            }
        }
        drop(inner);

        let mut started = self
            .open_or_restart(input, TerminalStartKind::Started)
            .await?;
        started.started_new_process = Some(true);
        Ok(started)
    }

    fn resolve_cwd(&self, requested: &str) -> TerminalResult<PathBuf> {
        // A nominal task path names "this conversation's checkout", not a real
        // directory — it is the opaque root T3 was handed and hands back. It
        // lands in the primary repo, which is what a script like `bun run dev`
        // needs: the workspace root is a level above the manifest and every
        // other repo-scoped operation already targets the checkout.
        let path = if requested.starts_with("/moatless/tasks/") {
            self.repo_dir.as_ref().clone()
        } else {
            let requested_path = Path::new(requested);
            if requested_path.is_absolute() {
                requested_path.to_path_buf()
            } else {
                self.workspace_dir.join(requested_path)
            }
        };
        let canonical_workspace = self
            .workspace_dir
            .canonicalize()
            .map_err(|_| TerminalError::CwdNotFound(self.workspace_dir.display().to_string()))?;
        let canonical_path = path
            .canonicalize()
            .map_err(|_| TerminalError::CwdNotFound(path.display().to_string()))?;
        if !canonical_path.starts_with(&canonical_workspace) {
            return Err(TerminalError::CwdOutsideWorkspace);
        }
        if !canonical_path.is_dir() {
            return Err(TerminalError::CwdNotDirectory(
                canonical_path.display().to_string(),
            ));
        }
        Ok(canonical_path)
    }

    async fn emit_output(&self, key: &SessionKey, data: String) {
        let mut inner = self.inner.lock().await;
        let Some(session) = inner.sessions.get_mut(key) else {
            return;
        };
        if session.status != TerminalSessionStatus::Running {
            return;
        }
        append_history(session, &data);
        let sequence = advance(session);
        publish(
            session,
            TerminalEvent::Output(TerminalOutputEvent { data }),
            sequence,
        );
    }

    async fn finish_session(
        &self,
        key: &SessionKey,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
    ) {
        let mut inner = self.inner.lock().await;
        let Some(session) = inner.sessions.get_mut(key) else {
            return;
        };
        if session.status != TerminalSessionStatus::Running {
            return;
        }
        session.status = TerminalSessionStatus::Exited;
        session.pid = None;
        session.writer = None;
        session.killer = None;
        session.master = None;
        session.exit_code = exit_code;
        session.exit_signal = exit_signal;
        let sequence = advance(session);
        publish(
            session,
            TerminalEvent::Exited(TerminalExitedEvent {
                exit_code,
                exit_signal,
            }),
            sequence,
        );
    }

    async fn fail_session(&self, key: &SessionKey, message: String) {
        let mut inner = self.inner.lock().await;
        let Some(session) = inner.sessions.get_mut(key) else {
            return;
        };
        session.status = TerminalSessionStatus::Error;
        session.pid = None;
        session.writer = None;
        session.killer = None;
        session.master = None;
        let sequence = advance(session);
        publish(
            session,
            TerminalEvent::Error(TerminalErrorEvent { message }),
            sequence,
        );
    }
}

#[derive(Clone)]
struct StartInput {
    thread_id: String,
    terminal_id: String,
    cwd: String,
    worktree_path: Option<String>,
    cols: u16,
    rows: u16,
    env: HashMap<String, String>,
}

impl StartInput {
    fn from_open(request: TerminalOpenRequest) -> TerminalResult<Self> {
        Ok(Self {
            thread_id: non_empty(request.thread_id, "thread_id")?,
            terminal_id: non_empty(request.terminal_id, "terminal_id")?,
            cwd: non_empty(request.cwd, "cwd")?,
            worktree_path: request.worktree_path,
            cols: cols(request.cols)?.unwrap_or(DEFAULT_COLS),
            rows: rows(request.rows)?.unwrap_or(DEFAULT_ROWS),
            env: request.env,
        })
    }

    fn from_restart(request: TerminalRestartRequest) -> TerminalResult<Self> {
        Ok(Self {
            thread_id: non_empty(request.thread_id, "thread_id")?,
            terminal_id: non_empty(request.terminal_id, "terminal_id")?,
            cwd: non_empty(request.cwd, "cwd")?,
            worktree_path: request.worktree_path,
            cols: cols(request.cols)?.unwrap_or(DEFAULT_COLS),
            rows: rows(request.rows)?.unwrap_or(DEFAULT_ROWS),
            env: request.env,
        })
    }
}

#[derive(Clone, Copy)]
enum TerminalStartKind {
    Started,
    Restarted,
}

async fn spawn_terminal(
    shell: Arc<str>,
    cwd: PathBuf,
    cols: u16,
    rows: u16,
    env: &HashMap<String, String>,
) -> TerminalResult<SpawnedTerminal> {
    let env = env.clone();
    tokio::task::spawn_blocking(move || {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| TerminalError::Operation(error.to_string()))?;
        let mut command = CommandBuilder::new(&*shell);
        command.cwd(&cwd);
        command.env("TERM", "xterm-256color");
        for (key, value) in env {
            command.env(key, value);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| TerminalError::Operation(error.to_string()))?;
        let pid = child.process_id();
        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| TerminalError::Operation(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| TerminalError::Operation(error.to_string()))?;
        Ok(SpawnedTerminal {
            master: pair.master,
            writer,
            reader,
            child,
            killer,
            pid,
        })
    })
    .await
    .map_err(|error| TerminalError::Operation(error.to_string()))?
}

fn read_output_loop(
    handle: tokio::runtime::Handle,
    manager: TerminalManager,
    key: SessionKey,
    mut reader: Box<dyn Read + Send>,
) {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return,
            Ok(n) => {
                let data = String::from_utf8_lossy(&buffer[..n]).into_owned();
                handle.block_on(manager.emit_output(&key, data));
            }
            Err(error) => {
                debug!(%error, "terminal reader stopped");
                return;
            }
        }
    }
}

fn close_session(mut session: Session) {
    stop_session_process(&mut session);
    let sequence = session.sequence.saturating_add(1);
    publish(&session, TerminalEvent::Closed(true), sequence);
}

fn stop_session_process(session: &mut Session) {
    if let Some(task) = session.reader_task.take() {
        task.abort();
    }
    if let Some(task) = session.wait_task.take() {
        task.abort();
    }
    if let Some(mut killer) = session.killer.take()
        && let Err(error) = killer.kill()
    {
        debug!(%error, "failed to kill terminal child");
    }
    session.writer = None;
    session.master = None;
    session.pid = None;
}

fn resize_session(session: &mut Session, cols: u16, rows: u16) -> TerminalResult<()> {
    session.cols = cols;
    session.rows = rows;
    session.updated_at = now();
    if let Some(master) = &session.master {
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| TerminalError::Operation(error.to_string()))?;
    }
    Ok(())
}

fn snapshot(session: &Session) -> TerminalSessionSnapshot {
    TerminalSessionSnapshot {
        thread_id: session.thread_id.clone(),
        terminal_id: session.terminal_id.clone(),
        cwd: session.requested_cwd.clone(),
        worktree_path: session.worktree_path.clone(),
        status: session.status,
        pid: session.pid,
        history: session.history.clone(),
        exit_code: session.exit_code,
        exit_signal: session.exit_signal,
        label: terminal_label(&session.terminal_id),
        updated_at: session.updated_at.clone(),
        sequence: session.sequence,
        // Only the open path knows whether it started or reused; every other
        // caller of this helper leaves the answer absent.
        started_new_process: None,
    }
}

fn publish(session: &Session, event: TerminalEvent, sequence: u64) {
    let _ = session.events.send(TerminalAttachStreamEvent {
        thread_id: session.thread_id.clone(),
        terminal_id: session.terminal_id.clone(),
        sequence,
        event: Some(event),
    });
}

fn advance(session: &mut Session) -> u64 {
    session.sequence = session.sequence.saturating_add(1);
    session.updated_at = now();
    session.sequence
}

fn append_history(session: &mut Session, data: &str) {
    session.history.push_str(data);
    if session.history.len() <= MAX_HISTORY_BYTES {
        return;
    }
    let excess = session.history.len() - MAX_HISTORY_BYTES;
    let start = session
        .history
        .char_indices()
        .find_map(|(index, _)| (index >= excess).then_some(index))
        .unwrap_or(session.history.len());
    session.history.drain(..start);
}

fn terminal_label(terminal_id: &str) -> String {
    terminal_id
        .strip_prefix("term-")
        .and_then(|suffix| suffix.parse::<u32>().ok())
        .map_or_else(
            || terminal_id.to_string(),
            |number| format!("Terminal {number}"),
        )
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn session_key(thread_id: &str, terminal_id: &str) -> TerminalResult<SessionKey> {
    Ok(SessionKey {
        thread_id: non_empty(thread_id.to_string(), "thread_id")?,
        terminal_id: non_empty(terminal_id.to_string(), "terminal_id")?,
    })
}

fn non_empty(value: String, field: &'static str) -> TerminalResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(TerminalError::Missing(field));
    }
    Ok(trimmed.to_string())
}

fn cols(value: u32) -> TerminalResult<Option<u16>> {
    dimension(value, MAX_COLS, "cols")
}

fn rows(value: u32) -> TerminalResult<Option<u16>> {
    dimension(value, MAX_ROWS, "rows")
}

fn dimension(value: u32, max: u32, field: &'static str) -> TerminalResult<Option<u16>> {
    if value == 0 {
        return Ok(None);
    }
    if value > max {
        return Err(TerminalError::InvalidDimension(field));
    }
    u16::try_from(value)
        .map(Some)
        .map_err(|_| TerminalError::InvalidDimension(field))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_reuses_matching_running_session() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let manager = TerminalManager::new(tmp.path());
        let (events, _) = broadcast::channel(TERMINAL_EVENT_BUFFER);
        let key = SessionKey {
            thread_id: "thread-1".to_string(),
            terminal_id: "term-1".to_string(),
        };
        let mut env = HashMap::new();
        env.insert("EXAMPLE".to_string(), "1".to_string());
        {
            let mut inner = manager.inner.lock().await;
            inner.sessions.insert(
                key,
                Session {
                    thread_id: "thread-1".to_string(),
                    terminal_id: "term-1".to_string(),
                    requested_cwd: ".".to_string(),
                    worktree_path: None,
                    status: TerminalSessionStatus::Running,
                    pid: Some(4242),
                    history: "existing history".to_string(),
                    exit_code: None,
                    exit_signal: None,
                    updated_at: "2026-08-02T00:00:00.000Z".to_string(),
                    sequence: 7,
                    cols: 80,
                    rows: 24,
                    env: env.clone(),
                    writer: Some(Box::new(Vec::<u8>::new())),
                    master: None,
                    killer: None,
                    events,
                    reader_task: None,
                    wait_task: None,
                },
            );
        }

        let snapshot = manager
            .open(TerminalOpenRequest {
                thread_id: "thread-1".to_string(),
                terminal_id: "term-1".to_string(),
                cwd: ".".to_string(),
                worktree_path: None,
                cols: 120,
                rows: 30,
                env,
            })
            .await
            .expect("open reuses running session");

        assert_eq!(snapshot.pid, Some(4242));
        assert_eq!(snapshot.history, "existing history");
        assert_eq!(snapshot.sequence, 7);
        // The reuse the caller must be able to see: nothing was started, so
        // scripts.run must not type its command into the running session.
        assert_eq!(snapshot.started_new_process, Some(false));

        let inner = manager.inner.lock().await;
        let session = inner
            .sessions
            .get(&SessionKey {
                thread_id: "thread-1".to_string(),
                terminal_id: "term-1".to_string(),
            })
            .expect("session remains registered");
        assert_eq!(session.cols, 120);
        assert_eq!(session.rows, 30);
    }

    /// A session outlives its shell in this map, so counting records rather
    /// than live shells would leave one finished terminal denying every later
    /// shutdown its final-tree tag for the pod's whole life.
    #[tokio::test]
    async fn a_session_that_has_exited_is_no_longer_a_workspace_writer() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let manager = TerminalManager::new(tmp.path());
        let (events, _) = broadcast::channel(TERMINAL_EVENT_BUFFER);

        let insert = async |status: TerminalSessionStatus| {
            let mut inner = manager.inner.lock().await;
            inner.sessions.clear();
            inner.sessions.insert(
                SessionKey {
                    thread_id: "thread-1".to_string(),
                    terminal_id: "term-1".to_string(),
                },
                Session {
                    thread_id: "thread-1".to_string(),
                    terminal_id: "term-1".to_string(),
                    requested_cwd: ".".to_string(),
                    worktree_path: None,
                    status,
                    pid: None,
                    history: String::new(),
                    exit_code: None,
                    exit_signal: None,
                    updated_at: "2026-08-02T00:00:00.000Z".to_string(),
                    sequence: 1,
                    cols: 80,
                    rows: 24,
                    env: HashMap::new(),
                    writer: None,
                    master: None,
                    killer: None,
                    events: events.clone(),
                    reader_task: None,
                    wait_task: None,
                },
            );
        };

        assert!(!manager.has_live_sessions().await, "no session at all");
        for gone in [TerminalSessionStatus::Exited, TerminalSessionStatus::Error] {
            insert(gone).await;
            assert!(
                !manager.has_live_sessions().await,
                "{gone:?} holds no shell"
            );
        }
        for live in [
            TerminalSessionStatus::Starting,
            TerminalSessionStatus::Running,
        ] {
            insert(live).await;
            assert!(
                manager.has_live_sessions().await,
                "{live:?} can still write"
            );
        }
    }

    #[tokio::test]
    async fn open_of_a_fresh_id_reports_a_started_process() {
        // No session under this id: the open spawns one, and must say so, so
        // scripts.run knows this is the run that gets the command written to it.
        let tmp = tempfile::tempdir().expect("temp dir");
        let manager = TerminalManager::new(tmp.path());

        let snapshot = manager
            .open(TerminalOpenRequest {
                thread_id: "thread-1".to_string(),
                terminal_id: "term-1".to_string(),
                cwd: ".".to_string(),
                worktree_path: None,
                cols: 80,
                rows: 24,
                env: HashMap::new(),
            })
            .await
            .expect("open starts a fresh session");

        assert_eq!(snapshot.started_new_process, Some(true));
        assert!(snapshot.pid.is_some());

        manager
            .close(TerminalCloseRequest {
                thread_id: "thread-1".to_string(),
                terminal_id: Some("term-1".to_string()),
                delete_history: false,
            })
            .await
            .expect("close fresh terminal");
    }

    #[tokio::test]
    async fn restart_publishes_on_existing_session_channel() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let manager = TerminalManager::new(tmp.path());
        let (events, _) = broadcast::channel(TERMINAL_EVENT_BUFFER);
        let mut rx = events.subscribe();
        let key = SessionKey {
            thread_id: "thread-1".to_string(),
            terminal_id: "term-1".to_string(),
        };
        {
            let mut inner = manager.inner.lock().await;
            inner.sessions.insert(
                key,
                Session {
                    thread_id: "thread-1".to_string(),
                    terminal_id: "term-1".to_string(),
                    requested_cwd: ".".to_string(),
                    worktree_path: None,
                    status: TerminalSessionStatus::Running,
                    pid: Some(1111),
                    history: "existing history".to_string(),
                    exit_code: None,
                    exit_signal: None,
                    updated_at: "2026-08-02T00:00:00.000Z".to_string(),
                    sequence: 7,
                    cols: 80,
                    rows: 24,
                    env: HashMap::new(),
                    writer: Some(Box::new(Vec::<u8>::new())),
                    master: None,
                    killer: None,
                    events,
                    reader_task: None,
                    wait_task: None,
                },
            );
        }

        let snapshot = manager
            .restart(TerminalRestartRequest {
                thread_id: "thread-1".to_string(),
                terminal_id: "term-1".to_string(),
                cwd: ".".to_string(),
                worktree_path: None,
                cols: 80,
                rows: 24,
                env: HashMap::new(),
            })
            .await
            .expect("restart succeeds");
        assert_eq!(snapshot.sequence, 8);
        assert!(snapshot.pid.is_some());
        assert!(snapshot.history.is_empty());

        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("restart event is published")
            .expect("restart event is readable");
        assert!(matches!(event.event, Some(TerminalEvent::Restarted(_))));

        manager
            .close(TerminalCloseRequest {
                thread_id: "thread-1".to_string(),
                terminal_id: Some("term-1".to_string()),
                delete_history: false,
            })
            .await
            .expect("close restarted terminal");
    }

    #[test]
    fn nominal_t3_cwd_resolves_to_the_primary_repo() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let repo = tmp.path().join("soap-frontend");
        std::fs::create_dir(&repo).expect("repo dir");
        let manager = TerminalManager::with_repo_dir(tmp.path(), &repo);
        // Not the workspace root a level above: a script the workspace declares
        // runs against the checkout's manifest, like every other repo-scoped op.
        assert_eq!(
            manager
                .resolve_cwd("/moatless/tasks/t-1")
                .expect("nominal cwd resolves"),
            repo.canonicalize().expect("canonical repo")
        );
    }

    #[test]
    fn nominal_t3_cwd_falls_back_to_the_workspace_without_a_repo() {
        let tmp = tempfile::tempdir().expect("temp dir");
        // A blank workspace clones nothing, so `repo_dir` *is* the workspace dir
        // and the nominal path has nowhere else to land.
        let manager = TerminalManager::new(tmp.path());
        assert_eq!(
            manager
                .resolve_cwd("/moatless/tasks/t-1")
                .expect("nominal cwd resolves"),
            tmp.path().canonicalize().expect("canonical temp")
        );
    }

    #[test]
    fn relative_cwd_cannot_escape_workspace() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let manager = TerminalManager::new(tmp.path());
        let err = manager
            .resolve_cwd("../")
            .expect_err("parent traversal is refused");
        assert!(matches!(err, TerminalError::CwdOutsideWorkspace));
    }
}
