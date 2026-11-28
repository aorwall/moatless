//! Workspace commands: list, get, delete, credentials and scripts.
//!
//! A Workspace is created and edited only through git, in a repository's
//! `.moatless/workspaces.json`.

use clap::Subcommand;
use clap::builder::PossibleValuesParser;
use moatless_workspace_types::schemas::workspace::{ScriptConfig, WorkspaceResponse};
use moatless_workspace_types::workspace::WorkspaceCredentialProvider;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum WorkspaceAction {
    /// List workspaces
    List,
    /// Get workspace details
    Get { workspace_id: String },
    /// Delete a legacy manual workspace (soft-delete). A git-synced one is
    /// removed by deleting its entry from `.moatless/workspaces.json`
    Delete {
        workspace_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Manage provider credentials pinned to a workspace.
    ///
    /// A workspace credential (e.g. a GitHub PAT) overrides the run-as user's
    /// own for every task on it — sandbox clone/push and backend PR/repo-access
    /// calls alike — reaching repos that user's own token cannot (e.g. outside
    /// its org).
    Credentials {
        #[command(subcommand)]
        action: CredentialAction,
    },
    /// Show a workspace's scripts; each becomes a Run button in the chat
    /// header's Project actions, or runs via `moat tasks scripts run`.
    /// Declare them under `scripts` in `.moatless/workspaces.json`.
    Scripts {
        #[command(subcommand)]
        action: WorkspaceScriptAction,
    },
}

#[derive(Subcommand)]
pub enum WorkspaceScriptAction {
    /// List the workspace's declared scripts
    List { workspace_id: String },
}

#[derive(Subcommand)]
pub enum CredentialAction {
    /// List the credentials pinned to a workspace (metadata only; never values)
    List { workspace_id: String },
    /// Pin a provider credential to a workspace, replacing any current value
    Set {
        workspace_id: String,
        /// The provider this credential authenticates against
        #[arg(value_parser = provider_values())]
        provider: String,
        /// Credential value (use "-" to read from stdin)
        #[arg(long, short = 'v')]
        value: Option<String>,
        /// Read the value from a file instead of --value
        #[arg(long, conflicts_with = "value")]
        value_file: Option<String>,
    },
    /// Remove a pinned credential; the workspace falls back to the run-as user
    Remove {
        workspace_id: String,
        /// The provider this credential authenticates against
        #[arg(value_parser = provider_values())]
        provider: String,
    },
}

/// The provider names `--help` offers, read from the enum the backend accepts,
/// so a provider added there cannot leave this list stale.
fn provider_values() -> PossibleValuesParser {
    PossibleValuesParser::new(WorkspaceCredentialProvider::ALL.iter().map(|p| p.as_str()))
}

pub async fn run(
    client: &MoatClient,
    action: WorkspaceAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        WorkspaceAction::List => {
            let workspaces = client.api().list_workspaces().await?;
            output::emit(json, &workspaces, || format_workspace_list(&workspaces));
        }
        WorkspaceAction::Get { workspace_id } => {
            let ws = client.api().get_workspace(&workspace_id).await?;
            output::emit(json, &ws, || format_workspace_detail(&ws));
        }
        WorkspaceAction::Delete { workspace_id, yes } => {
            let ws = client.api().get_workspace(&workspace_id).await?;
            let name = ws.name.clone();
            let id = ws.id.clone();

            if !yes {
                use std::io::{IsTerminal, Write};
                if !std::io::stdin().is_terminal() {
                    return Err(ClientError::Other(
                        "Refusing to delete without --yes in non-interactive mode".to_string(),
                    ));
                }
                print!("Delete workspace '{name}' ({id})? This cannot be undone. [y/N] ");
                std::io::stdout()
                    .flush()
                    .map_err(|e| ClientError::Other(format!("Failed to flush stdout: {e}")))?;
                let mut line = String::new();
                std::io::stdin()
                    .read_line(&mut line)
                    .map_err(|e| ClientError::Other(format!("Failed to read stdin: {e}")))?;
                let answer = line.trim().to_lowercase();
                if answer != "y" && answer != "yes" {
                    return Err(ClientError::Other("Aborted.".to_string()));
                }
            }

            client.api().delete_workspace(&id).await?;
            output::emit(
                json,
                &serde_json::json!({
                    "id": id,
                    "deleted": true,
                }),
                || println!("Deleted workspace {name} ({id})"),
            );
        }
        WorkspaceAction::Credentials { action } => {
            run_credentials(client, action, json).await?;
        }
        WorkspaceAction::Scripts { action } => {
            run_workspace_scripts(client, action, json).await?;
        }
    }
    Ok(())
}

async fn run_workspace_scripts(
    client: &MoatClient,
    action: WorkspaceScriptAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        WorkspaceScriptAction::List { workspace_id } => {
            let ws = client.api().get_workspace(&workspace_id).await?;
            let scripts = ws.scripts.unwrap_or_default();
            if json {
                output::print_json_typed(&scripts);
            } else if scripts.is_empty() {
                println!("No scripts declared on workspace {workspace_id}.");
            } else {
                format_scripts_list(&scripts);
            }
        }
    }
    Ok(())
}

/// One line per script this workspace declares.
pub(crate) fn format_scripts_list(scripts: &[ScriptConfig]) {
    let rows: Vec<Vec<String>> = scripts
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.label.clone().unwrap_or_default(),
                summarize_command(&s.command),
                s.port.map_or_default(|p| p.to_string()),
                if s.run_on_start { "yes" } else { "" }.to_string(),
            ]
        })
        .collect();
    output::print_table(&["NAME", "LABEL", "COMMAND", "PORT", "ON START"], &rows);
}

/// A multi-line command folded onto one line for a table (display only — the
/// stored command is untouched).
pub(crate) fn summarize_command(command: &str) -> String {
    let one_line = command.replace('\n', " \u{23ce} ");
    if one_line.chars().count() > 60 {
        format!("{}...", one_line.chars().take(57).collect::<String>())
    } else {
        one_line
    }
}

async fn run_credentials(
    client: &MoatClient,
    action: CredentialAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        CredentialAction::List { workspace_id } => {
            let creds = client
                .api()
                .list_workspace_credentials(&workspace_id)
                .await?;
            if json {
                output::print_json_typed(&creds);
            } else if creds.is_empty() {
                println!("No credentials pinned to workspace {workspace_id}.");
            } else {
                let rows: Vec<Vec<String>> = creds
                    .iter()
                    .map(|c| {
                        vec![
                            c.provider.to_string(),
                            c.env_key.clone(),
                            c.updated_by.clone(),
                            c.updated_at.clone(),
                        ]
                    })
                    .collect();
                output::print_table(&["PROVIDER", "ENV KEY", "UPDATED BY", "UPDATED"], &rows);
            }
        }
        CredentialAction::Set {
            workspace_id,
            provider,
            value,
            value_file,
        } => {
            let resolved = match value_file.as_deref() {
                Some(path) => read_value_file(path)?,
                None => read_value(value)?.ok_or_else(|| {
                    ClientError::Other("Either --value or --value-file is required".into())
                })?,
            };
            let stored = client
                .api()
                .set_workspace_credential(&workspace_id, &provider, &resolved)
                .await?;
            output::emit(json, &stored, || {
                println!(
                    "Pinned {} credential to workspace {} (delivered as {}).",
                    stored.provider, workspace_id, stored.env_key
                );
            });
        }
        CredentialAction::Remove {
            workspace_id,
            provider,
        } => {
            client
                .api()
                .delete_workspace_credential(&workspace_id, &provider)
                .await?;
            output::emit(
                json,
                &serde_json::json!({
                    "workspaceId": workspace_id,
                    "provider": provider,
                    "removed": true,
                }),
                || println!("Removed {provider} credential from workspace {workspace_id}."),
            );
        }
    }
    Ok(())
}

/// Read a credential value from `--value`, honouring `-` as "read stdin".
fn read_value(value: Option<String>) -> Result<Option<String>, ClientError> {
    use std::io::{IsTerminal, Read};
    match value.as_deref() {
        Some("-") => {
            if std::io::stdin().is_terminal() {
                return Err(ClientError::Other(
                    "Cannot read from stdin in interactive mode. Pipe input or use --value directly."
                        .into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| ClientError::Other(format!("Failed to read from stdin: {e}")))?;
            Ok(Some(buf.trim_end_matches('\n').to_string()))
        }
        Some(v) => Ok(Some(v.to_string())),
        None => Ok(None),
    }
}

fn read_value_file(path: &str) -> Result<String, ClientError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| ClientError::Other(format!("Failed to read value file '{path}': {e}")))?;
    Ok(contents.trim_end_matches('\n').to_string())
}

fn format_workspace_list(workspaces: &[WorkspaceResponse]) {
    let rows: Vec<Vec<String>> = workspaces
        .iter()
        .map(|w| {
            let source = if w.synced_from_git {
                "git".to_string()
            } else if w.source_repository_id.is_some() {
                "manual (was git)".to_string()
            } else {
                "manual".to_string()
            };
            vec![w.id.clone(), w.name.clone(), w.kind.clone(), source]
        })
        .collect();
    output::print_table(&["ID", "NAME", "KIND", "SOURCE"], &rows);
}

fn format_workspace_detail(w: &WorkspaceResponse) {
    println!("ID:          {}", w.id);
    println!("Name:        {}", w.name);
    if let Some(ref d) = w.description {
        println!("Description: {d}");
    }
    println!("Kind:        {}", w.kind);
    println!("Scope:       {}", w.scope);
    if let Some(ref owner) = w.owner_user_id {
        println!("Owner:       {owner}");
    }

    let source_label = if w.synced_from_git {
        "git".to_string()
    } else if w.source_repository_id.is_some() {
        "manual (was git — overridden)".to_string()
    } else {
        "manual".to_string()
    };
    println!("Source:      {source_label}");
    if let Some(ref path) = w.source_config_path {
        println!("Config path: {path}");
    }
    if let Some(ref repo_id) = w.source_repository_id {
        println!("Source repo: {repo_id}");
    }
    if let Some(ref key) = w.source_key {
        println!("Source key:  {key}");
    }

    if let Some(ref img) = w.docker_image {
        println!("Docker image: {img}");
    }
    if let Some(docker_access) = w.docker_access {
        println!("Docker access: {docker_access}");
    }
    if let Some(ref svc) = w.sandbox_service_account {
        println!("Service account: {svc}");
    }
    if let Some(ref profile) = w.network_profile {
        println!("Network profile: {profile}");
    }
    if let Some(labels) = w.node_selector.as_ref().filter(|s| !s.is_empty()) {
        let rendered: Vec<String> = labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        println!("Node selector: {}", rendered.join(","));
    }
    if let Some(ref prompt) = w.system_prompt {
        let preview: String = prompt.chars().take(80).collect();
        let ellipsis = if prompt.len() > 80 { "…" } else { "" };
        println!("System prompt: {preview}{ellipsis}");
    }
    if !w.repos.is_empty() {
        println!("Repos ({}):", w.repos.len());
        for r in &w.repos {
            let primary = if r.is_primary { " [primary]" } else { "" };
            let branch = r.branch.as_deref().unwrap_or("default");
            println!("  {} branch={branch}{primary}", r.repository_id);
        }
    }
    if let Some(ref servers) = w.language_servers {
        let ids: Vec<&str> = servers.iter().map(|s| s.as_str()).collect();
        println!(
            "Language servers: {}",
            if ids.is_empty() {
                "none".to_string()
            } else {
                ids.join(", ")
            }
        );
    }
    if let Some(ref cmds) = w.setup_commands
        && !cmds.is_empty()
    {
        println!("Setup commands:");
        for cmd in cmds {
            println!("  - {cmd}");
        }
    }
}
