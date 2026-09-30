//! Replacing a task's pod, and reporting what came back.

use crate::client::{ClientError, MoatClient};
use crate::config::Config;
use moatless_api_schemas::sandbox::SandboxStatusFullResponse;

/// True if `task_id` names the sandbox this command is running inside.
///
/// Restarting or redeploying that one takes the process issuing the request
/// down with it; any other task is fair game.
fn is_own_sandbox(config: &Config, task_id: &str) -> bool {
    config.task_id() == Some(task_id)
}

/// A pod-replacing operation, named for the caller it warns and the note it
/// leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SandboxOperation {
    Restart,
    Redeploy,
}

impl SandboxOperation {
    fn command(self) -> &'static str {
        match self {
            Self::Restart => "moat tasks restart",
            Self::Redeploy => "moat tasks redeploy",
        }
    }

    /// What the new pod is, which is the one way the two differ: a redeploy
    /// rebuilds the pod from the current repository configuration.
    fn outcome(self) -> &'static str {
        match self {
            Self::Restart => "The sandbox came back with the same configuration.",
            Self::Redeploy => "The sandbox came back with the current repository configuration.",
        }
    }
}

/// The wake note to record before `op` runs, or `None` when the caller isn't
/// operating on its own sandbox.
///
/// Errors when `yes` is false: the pod dies, and the turn that ran the command
/// dies with it.
pub(super) fn self_restart_note(
    config: &Config,
    task_id: &str,
    yes: bool,
    op: SandboxOperation,
) -> Result<Option<String>, ClientError> {
    if !is_own_sandbox(config, task_id) {
        return Ok(None);
    }
    let command = op.command();
    if !yes {
        return Err(ClientError::Api {
            status: 400,
            body: format!(
                "`{command}` on your own sandbox replaces this pod. This command and your \
                 current turn die with it, and nothing you print after it reaches anybody. \
                 The new pod resumes your session and sends you a <sandbox-restart-note> \
                 that says the operation finished, so you continue from there. Gitignored \
                 build artifacts do not survive. Run `{command} $TASK_ID --yes` to do it."
            ),
        });
    }
    Ok(Some(format!(
        "You ran `{command} --yes` on your own sandbox, and the pod was replaced. {} \
         Gitignored build artifacts were wiped, so re-run install and build steps before \
         you rely on compiled output. Continue the task.",
        op.outcome()
    )))
}

/// Leave `text` with the sandbox, for the agent to be given after the pod comes
/// back.
///
/// Runs before the operation, because the operation never returns — the pod
/// dies while the request is in flight, taking this process with it.
pub(super) async fn record_wake_note(
    grpc_port: &str,
    task_id: &str,
    text: Option<&str>,
) -> Result<(), ClientError> {
    let Some(text) = text else {
        return Ok(());
    };
    let mut client = crate::commands::local_sandbox::connect(grpc_port)
        .await
        .map_err(|e| ClientError::Other(e.to_string()))?;
    client
        .record_wake_note(sandbox_proto::RecordWakeNoteRequest {
            text: text.to_string(),
            task_id: Some(task_id.to_string()),
        })
        .await
        .map_err(|e| {
            ClientError::Other(format!(
                "the sandbox refused the wake note, so the restart was not started: {e}"
            ))
        })?;
    Ok(())
}

/// Take the note back after a failed operation, so no later pod replacement
/// delivers it.
///
/// Only a backend refusal (`Api`, `Unauthorized`, `NotLoggedIn`) cancels it; a
/// transport error keeps it, since it can't be told apart from a pod already
/// going down.
pub(super) async fn cancel_wake_note(
    grpc_port: &str,
    task_id: &str,
    recorded: bool,
    error: &ClientError,
) {
    let refused = matches!(
        error,
        ClientError::Api { .. } | ClientError::Unauthorized | ClientError::NotLoggedIn
    );
    if !recorded || !refused {
        return;
    }
    let outcome = async {
        crate::commands::local_sandbox::connect(grpc_port)
            .await
            .map_err(|e| e.to_string())?
            .cancel_wake_note(sandbox_proto::CancelWakeNoteRequest {
                task_id: Some(task_id.to_string()),
            })
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    if let Err(e) = outcome {
        eprintln!("Warning: the wake note is still recorded and will be delivered later: {e}");
    }
}

pub(super) fn format_sandbox_status(status: &SandboxStatusFullResponse) {
    println!("Sandbox:    {}", status.sandbox_status);
    if let Some(ref err) = status.sandbox_error {
        println!("Error:       {err}");
    }
    let agent_status = status
        .agent_status
        .map_or_else(|| "n/a".to_string(), |value| value.to_string());
    println!("Agent:       {agent_status}");

    format_container_diagnostics_json(None);
}

fn format_container_diagnostics_json(diagnostics: Option<&Vec<serde_json::Value>>) {
    let items = match diagnostics {
        Some(items) if !items.is_empty() => items,
        _ => return,
    };

    println!("\nContainer Diagnostics:");
    for c in items {
        let name = c
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        let state = c
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        let restarts = c
            .get("restartCount")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        println!("  {name}:");
        println!("    State:          {state}");
        println!("    Restart Count:  {restarts}");

        if let Some(code) = c.get("exitCode").and_then(serde_json::Value::as_i64) {
            println!("    Exit Code:      {code}");
        }
        if let Some(reason) = c
            .get("terminatedReason")
            .and_then(serde_json::Value::as_str)
        {
            println!("    Terminated:     {reason}");
        }
        if let Some(code) = c.get("lastExitCode").and_then(serde_json::Value::as_i64) {
            println!("    Last Exit Code: {code}");
        }
        if let Some(reason) = c
            .get("lastTerminatedReason")
            .and_then(serde_json::Value::as_str)
        {
            println!("    Last Terminated: {reason}");
        }
        if let Some(reason) = c.get("waitingReason").and_then(serde_json::Value::as_str) {
            println!("    Waiting:        {reason}");
        }
        // Resource usage (from K8s Metrics API)
        let has_cpu = c.get("cpuMillicores").and_then(serde_json::Value::as_u64);
        let has_mem = c.get("memoryMb").and_then(serde_json::Value::as_u64);
        if has_cpu.is_some() || has_mem.is_some() {
            if let Some(cpu) = has_cpu {
                println!("    CPU:            {cpu}m");
            }
            if let Some(mem) = has_mem {
                println!("    Memory:         {mem}Mi");
            }
        }
    }
}

/// Diagnose sandbox issues: aggregates status + container diagnostics + recent
/// errors.
pub(super) async fn run_diagnose(
    client: &MoatClient,
    task_id: &str,
    limit: i64,
    json: bool,
) -> Result<(), ClientError> {
    // Fetch status (includes container diagnostics from Phase 1)
    let status_path = format!("/api/sandbox/v1/tasks/{task_id}/status");
    let status = client.raw_get(&status_path).await?;

    // Fetch recent error events
    let events_path = format!("/api/v1/tasks/{task_id}/events/history?limit={limit}&types=sandbox");
    let events_resp = client.raw_get(&events_path).await;

    if json {
        let mut report = serde_json::json!({
            "taskId": task_id,
            "status": status,
        });
        if let Ok(ref events) = events_resp {
            report["events"] = events.clone();
        }
        let pretty = serde_json::to_string_pretty(&report).unwrap_or_default();
        println!("{pretty}");
        return Ok(());
    }

    // Format human-readable report
    println!("=== Sandbox Diagnostics: {task_id} ===\n");

    // Status section
    println!("Status:");
    let sandbox_status = status
        .get("sandboxStatus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    println!("  Sandbox:      {sandbox_status}");

    let agent_status = status
        .get("agentStatus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("n/a");
    println!("  Agent:        {agent_status}");

    if let Some(err) = status
        .get("sandboxError")
        .and_then(serde_json::Value::as_str)
    {
        println!("  Error:        {err}");
    }

    // Container diagnostics section
    let diagnostics = status
        .get("containerDiagnostics")
        .and_then(serde_json::Value::as_array);
    format_container_diagnostics_json(diagnostics);

    // Runtime events section (from K8s/Docker events)
    if let Some(rt_events) = status
        .get("runtimeEvents")
        .and_then(serde_json::Value::as_array)
    {
        if !rt_events.is_empty() {
            println!("\nRuntime Events:");
            for e in rt_events {
                let reason = e
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let message = e
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let severity = e
                    .get("severity")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("normal");
                let count = e
                    .get("count")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(1);
                let ts = e
                    .get("lastTimestamp")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let ts = if ts.len() > 19 { &ts[..19] } else { ts };
                let sev_tag = if severity == "warning" { " [WARN]" } else { "" };
                println!("  {ts}  {reason}{sev_tag} (x{count})  {message}");
            }
        }
    }

    // Recent events section
    if let Ok(events) = events_resp {
        let items = events
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Filter to error events only
        let errors: Vec<_> = items
            .iter()
            .filter(|e| {
                e.get("isError")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            })
            .collect();

        if errors.is_empty() {
            println!("\nNo recent error events.");
        } else {
            println!("\nRecent Error Events:");
            for e in &errors {
                let ts = e
                    .get("timestamp")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let ts = if ts.len() > 19 { &ts[..19] } else { ts };
                let event_type = e
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let sub_type = e
                    .get("subType")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let payload = e
                    .get("payload")
                    .and_then(|v| {
                        v.get("message")
                            .or_else(|| v.get("error"))
                            .or_else(|| v.get("status"))
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or("");
                println!("  {ts}  {event_type:<8} {sub_type:<10} {payload}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthMode, HttpConfig};

    const OWN: &str = "11111111-2222-3333-4444-555555555555";
    const OTHER: &str = "99999999-8888-7777-6666-555555555555";

    /// A sandbox-mode config for the task `OWN`, which is the only task these
    /// commands treat differently.
    fn config_inside(task_id: &str) -> Config {
        Config {
            api_url: "https://example.invalid".to_string(),
            auth_mode: AuthMode::Sandbox {
                user_auth_token: "token".to_string(),
                task_id: Some(task_id.to_string()),
            },
            session_file: "/tmp/.moat_session".to_string(),
            cookie_name: "moatless_session".to_string(),
            login: crate::config::LoginEnv::default(),
            sandbox_grpc_port: "50052".to_string(),
            gh_repo_set: false,
            github_token_set: false,
            http: HttpConfig::default(),
        }
    }

    /// Another task's sandbox is somebody else's, so the flag changes nothing
    /// and no note is left.
    #[test]
    fn another_task_needs_no_confirmation() {
        let config = config_inside(OWN);
        for yes in [false, true] {
            let note = self_restart_note(&config, OTHER, yes, SandboxOperation::Restart)
                .expect("another task must not be refused");
            assert_eq!(note, None);
        }
    }

    #[test]
    fn a_standalone_caller_needs_no_confirmation() {
        let config = Config {
            auth_mode: AuthMode::Standalone { api_key: None },
            ..config_inside(OWN)
        };
        let note = self_restart_note(&config, OWN, false, SandboxOperation::Restart)
            .expect("a caller outside the sandbox owns no pod");
        assert_eq!(note, None);
    }

    /// The agent has to learn what this costs before it pays it.
    #[test]
    fn your_own_sandbox_warns_before_it_acts() {
        let err = self_restart_note(&config_inside(OWN), OWN, false, SandboxOperation::Restart)
            .expect_err("an unconfirmed self-restart must warn");
        let body = err.to_string();
        assert!(body.contains("--yes"), "{body}");
        assert!(body.contains("<sandbox-restart-note>"), "{body}");
        assert!(body.contains("moat tasks restart"), "{body}");
    }

    #[test]
    fn a_confirmed_self_restart_leaves_a_note() {
        let note = self_restart_note(&config_inside(OWN), OWN, true, SandboxOperation::Restart)
            .expect("a confirmed self-restart proceeds")
            .expect("a confirmed self-restart must leave a note");
        assert!(note.contains("moat tasks restart --yes"), "{note}");
        assert!(note.contains("Continue the task"), "{note}");
    }

    /// The two operations differ in what the new pod is, so their notes must
    /// not be interchangeable.
    #[test]
    fn a_redeploy_note_names_the_redeploy() {
        let note = self_restart_note(&config_inside(OWN), OWN, true, SandboxOperation::Redeploy)
            .expect("a confirmed self-redeploy proceeds")
            .expect("note");
        assert!(note.contains("moat tasks redeploy --yes"), "{note}");
        assert!(note.contains("current repository configuration"), "{note}");
    }
}
