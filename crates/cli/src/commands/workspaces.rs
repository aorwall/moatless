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
        /// Access scope: global, team, or user
        #[arg(long)]
        scope: Option<String>,
        /// Team ID (required when scope is team)
        #[arg(long)]
        team_id: Option<String>,
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
        /// Plugin names as a JSON array, e.g. `["plugin-a"]`
        #[arg(long)]
        plugins_json: Option<String>,
        /// Resources config as a JSON object
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
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
        /// Change access scope: global, team, or user
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
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
        plugins_json: Option<String>,
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
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
        plugins_json: Option<String>,
        #[arg(long)]
        resources_json: Option<String>,
        #[arg(long)]
        network_profile: Option<String>,
    },
    /// Reset an overridden workspace back to git-managed. The next git sync
    /// will update its run-config from the repository.
    #[command(name = "reset-git")]
    ResetGit { workspace_id: String },
    /// Delete a workspace (soft-delete; manual workspaces only)
    Delete {
        workspace_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
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
            team_id,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            servers_json,
            plugins_json,
            resources_json,
            network_profile,
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
                team_id,
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                servers: parse_json_opt(&servers_json, "--servers-json")?,
                plugins: parse_json_opt(&plugins_json, "--plugins-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
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
            team_id,
            docker_image,
            docker_access,
            sandbox_service_account,
            system_prompt,
            setup_commands_json,
            env_vars_json,
            servers_json,
            plugins_json,
            resources_json,
            network_profile,
        } => {
            let parsed_scope = parse_scope_arg(scope.as_deref())?;
            let req = UpdateWorkspaceRequest {
                name,
                description,
                scope: parsed_scope,
                team_id,
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                servers: parse_json_opt(&servers_json, "--servers-json")?,
                plugins: parse_json_opt(&plugins_json, "--plugins-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
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
            plugins_json,
            resources_json,
            network_profile,
        } => {
            let req = UpdateWorkspaceRequest {
                name,
                description,
                scope: None,
                team_id: None,
                docker_image,
                docker_access,
                sandbox_service_account,
                system_prompt,
                setup_commands: parse_json_opt(&setup_commands_json, "--setup-commands-json")?,
                env_vars: parse_json_opt(&env_vars_json, "--env-vars-json")?,
                servers: parse_json_opt(&servers_json, "--servers-json")?,
                plugins: parse_json_opt(&plugins_json, "--plugins-json")?,
                resources: parse_json_opt(&resources_json, "--resources-json")?,
                network_profile,
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
            let updated = client.api().reset_git_workspace(&workspace_id).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!(
                    "Workspace {} reset to git-managed. Next sync will update it.",
                    updated.id
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
    }
    Ok(())
}

fn parse_scope(s: &str) -> Option<Scope> {
    match s {
        "global" => Some(Scope::Global),
        "team" => Some(Scope::Team),
        "user" => Some(Scope::User),
        _ => None,
    }
}

fn parse_scope_arg(s: Option<&str>) -> Result<Option<Scope>, ClientError> {
    match s {
        None => Ok(None),
        Some(v) => parse_scope(v)
            .map(Some)
            .ok_or_else(|| ClientError::Other(format!("Invalid --scope '{v}' (global/team/user)"))),
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
    if let Some(ref tid) = w.team_id {
        println!("Team ID:     {tid}");
    }
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
