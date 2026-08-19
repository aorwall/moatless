//! Running and editing a workspace Script.
//!
//! A Script is declared on the Workspace ([`ScriptConfig`]) and listed on the
//! T3 project by [`projection::scripts`](crate::projection). This module is the
//! other half: the two writes the listing implies. `scripts.run` resolves a
//! Script for the viewer, publishes its port if it declares one, then hosts it
//! in a named terminal session and runs the command as that session.
//! [`replace`] is the editing half — the `project.meta.update` command's
//! `scripts` field, written back to the Workspace the projection read it from.
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
use moatless_api_schemas::repository::ScriptConfig;
use moatless_backend_core::sandbox::{SandboxConnector, SandboxError, SandboxPortPublisher};
use moatless_types::access_control::ScopeContext;

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

/// Why a Scripts write could not be carried out.
///
/// Separate from [`ScriptsError`] because the two halves fail differently and
/// answer to different callers: running is an RPC that fails, editing is a
/// dispatched command that is *refused*, and the caller maps these onto the
/// contract's refusal shapes.
#[derive(Debug, thiserror::Error)]
pub enum ScriptsWriteError {
    /// The viewer cannot be shown this Workspace, or it does not exist. One
    /// answer, as everywhere else a scoped read finds nothing.
    #[error("workspace not found")]
    NotFound,
    /// The Workspace's run config is synced from git, so a write here would be
    /// undone by the next reconcile. Refused rather than silently lost.
    #[error("workspace is synced from Git")]
    GitSynced,
    /// The submitted list is not one this surface can store.
    #[error("{0}")]
    Invalid(String),
    #[error("the workspace could not be written")]
    Database(#[source] sqlx::Error),
}

impl From<sqlx::Error> for ScriptsWriteError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

/// Replace a Workspace's declared Scripts with the list the client submitted.
///
/// Wholesale, because that is how the client edits: adding, editing and
/// deleting one Script are all the same `project.meta.update` carrying the list
/// as it should now be. Reading the current list first is therefore not a
/// read-modify-write of the *list* — the client already resolved that — but of
/// each Script's fields the client never sees ([`carry_over_env`]).
pub async fn replace(
    workspaces: &dyn T3WorkspaceDao,
    viewer: &ScopeContext,
    workspace_id: &str,
    submitted: &Value,
) -> Result<(), ScriptsWriteError> {
    let target = workspaces
        .scripts_write_target(viewer, workspace_id)
        .await?
        .ok_or(ScriptsWriteError::NotFound)?;
    if target.is_git_synced() {
        return Err(ScriptsWriteError::GitSynced);
    }

    let next = script_configs(submitted, &target.scripts)?;
    if !workspaces.replace_scripts(viewer, workspace_id, &next).await? {
        // The row was there for the read and is not there for the write:
        // deleted, unshared or git-synced in between. Same answer as the read
        // finding nothing, because from here it is the same situation.
        return Err(ScriptsWriteError::NotFound);
    }
    Ok(())
}

/// The submitted wire Scripts as stored [`ScriptConfig`]s, folded onto what the
/// Workspace declares today.
fn script_configs(
    submitted: &Value,
    existing: &[ScriptConfig],
) -> Result<Vec<ScriptConfig>, ScriptsWriteError> {
    let submitted = submitted
        .as_array()
        .ok_or_else(|| ScriptsWriteError::Invalid("`scripts` must be a list.".to_string()))?;

    let mut next: Vec<ScriptConfig> = Vec::with_capacity(submitted.len());
    for script in submitted {
        let config = script_config(script, existing)?;
        // The name is the identity — it is what `scripts.run` resolves and what
        // has to survive a git round trip — so a list that names one twice has
        // no single answer for either.
        if next.iter().any(|kept| kept.name == config.name) {
            return Err(ScriptsWriteError::Invalid(format!(
                "Two scripts share the name `{}`.",
                config.name
            )));
        }
        next.push(config);
    }
    Ok(next)
}

/// One wire Script as a stored one.
fn script_config(
    script: &Value,
    existing: &[ScriptConfig],
) -> Result<ScriptConfig, ScriptsWriteError> {
    // `id` is the stored name: the projection reports the name as the id
    // precisely because a Script has no backend-allocated id to report.
    let name = required_field(script, "id")?;
    let command = required_field(script, "command")?;

    // The display name, kept only when it says something the name does not.
    // Writing it back unconditionally would give every Script a label equal to
    // its name, which reads as deliberate and is not.
    let label = script
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty() && *label != name)
        .map(str::to_string);

    let icon = script
        .get("icon")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|icon| projection::is_script_icon(icon))
        .map(str::to_string);

    Ok(ScriptConfig {
        env_vars: carry_over_env(&name, existing),
        port: port(script)?,
        name,
        label,
        command,
        icon,
    })
}

/// A trimmed, non-empty string field, or the refusal that says which one.
fn required_field(script: &Value, field: &str) -> Result<String, ScriptsWriteError> {
    script
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ScriptsWriteError::Invalid(format!("Every script needs a {field}.")))
}

/// The port a Script serves on, if it declares one.
///
/// Absent and null both mean "serves nothing", which is what a client clearing
/// the field sends. Anything outside a port's range is refused rather than
/// clamped: a mistyped port that silently became 65535 would publish a route to
/// a port nothing is listening on.
fn port(script: &Value) -> Result<Option<u16>, ScriptsWriteError> {
    let Some(port) = script.get("port") else {
        return Ok(None);
    };
    if port.is_null() {
        return Ok(None);
    }
    port.as_u64()
        .filter(|port| (1..=u64::from(u16::MAX)).contains(port))
        .map(|port| Some(port as u16))
        .ok_or_else(|| {
            ScriptsWriteError::Invalid(format!(
                "`{port}` is not a port. Use a number from 1 to 65535, or leave it empty."
            ))
        })
}

/// The environment a Script of this name already carried.
///
/// `envVars` is stored on a Script and is not on T3's wire — there is no field
/// for it in the contract and no control for it in the client. A write that
/// took the submitted Script at face value would therefore clear it every time
/// somebody renamed a Script's command, so it is carried across by name.
fn carry_over_env(
    name: &str,
    existing: &[ScriptConfig],
) -> Option<std::collections::HashMap<String, String>> {
    existing
        .iter()
        .find(|script| script.name == name)
        .and_then(|script| script.env_vars.clone())
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
