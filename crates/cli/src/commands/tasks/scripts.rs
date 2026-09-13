//! The task-scoped half of Scripts: running what a workspace already declares.
//! Declaring one is `moat workspaces scripts`.

use crate::client::{ClientError, MoatClient};
use crate::commands::workspaces::format_scripts_list;
use crate::output;
use clap::Subcommand;
use moatless_api_schemas::workspace::ScriptConfig;

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
                let terminal_id = resp
                    .get("terminalId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                println!("Started \"{name}\" in terminal {terminal_id}.");
                if let Some(url) = resp.get("url").and_then(serde_json::Value::as_str) {
                    println!("Preview: {url}");
                    // Only when the backend says it opened one. An older one
                    // answers without the field, and its preview is the URL
                    // above and nothing else.
                    if resp
                        .get("previewTabId")
                        .is_some_and(serde_json::Value::is_string)
                    {
                        println!("Opened a browser tab on it in the T3 UI.");
                    }
                } else {
                    println!(
                        "No port declared, so no preview — open the T3 UI to watch its output, or add --port with `moat workspaces scripts update`."
                    );
                }
            }
        }
    }
    Ok(())
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
