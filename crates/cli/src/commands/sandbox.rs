//! Sandbox commands: status, start, stop, restart.

use clap::Subcommand;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum SandboxAction {
    /// Show sandbox status for a task
    Status { task_id: String },
    /// Start sandbox for a task
    Start { task_id: String },
    /// Stop sandbox for a task
    Stop { task_id: String },
    /// Restart sandbox for a task
    Restart { task_id: String },
}

pub async fn run(
    client: &MoatClient,
    action: SandboxAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        SandboxAction::Status { task_id } => {
            let resp = client
                .get(&format!("/api/sandbox/v1/tasks/{task_id}/status"))
                .await?;
            if json {
                output::print_json(&resp);
            } else {
                format_sandbox_status(&resp);
            }
        }
        SandboxAction::Start { task_id } => {
            client
                .post_empty(&format!("/api/sandbox/v1/tasks/{task_id}/start"))
                .await?;
            println!("Sandbox for {task_id} started.");
        }
        SandboxAction::Stop { task_id } => {
            client
                .post_empty(&format!("/api/sandbox/v1/tasks/{task_id}/stop"))
                .await?;
            println!("Sandbox for {task_id} stopped.");
        }
        SandboxAction::Restart { task_id } => {
            client
                .post_empty(&format!("/api/sandbox/v1/tasks/{task_id}/restart"))
                .await?;
            println!("Sandbox for {task_id} restarted.");
        }
    }
    Ok(())
}

fn format_sandbox_status(resp: &serde_json::Value) {
    let env_status = resp
        .get("environmentStatus")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let agent_state = resp
        .get("agentState")
        .and_then(|v| v.as_str())
        .unwrap_or("-");

    println!("Environment: {env_status}");
    println!("Agent:       {agent_state}");

    if let Some(servers) = resp.get("servers").and_then(|v| v.as_array()) {
        if !servers.is_empty() {
            println!("\nServers:");
            for s in servers {
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("-");
                let status = s.get("status").and_then(|v| v.as_str()).unwrap_or("-");
                let port = s
                    .get("port")
                    .and_then(serde_json::Value::as_u64)
                    .map_or("-".to_string(), |p| p.to_string());
                println!("  {name}: {status} (port {port})");
            }
        }
    }
}
