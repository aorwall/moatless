//! Registry of long-running commands started through `moat cmd`.
//!
//! The sandbox spawns each command itself, so it stays the parent: it reaps the
//! child (an orphan reparented to PID 1 would linger as a zombie and make any
//! PID-liveness check read "alive" forever) and it owns the deadline. Only a
//! command in this registry counts toward `busy_processes`, so a process the
//! agent started some other way cannot hold the sandbox open.
//!
//! Each command runs in its own process group, so a kill reaches the whole tree
//! a build spawns rather than just the `bash -lc` wrapper.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use moatless_types::agent::AgentSubType;
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::backend_client::BackendClient;
use crate::events::GrpcEventPublisher;

/// Grace period between the SIGTERM and the SIGKILL of a timed-out command.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// Exit code reported for a command the sandbox killed at its deadline.
/// Matches coreutils `timeout` so a caller reading the log sees the usual value.
const TIMEOUT_EXIT_CODE: i32 = 124;

/// How much of the command string becomes the default label.
const LABEL_PREVIEW_CHARS: usize = 60;

/// Lifecycle of a registered command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    Running,
    Exited,
    TimedOut,
    Killed,
}

impl CommandState {
    /// Whether the command has stopped. A terminal command never counts toward
    /// `busy_processes` and is what the turn-end watch wakes the agent for.
    pub fn is_terminal(self) -> bool {
        !matches!(self, CommandState::Running)
    }

    /// The form the backend stores an unreported command in. Identical to this
    /// type's serde representation, which the journal on disk already uses.
    pub fn as_str(self) -> &'static str {
        match self {
            CommandState::Running => "running",
            CommandState::Exited => "exited",
            CommandState::TimedOut => "timed_out",
            CommandState::Killed => "killed",
        }
    }

    /// Parse [`Self::as_str`]. An unrecognized value becomes `Running`, whose
    /// wake text claims only that the outcome is unknown — which is what a state
    /// this pod cannot interpret leaves true.
    pub fn from_wire(value: &str) -> Self {
        match value {
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
    /// The agent has been shown this command's terminal outcome — either it
    /// read the command back through `moat cmd`, or a wake named it. Until
    /// that is true a finished command still owes the agent an answer, even
    /// though it no longer defers the reaper.
    reported: bool,
}

/// A command that owed the agent an outcome, as recorded in the journal.
///
/// Carries what a replacement pod can still say about it. The log cannot be
/// among that: it lives outside the workspace snapshot and dies with the pod.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournaledCommand {
    pub id: String,
    pub command: String,
    pub label: String,
    pub started_at_unix_ms: i64,
    pub state: CommandState,
    pub exit_code: Option<i32>,
}

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
        })
        .collect()
}

/// Durable mirror of the unreported commands.
///
/// The registry is in-memory, so a pod replacement destroys every `moat cmd`
/// process along with the watch that promised the agent a wake. This file is
/// what a replacement pod reads to name what it lost. It lives under
/// `WORKSPACE_DIR` because the S3 workspace snapshot is the only sandbox state
/// that rides a pod replacement.
#[derive(Debug, Clone)]
struct Journal {
    path: PathBuf,
}

impl Journal {
    /// Rewrite the file with the entries that still owe the agent an outcome.
    /// Called with the entries lock held, so the file never records a command the
    /// registry has already discharged.
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
/// Clearing it here is what makes the loss reported exactly once: the file is
/// gone whether or not it parsed, so a corrupt journal cannot be re-read on
/// every subsequent start.
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

/// Where a task's command journal lives. Inside `WORKSPACE_DIR` so it rides the
/// snapshot into a replacement pod, and a sibling of the checkouts rather than
/// a path inside one, so it stays out of the user's `git status`.
pub fn default_journal_path(workspace_dir: &str) -> PathBuf {
    Path::new(workspace_dir).join(".moat-commands.json")
}

/// Publishes the running-command count to the backend whenever it moves.
///
/// The count is what reaches a Task's sidebar row, so an unreported change
/// leaves the row claiming the opposite of what the sandbox is doing until the
/// next command moves. Absolute rather than a delta — see
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
/// The journal is the first, and it rides the S3 workspace snapshot — which a
/// failed restore takes with it, leaving the agent never told what the restart
/// destroyed. This copy is on a row no restore touches.
#[derive(Clone)]
struct UnreportedMirror {
    client: Arc<dyn BackendClient>,
    task_id: String,
    /// Stamped on each snapshot under the entries lock, and compared by the
    /// backend. The send happens off that lock, so two of them can arrive out of
    /// order and an older, larger set would resurrect a command the agent has
    /// already been told about.
    revision: Arc<AtomicI64>,
}

impl UnreportedMirror {
    /// Snapshot and stamp under the caller's entries lock, then send off it.
    ///
    /// Detached rather than awaited: every caller holds the write lock, and
    /// waiting for a gRPC round trip there would serialize every reader behind
    /// the backend's latency.
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
    journal: Option<Journal>,
    row_mirror: Option<UnreportedMirror>,
    liveness: Option<LivenessReporter>,
}

impl CommandRegistry {
    pub fn new(default_timeout: Duration, log_dir: PathBuf) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            default_timeout,
            log_dir,
            journal: None,
            row_mirror: None,
            liveness: None,
        }
    }

    /// Report the running-command count to the backend on every change, so a
    /// Task whose turn ended with a build still running does not read as idle.
    pub fn with_liveness(mut self, publisher: GrpcEventPublisher, task_id: String) -> Self {
        self.liveness = Some(LivenessReporter { publisher, task_id });
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

    /// Publish the count this registry starts life with, which is what clears a
    /// stale non-zero left by the pod this one replaced. The registry is
    /// in-memory, so nothing else would ever report that command gone: it died
    /// with its pod and no transition remains to publish.
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
    pub fn with_backend_mirror(mut self, client: Arc<dyn BackendClient>, task_id: String) -> Self {
        self.row_mirror = Some(UnreportedMirror {
            client,
            task_id,
            // Seeded from the clock, not from zero: `take_unreported_commands` leaves
            // the prior pod's revision on the row, so a counter that restarted
            // would lose the backend's `<` guard until it caught up — silently
            // dropping this pod's whole mirror, which is the case the mirror
            // exists for.
            revision: Arc::new(AtomicI64::new(unix_now_ms())),
        });
        self
    }

    /// Spawn `spec` and register it. The returned snapshot is taken before the
    /// supervisor can observe an exit, so its state is always `Running`.
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
        let log_err = log.try_clone().map_err(|source| CommandError::Log {
            path: log_path.clone(),
            source,
        })?;

        let mut cmd = Command::new("bash");
        cmd.arg("-lc")
            .arg(&spec.command)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(false);
        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        // Its own process group, so the deadline kill reaches everything the
        // command spawned rather than only the `bash -lc` wrapper.
        cmd.process_group(0);

        let child = cmd.spawn().map_err(CommandError::Spawn)?;
        let pid = child.id().unwrap_or_default();
        let started_at_unix_ms = unix_now_ms();
        let command = CommandRecord {
            id: id.clone(),
            command: spec.command.clone(),
            label: spec.label.unwrap_or_else(|| preview(&spec.command)),
            pid,
            started_at_unix_ms,
            timeout,
            deadline_unix_ms: started_at_unix_ms + timeout.as_millis() as i64,
            state: CommandState::Running,
            exit_code: None,
            log_path: log_path.clone(),
            finished_at_unix_ms: 0,
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

        tokio::spawn(supervise(
            CommandSinks {
                entries: Arc::clone(&self.entries),
                journal: self.journal.clone(),
                row_mirror: self.row_mirror.clone(),
                liveness: self.liveness.clone(),
            },
            id,
            child,
            pid,
            timeout,
            kill,
        ));

        info!(
            id = %command.id,
            pid,
            timeout_secs = timeout.as_secs(),
            command = %preview(&command.command),
            "Registered background command"
        );
        self.report_liveness().await;
        Ok(command)
    }

    /// Registry over a throwaway log dir, for fixtures that need one but do
    /// not exercise commands.
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self::new(
            Duration::from_mins(1),
            std::env::temp_dir().join(format!("moat-cmd-fixture-{}", short_id())),
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

    /// Read commands back on the agent's behalf, marking any that have already
    /// finished as reported: the agent has now seen the outcome, so the
    /// turn-end watch owes it no wake. `None` reads every command.
    ///
    /// Separate from `list` because the backend polls status on its own
    /// schedule; letting that poll mark a command seen would swallow the very
    /// wake this exists to guarantee.
    pub async fn read_for_agent(
        &self,
        id: Option<&str>,
    ) -> Result<Vec<CommandRecord>, CommandError> {
        let mut entries = self.entries.write().await;
        let mut out: Vec<CommandRecord> = match id {
            Some(id) => {
                let entry = entries
                    .get_mut(id)
                    .ok_or_else(|| CommandError::NotFound(id.to_string()))?;
                entry.reported |= entry.command.state.is_terminal();
                vec![entry.command.clone()]
            }
            None => entries
                .values_mut()
                .map(|e| {
                    e.reported |= e.command.state.is_terminal();
                    e.command.clone()
                })
                .collect(),
        };
        out.sort_by_key(|c| c.started_at_unix_ms);
        self.persist(&entries).await;
        Ok(out)
    }

    /// Signal a command to stop. The supervisor performs the kill and records
    /// the terminal state, so the snapshot returned here is still `Running`.
    ///
    /// The kill counts as reporting: the agent asked for this outcome, so
    /// waking it to announce the death it ordered is noise.
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

    /// Whether any registered command is still running. This is the whole of
    /// `busy_processes`: the supervisor clears a command's `Running` state at
    /// its deadline, so a hung command stops deferring the reaper on its own.
    pub async fn busy(&self) -> bool {
        self.entries
            .read()
            .await
            .values()
            .any(|e| !e.command.state.is_terminal())
    }

    /// The subset of `ids` that still owes the agent an outcome.
    ///
    /// A watch re-reads this on every poll rather than trusting the set it
    /// armed with, because an entry can be reported from under it: a later
    /// turn can kill or read back a command this watch is still waiting on. An
    /// unknown id owes nothing, so a dropped entry cannot hold a watch open.
    pub async fn unreported_among(&self, ids: &[String]) -> Vec<CommandRecord> {
        let entries = self.entries.read().await;
        ids.iter()
            .filter_map(|id| entries.get(id))
            .filter(|e| !e.reported)
            .map(|e| e.command.clone())
            .collect()
    }

    /// Commands that still owe the agent an outcome, whatever state they are in.
    ///
    /// This is the whole of what the turn-end watch waits on, and it keys off
    /// `reported` rather than the state for two reasons. A command can die
    /// between being spawned and the turn ending — a bad path takes under a
    /// second — so it is already terminal when the watch arms and a
    /// still-running filter would drop it. And `kill` marks it reported while
    /// the process is still winding down, so a state filter would pick the
    /// command back up and wake the agent about the death it just ordered.
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

    /// Mirror the unreported commands to whichever sinks are configured. Takes
    /// the live entries so the caller holds the lock across the write: a snapshot
    /// taken and written separately could land after a later, smaller set and
    /// resurrect a command the agent has already been told about.
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

/// Where a finished command's outcome is written: the live entries, plus the
/// mirrors that must move with them — the two a replacement pod reads the
/// unreported commands from, and the count the backend renders.
#[derive(Clone)]
struct CommandSinks {
    entries: Arc<RwLock<HashMap<String, Entry>>>,
    journal: Option<Journal>,
    row_mirror: Option<UnreportedMirror>,
    liveness: Option<LivenessReporter>,
}

/// Wait for the child, its deadline, or a kill request, then record the outcome.
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
    let (state, exit_code) = tokio::select! {
        status = child.wait() => match status {
            Ok(status) => (CommandState::Exited, status.code()),
            Err(error) => {
                warn!(id = %id, %error, "Failed to wait on command");
                (CommandState::Exited, None)
            }
        },
        () = tokio::time::sleep(timeout) => {
            warn!(id = %id, pid, timeout_secs = timeout.as_secs(), "Command hit its timeout; killing");
            terminate(pid, &mut child).await;
            (CommandState::TimedOut, Some(TIMEOUT_EXIT_CODE))
        }
        () = kill.cancelled() => {
            info!(id = %id, pid, "Command killed on request");
            terminate(pid, &mut child).await;
            (CommandState::Killed, None)
        }
    };

    let running = {
        let mut entries = entries.write().await;
        if let Some(entry) = entries.get_mut(&id) {
            entry.command.state = state;
            entry.command.exit_code = exit_code;
            entry.command.finished_at_unix_ms = unix_now_ms();
        }
        mirror(journal.as_ref(), row_mirror.as_ref(), &entries).await;
        running_count(&entries)
    };
    info!(id = %id, ?state, ?exit_code, "Background command finished");
    if let Some(liveness) = &liveness {
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

/// Signal every process in `pid`'s group. `pid` is the group leader because the
/// command was spawned with `process_group(0)`, and a negative pid is how `kill`
/// addresses a group. Shelled out because this crate denies `unsafe_code`, so
/// `killpg` is not available.
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

/// The end of a command's log, for telling the agent *why* it failed rather
/// than only that it did. Reads from the end: a build log is unbounded and only
/// its tail explains the outcome. Returns `None` when there is nothing to show.
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
    Path::new("/tmp/moat-commands").join(task_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `pid` is a live process. A killed orphan reparents to PID 1,
    /// which does not reap in this container, so it lingers as a zombie and any
    /// `kill -0` probe would report it alive forever.
    fn is_live(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        // The state letter follows the parenthesised comm, which may itself
        // contain spaces or parens.
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_some_and(|state| !matches!(state, "Z" | "X" | "x"))
    }

    fn registry(timeout: Duration) -> CommandRegistry {
        let dir = std::env::temp_dir().join(format!("moat-cmd-test-{}", short_id()));
        CommandRegistry::new(timeout, dir)
    }

    /// A registry whose liveness reports land in the returned backend, so a
    /// test can read the running counts the registry published.
    fn with_capture(
        timeout: Duration,
    ) -> (
        CommandRegistry,
        Arc<crate::events::test_backend::CapturingBackend>,
        CancellationToken,
    ) {
        use crate::backend_client::BackendClient;
        let backend = Arc::new(crate::events::test_backend::CapturingBackend::default());
        let cancel = CancellationToken::new();
        let (outbox, _drainer) = crate::outbox::EventOutbox::spawn(
            Arc::clone(&backend) as Arc<dyn BackendClient>,
            "task-1".to_string(),
            cancel.clone(),
        );
        let dir = std::env::temp_dir().join(format!("moat-cmd-live-{}", short_id()));
        let reg = CommandRegistry::new(timeout, dir)
            .with_liveness(GrpcEventPublisher::new(outbox), "task-1".to_string());
        (reg, backend, cancel)
    }

    /// The `runningCommands` counts the backend received, oldest first. Drains
    /// through the outbox, so it polls until at least `want` have landed.
    async fn published_counts(
        backend: &crate::events::test_backend::CapturingBackend,
        want: usize,
    ) -> Vec<i64> {
        for _ in 0..200 {
            let counts = counts_now(backend);
            if counts.len() >= want {
                return counts;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        counts_now(backend)
    }

    fn counts_now(backend: &crate::events::test_backend::CapturingBackend) -> Vec<i64> {
        backend
            .received
            .lock()
            .expect("lock")
            .iter()
            .filter(|r| r.sub_type == AgentSubType::CommandsChanged.as_str())
            .map(|r| {
                serde_json::from_str::<serde_json::Value>(&r.payload_json).expect("payload JSON")
                    ["runningCommands"]
                    .as_i64()
                    .expect("count")
            })
            .collect()
    }

    /// A registry whose journal is a throwaway file, plus that file's path.
    fn journalled(timeout: Duration) -> (CommandRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("moat-cmd-journal-{}", short_id()));
        std::fs::create_dir_all(&dir).expect("journal dir");
        let path = dir.join(".moat-commands.json");
        (
            CommandRegistry::new(timeout, dir).with_journal(path.clone()),
            path,
        )
    }

    async fn journal_now(path: &Path) -> Vec<JournaledCommand> {
        match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).expect("journal parses"),
            Err(_) => Vec::new(),
        }
    }

    async fn wait_terminal(reg: &CommandRegistry, id: &str) -> CommandRecord {
        for _ in 0..200 {
            let cmd = reg.get(id).await.expect("command registered");
            if cmd.state.is_terminal() {
                return cmd;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("command never reached a terminal state");
    }

    #[tokio::test]
    async fn publishes_the_running_count_when_a_command_starts_and_finishes() {
        let (reg, backend, cancel) = with_capture(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "true".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;

        // One at start (1 running) and one at exit (0), in that order.
        let counts = published_counts(&backend, 2).await;
        assert_eq!(counts, vec![1, 0], "got {counts:?}");
        cancel.cancel();
    }

    #[tokio::test]
    async fn a_fresh_registry_publishes_its_zero_so_a_stale_row_is_cleared() {
        let (reg, backend, cancel) = with_capture(Duration::from_mins(1));
        reg.report_initial_liveness().await;

        let counts = published_counts(&backend, 1).await;
        assert_eq!(counts, vec![0], "a booting pod owns no command");
        cancel.cancel();
    }

    #[tokio::test]
    async fn records_exit_code_and_log() {
        let reg = registry(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "echo hello; exit 7".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        assert_eq!(cmd.state, CommandState::Running);

        let done = wait_terminal(&reg, &cmd.id).await;
        assert_eq!(done.state, CommandState::Exited);
        assert_eq!(done.exit_code, Some(7));
        let log = std::fs::read_to_string(&done.log_path).expect("log written");
        assert!(log.contains("hello"), "log was {log:?}");
    }

    #[tokio::test]
    async fn busy_only_while_running() {
        let reg = registry(Duration::from_mins(1));
        assert!(!reg.busy().await, "empty registry is not busy");
        let cmd = reg
            .run(CommandSpec {
                command: "true".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;
        assert!(
            !reg.busy().await,
            "finished command must not defer the reaper"
        );
    }

    #[tokio::test]
    async fn timeout_kills_and_marks_timed_out() {
        let reg = registry(Duration::from_millis(200));
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 60".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");

        let done = wait_terminal(&reg, &cmd.id).await;
        assert_eq!(done.state, CommandState::TimedOut);
        assert_eq!(done.exit_code, Some(TIMEOUT_EXIT_CODE));
        assert!(
            !reg.busy().await,
            "a timed-out command stops deferring the reaper"
        );
    }

    #[tokio::test]
    async fn per_command_timeout_overrides_the_default() {
        // The default is long; the command's own value is what applies.
        let reg = registry(Duration::from_hours(1));
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 60".into(),
                cwd: None,
                timeout: Some(Duration::from_millis(200)),
                label: None,
            })
            .await
            .expect("spawn");
        let done = wait_terminal(&reg, &cmd.id).await;
        assert_eq!(done.state, CommandState::TimedOut);
    }

    #[tokio::test]
    async fn kill_marks_killed() {
        let reg = registry(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 60".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        reg.kill(&cmd.id).await.expect("kill");
        let done = wait_terminal(&reg, &cmd.id).await;
        assert_eq!(done.state, CommandState::Killed);
    }

    #[tokio::test]
    async fn kill_unknown_id_is_not_found() {
        let reg = registry(Duration::from_mins(1));
        assert!(matches!(
            reg.kill("nope").await,
            Err(CommandError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn an_unknown_id_owes_nothing() {
        let reg = registry(Duration::from_mins(1));
        assert!(reg.unreported_among(&["gone".to_string()]).await.is_empty());
    }

    #[tokio::test]
    async fn kill_reaches_the_whole_process_group() {
        let reg = registry(Duration::from_mins(1));
        // The wrapper exits immediately; the grandchild is what must die.
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 60 & echo $!; wait".into(),
                cwd: None,
                timeout: Some(Duration::from_millis(200)),
                label: None,
            })
            .await
            .expect("spawn");
        let done = wait_terminal(&reg, &cmd.id).await;
        let grandchild: u32 = std::fs::read_to_string(&done.log_path)
            .expect("log")
            .trim()
            .parse()
            .expect("pid line");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !is_live(grandchild),
            "grandchild {grandchild} survived the group kill"
        );
    }

    #[tokio::test]
    async fn a_finished_command_stays_unreported_until_the_agent_reads_it() {
        let reg = registry(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "exit 3".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;

        let owed = reg.unreported().await;
        assert_eq!(owed.len(), 1, "a finished command the agent never saw");

        reg.read_for_agent(None).await.expect("read");
        assert!(
            reg.unreported().await.is_empty(),
            "reading it back is the agent seeing it"
        );
    }

    /// Reading a *running* command tells the agent nothing about how it ends,
    /// so it must not discharge the wake owed when it later fails.
    #[tokio::test]
    async fn reading_a_running_command_does_not_discharge_its_wake() {
        let reg = registry(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 0.2; exit 4".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");

        reg.read_for_agent(None).await.expect("read while running");
        wait_terminal(&reg, &cmd.id).await;

        let owed = reg.unreported().await;
        assert_eq!(owed.len(), 1, "still owes the agent the outcome");
    }

    #[tokio::test]
    async fn killing_a_command_reports_it_without_a_wake() {
        let reg = registry(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 30".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        reg.kill(&cmd.id).await.expect("kill");
        wait_terminal(&reg, &cmd.id).await;

        assert!(
            reg.unreported().await.is_empty(),
            "the agent asked for this death; announcing it is noise"
        );
    }

    #[tokio::test]
    async fn a_command_runs_in_the_requested_cwd() {
        let reg = registry(Duration::from_mins(1));
        let dir = std::env::temp_dir().join(format!("moat-cmd-cwd-{}", short_id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let cmd = reg
            .run(CommandSpec {
                command: "pwd".into(),
                cwd: Some(dir.display().to_string()),
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");

        let done = wait_terminal(&reg, &cmd.id).await;
        let log = std::fs::read_to_string(&done.log_path).expect("log");
        assert!(
            log.trim()
                .ends_with(&dir.file_name().unwrap().to_string_lossy().to_string()),
            "ran in {log:?}, wanted {}",
            dir.display()
        );
    }

    #[tokio::test]
    async fn log_tail_reads_the_end_and_drops_the_partial_line() {
        let dir = std::env::temp_dir().join(format!("moat-cmd-tail-{}", short_id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("t.log");
        let body = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, &body).expect("write");

        let tail = log_tail(&path, 3, 4096).expect("tail");
        assert_eq!(tail, "line 497\nline 498\nline 499");

        // Seeking into the middle of the file must not emit a half line.
        let tail = log_tail(&path, 500, 64).expect("tail");
        assert!(
            tail.lines().all(|l| l.starts_with("line ")),
            "tail was {tail:?}"
        );
    }

    #[tokio::test]
    async fn log_tail_of_a_missing_or_empty_log_is_none() {
        let dir = std::env::temp_dir().join(format!("moat-cmd-tail-{}", short_id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let empty = dir.join("empty.log");
        std::fs::write(&empty, "").expect("write");

        assert!(log_tail(&dir.join("nope.log"), 5, 4096).is_none());
        assert!(log_tail(&empty, 5, 4096).is_none());
    }

    /// The mirror writes the backend has seen, oldest first. The send is detached
    /// from the entries lock, so it polls until at least `want` have landed.
    async fn mirror_writes(
        backend: &crate::events::test_backend::CapturingBackend,
        want: usize,
    ) -> Vec<(i64, Vec<JournaledCommand>)> {
        for _ in 0..200 {
            let writes = backend.unreported_commands.lock().expect("lock").clone();
            if writes.len() >= want {
                return writes;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        backend.unreported_commands.lock().expect("lock").clone()
    }

    /// The failure the row exists for: a replacement whose workspace restore
    /// fails has no journal, so the set must also reach the backend — with a
    /// revision, because the send happens off the entries lock.
    #[tokio::test]
    async fn the_unreported_set_is_mirrored_to_the_backend_with_a_rising_revision() {
        let backend = Arc::new(crate::events::test_backend::CapturingBackend::default());
        let dir = std::env::temp_dir().join(format!("moat-cmd-mirror-{}", short_id()));
        let start = unix_now_ms();
        let reg = CommandRegistry::new(Duration::from_mins(1), dir)
            .with_backend_mirror(Arc::clone(&backend) as Arc<dyn BackendClient>, "t1".into());

        let cmd = reg
            .run(CommandSpec {
                command: "exit 4".into(),
                cwd: None,
                timeout: None,
                label: Some("verify".into()),
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;

        // The spawn and the exit each mirror: running, then the outcome.
        let writes = mirror_writes(&backend, 2).await;
        assert!(writes.len() >= 2, "spawn and exit each write the mirror");
        // Seeded from the clock so it outruns a prior pod's stored revision,
        // never from 1.
        assert!(
            writes[0].0 >= start,
            "the first revision starts at the clock, not at zero"
        );
        assert_eq!(writes[0].1[0].state, CommandState::Running);
        let last = writes.last().expect("a write");
        assert!(last.0 > writes[0].0, "each write carries a higher revision");
        assert_eq!(last.1[0].id, cmd.id);
        assert_eq!(last.1[0].label, "verify");
        assert_eq!(last.1[0].state, CommandState::Exited);
        assert_eq!(last.1[0].exit_code, Some(4));

        // Reading it back marks it reported, so the row stops naming it.
        reg.read_for_agent(Some(&cmd.id)).await.expect("read back");
        let writes = mirror_writes(&backend, writes.len() + 1).await;
        assert!(
            writes.last().expect("a write").1.is_empty(),
            "a command the agent has been shown is owed nothing"
        );
    }

    /// The restart case: the pod dies with a command in flight. Its process and
    /// log go with it, so the journal is the only thing a replacement pod can
    /// read to know the agent is still owed an answer.
    #[tokio::test]
    async fn a_running_command_is_journalled_for_a_replacement_pod() {
        let (reg, path) = journalled(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 30".into(),
                cwd: None,
                timeout: None,
                label: Some("verify".into()),
            })
            .await
            .expect("spawn");

        let journal = journal_now(&path).await;
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].id, cmd.id);
        assert_eq!(journal[0].label, "verify");
        assert_eq!(journal[0].state, CommandState::Running);
    }

    /// The journal is installed by renaming a sibling temp file over it, so a
    /// crash mid-write never leaves a truncated file. The temp file must not
    /// survive a successful write — a leftover would ride the workspace snapshot
    /// as untracked cruft.
    #[tokio::test]
    async fn writing_the_journal_leaves_no_temp_file() {
        let (reg, path) = journalled(Duration::from_mins(1));
        reg.run(CommandSpec {
            command: "sleep 30".into(),
            cwd: None,
            timeout: None,
            label: None,
        })
        .await
        .expect("spawn");

        assert!(path.exists(), "the journal was written");
        assert!(
            !path.with_extension("json.tmp").exists(),
            "the staging file must be renamed away, not left behind"
        );
    }

    /// A finished command the agent never read still owes it an answer, so the
    /// journal keeps it — with the outcome, which outlives the log.
    #[tokio::test]
    async fn a_finished_but_unread_command_stays_journalled_with_its_outcome() {
        let (reg, path) = journalled(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "exit 4".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;

        let journal = journal_now(&path).await;
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].state, CommandState::Exited);
        assert_eq!(journal[0].exit_code, Some(4));
    }

    /// Reading the outcome back marks it reported, so a restart after that
    /// must not re-announce a command the agent has already seen.
    #[tokio::test]
    async fn reading_a_command_back_clears_it_from_the_journal() {
        let (reg, path) = journalled(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "true".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;
        assert_eq!(journal_now(&path).await.len(), 1);

        reg.read_for_agent(None).await.expect("agent reads it back");
        assert!(
            journal_now(&path).await.is_empty(),
            "a reported command must not survive into the next pod"
        );
    }

    /// `kill` marks the command reported at the request, while the process is
    /// still winding down. The journal keys off `reported`, so it drops the command
    /// then rather than when the supervisor records the death.
    #[tokio::test]
    async fn killing_a_command_clears_it_from_the_journal() {
        let (reg, path) = journalled(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "sleep 30".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        assert_eq!(journal_now(&path).await.len(), 1);

        reg.kill(&cmd.id).await.expect("kill");
        assert!(journal_now(&path).await.is_empty());
        wait_terminal(&reg, &cmd.id).await;
        assert!(journal_now(&path).await.is_empty());
    }

    #[tokio::test]
    async fn a_wake_clears_the_journal_so_a_restart_does_not_repeat_it() {
        let (reg, path) = journalled(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "true".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;

        reg.mark_reported(std::slice::from_ref(&cmd.id)).await;
        assert!(journal_now(&path).await.is_empty());
    }

    /// Reading the journal clears it, so the loss is announced once however
    /// many times the pod restarts afterwards.
    #[tokio::test]
    async fn take_journal_reads_once_and_clears_the_file() {
        let (reg, path) = journalled(Duration::from_mins(1));
        reg.run(CommandSpec {
            command: "sleep 30".into(),
            cwd: None,
            timeout: None,
            label: None,
        })
        .await
        .expect("spawn");

        assert_eq!(take_journal(&path).await.len(), 1);
        assert!(!path.exists(), "the journal must not be read twice");
        assert!(take_journal(&path).await.is_empty());
    }

    #[tokio::test]
    async fn take_journal_of_a_missing_or_corrupt_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("moat-cmd-journal-{}", short_id()));
        std::fs::create_dir_all(&dir).expect("dir");
        assert!(take_journal(&dir.join("absent.json")).await.is_empty());

        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, b"{ not json").expect("write");
        assert!(take_journal(&corrupt).await.is_empty());
        assert!(
            !corrupt.exists(),
            "a journal that cannot be parsed must not be retried on every start"
        );
    }

    /// A registry with no journal configured must behave exactly as before.
    #[tokio::test]
    async fn a_registry_without_a_journal_writes_nothing() {
        let reg = registry(Duration::from_mins(1));
        let cmd = reg
            .run(CommandSpec {
                command: "true".into(),
                cwd: None,
                timeout: None,
                label: None,
            })
            .await
            .expect("spawn");
        wait_terminal(&reg, &cmd.id).await;
        assert_eq!(reg.unreported().await.len(), 1);
    }
}
