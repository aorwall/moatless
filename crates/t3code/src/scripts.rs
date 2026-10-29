//! Running and editing a Script.
//!
//! A Script is declared either on the Workspace ([`ScriptConfig`], listed on
//! the T3 project by [`projection::scripts`](crate::projection)) or on one Task
//! by [`declare`], which stores it in that Task's sandbox. [`resolve`] merges
//! the two. `run` resolves a Script, publishes its port if declared, then execs
//! the command in a named terminal session; running one already running
//! attaches instead of starting a second. [`replace`] writes the
//! `project.meta.update` command's `scripts` field back to the Workspace.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::dao::ConversationDao;
use crate::nominal_path;
use crate::ports::WorkspaceScriptSource;
use crate::preview::{self, PreviewState};
use crate::projection;
use crate::terminal::{self, TerminalError, TerminalHub};
use moatless_ports::sandbox::{SandboxError, SandboxPortPublisher};
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use moatless_workspace_types::schemas::repository::ScriptConfig;
use moatless_workspaces::workspace::scripts::{ResolvedScript, ScriptOrigin, merge};
use sandbox_proto::TaskScript;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};

/// Everything `scripts.run` reaches that the terminal path does not already
/// hand it; conversation, sandbox and terminal handles are passed through from
/// [`terminal::open`] rather than held here.
#[derive(Clone)]
pub(crate) struct ScriptsState {
    /// Reads a Task's Workspace scripts, scoped to the viewer.
    pub(crate) workspace_scripts: Arc<dyn WorkspaceScriptSource>,
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
    /// The Task's own Scripts could not be read or written. A stopped sandbox
    /// arrives here as [`SandboxError::NotReady`], a rejected name as
    /// [`SandboxError::BadRequest`].
    #[error("the task's own scripts could not be reached")]
    Sandbox(#[source] SandboxError),
    /// A submitted field is not one this surface can store. Carries the whole
    /// refusal, which names the field and what it would accept.
    #[error("{0}")]
    Invalid(String),
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
    Database(#[source] StoreError),
}

impl From<StoreError> for ScriptsError {
    fn from(value: StoreError) -> Self {
        Self::Database(value)
    }
}

fn required_str<'a>(payload: &'a Value, field: &'static str) -> Result<&'a str, ScriptsError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map_or(Err(ScriptsError::Malformed(field)), |value| {
            nonblank(value, field)
        })
}

/// `value` trimmed, or [`ScriptsError::Malformed`] naming `field` when that
/// leaves nothing.
fn nonblank<'a>(value: &'a str, field: &'static str) -> Result<&'a str, ScriptsError> {
    Some(value.trim())
        .filter(|value| !value.is_empty())
        .ok_or(ScriptsError::Malformed(field))
}

/// The contract's `ScriptsRunInput`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScriptsRunInput {
    pub(crate) thread_id: String,
    pub(crate) script_id: String,
}

/// Where `script` was declared, under the name both wire surfaces give it.
pub fn scope(script: &ResolvedScript) -> &'static str {
    match script.declared_by {
        ScriptOrigin::Task => "task",
        ScriptOrigin::Workspace => "workspace",
    }
}

/// Every Script a Task can run, and whether its own were readable.
pub(crate) struct Resolved {
    pub scripts: Vec<ResolvedScript>,
    /// Why the Task's own Scripts were left out, when they were. A name absent
    /// from `scripts` may still be declared, so a caller resolving one reports
    /// this rather than "no such script".
    pub own_unread: Option<SandboxError>,
}

/// Every Script this Task can run: its Workspace's, with its own merged over
/// them, a Task Script shadowing a Workspace Script of the same name.
///
/// The Workspace read is the authorization gate — `None` from it is
/// [`ScriptsError::NotFound`]. A sandbox that cannot be read costs the Task's
/// own Scripts and not the Workspace's, reported on [`Resolved::own_unread`].
pub(crate) async fn resolve(
    state: &ScriptsState,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Resolved, ScriptsError> {
    let workspace = state
        .workspace_scripts
        .scripts_for_task(viewer, task_id)
        .await?
        .ok_or(ScriptsError::NotFound)?;

    let (own, own_unread) = match read_own(sandboxes, task_id).await {
        Ok(own) => (own, None),
        Err(error) => {
            debug!(%error, task_id, "A task's own scripts were not read");
            (BTreeMap::new(), Some(error))
        }
    };

    Ok(Resolved {
        scripts: merge(workspace, own),
        own_unread,
    })
}

/// The Scripts a Task declared on itself, keyed by name.
async fn read_own(
    sandboxes: &dyn SandboxConnector,
    task_id: &str,
) -> Result<BTreeMap<String, ScriptConfig>, SandboxError> {
    let client = sandboxes.sandbox_client(task_id).await?;
    let scripts = client
        .get_task_scripts(task_id)
        .await
        .map_err(|status| classify_grpc_error(status, "get_task_scripts"))?;
    Ok(scripts
        .into_iter()
        .map(|(name, script)| (name, stored_script(script)))
        .collect())
}

/// One stored Task Script as the shape the rest of this module runs.
///
/// `env_vars` is always `None`: a Task Script is declared over a surface a
/// task-scoped bearer key reaches, which does not carry the workspace-level
/// access a pinned secret would assume, so the wire has no field for it.
fn stored_script(script: TaskScript) -> ScriptConfig {
    ScriptConfig {
        name: script.name,
        label: script.label,
        command: script.command,
        icon: script.icon,
        // A port past a `u16` is not a port; the sandbox refuses one on the way
        // in, so anything else on disk is dropped rather than failing the read.
        port: script.port.and_then(|port| u16::try_from(port).ok()),
        env_vars: None,
        run_on_start: script.run_on_start,
    }
}

/// Every Script a Task can run, for a caller that only wants to enumerate them
/// (`moat tasks scripts list`, over HTTP rather than the RPC/WS surface).
pub(crate) async fn list(
    state: &ScriptsState,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Vec<ResolvedScript>, ScriptsError> {
    Ok(resolve(state, sandboxes, viewer, task_id).await?.scripts)
}

/// A Script as `moat tasks scripts list` reports it.
///
/// Omits `env_vars`: a task-scoped bearer key reads this route without the
/// workspace-level access a pinned secret would assume.
pub fn public_script_list(scripts: &[ResolvedScript]) -> Vec<Value> {
    scripts
        .iter()
        .map(|script| {
            json!({
                "name": script.config.name,
                "label": script.config.label,
                "command": script.config.command,
                "icon": script.config.icon,
                "port": script.config.port,
                "runOnStart": script.config.run_on_start,
                "scope": scope(script),
            })
        })
        .collect()
}

/// A Script as `scripts.listForThread` reports it: the contract's
/// `ProjectScript` with the `scope` it was declared at.
///
/// Shaped by [`projection::ui_script`], which is also what lists a project's
/// own Scripts, so the same Script reads the same on both surfaces.
pub(crate) fn ui_script_list(scripts: &[ResolvedScript]) -> Vec<Value> {
    scripts
        .iter()
        .filter_map(|script| projection::ui_script(&script.config, Some(scope(script))))
        .collect()
}

/// Start a Script and return the terminal it runs in, plus — when it serves a
/// port — its preview URL and the browser tab now pointing at it.
///
/// `started` is false when a session already held this Script's terminal id and
/// the call attached to it instead: nothing was run, and what that session is
/// running need not be `command`.
pub(crate) async fn run(
    state: &ScriptsState,
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    hub: &TerminalHub,
    viewer: &ScopeContext,
    input: &ScriptsRunInput,
) -> Result<Value, ScriptsError> {
    let task_id = nonblank(&input.thread_id, "threadId")?;
    let script_id = nonblank(&input.script_id, "scriptId")?;
    // Before the port: a caller the terminal would refuse must not publish one.
    terminal::written_thread(conversations, viewer, task_id)
        .await
        .map_err(ScriptsError::Terminal)?;

    let resolved = resolve(state, sandboxes, viewer, task_id).await?;
    let script = match resolved
        .scripts
        .into_iter()
        .find(|script| script.config.name == script_id)
    {
        Some(script) => script.config,
        // A name this call could not look up is not a name that is absent.
        None => {
            return Err(match resolved.own_unread {
                Some(error) => ScriptsError::Sandbox(error),
                None => ScriptsError::ScriptNotFound(script_id.to_string()),
            });
        }
    };

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

    let open = terminal::TerminalInput {
        thread_id: Some(task_id.to_string()),
        terminal_id: Some(terminal_id.clone()),
        cwd: Some(cwd),
        env: script.env_vars.clone(),
        ..Default::default()
    };
    let (_, is_new_process) =
        terminal::open_from_environment(conversations, sandboxes, hub, viewer, &open)
            .await
            .map_err(ScriptsError::Terminal)?;

    // Only a session this call started gets the command. A session already
    // under this id is this same script still running: `terminal.open` reused
    // its shell, and writing again would type the command into the running
    // process's stdin rather than start a second run. Running twice is the
    // client asking to watch the one that is running, so return it as-is.
    if is_new_process {
        let write = terminal::TerminalInput {
            thread_id: Some(task_id.to_string()),
            terminal_id: Some(terminal_id.clone()),
            data: Some(exec_line(&script.command)),
            ..Default::default()
        };
        terminal::write_from_environment(conversations, sandboxes, viewer, &write)
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
        // False when this call attached to a session already under this id
        // instead of starting anything. Without it a caller cannot tell the two
        // apart, and reports a start for a process someone else's `command`
        // started.
        "started": is_new_process,
        "command": script.command,
        "url": url,
        "previewTabId": preview_tab_id,
    }))
}

/// Declare `name` on this Task alone, replacing whatever it held under that
/// name.
///
/// Stored in the Task's sandbox under `WORKSPACE_DIR`, so it rides the
/// workspace snapshot into a replacement pod rather than editing the Workspace,
/// which would put a control on every Task under it.
pub(crate) async fn declare(
    state: &ScriptsState,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    task_id: &str,
    name: &str,
    payload: &Value,
) -> Result<ScriptConfig, ScriptsError> {
    authorize(state, viewer, task_id).await?;
    let script = TaskScript {
        name: name.to_string(),
        command: required_str(payload, "command")?.to_string(),
        label: optional_str(payload, "label").filter(|label| label != name),
        icon: optional_str(payload, "icon").filter(|icon| projection::is_script_icon(icon)),
        port: port(payload)?.map(u32::from),
        run_on_start: payload
            .get("runOnStart")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
    };
    apply_own(sandboxes, task_id, script.clone(), false).await?;
    Ok(stored_script(script))
}

/// Drop the Script `name` declares on this Task, leaving any Workspace Script
/// of the same name in place.
///
/// Removing one that was never declared succeeds.
pub(crate) async fn remove(
    state: &ScriptsState,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    task_id: &str,
    name: &str,
) -> Result<(), ScriptsError> {
    authorize(state, viewer, task_id).await?;
    apply_own(
        sandboxes,
        task_id,
        TaskScript {
            name: name.to_string(),
            ..TaskScript::default()
        },
        true,
    )
    .await
}

/// The scoped Workspace read every Task Script write goes through first.
///
/// A Task Script is stored in the sandbox, which has no viewer scoping of its
/// own, so this is the only thing standing between a viewer and another
/// tenant's Task.
async fn authorize(
    state: &ScriptsState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<(), ScriptsError> {
    state
        .workspace_scripts
        .scripts_for_task(viewer, task_id)
        .await?
        .ok_or(ScriptsError::NotFound)?;
    Ok(())
}

async fn apply_own(
    sandboxes: &dyn SandboxConnector,
    task_id: &str,
    script: TaskScript,
    remove: bool,
) -> Result<(), ScriptsError> {
    sandboxes
        .sandbox_client(task_id)
        .await
        .map_err(ScriptsError::Sandbox)?
        .apply_task_script(task_id, script, remove)
        .await
        .map_err(|status| ScriptsError::Sandbox(classify_grpc_error(status, "apply_task_script")))
}

/// A trimmed, non-empty string field, or `None`.
fn optional_str(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
/// `.workspace-ready` sentinel.
const SETUP_GATE: &str = concat!(
    r#"if [ ! -f "${DATA_DIR:-/opt/moatless/data}/.workspace-ready" ]; then "#,
    r#"echo "Waiting for workspace setup to finish..."; "#,
    r#"until [ -f "${DATA_DIR:-/opt/moatless/data}/.workspace-ready" ]; do sleep 2; done; "#,
    "fi; ",
);

/// The port a Script serves on, if it declares one.
///
/// Absent and null both mean "serves nothing". Out-of-range is refused, not
/// clamped, so a mistyped port never silently becomes 65535.
fn port(script: &Value) -> Result<Option<u16>, ScriptsError> {
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
            ScriptsError::Invalid(format!(
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

    fn config(name: &str, command: &str) -> ScriptConfig {
        ScriptConfig {
            name: name.to_string(),
            label: None,
            command: command.to_string(),
            icon: None,
            port: None,
            env_vars: None,
            run_on_start: false,
        }
    }

    /// The client reads the merged listing as its `ProjectScript`, and tells
    /// the two apart by `scope` alone.
    #[test]
    fn the_ui_listing_says_which_side_declared_each_script() {
        let own = BTreeMap::from([("demo".to_string(), config("demo", "python3 -m http.server"))]);
        let listed = ui_script_list(&merge(vec![config("dev", "bun dev")], own));

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["id"], "dev");
        assert_eq!(listed[0]["scope"], "workspace");
        assert_eq!(listed[1]["id"], "demo");
        assert_eq!(listed[1]["command"], "python3 -m http.server");
        assert_eq!(listed[1]["scope"], "task");
    }

    /// The same rule the project listing applies: one Script the client cannot
    /// decode would cost the Task every other Script it can run.
    #[test]
    fn a_script_missing_a_command_is_left_out_of_the_ui_listing() {
        let listed = ui_script_list(&merge(
            vec![config("dev", "bun dev"), config("broken", "")],
            BTreeMap::new(),
        ));

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["id"], "dev");
    }
}
