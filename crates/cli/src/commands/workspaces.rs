//! Workspace commands: list, get, create, update, override, reset-git, delete.

use clap::Subcommand;
use moatless_api_schemas::enums::Scope;
use moatless_api_schemas::workspace::{
    CreateWorkspaceRequest, UpdateWorkspaceRequest, WorkspaceRepoInput, WorkspaceResponse,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum WorkspaceAction {
    /// List workspaces
    List,
    /// Get workspace details
    Get { workspace_id: String },
    /// Create a workspace
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: Option<String>,
        /// Access scope: global or user
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        docker_image: Option<String>,
        /// Allow docker-in-docker access in the sandbox
        #[arg(long)]
        docker_access: Option<bool>,
        #[arg(long)]
        sandbox_service_account: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        /// Setup commands as a JSON array, e.g. `["npm install","npm run build"]`
        #[arg(long)]
        setup_commands_json: Option<String>,
        /// Environment variables as a JSON object, e.g. `{"FOO":"bar"}`
        #[arg(long)]
        env_vars_json: Option<String>,
        /// Server configs as a JSON array
        #[arg(long)]
        servers_json: Option<String>,
        /// Script configs as a JSON array, e.g. `[{"name":"test","command":"bun test"}]`
        #[arg(long)]
        scripts_json: Option<String>,
        /// Resources config as a JSON object
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
        /// Identity every task on this workspace acts as on a git host.
        /// Admin only. Pass an empty string to clear it.
        #[arg(long)]
        run_as_user_id: Option<String>,
        /// Repository placements as a JSON array of {repositoryId,...} objects
        #[arg(long)]
        repos_json: Option<String>,
    },
    /// Update a workspace (manual workspaces only; use `override` for git-synced)
    Update {
        workspace_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Change access scope: global or user
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        docker_image: Option<String>,
        #[arg(long)]
        docker_access: Option<bool>,
        #[arg(long)]
        sandbox_service_account: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        #[arg(long)]
        setup_commands_json: Option<String>,
        #[arg(long)]
        env_vars_json: Option<String>,
        #[arg(long)]
        servers_json: Option<String>,
        #[arg(long)]
        scripts_json: Option<String>,
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
        /// Identity every task on this workspace acts as on a git host.
        /// Admin only. Pass an empty string to clear it.
        #[arg(long)]
        run_as_user_id: Option<String>,
    },
    /// Override a git-synced workspace. Sets source=manual so git sync skips
    /// it going forward. Provenance fields are kept so the origin is visible.
    /// Use `reset-git` to re-enable git sync.
    Override {
        workspace_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        docker_image: Option<String>,
        #[arg(long)]
        docker_access: Option<bool>,
        #[arg(long)]
        sandbox_service_account: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        #[arg(long)]
        setup_commands_json: Option<String>,
        #[arg(long)]
        env_vars_json: Option<String>,
        #[arg(long)]
        servers_json: Option<String>,
        #[arg(long)]
        scripts_json: Option<String>,
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
        /// Identity every task on this workspace acts as on a git host.
        /// Admin only. Pass an empty string to clear it.
        #[arg(long)]
        run_as_user_id: Option<String>,
    },
    /// Reset an overridden workspace back to git-managed. Re-syncs immediately
    /// from the repository — warning: local changes to this workspace, including
    /// its repo placements, are replaced by the committed config.
    #[command(name = "reset-git")]
    ResetGit { workspace_id: String },
    /// Delete a workspace (soft-delete; manual workspaces only)
    Delete {
        workspace_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Manage provider credentials pinned to a workspace. A workspace
    /// credential (a GitLab token, say) overrides the run-as user's own, so
    /// every task on the workspace authenticates with it. GitHub is not one of
    /// them — set `--run-as-user-id` on the workspace instead.
    Credentials {
        #[command(subcommand)]
        action: CredentialAction,
    },
}

#[derive(Subcommand)]
pub enum CredentialAction {
    /// List the credentials pinned to a workspace (metadata only; never values)
    List { workspace_id: String },
    /// Pin a provider credential to a workspace, replacing any current value
    Set {
        workspace_id: String,
        /// Provider: gitlab, gitness, openai, or anthropic
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
        /// Provider: gitlab, gitness, openai, or anthropic
        provider: String,
    },
}

pub async fn run(
    client: &MoatClient,
    action: WorkspaceAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        WorkspaceAction::List => {
            let workspaces = client.api().list_workspaces().await?;
            if json {
                output::print_json_typed(&workspaces);
            } else {
                format_workspace_list(&workspaces);
            }
        }
        WorkspaceAction::Get { workspace_id } => {
            let ws = client.api().get_workspace(&workspace_id).await?;
            if json {
                output::print_json_typed(&ws);
            } else {
                format_workspace_detail(&ws);
            }
        }
        WorkspaceAction::Create {
            name,
            description,
            scope,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            servers_json,
            scripts_json,
            resources_json,
            network_profile,
            run_as_user_id,
            repos_json,
        } => {
            let repos: Vec<WorkspaceRepoInput> = match repos_json.as_deref() {
                Some(s) => serde_json::from_str(s)
                    .map_err(|e| ClientError::Other(format!("Invalid --repos-json: {e}")))?,
                None => vec![],
            };
            let req = CreateWorkspaceRequest {
                name,
                description,
                scope: scope.as_deref().and_then(parse_scope),
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                servers: parse_json_opt(&servers_json, "--servers-json")?,
                scripts: parse_json_opt(&scripts_json, "--scripts-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
                run_as_user_id,
                repos,
            };
            let created = client.api().create_workspace(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Workspace created: {}", created.id);
            }
        }
        WorkspaceAction::Update {
            workspace_id,
            name,
            description,
            scope,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            servers_json,
            scripts_json,
            resources_json,
            network_profile,
            run_as_user_id,
        } => {
            let parsed_scope = parse_scope_arg(scope.as_deref())?;
            let req = UpdateWorkspaceRequest {
                name,
                description,
                scope: parsed_scope,
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                servers: parse_json_opt(&servers_json, "--servers-json")?,
                scripts: parse_json_opt(&scripts_json, "--scripts-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
                run_as_user_id,
            };
            let updated = client.api().update_workspace(&workspace_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Workspace {} updated.", updated.id);
            }
        }
        WorkspaceAction::Override {
            workspace_id,
            name,
            description,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            servers_json,
            scripts_json,
            resources_json,
            network_profile,
            run_as_user_id,
        } => {
            let req = UpdateWorkspaceRequest {
                name,
                description,
                scope: None,
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                servers: parse_json_opt(&servers_json, "--servers-json")?,
                scripts: parse_json_opt(&scripts_json, "--scripts-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
                run_as_user_id,
            };
            let updated = client.api().override_workspace(&workspace_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!(
                    "Workspace {} overridden (source=manual). Git sync will skip it until reset.",
                    updated.id
                );
                if let Some(ref path) = updated.source_config_path {
                    println!("  Originally synced from: {path}");
                }
                if let Some(ref repo_id) = updated.source_repository_id {
                    println!("  Source repository: {repo_id}");
                }
            }
        }
        WorkspaceAction::ResetGit { workspace_id } => {
            let result = client.api().reset_git_workspace(&workspace_id).await?;
            if json {
                output::print_json_typed(&result);
            } else if result.resynced {
                println!(
                    "Workspace {} reset to git-managed; local changes replaced by {}.",
                    result.workspace.id,
                    result
                        .workspace
                        .source_config_path
                        .as_deref()
                        .unwrap_or("the repository config")
                );
            } else {
                println!(
                    "Workspace {} left overridden: {}",
                    result.workspace.id,
                    result.error.as_deref().unwrap_or("re-sync did not run")
                );
            }
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
            if json {
                output::print_json_typed(&serde_json::json!({
                    "id": id,
                    "deleted": true,
                }));
            } else {
                println!("Deleted workspace {name} ({id})");
            }
        }
        WorkspaceAction::Credentials { action } => {
            run_credentials(client, action, json).await?;
        }
    }
    Ok(())
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
            if json {
                output::print_json_typed(&stored);
            } else {
                println!(
                    "Pinned {} credential to workspace {} (delivered as {}).",
                    stored.provider, workspace_id, stored.env_key
                );
            }
        }
        CredentialAction::Remove {
            workspace_id,
            provider,
        } => {
            client
                .api()
                .delete_workspace_credential(&workspace_id, &provider)
                .await?;
            if json {
                output::print_json_typed(&serde_json::json!({
                    "workspaceId": workspace_id,
                    "provider": provider,
                    "removed": true,
                }));
            } else {
                println!("Removed {provider} credential from workspace {workspace_id}.");
            }
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

fn parse_scope(s: &str) -> Option<Scope> {
    match s {
        "global" => Some(Scope::Global),
        "user" => Some(Scope::User),
        _ => None,
    }
}

fn parse_scope_arg(s: Option<&str>) -> Result<Option<Scope>, ClientError> {
    match s {
        None => Ok(None),
        Some(v) => parse_scope(v)
            .map(Some)
            .ok_or_else(|| ClientError::Other(format!("Invalid --scope '{v}' (global/user)"))),
    }
}

fn parse_json_opt<T: serde::de::DeserializeOwned>(
    raw: &Option<String>,
    flag: &str,
) -> Result<Option<T>, ClientError> {
    match raw.as_deref() {
        None => Ok(None),
        Some(s) => serde_json::from_str(s)
            .map(Some)
            .map_err(|e| ClientError::Other(format!("Invalid {flag}: {e}"))),
    }
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
    if let Some(ref cmds) = w.setup_commands {
        if !cmds.is_empty() {
            println!("Setup commands:");
            for cmd in cmds {
                println!("  - {cmd}");
            }
        }
    }
}
