//! The task-scoped half of Scripts: declaring one on this task alone, and
//! running it or anything its workspace declares.
//!
//! `moat workspaces scripts` declares on the workspace, which puts the script on
//! every task under it; `run --command` declares on this task only. The record
//! lives in the sandbox under `WORKSPACE_DIR`, so it survives a restart and `run
//! <name>` alone starts it again afterwards.

use crate::client::{ClientError, MoatClient};
use crate::commands::workspaces::summarize_command;
use crate::output;
use clap::Subcommand;
use serde::Deserialize;

#[derive(Subcommand)]
pub enum TaskScriptAction {
    /// List the scripts this task can run: its workspace's, plus its own
    List,
    /// Start a script, or attach to it if it's already running (running twice
    /// attaches rather than starting a second copy). With --command, declare it
    /// on this task first
    Run {
        /// Script id: one from `list`, or a new name to declare with --command
        name: String,
        /// Shell command to run. Declares the script on this task alone,
        /// replacing whatever it held under this name
        #[arg(long)]
        command: Option<String>,
        /// Port the command serves, which is what earns it a preview URL and a
        /// browser tab in the T3 UI
        #[arg(long)]
        port: Option<u16>,
        /// Display name in the T3 UI
        #[arg(long)]
        label: Option<String>,
        /// Icon name in the T3 UI
        #[arg(long)]
        icon: Option<String>,
        /// Start this script again whenever the sandbox becomes ready
        #[arg(long)]
        run_on_start: bool,
    },
    /// Drop a script this task declared on itself, leaving any workspace script
    /// of the same name in place
    Remove {
        /// Script id, as listed with scope `task`
        name: String,
    },
}

/// One entry of the task's script list, as `public_script_list` writes it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedScript {
    name: String,
    label: Option<String>,
    command: String,
    port: Option<u16>,
    run_on_start: bool,
    /// `"task"` or `"workspace"`. A backend from before task scripts omits it,
    /// and everything it lists is a workspace script.
    scope: Option<String>,
}

/// Lists, declares, starts or drops a script, over the same HTTP routes the T3
/// UI uses (`scripts_run_http` and its siblings).
///
/// Plain bearer-key routes — no WebSocket ticket needed.
pub(super) async fn run_task_script_action(
    client: &MoatClient,
    task_id: &str,
    action: TaskScriptAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        TaskScriptAction::List => {
            let resp = client
                .raw_get(&format!("/api/orchestration/scripts/{task_id}"))
                .await
                .map_err(|e| friendlier_scripts_error(e, task_id))?;
            let listed = resp
                .get("scripts")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![]));
            if json {
                output::print_json_typed(&listed);
                return Ok(());
            }
            let scripts: Vec<ListedScript> = serde_json::from_value(listed)
                .map_err(|e| ClientError::Other(format!("Unexpected response: {e}")))?;
            if scripts.is_empty() {
                println!("No scripts declared for task {task_id}.");
            } else {
                format_task_scripts_list(&scripts);
            }
        }
        TaskScriptAction::Run {
            name,
            command,
            port,
            label,
            icon,
            run_on_start,
        } => {
            let script_id = urlencoding::encode(&name);
            let resp = client
                .raw_post(
                    &format!("/api/orchestration/scripts/{task_id}/{script_id}/run"),
                    &declaration(command, port, label, icon, run_on_start),
                )
                .await
                .map_err(|e| friendlier_scripts_error(e, task_id))?;
            output::emit(json, &resp, || {
                for line in run_report(&name, &resp) {
                    println!("{line}");
                }
            });
        }
        TaskScriptAction::Remove { name } => {
            let script_id = urlencoding::encode(&name);
            let resp = client
                .raw_delete(&format!("/api/orchestration/scripts/{task_id}/{script_id}"))
                .await
                .map_err(|e| friendlier_scripts_error(e, task_id))?;
            output::emit(json, &resp, || {
                println!("Removed \"{name}\" from task {task_id}.");
            });
        }
    }
    Ok(())
}

/// The body `run` posts. Without `--command` it declares nothing and the
/// backend runs whatever is already declared, so the other flags are dropped
/// rather than half-declaring a script with no command.
fn declaration(
    command: Option<String>,
    port: Option<u16>,
    label: Option<String>,
    icon: Option<String>,
    run_on_start: bool,
) -> serde_json::Value {
    let Some(command) = command else {
        return serde_json::json!({});
    };
    serde_json::json!({
        "command": command,
        "port": port,
        "label": label,
        "icon": icon,
        "runOnStart": run_on_start,
    })
}

/// One line per script, with where it was declared.
///
/// Not [`crate::commands::workspaces::format_scripts_list`]: this list merges
/// two scopes, and which one a script came from is what says whether `moat
/// workspaces scripts update` or `moat tasks scripts remove` is the way to
/// change it.
fn format_task_scripts_list(scripts: &[ListedScript]) {
    output::print_table(
        &["NAME", "SCOPE", "LABEL", "COMMAND", "PORT", "ON START"],
        &script_rows(scripts),
    );
}

fn script_rows(scripts: &[ListedScript]) -> Vec<Vec<String>> {
    scripts
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.scope.clone().unwrap_or_else(|| "workspace".to_string()),
                s.label.clone().unwrap_or_default(),
                summarize_command(&s.command),
                s.port.map(|p| p.to_string()).unwrap_or_default(),
                if s.run_on_start { "yes" } else { "" }.to_string(),
            ]
        })
        .collect()
}

/// What `run` did, as lines to print.
///
/// `scripts.run` starts a Script or attaches to a session already holding its
/// terminal id, running nothing. Reporting an attach as `Started "<name>"`
/// names a command this call did not run, over a port the live process need
/// not be serving — as does claiming either when the backend reports neither.
fn run_report(name: &str, resp: &serde_json::Value) -> Vec<String> {
    let terminal_id = resp
        .get("terminalId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?");
    // A backend from before `started` attached under exactly the same answer it
    // gives a start, so reading the absent field as either one reports what this
    // call cannot know.
    let started = resp.get("started").and_then(serde_json::Value::as_bool);
    let command = resp.get("command").and_then(serde_json::Value::as_str);

    let mut lines = Vec::new();
    match started {
        Some(true) => {
            lines.push(format!("Started \"{name}\" in terminal {terminal_id}."));
            if let Some(command) = command {
                lines.push(format!("  running: {command}"));
            }
        }
        Some(false) => {
            lines.push(format!(
                "Attached to \"{name}\" in terminal {terminal_id}: it was already running, so nothing was started."
            ));
            lines.push(match command {
                Some(command) => format!(
                    "  That session need not be running this script's command ({command}) — read the terminal in the T3 UI to see what it is running."
                ),
                None => "  Read the terminal in the T3 UI to see what it is running.".to_string(),
            });
        }
        None => {
            lines.push(format!("Ran \"{name}\" in terminal {terminal_id}."));
            lines.push(format!(
                "  This backend does not say whether it started the script or attached to a session already under that id{}, so read the terminal in the T3 UI to see what it is running.",
                match command {
                    Some(command) => format!(" running something other than {command}"),
                    None => String::new(),
                }
            ));
        }
    }

    match resp.get("url").and_then(serde_json::Value::as_str) {
        Some(url) => {
            lines.push(format!("Preview: {url}"));
            // Only when the backend says it opened one. An older one answers
            // without the field, and its preview is the URL above and nothing
            // else.
            if resp
                .get("previewTabId")
                .is_some_and(serde_json::Value::is_string)
            {
                lines.push("Opened a browser tab on it in the T3 UI.".to_string());
            }
        }
        None => lines.push(
            "No port declared, so no preview — open the T3 UI to watch its output, or re-run with --command and --port."
                .to_string(),
        ),
    }
    lines
}

/// `scripts.run`'s refusals are typed and already say the specific thing (`"No
/// script named \"nope\"."`, `"Sandbox not running."`) — this only adds the
/// next command, the same way `resolve_task_id`'s own errors do.
fn friendlier_scripts_error(err: ClientError, task_id: &str) -> ClientError {
    let ClientError::Api { status, body } = &err else {
        return err;
    };
    let Some(message) = super::format::api_error_message(body) else {
        return err;
    };
    match *status {
        404 => ClientError::Other(format!(
            "{message} Run `moat tasks scripts {task_id} list` to see what's declared."
        )),
        409 => ClientError::Other(format!(
            "{message} Run `moat tasks start {task_id}`, then retry."
        )),
        _ => ClientError::Other(format!("({status}) {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(resp: serde_json::Value) -> String {
        run_report("preview-static", &resp).join("\n")
    }

    #[test]
    fn a_start_names_the_command_it_ran() {
        let lines = report(json!({
            "terminalId": "script-preview-static",
            "started": true,
            "command": "bunx serve dist -l 3001",
            "url": "https://preview.example/3001",
            "previewTabId": "script-preview-static",
        }));
        assert!(lines.contains("Started \"preview-static\""), "{lines}");
        assert!(
            lines.contains("running: bunx serve dist -l 3001"),
            "{lines}"
        );
    }

    /// Reporting an attach as a start is what sent a reader looking for
    /// `bunx serve` on port 3001 when the session was running `bun run dev`.
    #[test]
    fn an_attach_says_nothing_was_started_and_does_not_vouch_for_the_command() {
        let lines = report(json!({
            "terminalId": "script-preview-static",
            "started": false,
            "command": "bunx serve dist -l 3001",
            "url": "https://preview.example/3001",
            "previewTabId": null,
        }));
        assert!(!lines.contains("Started"), "{lines}");
        assert!(lines.contains("Attached"), "{lines}");
        assert!(lines.contains("nothing was started"), "{lines}");
        assert!(lines.contains("need not be running"), "{lines}");
    }

    /// A backend without the field is the one that reported an attach as a
    /// start, so claiming either from its answer reports what it never said.
    #[test]
    fn a_response_without_the_field_claims_neither_a_start_nor_an_attach() {
        let lines = report(json!({ "terminalId": "script-dev" }));
        assert!(!lines.contains("Started"), "{lines}");
        assert!(!lines.contains("Attached"), "{lines}");
        assert!(lines.contains("Ran \"preview-static\""), "{lines}");
        assert!(lines.contains("does not say whether"), "{lines}");
        assert!(lines.contains("No port declared"), "{lines}");
    }

    fn rows(listed: serde_json::Value) -> Vec<Vec<String>> {
        script_rows(&serde_json::from_value::<Vec<ListedScript>>(listed).expect("list parses"))
    }

    /// Which scope a script came from is what says whether `moat tasks scripts
    /// remove` can drop it or the workspace has to be edited.
    #[test]
    fn the_scope_column_separates_a_task_script_from_a_workspace_one() {
        let rows = rows(json!([
            { "name": "dev", "command": "bun dev", "runOnStart": true, "scope": "workspace" },
            { "name": "demo", "command": "python3 -m http.server 4321", "port": 4321,
              "runOnStart": false, "scope": "task" },
        ]));
        assert_eq!(rows[0][0..2], ["dev".to_string(), "workspace".to_string()]);
        assert_eq!(rows[1][0..2], ["demo".to_string(), "task".to_string()]);
        assert_eq!(rows[1][4], "4321");
    }

    /// A backend from before task scripts lists only workspace ones, so an
    /// absent field is that scope rather than an empty column.
    #[test]
    fn a_response_without_a_scope_reads_as_a_workspace_script() {
        let rows = rows(json!([{ "name": "dev", "command": "bun dev", "runOnStart": false }]));
        assert_eq!(rows[0][1], "workspace");
    }

    /// Without `--command` nothing is declared, so sending a port would ask the
    /// backend to store a script with no command to run.
    #[test]
    fn the_other_flags_are_dropped_when_no_command_is_given() {
        assert_eq!(declaration(None, Some(4321), None, None, true), json!({}));
        assert_eq!(
            declaration(Some("serve".into()), Some(4321), None, None, true),
            json!({
                "command": "serve", "port": 4321, "label": null, "icon": null,
                "runOnStart": true,
            })
        );
    }
}
