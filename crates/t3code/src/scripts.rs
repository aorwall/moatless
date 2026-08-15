//! Running a workspace Script.
//!
//! A Script is declared on the Workspace ([`ScriptConfig`]) and listed on the
//! T3 project by [`projection::scripts`](crate::projection). This module is the
//! other half: the one write the listing implies. `scripts.run` resolves a
//! Script for the viewer, publishes its port if it declares one, then hosts it
//! in a named terminal session and runs the command as that session.
//!
//! The terminal is not new machinery — it is the same PTY surface
//! [`terminal`](crate::terminal) already exposes, addressed by the Task's
//! nominal worktree path so the sandbox resolves it to the real checkout. The
//! port, when there is one, is published *before* the PTY starts, so a Script
//! that binds instantly is never briefly unreachable.
//!
//! Two things make the session the Script's rather than a shell that happens to
//! be running it. The command is `exec`'d, so the session ends when the command
//! does and reports its exit status ([`exec_line`]). And the command is written
//! only when this call is what started the session — the sandbox, which owns the
//! session, reports that — so running a Script that is already running attaches
//! to it instead of typing into it, on any replica.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::dao::ConversationDao;
use crate::data::T3WorkspaceDao;
use crate::projection;
use crate::terminal::{self, TerminalError, TerminalHub};
use moatless_backend_core::sandbox::{SandboxConnector, SandboxError, SandboxPortPublisher};
use moatless_backend_core::scope::ScopeContext;

/// Everything `scripts.run` reaches that the terminal path does not already
/// hand it. The conversation, sandbox and terminal handles are the ones
/// [`terminal::open`] already takes, so they are passed through rather than held
/// here.
#[derive(Clone)]
pub struct ScriptsState {
    /// Reads a Task's Workspace scripts, scoped to the viewer.
    pub workspaces: Arc<dyn T3WorkspaceDao>,
    /// Publishes a Script's port on the running sandbox. A no-op that returns
    /// `None` where the runtime cannot publish ports (Docker), and refuses a
    /// reserved infrastructure port.
    pub ports: Arc<dyn SandboxPortPublisher>,
}

/// Why a `scripts.run` could not be carried out.
#[derive(Debug, thiserror::Error)]
pub enum ScriptsError {
    /// The request was missing a required field.
    #[error("scripts.run requires {0}")]
    Malformed(&'static str),
    /// The viewer cannot read this Task, or it does not exist — one answer, as
    /// everywhere else a scoped read is made.
    #[error("conversation not found")]
    NotFound,
    /// The Task is readable but declares no Script by that id.
    #[error("no script named {0}")]
    ScriptNotFound(String),
    /// The port could not be published. A reserved port arrives here as
    /// [`SandboxError::BadRequest`]; a stopped sandbox as
    /// [`SandboxError::NotReady`].
    #[error("the script's port could not be published")]
    Port(#[source] SandboxError),
    /// The terminal could not be opened or written to. `TerminalError` is not a
    /// `std::error::Error`, so it is not a `#[source]`; the dispatch delegates it
    /// to the terminal path's own failure rendering.
    #[error("the script's terminal could not be started")]
    Terminal(TerminalError),
    #[error("the task could not be read")]
    Database(#[source] sqlx::Error),
}

impl From<sqlx::Error> for ScriptsError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

fn required_str<'a>(payload: &'a Value, field: &'static str) -> Result<&'a str, ScriptsError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ScriptsError::Malformed(field))
}

/// Start a Script and return the terminal it runs in, plus its preview URL when
/// it serves a port.
pub async fn run(
    state: &ScriptsState,
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, ScriptsError> {
    let task_id = required_str(payload, "threadId")?;
    let script_id = required_str(payload, "scriptId")?;

    let scripts = state
        .workspaces
        .scripts_for_task(viewer, task_id)
        .await?
        .ok_or(ScriptsError::NotFound)?;
    let script = scripts
        .into_iter()
        .find(|script| script.name == script_id)
        .ok_or_else(|| ScriptsError::ScriptNotFound(script_id.to_string()))?;

    // The port first: it is a secondary product of the call (the console is the
    // primary one), so a route failure yields no URL rather than failing the
    // run — but a reserved port is refused here, before anything starts.
    let url = match script.port {
        Some(port) => state
            .ports
            .publish_port(task_id, &script.name, port)
            .await
            .map_err(ScriptsError::Port)?,
        None => None,
    };

    // A stable, self-describing id so the session is recognisable in the
    // terminal list, and so a re-run addresses the same one — which is what
    // makes the new-process check below able to tell a re-run from a first run.
    let terminal_id = format!("script-{}", script.name);
    // The nominal worktree path the sandbox maps back to the real checkout.
    let cwd = projection::nominal_worktree(task_id);

    let mut open_payload = json!({
        "threadId": task_id,
        "terminalId": terminal_id,
        "cwd": cwd,
    });
    if let Some(env) = &script.env_vars {
        // Only when present: the terminal helper treats a null `env` as
        // malformed, so an absent field must stay absent.
        open_payload["env"] = json!(env);
    }
    let (_, is_new_process) =
        terminal::open_reporting_new_process(conversations, sandboxes, hub, viewer, &open_payload)
            .await
            .map_err(ScriptsError::Terminal)?;

    // Only a session this call started gets the command. A session already
    // under this id is this same script still running: `terminal.open` reused
    // its shell, and writing again would type the command into the running
    // process's stdin rather than start a second run. Running twice is the
    // client asking to watch the one that is running, so return it as-is.
    if is_new_process {
        let write_payload = json!({
            "threadId": task_id,
            "terminalId": terminal_id,
            "data": exec_line(&script.command),
        });
        terminal::write(conversations, sandboxes, viewer, &write_payload)
            .await
            .map_err(ScriptsError::Terminal)?;
    }

    Ok(json!({ "terminalId": terminal_id, "url": url }))
}

/// The line written into a freshly opened shell to run `command` *as* that
/// shell, rather than under it.
///
/// `exec` replaces the shell process with the command, which is what makes the
/// session's lifetime the command's lifetime: the PTY closes when the command
/// exits, so the terminal reports `exited` with the command's status instead of
/// sitting `running` on a shell prompt that outlived a failed test run.
///
/// The command goes to `sh -c` rather than after `exec` directly so a pipeline,
/// `&&`, or a redirect still means what it says — `exec` takes a simple command,
/// and appending a compound one would exec only its first word. Single quotes in
/// the command are escaped for the single-quoted argument they land in.
fn exec_line(command: &str) -> String {
    format!("exec sh -c '{}'\r", command.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_command_runs_as_the_session_itself() {
        // `exec` is the whole point: without it the shell outlives the command
        // and the terminal never reports the command's exit.
        assert_eq!(exec_line("pnpm dev"), "exec sh -c 'pnpm dev'\r");
    }

    #[test]
    fn a_compound_command_keeps_its_shell_meaning() {
        // Appending this to a bare `exec` would exec `pnpm` and drop the rest.
        assert_eq!(
            exec_line("pnpm build && pnpm start"),
            "exec sh -c 'pnpm build && pnpm start'\r"
        );
    }

    #[test]
    fn a_single_quote_in_the_command_does_not_end_the_argument() {
        // The classic injection shape: an unescaped quote would close `sh -c`'s
        // argument and leave the rest of the line as shell words of its own.
        assert_eq!(
            exec_line("echo 'hi'"),
            r"exec sh -c 'echo '\''hi'\'''".to_string() + "\r"
        );
    }

    #[test]
    fn every_line_ends_with_the_carriage_return_that_runs_it() {
        assert!(exec_line("true").ends_with('\r'));
    }
}
