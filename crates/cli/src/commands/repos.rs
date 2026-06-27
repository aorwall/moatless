//! Repository commands: list, get, create, update.

use std::collections::HashMap;

use clap::Subcommand;
use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider, Scope};
use moatless_api_schemas::repository::{
    CreateRepositoryRequest, RepositoryResponse, ResourceConfig, ServerConfig,
    UpdateRepositoryRequest,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum RepoAction {
    /// List repositories
    List,
    /// Get repository details
    Get { repo_id: String },
    /// Create a repository
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        git_url: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long, default_value = "github")]
        provider: String,
        /// Repository kind: remote, template, or git
        #[arg(long, default_value = "remote")]
        kind: String,
        /// CPU request (e.g. "200m")
        #[arg(long)]
        resource_cpu_request: Option<String>,
        /// CPU limit (e.g. "4")
        #[arg(long)]
        resource_cpu_limit: Option<String>,
        /// Memory request (e.g. "2560Mi")
        #[arg(long)]
        resource_memory_request: Option<String>,
        /// Memory limit (e.g. "4096Mi")
        #[arg(long)]
        resource_memory_limit: Option<String>,
        /// Workspace size override, e.g. "8Gi" (sizes the PVC or the ephemeral emptyDir).
        /// Like the --resource-* flags, a resources update replaces the whole resources
        /// object, so pass all desired resource flags together.
        #[arg(long)]
        storage: Option<String>,
        /// Storage tier: "ebs" (default, per-task PVC) or "ephemeral" (node-local emptyDir, no PVC)
        #[arg(long)]
        storage_mode: Option<String>,
        /// Docker image for sandbox (e.g. "myregistry/my-sandbox:latest")
        #[arg(long)]
        docker_image: Option<String>,
        /// Plugin references (repeatable, e.g. "name@url")
        #[arg(long = "plugin")]
        plugins: Vec<String>,
        /// System prompt for the repository
        #[arg(long)]
        system_prompt: Option<String>,
        /// Repository description
        #[arg(long)]
        description: Option<String>,
        /// Build command
        #[arg(long)]
        build_command: Option<String>,
        /// Build output directory
        #[arg(long)]
        build_output_dir: Option<String>,
        /// Enable inspector
        #[arg(long)]
        enable_inspector: Option<bool>,
        /// Enable auto-reload
        #[arg(long)]
        auto_reload: Option<bool>,
        /// Repo-level environment variables as KEY=VALUE (repeatable)
        #[arg(long = "env-var")]
        env_vars: Vec<String>,
        /// Setup commands run before server starts (repeatable)
        #[arg(long = "setup-command")]
        setup_commands: Vec<String>,
        /// Legacy server start commands (repeatable)
        #[arg(long = "server-start-command")]
        server_start_commands: Vec<String>,
        /// Full servers config as JSON string
        #[arg(long)]
        servers_json: Option<String>,
        /// Startup timeout in seconds
        #[arg(long)]
        startup_timeout: Option<i32>,
        /// Team ID
        #[arg(long)]
        team_id: Option<String>,
        /// Scope: global, team, or user
        #[arg(long)]
        scope: Option<String>,
    },
    /// Update a repository
    Update {
        repo_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        git_url: Option<String>,
        /// CPU request (e.g. "200m")
        #[arg(long)]
        resource_cpu_request: Option<String>,
        /// CPU limit (e.g. "4")
        #[arg(long)]
        resource_cpu_limit: Option<String>,
        /// Memory request (e.g. "2560Mi")
        #[arg(long)]
        resource_memory_request: Option<String>,
        /// Memory limit (e.g. "4096Mi")
        #[arg(long)]
        resource_memory_limit: Option<String>,
        /// Workspace size override, e.g. "8Gi" (sizes the PVC or the ephemeral emptyDir).
        /// Like the --resource-* flags, a resources update replaces the whole resources
        /// object, so pass all desired resource flags together.
        #[arg(long)]
        storage: Option<String>,
        /// Storage tier: "ebs" (default, per-task PVC) or "ephemeral" (node-local emptyDir, no PVC)
        #[arg(long)]
        storage_mode: Option<String>,
        /// Docker image for sandbox (e.g. "myregistry/my-sandbox:latest")
        #[arg(long)]
        docker_image: Option<String>,
        /// Setup commands run before server starts (repeatable)
        #[arg(long = "setup-command")]
        setup_commands: Vec<String>,
        /// Legacy server start commands (repeatable)
        #[arg(long = "server-start-command")]
        server_start_commands: Vec<String>,
        /// Repo-level environment variables as KEY=VALUE (repeatable)
        #[arg(long = "env-var")]
        env_vars: Vec<String>,
        /// Startup timeout in seconds
        #[arg(long)]
        startup_timeout: Option<i32>,
        /// Full servers config as JSON string
        #[arg(long)]
        servers_json: Option<String>,
        /// Plugin references (repeatable, e.g. "name@url")
        #[arg(long = "plugin")]
        plugins: Vec<String>,
        /// System prompt for the repository
        #[arg(long)]
        system_prompt: Option<String>,
        /// Repository description
        #[arg(long)]
        description: Option<String>,
        /// Build command
        #[arg(long)]
        build_command: Option<String>,
        /// Build output directory
        #[arg(long)]
        build_output_dir: Option<String>,
        /// Enable inspector
        #[arg(long)]
        enable_inspector: Option<bool>,
        /// Enable auto-reload
        #[arg(long)]
        auto_reload: Option<bool>,
    },
    /// Sync .moatless/config.json from the remote repository
    SyncConfig { repo_id: String },
    /// Delete a repository (soft-delete — marks as deleted, preserves history)
    Delete {
        repo_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

pub async fn run(client: &MoatClient, action: RepoAction, json: bool) -> Result<(), ClientError> {
    match action {
        RepoAction::List => {
            let repos = client.api().list_repositories().await?;
            if json {
                output::print_json_typed(&repos);
            } else {
                format_repo_list(&repos);
            }
        }
        RepoAction::Get { repo_id } => {
            let repo = client.api().get_repository(&repo_id).await?;
            if json {
                output::print_json_typed(&repo);
            } else {
                format_repo_detail(&repo);
            }
        }
        RepoAction::Create {
            name,
            git_url,
            branch,
            provider,
            kind,
            resource_cpu_request,
            resource_cpu_limit,
            resource_memory_request,
            resource_memory_limit,
            storage,
            storage_mode,
            docker_image,
            plugins,
            system_prompt,
            description,
            build_command,
            build_output_dir,
            enable_inspector,
            auto_reload,
            env_vars,
            setup_commands,
            server_start_commands,
            servers_json,
            startup_timeout,
            team_id,
            scope,
        } => {
            let resources = build_resource_config(
                resource_cpu_request,
                resource_cpu_limit,
                resource_memory_request,
                resource_memory_limit,
                storage,
                storage_mode,
            );
            let servers = match servers_json {
                Some(ref json_str) => {
                    let parsed: Vec<ServerConfig> = serde_json::from_str(json_str)
                        .map_err(|e| ClientError::Other(format!("Invalid --servers-json: {e}")))?;
                    Some(parsed)
                }
                None => None,
            };
            let req = CreateRepositoryRequest {
                name,
                kind: parse_repo_kind(&kind),
                remote_url: Some(git_url),
                default_branch: Some(branch),
                provider: parse_repo_provider(&provider),
                auto_reload,
                build_command,
                build_output_directory: build_output_dir,
                description,
                docker_image,
                enable_inspector,
                env_vars: parse_env_vars(&env_vars),
                plugins: if plugins.is_empty() {
                    None
                } else {
                    Some(plugins)
                },
                server_start_commands: if server_start_commands.is_empty() {
                    None
                } else {
                    Some(server_start_commands)
                },
                servers,
                setup_commands: if setup_commands.is_empty() {
                    None
                } else {
                    Some(setup_commands)
                },
                startup_timeout_seconds: startup_timeout,
                scope: scope.as_deref().and_then(parse_scope),
                system_prompt,
                team_id,
                resources,
            };
            let created = client.api().create_repository(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Repository created: {}", created.id);
            }
        }
        RepoAction::Update {
            repo_id,
            name,
            branch,
            git_url,
            resource_cpu_request,
            resource_cpu_limit,
            resource_memory_request,
            resource_memory_limit,
            storage,
            storage_mode,
            docker_image,
            setup_commands,
            server_start_commands,
            env_vars,
            startup_timeout,
            servers_json,
            plugins,
            system_prompt,
            description,
            build_command,
            build_output_dir,
            enable_inspector,
            auto_reload,
        } => {
            let resources = build_resource_config(
                resource_cpu_request,
                resource_cpu_limit,
                resource_memory_request,
                resource_memory_limit,
                storage,
                storage_mode,
            );
            let servers = match servers_json {
                Some(ref json_str) => {
                    let parsed: Vec<ServerConfig> = serde_json::from_str(json_str)
                        .map_err(|e| ClientError::Other(format!("Invalid --servers-json: {e}")))?;
                    Some(parsed)
                }
                None => None,
            };
            let req = UpdateRepositoryRequest {
                name,
                default_branch: branch,
                remote_url: git_url,
                auto_reload,
                build_command,
                build_output_directory: build_output_dir,
                description,
                docker_image,
                enable_inspector,
                env_vars: parse_env_vars(&env_vars),
                plugins: if plugins.is_empty() {
                    None
                } else {
                    Some(plugins)
                },
                server_start_commands: if server_start_commands.is_empty() {
                    None
                } else {
                    Some(server_start_commands)
                },
                servers,
                setup_commands: if setup_commands.is_empty() {
                    None
                } else {
                    Some(setup_commands)
                },
                startup_timeout_seconds: startup_timeout,
                system_prompt,
                resources,
            };
            let updated = client.api().update_repository(&repo_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Repository {} updated.", updated.id);
            }
        }
        RepoAction::SyncConfig { repo_id } => {
            let result = client.api().sync_config(&repo_id).await?;
            if json {
                output::print_json_typed(&result);
            } else {
                let loops = &result.loops;
                println!("Config synced for repository {}.", result.repository.id);
                println!(
                    "Loops: {} created, {} updated, {} removed.",
                    loops.created, loops.updated, loops.removed
                );
                for skip in &loops.skipped {
                    println!("  skipped {}: {}", skip.loop_name, skip.reason);
                }
            }
        }
        RepoAction::Delete { repo_id, yes } => {
            let repo = client.api().get_repository(&repo_id).await?;
            let name = repo.name.clone();
            let id = repo.id.clone();

            if !yes {
                use std::io::{IsTerminal, Write};
                if !std::io::stdin().is_terminal() {
                    return Err(ClientError::Other(
                        "Refusing to delete without --yes in non-interactive mode".to_string(),
                    ));
                }
                print!("Delete repository '{name}' ({id})? This cannot be undone. [y/N] ");
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

            client.api().delete_repository(&id).await?;
            if json {
                output::print_json_typed(&serde_json::json!({
                    "id": id,
                    "deleted": true,
                }));
            } else {
                println!("Deleted repository {name} ({id})");
            }
        }
    }
    Ok(())
}

/// Build a `ResourceConfig` from individual CLI flags.
/// Returns `None` if no resource flags were provided.
fn build_resource_config(
    cpu_request: Option<String>,
    cpu_limit: Option<String>,
    memory_request: Option<String>,
    memory_limit: Option<String>,
    storage: Option<String>,
    storage_mode: Option<String>,
) -> Option<ResourceConfig> {
    let has_any = cpu_request.is_some()
        || cpu_limit.is_some()
        || memory_request.is_some()
        || memory_limit.is_some()
        || storage.is_some()
        || storage_mode.is_some();

    if !has_any {
        return None;
    }

    let mut requests = HashMap::new();
    let mut limits = HashMap::new();

    if let Some(v) = cpu_request {
        requests.insert("cpu".to_string(), v);
    }
    if let Some(v) = memory_request {
        requests.insert("memory".to_string(), v);
    }
    if let Some(v) = cpu_limit {
        limits.insert("cpu".to_string(), v);
    }
    if let Some(v) = memory_limit {
        limits.insert("memory".to_string(), v);
    }

    Some(ResourceConfig {
        requests: if requests.is_empty() {
            None
        } else {
            Some(requests)
        },
        limits: if limits.is_empty() {
            None
        } else {
            Some(limits)
        },
        storage,
        storage_mode,
    })
}

fn parse_repo_kind(s: &str) -> RepositoryKind {
    match s {
        "template" => RepositoryKind::Template,
        "git" => RepositoryKind::Git,
        _ => RepositoryKind::Remote,
    }
}

fn parse_repo_provider(s: &str) -> Option<RepositoryProvider> {
    match s {
        "github" => Some(RepositoryProvider::Github),
        "gitlab" => Some(RepositoryProvider::Gitlab),
        "gitness" => Some(RepositoryProvider::Gitness),
        _ => None,
    }
}

fn parse_scope(s: &str) -> Option<Scope> {
    match s {
        "global" => Some(Scope::Global),
        "team" => Some(Scope::Team),
        "user" => Some(Scope::User),
        _ => None,
    }
}

fn format_repo_list(repos: &[RepositoryResponse]) {
    let rows: Vec<Vec<String>> = repos
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.name.clone(),
                r.provider
                    .as_ref()
                    .map_or("-".to_string(), ToString::to_string),
                r.default_branch.as_deref().unwrap_or("-").to_string(),
            ]
        })
        .collect();

    output::print_table(&["ID", "NAME", "PROVIDER", "BRANCH"], &rows);
}

/// Parse `KEY=VALUE` strings into a `HashMap`. Returns `None` if the list is empty.
fn parse_env_vars(raw: &[String]) -> Option<HashMap<String, String>> {
    if raw.is_empty() {
        return None;
    }
    let map: HashMap<String, String> = raw
        .iter()
        .filter_map(|entry| {
            let (k, v) = entry.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect();
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

fn format_repo_detail(r: &RepositoryResponse) {
    println!("ID:       {}", r.id);
    println!("Name:     {}", r.name);
    println!(
        "Provider: {}",
        r.provider
            .as_ref()
            .map_or("-".to_string(), ToString::to_string)
    );
    println!("Branch:   {}", r.default_branch.as_deref().unwrap_or("-"));
    println!("Git URL:  {}", r.remote_url.as_deref().unwrap_or("-"));
    if let Some(ref img) = r.docker_image {
        println!("Image:    {img}");
    }
    if let Some(ref res) = r.resources {
        if let Some(ref req) = res.requests {
            let parts: Vec<String> = req.iter().map(|(k, v)| format!("{k}={v}")).collect();
            println!("Resource requests: {}", parts.join(", "));
        }
        if let Some(ref lim) = res.limits {
            let parts: Vec<String> = lim.iter().map(|(k, v)| format!("{k}={v}")).collect();
            println!("Resource limits:   {}", parts.join(", "));
        }
        if let Some(ref storage) = res.storage {
            println!("Storage:           {storage}");
        }
        if let Some(ref mode) = res.storage_mode {
            println!("Storage mode:      {mode}");
        }
    }
    if let Some(ref servers) = r.servers {
        if !servers.is_empty() {
            println!("Servers:");
            for s in servers {
                println!("  {}:", s.name);
                println!("    Port:     {}", s.port);
                if let Some(ref cmds) = s.start_commands {
                    println!("    Commands: {}", cmds.join("; "));
                }
                if let Some(ref vars) = s.env_vars {
                    if !vars.is_empty() {
                        let parts: Vec<String> =
                            vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
                        println!("    Env:      {}", parts.join(", "));
                    }
                }
                if let Some(timeout) = s.startup_timeout_seconds {
                    println!("    Timeout:  {timeout}s");
                }
            }
        }
    }
    if let Some(ref cmds) = r.setup_commands {
        if !cmds.is_empty() {
            println!("Setup commands: {}", cmds.join("; "));
        }
    }
    if let Some(ref cmds) = r.server_start_commands {
        if !cmds.is_empty() {
            println!("Server start commands: {}", cmds.join("; "));
        }
    }
    if let Some(ref vars) = r.env_vars {
        if !vars.is_empty() {
            let parts: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
            println!("Env vars: {}", parts.join(", "));
        }
    }
    if let Some(timeout) = r.startup_timeout_seconds {
        println!("Startup timeout: {timeout}s");
    }
    if let Some(ref cmd) = r.build_command {
        println!("Build command: {cmd}");
    }
    if let Some(ref dir) = r.build_output_directory {
        println!("Build output dir: {dir}");
    }
    if let Some(ref desc) = r.description {
        println!("Description: {desc}");
    }
    if let Some(ref prompt) = r.system_prompt {
        let display = if prompt.len() > 120 {
            format!("{}…", &prompt[..120])
        } else {
            prompt.clone()
        };
        println!("System prompt: {display}");
    }
    println!("Scope:      {}", r.scope);
    if let Some(ref tid) = r.team_id {
        println!("Team ID:    {tid}");
    }
    if let Some(ref owner) = r.owner_user_id {
        println!("Owner:      {owner}");
    }
    if let Some(ref plugins) = r.plugins {
        if !plugins.is_empty() {
            println!("Plugins: {}", plugins.join(", "));
        }
    }
    if let Some(inspector) = r.enable_inspector {
        println!("Inspector:   {inspector}");
    }
    if let Some(reload) = r.auto_reload {
        println!("Auto-reload: {reload}");
    }
}
