//! Registry of long-running commands started through `moat cmd`.
//!
//! The sandbox spawns each command itself and stays the parent: reparenting to
//! PID 1 would leave a zombie that reads "alive" forever. Only a command in
//! this registry counts toward `busy_processes`, so a process started some
//! other way cannot hold the sandbox open. Each command runs in its own process
//! group, so a kill reaches the whole tree a build spawns, not just the `bash
//! -lc` wrapper.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use moatless_types::agent::AgentSubType;
use tokio::process::Command;
use tokio::sync::{RwLock, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::backend_client::BackendClient;
use crate::events::GrpcEventPublisher;

/// Grace period between the SIGTERM and the SIGKILL of a timed-out command.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// Exit code reported for a command the sandbox killed at its deadline. Matches
/// coreutils `timeout` so a caller reading the log sees the usual value.
const TIMEOUT_EXIT_CODE: i32 = 124;

/// How much of the command string becomes the default label.
const LABEL_PREVIEW_CHARS: usize = 60;

/// Lifecycle of a registered command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    /// Registered with `queue`, waiting for the queued command ahead of it.
    Queued,
    Running,
    Exited,
    TimedOut,
    Killed,
}

impl CommandState {
    /// Whether the command has stopped. A terminal command never counts toward
    /// `busy_processes` and is what the turn-end watch wakes the agent for.
    pub fn is_terminal(self) -> bool {
        !matches!(self, CommandState::Queued | CommandState::Running)
    }

    /// The form the backend stores an unreported command in. Identical to this
    /// type's serde representation, which the journal on disk already uses.
    pub fn as_str(self) -> &'static str {
        match self {
            CommandState::Queued => "queued",
            CommandState::Running => "running",
            CommandState::Exited => "exited",
            CommandState::TimedOut => "timed_out",
            CommandState::Killed => "killed",
        }
    }

    /// Parse [`Self::as_str`]. An unrecognized value becomes `Running`, whose
    /// wake text claims only that the outcome is unknown — which is what a
    /// state this pod cannot interpret leaves true.
    pub fn from_wire(value: &str) -> Self {
        match value {
            "queued" => CommandState::Queued,
            "exited" => CommandState::Exited,
            "timed_out" => CommandState::TimedOut,
            "killed" => CommandState::Killed,
            _ => CommandState::Running,
        }
    }
}

/// What the caller asked to run.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub command: String,
    pub cwd: Option<String>,
    pub timeout: Option<Duration>,
    pub label: Option<String>,
    /// Start only after every earlier queued command of this registry has
    /// finished. A command without it starts at once, queue or not.
    pub queue: bool,
}

/// One registered command, as reported to the CLI, the UI and the reaper.
#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub id: String,
    pub command: String,
    pub label: String,
    pub pid: u32,
    pub started_at_unix_ms: i64,
    pub timeout: Duration,
    pub deadline_unix_ms: i64,
    pub state: CommandState,
    pub exit_code: Option<i32>,
    pub log_path: PathBuf,
    pub finished_at_unix_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("no command with id {0}")]
    NotFound(String),
    #[error("failed to prepare command log at {path}: {source}")]
    Log {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to spawn command: {0}")]
    Spawn(std::io::Error),
}

struct Entry {
    command: CommandRecord,
    /// Cancels the supervisor's wait, which then kills the process group.
    kill: CancellationToken,
    /// Whether the agent has seen this command's terminal outcome, via `moat
    /// cmd` or a wake. Until true, a finished command still owes an answer,
    /// though it no longer defers the reaper.
    reported: bool,
}

/// A command that owed the agent an outcome, as recorded in the journal.
///
/// Carries what a replacement pod can still say. The log file lives outside
/// the workspace snapshot, deliberately, and does not survive, so its last
/// lines ride along in `log_tail` rather than the file itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournaledCommand {
    pub id: String,
    pub command: String,
    pub label: String,
    pub started_at_unix_ms: i64,
    pub state: CommandState,
    pub exit_code: Option<i32>,
    /// Bounded to [`JOURNAL_LOG_LINES`]/[`JOURNAL_LOG_BYTES`] so the snapshot
    /// stays small. Defaulted rather than required: a journal written by an
    /// older pod has no tail, and must still deserialize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
}

/// How much of a lost command's log rides the journal.
///
/// Small on purpose. This is the snapshot that crosses a pod replacement, and
/// the question it answers is "how far did this get", not "what did it print" —
/// the tail of a test run names the package it died on, which is what decides
/// whether the work must be redone.
const JOURNAL_LOG_LINES: usize = 20;
const JOURNAL_LOG_BYTES: u64 = 4096;

/// How often a running command refreshes its journaled tail.
///
/// A command only writes the journal when its state changes, so without this a
/// command that was *running* when the pod died would carry the tail it had at
/// registration, which is empty. Long enough not to matter against a build,
/// short enough that the tail names the phase the eviction interrupted.
const JOURNAL_HEARTBEAT: Duration = Duration::from_secs(30);

/// The commands that still owe the agent an outcome. Both mirrors write this,
/// and neither may narrow it: a command the agent has not been shown is owed
/// whether or not its process has stopped.
fn owed(entries: &HashMap<String, Entry>) -> Vec<JournaledCommand> {
    entries
        .values()
        .filter(|e| !e.reported)
        .map(|e| JournaledCommand {
            id: e.command.id.clone(),
            command: e.command.command.clone(),
            label: e.command.label.clone(),
            started_at_unix_ms: e.command.started_at_unix_ms,
            state: e.command.state,
            exit_code: e.command.exit_code,
            // Bounded read of the log's last page, so this stays a cheap
            // syscall under the caller's entries lock rather than a file slurp.
            log_tail: log_tail(&e.command.log_path, JOURNAL_LOG_LINES, JOURNAL_LOG_BYTES),
        })
        .collect()
}

/// Durable mirror of the unreported commands.
///
/// Rides `WORKSPACE_DIR`, since only the S3 snapshot survives a pod
/// replacement, so the next pod can read what it lost.
#[derive(Debug, Clone)]
struct Journal {
    path: PathBuf,
}

impl Journal {
    /// Rewrite the file with the entries that still owe the agent an outcome.
    /// Called with the entries lock held, so the file never records a command
    /// the registry has already discharged.
    async fn write(&self, entries: &HashMap<String, Entry>) {
        let owed = owed(entries);

        if owed.is_empty() {
            if let Err(error) = tokio::fs::remove_file(&self.path).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(path = %self.path.display(), %error, "Failed to clear the command journal");
                }
            }
            return;
        }

        let bytes = match serde_json::to_vec(&owed) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(%error, "Failed to encode the command journal");
                return;
            }
        };
        // Write to a sibling temp file and rename over the target. `rename` is
        // atomic within a filesystem, so a crash or a snapshot taken mid-write
        // sees either the old journal or the new one whole, never the truncated
        // file `write` would leave — which `take_journal` would then discard,
        // losing the very wake this exists to deliver.
        let tmp = self.path.with_extension("json.tmp");
        if let Err(error) = tokio::fs::write(&tmp, &bytes).await {
            warn!(path = %tmp.display(), %error, "Failed to stage the command journal");
            return;
        }
        if let Err(error) = tokio::fs::rename(&tmp, &self.path).await {
            warn!(path = %self.path.display(), %error, "Failed to install the command journal");
            let _ = tokio::fs::remove_file(&tmp).await;
        }
    }
}

/// Read and clear the journal a previous incarnation of this pod left behind.
///
/// Clearing here makes the loss reported exactly once, whether or not the file
/// parsed.
pub async fn take_journal(path: &Path) -> Vec<JournaledCommand> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %path.display(), %error, "Failed to read the command journal");
            }
            return Vec::new();
        }
    };
    if let Err(error) = tokio::fs::remove_file(path).await {
        warn!(path = %path.display(), %error, "Failed to clear the command journal");
    }
    match serde_json::from_slice(&bytes) {
        Ok(commands) => commands,
        Err(error) => {
            warn!(path = %path.display(), %error, "Discarding an unreadable command journal");
            Vec::new()
        }
    }
}

/// Where a task's command journal lives.
///
/// A sibling of the checkouts under `WORKSPACE_DIR`, keyed by task so two
/// Members sharing a checkout don't overwrite each other's record.
pub fn default_journal_path(workspace_dir: &str, task_id: &str) -> PathBuf {
    Path::new(workspace_dir).join(format!(".moat-commands-{task_id}.json"))
}

/// Publishes the running-command count to the backend whenever it moves.
///
/// Feeds a Task's sidebar row as an absolute count, not a delta — see
/// [`AgentSubType::CommandsChanged`].
#[derive(Clone)]
struct LivenessReporter {
    publisher: GrpcEventPublisher,
    task_id: String,
}

impl LivenessReporter {
    async fn report(&self, running: usize) {
        self.publisher
            .publish_agent_event(
                &self.task_id,
                AgentSubType::CommandsChanged,
                serde_json::json!({ "runningCommands": running }),
            )
            .await;
    }
}

/// Second mirror of the unreported commands, on its own row.
///
/// The journal is the first, and rides the S3 snapshot; this copy lives on a
/// row no restore touches.
#[derive(Clone)]
struct UnreportedMirror {
    client: Arc<dyn BackendClient>,
    task_id: String,
    /// Stamped on each snapshot under the entries lock and compared by the
    /// backend; sent off-lock, so an older, larger revision arriving late must
    /// not resurrect a command already told to the agent.
    revision: Arc<AtomicI64>,
}

impl UnreportedMirror {
    /// Snapshot and stamp under the caller's entries lock, then send off it.
    ///
    /// Detached, not awaited: awaiting the round trip there would serialize
    /// every reader behind the backend's latency.
    fn spawn_report(&self, entries: &HashMap<String, Entry>) {
        let owed = owed(entries);
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let this = self.clone();
        tokio::spawn(async move {
            this.client
                .record_unreported_commands(&this.task_id, &owed, revision)
                .await;
        });
    }
}

/// Write the unreported set to both mirrors. Called with the entries lock held,
/// so neither can record a command the registry has already reported.
async fn mirror(
    journal: Option<&Journal>,
    row: Option<&UnreportedMirror>,
    entries: &HashMap<String, Entry>,
) {
    if let Some(journal) = journal {
        journal.write(entries).await;
    }
    if let Some(row) = row {
        row.spawn_report(entries);
    }
}

/// Registry of the commands this task started through `moat cmd`.
pub struct CommandRegistry {
    entries: Arc<RwLock<HashMap<String, Entry>>>,
    default_timeout: Duration,
    log_dir: PathBuf,
    task_id: String,
    journal: Option<Journal>,
    row_mirror: Option<UnreportedMirror>,
    liveness: Option<LivenessReporter>,
    /// Closes when the last-registered queued command finishes. Swapped in
    /// `run` itself, deliberately not in the spawned task: tasks are first
    /// polled in no fixed order, so a later command could start first.
    queue_tail: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
}

impl CommandRegistry {
    /// `task_id` is the Member this registry belongs to; see
    /// `crates/sandbox/AGENTS.md` "Spawning a child process".
    pub fn new(default_timeout: Duration, log_dir: PathBuf, task_id: String) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            default_timeout,
            log_dir,
            task_id,
            journal: None,
            row_mirror: None,
            liveness: None,
            queue_tail: std::sync::Mutex::new(None),
        }
    }

    /// Report the running-command count to the backend on every change, so a
    /// Task whose turn ended with a build still running does not read as idle.
    pub fn with_liveness(mut self, publisher: GrpcEventPublisher) -> Self {
        self.liveness = Some(LivenessReporter {
            publisher,
            task_id: self.task_id.clone(),
        });
        self
    }

    /// Publish the current count. Call after releasing the entries lock: the
    /// outbox enqueue is cheap but holding the write lock across it would
    /// serialize every reader behind delivery bookkeeping.
    async fn report_liveness(&self) {
        if let Some(liveness) = &self.liveness {
            let running = running_count(&*self.entries.read().await);
            liveness.report(running).await;
        }
    }

    /// Publish the count this registry starts life with, clearing a stale
    /// non-zero left by the replaced pod — the registry is in-memory, so
    /// nothing else would ever report that command gone.
    pub async fn report_initial_liveness(&self) {
        self.report_liveness().await;
    }

    /// Mirror the unreported commands to `path` so a replacement pod can tell
    /// the agent which commands the restart destroyed.
    pub fn with_journal(mut self, path: PathBuf) -> Self {
        self.journal = Some(Journal { path });
        self
    }

    /// Mirror the same commands to a row, which a failed workspace restore does
    /// not take with it. See [`UnreportedMirror`].
    pub fn with_backend_mirror(mut self, client: Arc<dyn BackendClient>) -> Self {
        self.row_mirror = Some(UnreportedMirror {
            client,
            task_id: self.task_id.clone(),
            // Seeded from the clock, not zero: `take_unreported_commands`
            // leaves the prior pod's revision on the row, so restarting
            // from zero would lose the backend's `<` guard until it caught
            // up, silently dropping this pod's whole mirror.
            revision: Arc::new(AtomicI64::new(unix_now_ms())),
        });
        self
    }

    /// Register `spec` and spawn it, or with `queue` leave it `Queued` behind
    /// the queued command ahead of it. The returned snapshot is taken before
    /// the supervisor can observe an exit, so its state is `Running` or
    /// `Queued`. A queued command that fails to spawn ends `Exited` with no
    /// exit code, where a direct one returns `CommandError::Spawn`.
    pub async fn run(&self, spec: CommandSpec) -> Result<CommandRecord, CommandError> {
        let id = short_id();
        let timeout = spec.timeout.unwrap_or(self.default_timeout);
        let log_path = self.log_dir.join(format!("{id}.log"));

        tokio::fs::create_dir_all(&self.log_dir)
            .await
            .map_err(|source| CommandError::Log {
                path: self.log_dir.clone(),
                source,
            })?;
        let log = std::fs::File::create(&log_path).map_err(|source| CommandError::Log {
            path: log_path.clone(),
            source,
        })?;

        let now = unix_now_ms();
        let mut command = CommandRecord {
            id: id.clone(),
            command: spec.command.clone(),
            label: spec.label.clone().unwrap_or_else(|| preview(&spec.command)),
            pid: 0,
            started_at_unix_ms: now,
            timeout,
            deadline_unix_ms: now + timeout.as_millis() as i64,
            state: CommandState::Queued,
            exit_code: None,
            log_path: log_path.clone(),
            finished_at_unix_ms: 0,
        };
        let child = if spec.queue {
            None
        } else {
            let child = spawn_child(&spec, log, &self.task_id, &log_path)?;
            command.pid = child.id().unwrap_or_default();
            command.state = CommandState::Running;
            Some(child)
        };

        let kill = CancellationToken::new();
        {
            let mut entries = self.entries.write().await;
            entries.insert(
                id.clone(),
                Entry {
                    command: command.clone(),
                    kill: kill.clone(),
                    reported: false,
                },
            );
            self.persist(&entries).await;
        }

        let sinks = CommandSinks {
            entries: Arc::clone(&self.entries),
            journal: self.journal.clone(),
            row_mirror: self.row_mirror.clone(),
            liveness: self.liveness.clone(),
        };
        if let Some(child) = child {
            let pid = command.pid;
            tokio::spawn(supervise(sinks, id, child, pid, timeout, kill));
        } else {
            let (done, next) = oneshot::channel();
            let ahead = self
                .queue_tail
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .replace(next);
            tokio::spawn(run_queued(
                sinks,
                QueuePlace { ahead, done },
                spec,
                self.task_id.clone(),
                id,
                timeout,
                kill,
            ));
        }

        info!(
            id = %command.id,
            pid = command.pid,
            queued = command.state == CommandState::Queued,
            timeout_secs = timeout.as_secs(),
            command = %preview(&command.command),
            "Registered background command"
        );
        self.report_liveness().await;
        Ok(command)
    }

    /// Registry over a throwaway log dir, for fixtures that need one but do not
    /// exercise commands.
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self::new(
            Duration::from_mins(1),
            std::env::temp_dir().join(format!("moat-cmd-fixture-{}", short_id())),
            "test-task".to_string(),
        )
    }

    pub async fn list(&self) -> Vec<CommandRecord> {
        let entries = self.entries.read().await;
        let mut out: Vec<CommandRecord> = entries.values().map(|e| e.command.clone()).collect();
        out.sort_by_key(|c| c.started_at_unix_ms);
        out
    }

    pub async fn get(&self, id: &str) -> Option<CommandRecord> {
        self.entries.read().await.get(id).map(|e| e.command.clone())
    }

    /// Read commands back for the agent, marking already-finished ones reported
    /// so the turn-end watch owes no wake; `None` reads every command.
    ///
    /// Separate from `list`, whose backend poll must never mark a command seen.
    /// Mirrors only when a read reports something: `moat cmd wait` reads every
    /// few seconds, and each mirror is a journal write plus a backend call.
    pub async fn read_for_agent(
        &self,
        id: Option<&str>,
    ) -> Result<Vec<CommandRecord>, CommandError> {
        let mut entries = self.entries.write().await;
        let mut newly_reported = false;
        let mut read = |e: &mut Entry| {
            if !e.reported && e.command.state.is_terminal() {
                e.reported = true;
                newly_reported = true;
            }
            e.command.clone()
        };
        let mut out: Vec<CommandRecord> = match id {
            Some(id) => {
                let entry = entries
                    .get_mut(id)
                    .ok_or_else(|| CommandError::NotFound(id.to_string()))?;
                vec![read(entry)]
            }
            None => entries.values_mut().map(read).collect(),
        };
        out.sort_by_key(|c| c.started_at_unix_ms);
        if newly_reported {
            self.persist(&entries).await;
        }
        Ok(out)
    }

    /// Signal a command to stop, returning a still-`Running` or `Queued` snapshot; the
    /// supervisor performs the kill. Also counts as reporting: waking the agent
    /// about an ordered death is noise.
    pub async fn kill(&self, id: &str) -> Result<CommandRecord, CommandError> {
        let mut entries = self.entries.write().await;
        let entry = entries
            .get_mut(id)
            .ok_or_else(|| CommandError::NotFound(id.to_string()))?;
        entry.kill.cancel();
        entry.reported = true;
        let command = entry.command.clone();
        self.persist(&entries).await;
        Ok(command)
    }

    /// Whether any registered command is still running — the whole of
    /// `busy_processes`, since the supervisor clears a `Running` state at its
    /// deadline so a hung command stops deferring the reaper.
    pub async fn busy(&self) -> bool {
        self.entries
            .read()
            .await
            .values()
            .any(|e| !e.command.state.is_terminal())
    }

    /// The subset of `ids` that still owes the agent an outcome.
    ///
    /// Re-read on every poll, not trusted from when the watch armed: a later
    /// turn can kill or read back a command; an unknown id owes nothing.
    pub async fn unreported_among(&self, ids: &[String]) -> Vec<CommandRecord> {
        let entries = self.entries.read().await;
        ids.iter()
            .filter_map(|id| entries.get(id))
            .filter(|e| !e.reported)
            .map(|e| e.command.clone())
            .collect()
    }

    /// Commands that still owe the agent an outcome, whatever state they are
    /// in.
    ///
    /// Keys off `reported`, never state: a command can die before the watch
    /// arms, and `kill` marks it reported while still winding down.
    pub async fn unreported(&self) -> Vec<CommandRecord> {
        let entries = self.entries.read().await;
        let mut out: Vec<CommandRecord> = entries
            .values()
            .filter(|e| !e.reported)
            .map(|e| e.command.clone())
            .collect();
        out.sort_by_key(|c| c.started_at_unix_ms);
        out
    }

    /// Record that the agent has been told about these commands.
    pub async fn mark_reported(&self, ids: &[String]) {
        let mut entries = self.entries.write().await;
        for id in ids {
            if let Some(entry) = entries.get_mut(id) {
                entry.reported = true;
            }
        }
        self.persist(&entries).await;
    }

    /// Mirror the unreported commands to whichever sinks are configured.
    ///
    /// Takes the live entries so the caller holds the lock across the write,
    /// since a separately-written snapshot could resurrect a reported command.
    async fn persist(&self, entries: &HashMap<String, Entry>) {
        mirror(self.journal.as_ref(), self.row_mirror.as_ref(), entries).await;
    }
}

/// How many registered commands are still running. The only definition of the
/// count that reaches the backend; `busy_processes` is this being non-zero.
fn running_count(entries: &HashMap<String, Entry>) -> usize {
    entries
        .values()
        .filter(|e| !e.command.state.is_terminal())
        .count()
}

/// Where a finished command's outcome is written: live entries plus the mirrors
/// that move with them — what a replacement pod reads unreported commands from,
/// and what the backend renders as the count.
#[derive(Clone)]
struct CommandSinks {
    entries: Arc<RwLock<HashMap<String, Entry>>>,
    journal: Option<Journal>,
    row_mirror: Option<UnreportedMirror>,
    liveness: Option<LivenessReporter>,
}

/// Spawn `spec` in its own process group, writing both streams to `log`.
fn spawn_child(
    spec: &CommandSpec,
    log: std::fs::File,
    task_id: &str,
    log_path: &Path,
) -> Result<tokio::process::Child, CommandError> {
    let log_err = log.try_clone().map_err(|source| CommandError::Log {
        path: log_path.to_path_buf(),
        source,
    })?;
    let mut cmd = Command::new("bash");
    cmd.arg("-lc")
        .arg(&spec.command)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .env("TASK_ID", task_id)
        .kill_on_drop(false);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    // Its own process group, so the deadline kill reaches everything the
    // command spawned rather than only the `bash -lc` wrapper.
    cmd.process_group(0);
    cmd.spawn().map_err(CommandError::Spawn)
}

/// A queued command's place: `ahead` closes when the command registered
/// before it finishes, and dropping `done` closes the next command's `ahead`.
struct QueuePlace {
    ahead: Option<oneshot::Receiver<()>>,
    done: oneshot::Sender<()>,
}

/// Wait for the command ahead, then spawn and supervise a `Queued` command,
/// holding its place until it finishes. Its deadline starts when it spawns.
async fn run_queued(
    sinks: CommandSinks,
    place: QueuePlace,
    spec: CommandSpec,
    task_id: String,
    id: String,
    timeout: Duration,
    kill: CancellationToken,
) {
    let QueuePlace { ahead, done } = place;
    let ahead = async {
        if let Some(ahead) = ahead {
            // A dropped sender is the signal, so `Err` means "finished" too.
            let _ = ahead.await;
        }
    };
    tokio::pin!(ahead);
    let killed = tokio::select! {
        () = &mut ahead => kill.is_cancelled(),
        () = kill.cancelled() => true,
    };
    if killed {
        info!(id = %id, "Queued command killed before it started");
        finish(&sinks, &id, CommandState::Killed, None).await;
        // Hold `done` until the command ahead finishes: releasing it now
        // would start the next command alongside that one.
        ahead.await;
        drop(done);
        return;
    }

    let log_path = {
        let entries = sinks.entries.read().await;
        match entries.get(&id) {
            Some(entry) => entry.command.log_path.clone(),
            None => return,
        }
    };
    let spawned = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .map_err(|source| CommandError::Log {
            path: log_path.clone(),
            source,
        })
        .and_then(|log| spawn_child(&spec, log, &task_id, &log_path));
    let child = match spawned {
        Ok(child) => child,
        Err(error) => {
            warn!(id = %id, %error, "Queued command failed to start");
            let _ = std::fs::write(&log_path, format!("moat: {error}\n"));
            return finish(&sinks, &id, CommandState::Exited, None).await;
        }
    };

    let pid = child.id().unwrap_or_default();
    {
        let mut entries = sinks.entries.write().await;
        if let Some(entry) = entries.get_mut(&id) {
            let now = unix_now_ms();
            entry.command.state = CommandState::Running;
            entry.command.pid = pid;
            entry.command.started_at_unix_ms = now;
            entry.command.deadline_unix_ms = now + timeout.as_millis() as i64;
        }
        mirror(sinks.journal.as_ref(), sinks.row_mirror.as_ref(), &entries).await;
    }
    info!(id = %id, pid, "Queued command started");
    supervise(sinks, id, child, pid, timeout, kill).await;
    drop(done);
}

/// Wait for the child, its deadline, or a kill request, then record the
/// outcome.
async fn supervise(
    sinks: CommandSinks,
    id: String,
    mut child: tokio::process::Child,
    pid: u32,
    timeout: Duration,
    kill: CancellationToken,
) {
    let CommandSinks {
        entries,
        journal,
        row_mirror,
        liveness,
    } = sinks;
    let deadline = tokio::time::Instant::now() + timeout;
    // Heartbeat, not a poll: the command is still awaited on its own branch, and
    // this branch only refreshes the journaled tail. A pod is destroyed without
    // warning, so a tail written only on a state change is a tail the one case
    // that needs it — a command running when the pod died — never gets.
    let mut heartbeat = tokio::time::interval(JOURNAL_HEARTBEAT);
    heartbeat.tick().await;

    let (state, exit_code) = loop {
        tokio::select! {
            status = child.wait() => break match status {
                Ok(status) => (CommandState::Exited, status.code()),
                Err(error) => {
                    warn!(id = %id, %error, "Failed to wait on command");
                    (CommandState::Exited, None)
                }
            },
            () = tokio::time::sleep_until(deadline) => {
                warn!(id = %id, pid, timeout_secs = timeout.as_secs(), "Command hit its timeout; killing");
                terminate(pid, &mut child).await;
                break (CommandState::TimedOut, Some(TIMEOUT_EXIT_CODE));
            }
            () = kill.cancelled() => {
                info!(id = %id, pid, "Command killed on request");
                terminate(pid, &mut child).await;
                break (CommandState::Killed, None);
            }
            _ = heartbeat.tick() => {
                let entries = entries.read().await;
                mirror(journal.as_ref(), row_mirror.as_ref(), &entries).await;
            }
        }
    };

    let sinks = CommandSinks {
        entries,
        journal,
        row_mirror,
        liveness,
    };
    finish(&sinks, &id, state, exit_code).await;
}

/// Record a command's terminal outcome in every sink.
async fn finish(sinks: &CommandSinks, id: &str, state: CommandState, exit_code: Option<i32>) {
    let running = {
        let mut entries = sinks.entries.write().await;
        if let Some(entry) = entries.get_mut(id) {
            entry.command.state = state;
            entry.command.exit_code = exit_code;
            entry.command.finished_at_unix_ms = unix_now_ms();
        }
        mirror(sinks.journal.as_ref(), sinks.row_mirror.as_ref(), &entries).await;
        running_count(&entries)
    };
    info!(id = %id, ?state, ?exit_code, "Background command finished");
    if let Some(liveness) = &sinks.liveness {
        liveness.report(running).await;
    }
}

/// SIGTERM the process group, then SIGKILL it if it has not gone. Always ends
/// with a direct kill of the child, so the wait cannot hang on a failed signal.
async fn terminate(pid: u32, child: &mut tokio::process::Child) {
    if signal_group(pid, "TERM").await
        && tokio::time::timeout(KILL_GRACE, child.wait()).await.is_ok()
    {
        return;
    }
    signal_group(pid, "KILL").await;
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Signal every process in `pid`'s group.
///
/// `pid` is the group leader (spawned with `process_group(0)`); shelled out
/// since this crate denies `unsafe_code`, so no `killpg`.
async fn signal_group(pid: u32, signal: &str) -> bool {
    if pid == 0 {
        return false;
    }
    Command::new("bash")
        .arg("-c")
        .arg(format!("kill -{signal} -{pid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// The end of a command's log, for telling the agent *why* it failed, not only
/// that it did.
///
/// Reads from the end since a build log is unbounded; returns `None` when there
/// is nothing to show.
pub fn log_tail(path: &Path, lines: usize, bytes: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let offset = len.saturating_sub(bytes);
    file.seek(std::io::SeekFrom::Start(offset)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;

    let text = String::from_utf8_lossy(&buf);
    // A non-zero offset lands mid-line; that fragment is not a line the reader
    // can use.
    let mut all: Vec<&str> = text.lines().collect();
    if offset > 0 && !all.is_empty() {
        all.remove(0);
    }
    let tail = all[all.len().saturating_sub(lines)..].join("\n");
    let tail = tail.trim();
    (!tail.is_empty()).then(|| tail.to_string())
}

fn preview(command: &str) -> String {
    let flat = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= LABEL_PREVIEW_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(LABEL_PREVIEW_CHARS).collect();
    format!("{head}…")
}

fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn unix_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Default log directory for a task's commands. Deliberately outside
/// `WORKSPACE_DIR`: build output is not worth archiving into the workspace
/// snapshot, and it must stay out of the user's `git status`.
pub fn default_log_dir(task_id: &str) -> PathBuf {
    std::env::temp_dir().join("moat-commands").join(task_id)
}

#[cfg(test)]
mod tests;
