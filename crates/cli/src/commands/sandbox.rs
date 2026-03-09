//! Sandbox commands: status, start, stop, restart.

use clap::Subcommand;
use moatless_api_client::types::SandboxStatusFullResponse;

use crate::client::{map_api_error, ClientError, MoatClient};
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
                .api()
                .sandbox_status_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            let status = resp.into_inner();
            if json {
                output::print_json_typed(&status);
            } else {
                format_sandbox_status(&status);
            }
        }
        SandboxAction::Start { task_id } => {
            client
                .api()
                .provision_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Sandbox for {task_id} started.");
        }
        SandboxAction::Stop { task_id } => {
            client
                .api()
                .stop_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Sandbox for {task_id} stopped.");
        }
        SandboxAction::Restart { task_id } => {
            client
                .api()
                .restart_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Sandbox for {task_id} restarted.");
        }
    }
    Ok(())
}

fn format_sandbox_status(status: &SandboxStatusFullResponse) {
    println!("Environment: {}", status.environment_status);
    println!("Agent:       {}", status.agent_state);

    if !status.servers.is_empty() {
        println!("\nServers:");
        for s in &status.servers {
            println!("  {}: {} (port {})", s.name, s.status, s.port);
        }
    }
}
