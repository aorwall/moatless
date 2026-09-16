//! The task-scoped half of Scripts: running what a workspace already declares.
//! Declaring one is `moat workspaces scripts`.

use crate::client::{ClientError, MoatClient};
use crate::commands::workspaces::format_scripts_list;
use crate::output;
use clap::Subcommand;
use moatless_workspace_types::schemas::workspace::ScriptConfig;

#[derive(Subcommand)]
pub enum TaskScriptAction {
    /// List the scripts this task's workspace declares. See `moat workspaces
    /// scripts add` to declare one
    List,
    /// Start a declared script, or attach to it if it's already running
    /// (running twice attaches rather than starting a second copy)
    Run {
        /// Script id, as declared on the workspace (see `list`)
        name: String,
    },
}

/// Lists or starts a script, via a REST wrapper around the same `scripts.run`
/// the T3 UI's chat header uses (`scripts_run_http`).
///
/// A plain bearer-key HTTP route — no WebSocket ticket needed.
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
            let scripts: Vec<ScriptConfig> = serde_json::from_value(
                resp.get("scripts")
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
            )
            .map_err(|e| ClientError::Other(format!("Unexpected response: {e}")))?;
            if json {
                output::print_json_typed(&scripts);
            } else if scripts.is_empty() {
                println!("No scripts declared for task {task_id}.");
            } else {
                format_scripts_list(&scripts);
            }
        }
        TaskScriptAction::Run { name } => {
            let script_id = urlencoding::encode(&name);
            let resp = client
                .raw_post(
                    &format!("/api/orchestration/scripts/{task_id}/{script_id}/run"),
                    &serde_json::json!({}),
                )
                .await
                .map_err(|e| friendlier_scripts_error(e, task_id))?;
            if json {
                output::print_json_typed(&resp);
            } else {
                for line in run_report(&name, &resp) {
                    println!("{line}");
                }
            }
        }
    }
    Ok(())
}

/// What `run` did, as lines to print.
///
/// `scripts.run` starts a Script or, when a session already holds its terminal
/// id, attaches to that session and runs nothing. Reporting the second as
/// `Started "<name>"` names a command this call did not run, over a port the
/// live process need not be serving, so the two are worded apart.
fn run_report(name: &str, resp: &serde_json::Value) -> Vec<String> {
    let terminal_id = resp
        .get("terminalId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?");
    // A backend from before `started` never attached silently, so reading an
    // absent field as a start leaves its output as it was.
    let started = resp
        .get("started")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let command = resp.get("command").and_then(serde_json::Value::as_str);

    let mut lines = Vec::new();
    if started {
        lines.push(format!("Started \"{name}\" in terminal {terminal_id}."));
        if let Some(command) = command {
            lines.push(format!("  running: {command}"));
        }
    } else {
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
            "No port declared, so no preview — open the T3 UI to watch its output, or add --port with `moat workspaces scripts update`."
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
    let Some(message) = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
    else {
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
        assert!(lines.contains("running: bunx serve dist -l 3001"), "{lines}");
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

    /// The field is new; a backend without it never attached silently, so its
    /// answer still reads as the start it was.
    #[test]
    fn a_response_without_the_field_still_reads_as_a_start() {
        let lines = report(json!({ "terminalId": "script-dev" }));
        assert!(lines.contains("Started \"preview-static\""), "{lines}");
        assert!(lines.contains("No port declared"), "{lines}");
    }
}
