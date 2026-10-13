//! `moat cmd` — sandbox-only command for running work that outlives one Bash
//! tool call.
//!
//! The sandbox owns the child, so it reaps it, enforces the deadline, and
//! reports it in `busy_processes`. That is what defers the reaper's idle stop
//! while a build runs and what wakes the agent when it finishes — a process
//! started any other way is invisible to both.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox_proto::sandbox_service_client::SandboxServiceClient;
use sandbox_proto::{
    KillCommandRequest, ListCommandsRequest, ManagedCommand, ManagedCommandState, RunCommandRequest,
};

use crate::commands::CommandExit;

/// Lines of the log tail printed by `status` and `logs` when no `--lines` is
/// given.
const DEFAULT_LOG_LINES: usize = 50;

/// How long `run` waits before checking that the command is still alive, so an
/// instant failure is reported here rather than only at the next wake.
const EARLY_EXIT_GRACE: Duration = Duration::from_secs(1);

/// How often `wait` re-reads the command's state.
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(clap::Subcommand)]
pub enum CmdAction {
    /// Start a long-running command and return immediately.
    Run {
        /// Shell command, run through `bash -lc`.
        command: String,
        /// Working directory, absolute or relative to this shell. Defaults to
        /// this shell's directory, so the command means what it would have
        /// meant typed here.
        #[arg(long, short = 'C')]
        cwd: Option<String>,
        /// Seconds before the sandbox kills it and marks it timed out. Defaults
        /// to 4h. Raise it for a longer build; lower it so a hung command
        /// cannot hold the sandbox open for hours.
        #[arg(long, short = 't')]
        timeout: Option<u64>,
        /// Human label for status output. Defaults to the command itself.
        #[arg(long, short = 'l')]
        label: Option<String>,
        /// Start it only after every earlier `--queue` command of this task has
        /// finished, one at a time, whatever their exit codes. Use it for
        /// builds that share a target directory. Without it the command starts
        /// at once.
        #[arg(long, short = 'q')]
        queue: bool,
    },

    /// List every command the sandbox is still holding.
    List,

    /// Show one command, with a tail of its log.
    ///
    /// The tail is the command's own output, so do not grep this for a state:
    /// `cargo test` prints `running N tests`. Use `wait` or `--json` instead.
    Status {
        id: String,
        /// Lines of log to show. 0 shows none.
        #[arg(long, short = 'n', default_value_t = DEFAULT_LOG_LINES)]
        lines: usize,
    },

    /// Block until a command finishes, then exit with its exit code.
    ///
    /// For work that only needs to run after another command, `run --queue`
    /// needs no waiting process. Counts as reading the outcome back, as
    /// `status` does.
    Wait { id: String },

    /// Print a command's combined stdout/stderr.
    Logs {
        id: String,
        /// Lines to show from the end. 0 prints the whole log.
        #[arg(long, short = 'n', default_value_t = DEFAULT_LOG_LINES)]
        lines: usize,
    },

    /// Kill a running command.
    Kill { id: String },
}

/// `grpc_port` is [`Config::sandbox_grpc_port`](crate::config::Config::sandbox_grpc_port).
pub async fn run(
    action: CmdAction,
    task_id: String,
    json: bool,
    grpc_port: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = crate::commands::local_sandbox::connect(grpc_port).await?;

    match action {
        CmdAction::Run {
            command,
            cwd,
            timeout,
            label,
            queue,
        } => {
            let cwd = resolve_cwd(cwd, std::env::current_dir().ok());
            let resp = client
                .run_command(RunCommandRequest {
                    command,
                    cwd,
                    timeout_secs: timeout,
                    label,
                    task_id: Some(task_id.clone()),
                    queue,
                })
                .await?
                .into_inner();
            let started = resp.command.ok_or("sandbox returned no command")?;

            // A command that dies on the first line — a bad path, a typo — is
            // gone before this returns, and reporting it as started would send
            // the caller off to wait for a wake about work that never ran.
            tokio::time::sleep(EARLY_EXIT_GRACE).await;
            let id = started.id.clone();
            let cmd = fetch_one(&mut client, &id, task_id)
                .await
                .unwrap_or(started);
            let died_immediately = !is_pending(cmd.state()) && cmd.exit_code != Some(0);

            if json {
                print_json(&[&cmd]);
            } else if cmd.state() == ManagedCommandState::Running {
                println!("Started {} (pid {})", cmd.id, cmd.pid);
                println!("  log:      {}", cmd.log_path);
                println!("  deadline: {}s", cmd.timeout_secs);
                println!(
                    "The sandbox stays awake until this finishes; you will be woken when it does. \
                     End your turn — do not poll it or schedule a fallback wakeup."
                );
                println!(
                    "To run work after it, start that when the wake arrives, or start both \
                     with --queue."
                );
            } else if cmd.state() == ManagedCommandState::Queued {
                println!("Queued {} behind the running --queue command", cmd.id);
                println!("  log:      {}", cmd.log_path);
                println!(
                    "  deadline: {}s, counted from when it starts",
                    cmd.timeout_secs
                );
                println!(
                    "It starts by itself; you will be woken when it finishes. \
                     End your turn — do not poll it or schedule a fallback wakeup."
                );
            } else {
                println!("{}", summary_line(&cmd));
                println!("  log:     {}", cmd.log_path);
                print_tail(&cmd.log_path, DEFAULT_LOG_LINES);
            }
            if died_immediately {
                return Err(format!("command {} did not stay running", cmd.id).into());
            }
        }

        CmdAction::List => {
            let resp = client
                .list_commands(ListCommandsRequest {
                    id: None,
                    task_id: Some(task_id),
                })
                .await?
                .into_inner();
            if json {
                print_json(&resp.commands.iter().collect::<Vec<_>>());
            } else if resp.commands.is_empty() {
                println!("No commands registered.");
            } else {
                for cmd in &resp.commands {
                    println!("{}", summary_line(cmd));
                }
            }
        }

        CmdAction::Status { id, lines } => {
            let cmd = fetch_one(&mut client, &id, task_id).await?;
            if json {
                print_json(&[&cmd]);
            } else {
                println!("{}", summary_line(&cmd));
                println!("  command: {}", cmd.command);
                println!("  log:     {}", cmd.log_path);
                if lines > 0 {
                    print_tail(&cmd.log_path, lines);
                }
            }
        }

        CmdAction::Wait { id } => {
            let cmd = loop {
                let cmd = fetch_one(&mut client, &id, task_id.clone()).await?;
                if !is_pending(cmd.state()) {
                    break cmd;
                }
                tokio::time::sleep(WAIT_POLL_INTERVAL).await;
            };
            if json {
                print_json(&[&cmd]);
            } else {
                println!("{}", summary_line(&cmd));
            }
            wait_outcome(&cmd)?;
        }

        CmdAction::Logs { id, lines } => {
            let cmd = fetch_one(&mut client, &id, task_id).await?;
            print_tail(&cmd.log_path, lines);
        }

        CmdAction::Kill { id } => {
            let resp = client
                .kill_command(KillCommandRequest {
                    id,
                    task_id: Some(task_id),
                })
                .await?
                .into_inner();
            let cmd = resp.command.ok_or("sandbox returned no command")?;
            if json {
                print_json(&[&cmd]);
            } else {
                println!("Killed {} ({})", cmd.id, cmd.label);
            }
        }
    }

    Ok(())
}

async fn fetch_one(
    client: &mut SandboxServiceClient<tonic::transport::Channel>,
    id: &str,
    task_id: String,
) -> Result<ManagedCommand, Box<dyn std::error::Error>> {
    let resp = client
        .list_commands(ListCommandsRequest {
            id: Some(id.to_string()),
            task_id: Some(task_id),
        })
        .await?
        .into_inner();
    resp.commands
        .into_iter()
        .next()
        .ok_or_else(|| format!("no command with id {id}").into())
}

/// Whether a command has yet to finish: running, or queued behind another.
fn is_pending(state: ManagedCommandState) -> bool {
    matches!(
        state,
        ManagedCommandState::Running | ManagedCommandState::Queued
    )
}

/// `wait`'s exit: the finished command's own code, and 1 for a command that
/// left none, such as one killed by a signal.
fn wait_outcome(cmd: &ManagedCommand) -> Result<(), CommandExit> {
    match cmd.exit_code {
        Some(0) => Ok(()),
        code => Err(CommandExit(code.unwrap_or(1))),
    }
}

fn summary_line(cmd: &ManagedCommand) -> String {
    let state = match cmd.state() {
        ManagedCommandState::Queued => "queued".to_string(),
        ManagedCommandState::Running => {
            let left = cmd.deadline_unix_ms - now_unix_ms();
            format!("running ({}s of deadline left)", left.max(0) / 1000)
        }
        ManagedCommandState::Exited => match cmd.exit_code {
            Some(0) => "succeeded".to_string(),
            Some(code) => format!("failed (exit {code})"),
            None => "exited".to_string(),
        },
        ManagedCommandState::TimedOut => format!("timed out after {}s", cmd.timeout_secs),
        ManagedCommandState::Killed => "killed".to_string(),
        ManagedCommandState::Unspecified => "unknown".to_string(),
    };
    format!("{}  {}  {}", cmd.id, state, cmd.label)
}

/// Print the last `lines` of `path`, or the whole file when `lines` is 0.
fn print_tail(path: &str, lines: usize) {
    match std::fs::read_to_string(path) {
        Ok(text) if lines == 0 => print!("{text}"),
        Ok(text) => {
            let all: Vec<&str> = text.lines().collect();
            let start = all.len().saturating_sub(lines);
            for line in &all[start..] {
                println!("{line}");
            }
        }
        Err(e) => eprintln!("cannot read {path}: {e}"),
    }
}

fn print_json(commands: &[&ManagedCommand]) {
    let rows: Vec<serde_json::Value> = commands
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "command": c.command,
                "label": c.label,
                "pid": c.pid,
                "state": c.state().as_str_name(),
                "exitCode": c.exit_code,
                "timeoutSecs": c.timeout_secs,
                "startedAtUnixMs": c.started_at_unix_ms,
                "deadlineUnixMs": c.deadline_unix_ms,
                "finishedAtUnixMs": c.finished_at_unix_ms,
                "logPath": c.log_path,
            })
        })
        .collect();
    crate::output::print_json_typed(&rows);
}

/// Resolve `--cwd` against `base`, the directory this CLI was run from.
///
/// The sandbox spawns the command from its own process, so a relative path
/// would otherwise resolve against *that* directory instead of the caller's.
/// Absolute values pass through; an absent one means the caller's directory.
fn resolve_cwd(cwd: Option<String>, base: Option<PathBuf>) -> Option<String> {
    match cwd {
        None => base.map(|p| p.display().to_string()),
        Some(cwd) if Path::new(&cwd).is_absolute() => Some(cwd),
        Some(cwd) => match base {
            Some(base) => Some(base.join(cwd).display().to_string()),
            None => Some(cwd),
        },
    }
}

fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(state: ManagedCommandState, exit_code: Option<i32>) -> ManagedCommand {
        let mut cmd = ManagedCommand {
            exit_code,
            ..Default::default()
        };
        cmd.set_state(state);
        cmd
    }

    /// `wait` must keep waiting through a queued command's wait for the queue.
    #[test]
    fn a_queued_command_is_still_pending() {
        assert!(is_pending(ManagedCommandState::Queued));
        assert!(is_pending(ManagedCommandState::Running));
        assert!(!is_pending(ManagedCommandState::Exited));
    }

    #[test]
    fn wait_exits_with_the_finished_commands_code() {
        assert!(wait_outcome(&finished(ManagedCommandState::Exited, Some(0))).is_ok());
        let failed = wait_outcome(&finished(ManagedCommandState::Exited, Some(101)));
        assert_eq!(failed.map_err(|e| e.0), Err(101));
        let timed_out = wait_outcome(&finished(ManagedCommandState::TimedOut, Some(124)));
        assert_eq!(timed_out.map_err(|e| e.0), Err(124));
    }

    /// A `&&` after `wait` must not run behind a killed command.
    #[test]
    fn wait_fails_for_a_command_that_left_no_exit_code() {
        let killed = wait_outcome(&finished(ManagedCommandState::Killed, None));
        assert_eq!(killed.map_err(|e| e.0), Err(1));
    }

    #[test]
    fn an_absent_cwd_is_the_callers_directory() {
        assert_eq!(
            resolve_cwd(None, Some(PathBuf::from("/ws/repo"))),
            Some("/ws/repo".to_string())
        );
    }

    /// The sandbox spawns from its own directory, so a relative value has to be
    /// made absolute here or it lands somewhere the caller never chose.
    #[test]
    fn a_relative_cwd_resolves_against_the_callers_directory() {
        assert_eq!(
            resolve_cwd(
                Some("apps/frontend".into()),
                Some(PathBuf::from("/ws/repo"))
            ),
            Some("/ws/repo/apps/frontend".to_string())
        );
    }

    #[test]
    fn an_absolute_cwd_is_left_alone() {
        assert_eq!(
            resolve_cwd(Some("/other/place".into()), Some(PathBuf::from("/ws/repo"))),
            Some("/other/place".to_string())
        );
    }

    /// Without a readable current directory there is nothing to resolve
    /// against; the sandbox still falls back to the workspace dir.
    #[test]
    fn an_unreadable_current_directory_passes_the_value_through() {
        assert_eq!(
            resolve_cwd(Some("apps".into()), None),
            Some("apps".to_string())
        );
        assert_eq!(resolve_cwd(None, None), None);
    }
}
