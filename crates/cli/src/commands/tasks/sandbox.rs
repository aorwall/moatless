//! Replacing a task's pod, and reporting what came back.

use crate::client::{ClientError, MoatClient};
use crate::config::Config;
use moatless_api_schemas::sandbox::SandboxStatusFullResponse;

/// True if `task_id` runs in the sandbox this command is running inside.
///
/// Compares Sandboxes rather than Tasks: a shared Sandbox holds several
/// Members, and stopping or replacing any one's pod takes this process down.
/// Reads the target's and then the caller's task, only when they differ.
async fn is_own_sandbox(
    client: &MoatClient,
    config: &Config,
    task_id: &str,
) -> Result<bool, ClientError> {
    let Some(own_task_id) = config.task_id() else {
        return Ok(false);
    };
    if own_task_id == task_id {
        return Ok(true);
    }
    let Some(target) = sandbox_of(client, task_id).await? else {
        return Ok(false);
    };
    Ok(sandbox_of(client, own_task_id).await?.as_deref() == Some(target.as_str()))
}

async fn sandbox_of(client: &MoatClient, task_id: &str) -> Result<Option<String>, ClientError> {
    let task = client.raw_get(&format!("/api/v1/tasks/{task_id}")).await?;
    Ok(task
        .get("sandboxId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string))
}

/// An operation that takes a task's pod down, named for the caller it warns
/// and the note it leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SandboxOperation {
    Stop,
    Restart,
    Redeploy,
}

impl SandboxOperation {
    fn command(self) -> &'static str {
        match self {
            Self::Stop => "moat tasks stop",
            Self::Restart => "moat tasks restart",
            Self::Redeploy => "moat tasks redeploy",
        }
    }

    /// What happens to this pod, for the warning.
    fn effect(self) -> &'static str {
        match self {
            Self::Stop => {
                "stops this pod, and nothing starts it again until a message or a wakeup \
                 arrives for the task. The pod started then"
            }
            Self::Restart | Self::Redeploy => "replaces this pod. The new pod",
        }
    }

    /// What the pod that read the note is, which is how the three differ: a
    /// redeploy rebuilds it from the current repository configuration, and a
    /// stop leaves it down until something else wakes the task.
    fn outcome(self) -> &'static str {
        match self {
            Self::Stop => {
                "the pod was stopped. Something woke the task since, and the sandbox came \
                 back with the same configuration."
            }
            Self::Restart => {
                "the pod was replaced. The sandbox came back with the same configuration."
            }
            Self::Redeploy => {
                "the pod was replaced. The sandbox came back with the current repository \
                 configuration."
            }
        }
    }
}

/// A wake note for the Member running this command.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct WakeNote {
    /// The caller's own task, which may differ from the target: a co-member's
    /// stop takes the caller's turn down too.
    task_id: String,
    text: String,
}

/// The wake note to record before `op` runs, or `None` when `task_id` is not in
/// the caller's own sandbox.
///
/// Errors when `yes` is false: the pod dies, and the turn that ran the command
/// dies with it.
async fn self_restart_note(
    client: &MoatClient,
    config: &Config,
    task_id: &str,
    yes: bool,
    op: SandboxOperation,
) -> Result<Option<WakeNote>, ClientError> {
    if !is_own_sandbox(client, config, task_id).await? {
        return Ok(None);
    }
    let command = op.command();
    if !yes {
        return Err(ClientError::Api {
            status: 400,
            body: format!(
                "`{command}` on your own sandbox {} resumes your session and sends you a \
                 <sandbox-restart-note> that says the operation finished, so you continue \
                 from there. This command and your current turn die with the pod, and \
                 nothing you print after it reaches anybody. Gitignored build artifacts do \
                 not survive. Run `{command} {task_id} --yes` to do it.",
                op.effect()
            ),
        });
    }
    let own_task_id = config.task_id().unwrap_or(task_id);
    Ok(Some(WakeNote {
        task_id: own_task_id.to_string(),
        text: format!(
            "You ran `{command} --yes` on your own sandbox, and {} Gitignored build \
             artifacts were wiped, so re-run install and build steps before you rely on \
             compiled output. Continue the task.",
            op.outcome()
        ),
    }))
}

/// Leave `note` with the sandbox, for the agent to be given after the pod comes
/// back.
///
/// Runs before the operation, because the operation never returns — the pod
/// dies while the request is in flight, taking this process with it.
async fn record_wake_note(grpc_port: &str, note: Option<&WakeNote>) -> Result<(), ClientError> {
    let Some(note) = note else {
        return Ok(());
    };
    let mut client = crate::commands::local_sandbox::connect(grpc_port)
        .await
        .map_err(|e| ClientError::Other(e.to_string()))?;
    client
        .record_wake_note(sandbox_proto::RecordWakeNoteRequest {
            text: note.text.clone(),
            task_id: Some(note.task_id.clone()),
        })
        .await
        .map_err(|e| {
            ClientError::Other(format!(
                "the sandbox refused the wake note, so the operation was not started: {e}"
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
async fn cancel_wake_note(grpc_port: &str, note: Option<&WakeNote>, error: &ClientError) {
    let refused = matches!(
        error,
        ClientError::Api { .. } | ClientError::Unauthorized | ClientError::NotLoggedIn
    );
    let Some(note) = note.filter(|_| refused) else {
        return;
    };
    let outcome = async {
        crate::commands::local_sandbox::connect(grpc_port)
            .await
            .map_err(|e| e.to_string())?
            .cancel_wake_note(sandbox_proto::CancelWakeNoteRequest {
                task_id: Some(note.task_id.clone()),
            })
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    if let Err(e) = outcome {
        eprintln!("Warning: the wake note is still recorded and will be delivered later: {e}");
    }
}

/// Run `run`, which takes `task_id`'s pod down as `op`, behind the warning and
/// wake note [`self_restart_note`] decides on.
pub(super) async fn take_pod_down<T, F, Fut>(
    client: &MoatClient,
    config: &Config,
    task_id: &str,
    yes: bool,
    op: SandboxOperation,
    run: F,
) -> Result<T, ClientError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, moatless_api_client::ApiError>>,
{
    let note = self_restart_note(client, config, task_id, yes, op).await?;
    record_wake_note(&config.sandbox_grpc_port, note.as_ref()).await?;
    match run().await {
        Ok(value) => Ok(value),
        Err(e) => {
            let e = ClientError::from(e);
            cancel_wake_note(&config.sandbox_grpc_port, note.as_ref(), &e).await;
            Err(e)
        }
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
    let status_path = format!("/api/sandbox/v1/tasks/{task_id}/status");
    let status = client.raw_get(&status_path).await?;

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

    println!("=== Sandbox Diagnostics: {task_id} ===\n");

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

    let diagnostics = status
        .get("containerDiagnostics")
        .and_then(serde_json::Value::as_array);
    format_container_diagnostics_json(diagnostics);

    // Runtime events section (from K8s/Docker events)
    if let Some(rt_events) = status
        .get("runtimeEvents")
        .and_then(serde_json::Value::as_array)
        && !rt_events.is_empty()
    {
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

    if let Ok(events) = events_resp {
        let items = events
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

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
    const CO_MEMBER: &str = "22222222-2222-3333-4444-555555555555";
    const OTHER: &str = "99999999-8888-7777-6666-555555555555";

    /// A sandbox-mode config for the task `OWN`.
    fn config_inside(task_id: &str) -> Config {
        Config {
            api_url: Some("https://example.invalid".to_string()),
            auth_mode: AuthMode::Sandbox {
                user_auth_token: "token".to_string(),
                task_id: Some(task_id.to_string()),
            },
            login: crate::config::LoginEnv::default(),
            sandbox_grpc_port: "50052".to_string(),
            gh_repo: None,
            github_token_set: false,
            http: HttpConfig::default(),
        }
    }

    /// A backend where `OWN` and `CO_MEMBER` share one Sandbox and `OTHER`
    /// has its own.
    async fn backend() -> MoatClient {
        use axum::extract::Path;
        let app = axum::Router::new().route(
            "/api/v1/tasks/{task_id}",
            axum::routing::get(|Path(task_id): Path<String>| async move {
                let sandbox = if task_id == OTHER {
                    "sb-other"
                } else {
                    "sb-shared"
                };
                axum::Json(serde_json::json!({ "taskId": task_id, "sandboxId": sandbox }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a port");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
        MoatClient::new_with_api_key(&format!("http://{addr}"), "key", HttpConfig::default())
    }

    async fn note_for(
        config: &Config,
        task_id: &str,
        yes: bool,
        op: SandboxOperation,
    ) -> Result<Option<WakeNote>, ClientError> {
        self_restart_note(&backend().await, config, task_id, yes, op).await
    }

    /// Another sandbox is somebody else's, so the flag changes nothing and no
    /// note is left.
    #[tokio::test]
    async fn another_sandbox_needs_no_confirmation() {
        let config = config_inside(OWN);
        for yes in [false, true] {
            let note = note_for(&config, OTHER, yes, SandboxOperation::Restart)
                .await
                .expect("another sandbox must not be refused");
            assert_eq!(note, None);
        }
    }

    /// A co-member's pod is this one, so restarting it strands the caller.
    #[tokio::test]
    async fn a_co_member_in_the_same_sandbox_is_your_own() {
        let config = config_inside(OWN);
        note_for(&config, CO_MEMBER, false, SandboxOperation::Restart)
            .await
            .expect_err("a co-member's restart takes this pod down too");
        let note = note_for(&config, CO_MEMBER, true, SandboxOperation::Restart)
            .await
            .expect("a confirmed restart proceeds")
            .expect("a confirmed restart must leave a note");
        assert_eq!(note.task_id, OWN, "the note belongs to the caller");
    }

    #[tokio::test]
    async fn a_standalone_caller_needs_no_confirmation() {
        let config = Config {
            auth_mode: AuthMode::Standalone { api_key: None },
            ..config_inside(OWN)
        };
        let note = note_for(&config, OWN, false, SandboxOperation::Restart)
            .await
            .expect("a caller outside the sandbox owns no pod");
        assert_eq!(note, None);
    }

    /// The agent has to learn what this costs before it pays it.
    #[tokio::test]
    async fn your_own_sandbox_warns_before_it_acts() {
        let err = note_for(&config_inside(OWN), OWN, false, SandboxOperation::Restart)
            .await
            .expect_err("an unconfirmed self-restart must warn");
        let body = err.to_string();
        assert!(body.contains("--yes"), "{body}");
        assert!(body.contains("<sandbox-restart-note>"), "{body}");
        assert!(body.contains("moat tasks restart"), "{body}");
    }

    #[tokio::test]
    async fn a_confirmed_self_restart_leaves_a_note() {
        let note = note_for(&config_inside(OWN), OWN, true, SandboxOperation::Restart)
            .await
            .expect("a confirmed self-restart proceeds")
            .expect("a confirmed self-restart must leave a note");
        assert!(
            note.text.contains("moat tasks restart --yes"),
            "{}",
            note.text
        );
        assert!(note.text.contains("Continue the task"), "{}", note.text);
    }

    /// The operations differ in what the pod is when the note is read, so
    /// their notes must not be interchangeable.
    #[tokio::test]
    async fn a_redeploy_note_names_the_redeploy() {
        let note = note_for(&config_inside(OWN), OWN, true, SandboxOperation::Redeploy)
            .await
            .expect("a confirmed self-redeploy proceeds")
            .expect("note");
        assert!(
            note.text.contains("moat tasks redeploy --yes"),
            "{}",
            note.text
        );
        assert!(
            note.text.contains("current repository configuration"),
            "{}",
            note.text
        );
    }

    /// A stop leaves the pod down, so the agent must hear that before it acts
    /// and that it was woken afterwards.
    #[tokio::test]
    async fn your_own_stop_warns_and_leaves_a_note() {
        let err = note_for(&config_inside(OWN), OWN, false, SandboxOperation::Stop)
            .await
            .expect_err("an unconfirmed self-stop must warn");
        assert!(err.to_string().contains("nothing starts it again"), "{err}");

        let note = note_for(&config_inside(OWN), OWN, true, SandboxOperation::Stop)
            .await
            .expect("a confirmed self-stop proceeds")
            .expect("a confirmed self-stop must leave a note");
        assert!(note.text.contains("moat tasks stop --yes"), "{}", note.text);
        assert!(note.text.contains("was stopped"), "{}", note.text);
    }
}
