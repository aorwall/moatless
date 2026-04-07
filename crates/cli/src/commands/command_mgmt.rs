//! Command management: list, get, create, update, delete.

use clap::Subcommand;
use moatless_api_schemas::command::{CreateCommandRequest, UpdateCommandRequest};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum CommandAction {
    /// List all commands
    List,
    /// Get a command by name
    Get {
        /// Command name
        name: String,
    },
    /// Create a new command
    Create {
        /// Command name (unique identifier)
        #[arg(long)]
        name: String,
        /// Human-readable description
        #[arg(long)]
        description: String,
        /// Prompt text sent to the agent
        #[arg(long)]
        prompt: String,
        /// Icon name (e.g. GitCommit, Play)
        #[arg(long)]
        icon: Option<String>,
        /// Whether the command prevents session resumption
        #[arg(long)]
        will_not_resume: Option<bool>,
    },
    /// Update an existing command
    Update {
        /// Command name to update
        name: String,
        /// New description
        #[arg(long)]
        description: Option<String>,
        /// New prompt text
        #[arg(long)]
        prompt: Option<String>,
        /// New icon name
        #[arg(long)]
        icon: Option<String>,
        /// Set active/inactive
        #[arg(long)]
        active: Option<bool>,
        /// Whether the command prevents session resumption
        #[arg(long)]
        will_not_resume: Option<bool>,
    },
    /// Delete a command
    Delete {
        /// Command name to delete
        name: String,
    },
}

pub async fn run(client: &MoatClient, action: CommandAction, json: bool) -> Result<(), ClientError> {
    match action {
        CommandAction::List => {
            let cmds = client.api().list_commands().await?;
            if json {
                output::print_json_typed(&cmds);
            } else {
                let rows: Vec<Vec<String>> = cmds
                    .iter()
                    .map(|c| {
                        vec![
                            c.name.clone(),
                            c.description.chars().take(60).collect(),
                        ]
                    })
                    .collect();
                output::print_table(&["NAME", "DESCRIPTION"], &rows);
            }
        }
        CommandAction::Get { name } => {
            let cmd = client.api().get_command(&name).await?;
            if json {
                output::print_json_typed(&cmd);
            } else {
                println!("Name:        {}", cmd.name);
                println!("Description: {}", cmd.description);
                println!("Prompt:      {}", cmd.prompt);
                if let Some(ref icon) = cmd.icon {
                    println!("Icon:        {icon}");
                }
                if let Some(active) = cmd.active {
                    println!("Active:      {active}");
                }
                if let Some(wnr) = cmd.will_not_resume {
                    println!("WillNotResume: {wnr}");
                }
            }
        }
        CommandAction::Create {
            name,
            description,
            prompt,
            icon,
            will_not_resume,
        } => {
            let req = CreateCommandRequest {
                name,
                description,
                prompt,
                conditions: None,
                icon,
                variant: None,
                active: Some(true),
                will_not_resume,
                agent_mode: None,
            };
            let created = client.api().create_command(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Command created: {}", created.name);
            }
        }
        CommandAction::Update {
            name,
            description,
            prompt,
            icon,
            active,
            will_not_resume,
        } => {
            let req = UpdateCommandRequest {
                description,
                prompt,
                conditions: None,
                icon,
                variant: None,
                active,
                will_not_resume,
                agent_mode: None,
            };
            let updated = client.api().update_command(&name, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Command updated: {}", updated.name);
            }
        }
        CommandAction::Delete { name } => {
            client.api().delete_command(&name).await?;
            println!("Command {name} deleted.");
        }
    }
    Ok(())
}
