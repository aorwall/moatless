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
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Grace period between the SIGTERM and the SIGKILL of a timed-out command.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// Exit code reported for a command the sandbox killed at its deadline.
/// Matches coreutils `timeout` so a caller reading the log sees the usual value.
const TIMEOUT_EXIT_CODE: i32 = 124;

/// How much of the command string becomes the default label.
const LABEL_PREVIEW_CHARS: usize = 60;

/// Lifecycle of a registered command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub struct ManagedCommand {
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
    command: ManagedCommand,
    /// Cancels the supervisor's wait, which then kills the process group.
    kill: CancellationToken,
    /// The agent has been shown this command's terminal outcome — either it
    /// read the command back through `moat cmd`, or a wake named it. Until
    /// that is true a finished command still owes the agent an answer, even
    /// though it no longer defers the reaper.
    reported: bool,
}

/// Registry of the commands this task started through `moat cmd`.
pub struct CommandRegistry {
    entries: Arc<RwLock<HashMap<String, Entry>>>,
    default_timeout: Duration,
    log_dir: PathBuf,
}

impl CommandRegistry {
    pub fn new(default_timeout: Duration, log_dir: PathBuf) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            default_timeout,
            log_dir,
        }
    }

    /// Spawn `spec` and register it. The returned snapshot is taken before the
    /// supervisor can observe an exit, so its state is always `Running`.
    pub async fn run(&self, spec: CommandSpec) -> Result<ManagedCommand, CommandError> {
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
        let command = ManagedCommand {
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
        self.entries.write().await.insert(
            id.clone(),
            Entry {
                command: command.clone(),
                kill: kill.clone(),
                reported: false,
            },
        );

        tokio::spawn(supervise(
            Arc::clone(&self.entries),
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

    pub async fn list(&self) -> Vec<ManagedCommand> {
        let entries = self.entries.read().await;
        let mut out: Vec<ManagedCommand> = entries.values().map(|e| e.command.clone()).collect();
        out.sort_by_key(|c| c.started_at_unix_ms);
        out
    }

    pub async fn get(&self, id: &str) -> Option<ManagedCommand> {
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
    ) -> Result<Vec<ManagedCommand>, CommandError> {
        let mut entries = self.entries.write().await;
        let mut out: Vec<ManagedCommand> = match id {
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
        Ok(out)
    }

    /// Signal a command to stop. The supervisor performs the kill and records
    /// the terminal state, so the snapshot returned here is still `Running`.
    ///
    /// The kill counts as reporting: the agent asked for this outcome, so
    /// waking it to announce the death it ordered is noise.
    pub async fn kill(&self, id: &str) -> Result<ManagedCommand, CommandError> {
        let mut entries = self.entries.write().await;
        let entry = entries
            .get_mut(id)
            .ok_or_else(|| CommandError::NotFound(id.to_string()))?;
        entry.kill.cancel();
        entry.reported = true;
        Ok(entry.command.clone())
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

    /// Ids of the commands still running, for the turn-end watch to wait on.
    pub async fn running_ids(&self) -> Vec<String> {
        self.entries
            .read()
            .await
            .values()
            .filter(|e| !e.command.state.is_terminal())
            .map(|e| e.command.id.clone())
            .collect()
    }

    /// The subset of `ids` that still owes the agent an outcome.
    ///
    /// A watch re-reads this on every poll rather than trusting the set it
    /// armed with, because the debt can be discharged from under it: a later
    /// turn can kill or read back a command this watch is still waiting on. An
    /// unknown id owes nothing, so a dropped entry cannot hold a watch open.
    pub async fn unreported_among(&self, ids: &[String]) -> Vec<ManagedCommand> {
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
    /// the debt rather than the state for two reasons. A command can die
    /// between being spawned and the turn ending — a bad path takes under a
    /// second — so it is already terminal when the watch arms and a
    /// still-running filter would drop it. And `kill` discharges the debt while
    /// the process is still winding down, so a state filter would pick the
    /// command back up and wake the agent about the death it just ordered.
    pub async fn unreported(&self) -> Vec<ManagedCommand> {
        let entries = self.entries.read().await;
        let mut out: Vec<ManagedCommand> = entries
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
    }
}

/// Wait for the child, its deadline, or a kill request, then record the outcome.
async fn supervise(
    entries: Arc<RwLock<HashMap<String, Entry>>>,
    id: String,
    mut child: tokio::process::Child,
    pid: u32,
    timeout: Duration,
    kill: CancellationToken,
) {
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

    if let Some(entry) = entries.write().await.get_mut(&id) {
        entry.command.state = state;
        entry.command.exit_code = exit_code;
        entry.command.finished_at_unix_ms = unix_now_ms();
    }
    info!(id = %id, ?state, ?exit_code, "Background command finished");
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

    async fn wait_terminal(reg: &CommandRegistry, id: &str) -> ManagedCommand {
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
}
