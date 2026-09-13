//! Running and editing a workspace Script.
//!
//! A Script is declared on the Workspace ([`ScriptConfig`]) and listed on the
//! T3 project by [`projection::scripts`](crate::projection). `run` resolves a
//! Script, publishes its port if declared, then execs the command in a named
//! terminal session; running one already running attaches instead of starting a
//! second. [`replace`] writes the `project.meta.update` command's `scripts`
//! field back to the Workspace.

use std::sync::Arc;

use serde_json::{Value, json};
use tracing::warn;

use crate::dao::ConversationDao;
use crate::nominal_path;
use crate::preview::{self, PreviewState};
use crate::projection;
use crate::terminal::{self, TerminalError, TerminalHub};
use crate::workspace_dao::T3WorkspaceDao;
use moatless_api_schemas::repository::ScriptConfig;
use moatless_ports::sandbox::{SandboxError, SandboxPortPublisher};
use moatless_types::access_control::ScopeContext;
use sandbox_proto::client::SandboxConnector;

/// Everything `scripts.run` reaches that the terminal path does not already
/// hand it; conversation, sandbox and terminal handles are passed through from
/// [`terminal::open`] rather than held here.
#[derive(Clone)]
pub(crate) struct ScriptsState {
    /// Reads a Task's Workspace scripts, scoped to the viewer.
    pub(crate) workspaces: Arc<dyn T3WorkspaceDao>,
    /// Publishes a Script's port on the running sandbox. A no-op that returns
    /// `None` where the runtime cannot publish ports (Docker), and refuses a
    /// reserved infrastructure port.
    pub ports: Arc<dyn SandboxPortPublisher>,
    /// Opens the browser tab a Script with a port serves into.
    pub preview: PreviewState,
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
    /// `std::error::Error`, so it is not a `#[source]`.
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

/// The Scripts a Task's Workspace declares — the same read [`run`] resolves a
/// Script id against, for a caller that only wants to enumerate them (`moat
/// tasks scripts list`, over HTTP rather than the RPC/WS surface).
pub(crate) async fn list(
    state: &ScriptsState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Vec<ScriptConfig>, ScriptsError> {
    state
        .workspaces
        .scripts_for_task(viewer, task_id)
        .await?
        .ok_or(ScriptsError::NotFound)
}

/// A Script as `moat tasks scripts list` reports it.
///
/// Omits `env_vars`: a task-scoped bearer key reads this route without the
/// workspace-level access a pinned secret would assume.
pub fn public_script_list(scripts: &[ScriptConfig]) -> Vec<Value> {
    scripts
        .iter()
        .map(|script| {
            json!({
                "name": script.name,
                "label": script.label,
                "command": script.command,
                "icon": script.icon,
                "port": script.port,
                "runOnStart": script.run_on_start,
            })
        })
        .collect()
}

/// Start a Script and return the terminal it runs in, plus — when it serves a
/// port — its preview URL and the browser tab now pointing at it.
pub(crate) async fn run(
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
    let cwd = nominal_path::nominal_worktree(task_id);

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

    // Last, and never fatal: the Script is running by this point, so a preview
    // store that cannot be reached costs the automatic tab and nothing else.
    // The tab is addressed by the same id as the terminal, which is what makes
    // a re-run reuse the tab it opened — see [`preview::open_at`].
    let preview_tab_id = match &url {
        Some(url) => {
            match preview::open_at(&state.preview, viewer, task_id, &terminal_id, url).await {
                Ok(_) => Some(terminal_id.clone()),
                Err(error) => {
                    warn!(%error, script = %script.name, "A script's preview tab was not opened");
                    None
                }
            }
        }
        None => None,
    };

    Ok(json!({
        "terminalId": terminal_id,
        "url": url,
        "previewTabId": preview_tab_id,
    }))
}

/// The line written into a freshly opened shell to run `command` as that shell,
/// not under it.
///
/// `exec` replaces the shell process so the PTY closes with the command's own
/// exit status; runs under `sh -c`, with single quotes escaped.
fn exec_line(command: &str) -> String {
    format!(
        "{SETUP_GATE}exec sh -c '{}'\r",
        command.replace('\'', r"'\''")
    )
}

/// Hold the Script until the sandbox's workspace setup has finished.
///
/// A Sandbox reports Ready before Phase 2 runs setup, so this waits on the same
/// `.workspace-ready` sentinel preview servers use.
const SETUP_GATE: &str = concat!(
    r#"if [ ! -f "${DATA_DIR:-/opt/moatless/data}/.workspace-ready" ]; then "#,
    r#"echo "Waiting for workspace setup to finish..."; "#,
    r#"until [ -f "${DATA_DIR:-/opt/moatless/data}/.workspace-ready" ]; do sleep 2; done; "#,
    "fi; ",
);

/// Why a Scripts write could not be carried out.
///
/// Separate from [`ScriptsError`]: running is an RPC that fails, editing is a
/// dispatched command that is *refused*.
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

/// Decode the client list and dispatch its replacement to the Workspace owner.
pub async fn replace(
    commands: &dyn crate::ports::WorkspaceScriptCommands,
    viewer: &ScopeContext,
    workspace_id: &str,
    submitted: &Value,
) -> Result<(), ScriptsWriteError> {
    let submitted = submitted
        .as_array()
        .ok_or_else(|| ScriptsWriteError::Invalid("`scripts` must be a list.".to_string()))?;
    let next = submitted
        .iter()
        .map(script_config)
        .collect::<Result<Vec<_>, _>>()?;
    commands.replace_scripts(viewer, workspace_id, &next).await
}

/// One wire Script as a stored one.
fn script_config(script: &Value) -> Result<ScriptConfig, ScriptsWriteError> {
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

    // Absent, null and non-bool all mean "run only on request": the flag is the
    // client's `runOnWorktreeCreate`, and a client that does not know about it
    // must not silently start turning it off.
    let run_on_start = script
        .get("runOnWorktreeCreate")
        .and_then(Value::as_bool)
        .unwrap_or_default();

    Ok(ScriptConfig {
        env_vars: None,
        port: port(script)?,
        name,
        label,
        command,
        icon,
        run_on_start,
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
/// Absent and null both mean "serves nothing". Out-of-range is refused, not
/// clamped, so a mistyped port never silently becomes 65535.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What `exec_line` appends after the setup gate — the assertions below are
    /// about the command half, which the gate prefixes without changing.
    fn command_half(command: &str) -> String {
        exec_line(command)
            .strip_prefix(SETUP_GATE)
            .expect("the setup gate precedes the command")
            .to_string()
    }

    #[test]
    fn a_plain_command_runs_as_the_session_itself() {
        // `exec` is the whole point: without it the shell outlives the command
        // and the terminal never reports the command's exit.
        assert_eq!(command_half("pnpm dev"), "exec sh -c 'pnpm dev'\r");
    }

    #[test]
    fn a_compound_command_keeps_its_shell_meaning() {
        // Appending this to a bare `exec` would exec `pnpm` and drop the rest.
        assert_eq!(
            command_half("pnpm build && pnpm start"),
            "exec sh -c 'pnpm build && pnpm start'\r"
        );
    }

    #[test]
    fn a_single_quote_in_the_command_does_not_end_the_argument() {
        // The classic injection shape: an unescaped quote would close `sh -c`'s
        // argument and leave the rest of the line as shell words of its own.
        assert_eq!(
            command_half("echo 'hi'"),
            r"exec sh -c 'echo '\''hi'\'''".to_string() + "\r"
        );
    }

    /// The gate keeps a Script from starting before `bun install` has run on a
    /// Sandbox that reported Ready, and must come *before* the `exec` — after
    /// it, the shell is already gone.
    #[test]
    fn the_command_waits_for_workspace_setup_before_it_execs() {
        let line = exec_line("bun run dev");

        let gate = line
            .find(".workspace-ready")
            .expect("waits on the sentinel");
        let exec = line.find("exec sh -c").expect("still execs the command");
        assert!(gate < exec, "the gate must precede the exec: {line}");
        assert!(
            line.contains("until [ -f"),
            "the gate must actually block: {line}"
        );
    }

    /// A warm Sandbox has written the sentinel already, so the gate must be a
    /// test rather than an unconditional sleep — otherwise every run of every
    /// Script pays for a race that is not happening.
    #[test]
    fn the_gate_costs_nothing_once_setup_has_finished() {
        assert!(
            SETUP_GATE.starts_with(r#"if [ ! -f "#),
            "the wait is guarded by a file test: {SETUP_GATE}"
        );
    }

    #[test]
    fn every_line_ends_with_the_carriage_return_that_runs_it() {
        assert!(exec_line("true").ends_with('\r'));
    }

    /// The flag the client calls `runOnWorktreeCreate` is stored as
    /// `run_on_start`, and an edit that carries it back must not lose it.
    #[test]
    fn a_flagged_wire_script_is_stored_as_run_on_start() {
        let stored = script_config(&json!({
            "id": "dev",
            "command": "bun run dev",
            "runOnWorktreeCreate": true
        }))
        .expect("a script with an id and a command decodes");

        assert!(stored.run_on_start);
    }

    /// Absent, null and a non-bool are one answer, so a malformed flag cannot
    /// make a Script start by itself.
    #[test]
    fn an_absent_or_unreadable_flag_means_run_on_request() {
        for script in [
            json!({"id": "dev", "command": "bun run dev"}),
            json!({"id": "dev", "command": "bun run dev", "runOnWorktreeCreate": null}),
            json!({"id": "dev", "command": "bun run dev", "runOnWorktreeCreate": "yes"}),
        ] {
            assert!(
                !script_config(&script)
                    .expect("a script with an id and a command decodes")
                    .run_on_start,
                "{script}"
            );
        }
    }
}
