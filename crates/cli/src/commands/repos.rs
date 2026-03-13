//! Repository commands: list, get, create, update.

use std::collections::HashMap;

use clap::Subcommand;
use moatless_api_schemas::enums::{RepositoryKind, RepositoryProvider};
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
        /// Docker image for sandbox (e.g. "myregistry/my-sandbox:latest")
        #[arg(long)]
        docker_image: Option<String>,
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
    },
    /// Sync .moatless/config.json from the remote repository
    SyncConfig { repo_id: String },
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
            docker_image,
        } => {
            let resources = build_resource_config(
                resource_cpu_request,
                resource_cpu_limit,
                resource_memory_request,
                resource_memory_limit,
            );
            let req = CreateRepositoryRequest {
                name,
                kind: parse_repo_kind(&kind),
                remote_url: Some(git_url),
                default_branch: Some(branch),
                provider: parse_repo_provider(&provider),
                auto_reload: None,
                build_command: None,
                build_output_directory: None,
                description: None,
                docker_image,
                enable_inspector: None,
                env_vars: None,
                plugins: None,
                server_start_commands: None,
                servers: None,
                setup_commands: None,
                startup_timeout_seconds: None,
                scope: None,
                system_prompt: None,
                team_id: None,
                visibility: None,
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
            docker_image,
            setup_commands,
            server_start_commands,
            env_vars,
            startup_timeout,
            servers_json,
        } => {
            let resources = build_resource_config(
                resource_cpu_request,
                resource_cpu_limit,
                resource_memory_request,
                resource_memory_limit,
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
                auto_reload: None,
                build_command: None,
                build_output_directory: None,
                description: None,
                docker_image,
                enable_inspector: None,
                env_vars: parse_env_vars(&env_vars),
                plugins: None,
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
                system_prompt: None,
                visibility: None,
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
            let updated = client.api().sync_config(&repo_id).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Config synced for repository {}.", updated.id);
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
) -> Option<ResourceConfig> {
    let has_any = cpu_request.is_some()
        || cpu_limit.is_some()
        || memory_request.is_some()
        || memory_limit.is_some();

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
    if map.is_empty() { None } else { Some(map) }
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
    println!(
        "Branch:   {}",
        r.default_branch.as_deref().unwrap_or("-")
    );
    println!(
        "Git URL:  {}",
        r.remote_url.as_deref().unwrap_or("-")
    );
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
}
