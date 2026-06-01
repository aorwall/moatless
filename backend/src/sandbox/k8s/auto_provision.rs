//! Auto-provisioning logic: reads task + repository from DB, resolves
//! credentials, and builds a `ProvisionRequest` so `start()` can
//! transparently provision a workspace that was never provisioned before.

use std::collections::HashMap;
use tracing::{info, warn};

use super::client::K8sClient;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::plugin_repo::dao::PluginRepoRepository;
use crate::repository::dao::RepoRepository;
use crate::repository::models::{Repository, RepositoryProvider, ResourceConfig};
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::{ProvisionPlan, ProvisionRequest, RepoConfig, ServerConfig};
use crate::task::dao::TaskRepository;
use crate::task::models::Task;

/// Build a `ProvisionRequest` from database state.
///
/// Steps:
/// 1. Read the task row to get repository_id + branch
/// 2. Read the repository row to get repo config
/// 3. Assemble the ProvisionRequest. Runtime credentials are fetched later
///    over gRPC by the sandbox.
pub async fn build_provision_request(
    tasks: &dyn TaskRepository,
    repos: &dyn RepoRepository,
    _provider_tokens: &dyn ProviderTokenRepository,
    plugin_repos: &dyn PluginRepoRepository,
    _k8s: &K8sClient,
    task_id: &str,
) -> Result<ProvisionRequest, SandboxError> {
    // 1. Read task from DB
    let task = tasks
        .find_by_id_internal(task_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("DB error reading task: {e}")))?
        .ok_or_else(|| SandboxError::Internal(format!("Task '{task_id}' not found in database")))?;

    info!(
        task_id = %task_id,
        repository_id = %task.repository_id,
        "Auto-provision: task found in DB"
    );

    // 2. Read repository from DB
    let repo = repos
        .find_repository_by_id(&task.repository_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("DB error reading repository: {e}")))?
        .ok_or_else(|| {
            SandboxError::Internal(format!(
                "Repository '{}' not found in database",
                task.repository_id
            ))
        })?;

    let git_provider = repo.provider.unwrap_or(RepositoryProvider::Github);

    let git_clone_url = repo.remote_url.clone().unwrap_or_default();
    // Read before any partial move of `task` below.
    let disabled = parse_disabled_servers(&task);
    let git_branch = task.branch.or(repo.default_branch.clone());

    let mut repo_config = build_repo_config(&repo);
    let env_vars = parse_env_vars(&repo);

    // Resolve plugin git repos from the join table
    match plugin_repos
        .find_by_repository_id(&task.repository_id)
        .await
    {
        Ok(plugins) if !plugins.is_empty() => {
            let urls: Vec<String> = plugins.into_iter().map(|p| p.git_url).collect();
            info!(
                task_id = %task_id,
                count = urls.len(),
                "Auto-provision: resolved plugin git repos"
            );
            repo_config.plugin_repos = Some(urls);
        }
        Ok(_) => {}
        Err(e) => {
            warn!(
                task_id = %task_id,
                error = %e,
                "Auto-provision: failed to query plugin repos (non-fatal)"
            );
        }
    }

    // Drop servers this task previously auto-disabled (e.g. a crash-looping
    // preview server). Persisted per-task so it survives stop/start.
    if !disabled.is_empty() {
        info!(task_id = %task_id, ?disabled, "Auto-provision: filtering disabled servers");
        filter_disabled_servers(&mut repo_config, &disabled);
    }

    let plan = ProvisionPlan {
        repo_config,
        git_clone_url,
        git_branch,
        git_provider: git_provider.as_str().to_string(),
        env_vars,
    };

    let user_id = task.created_by.unwrap_or_else(|| "system".to_string());

    Ok(ProvisionRequest { plan, user_id })
}

/// Parse the per-task `disabled_servers` JSONB column into a list of names.
///
/// Returns an empty vec when the column is null or not a JSON string array.
pub fn parse_disabled_servers(task: &Task) -> Vec<String> {
    task.disabled_servers
        .as_ref()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default()
}

/// Remove auto-disabled servers from a `RepoConfig` in place.
///
/// If filtering would drop the `default` server, a remaining server is
/// promoted to default so the sandbox still has a primary preview target.
pub fn filter_disabled_servers(repo_config: &mut RepoConfig, disabled: &[String]) {
    if disabled.is_empty() {
        return;
    }
    if let Some(servers) = repo_config.servers.as_mut() {
        servers.retain(|s| !disabled.contains(&s.name));
        if servers.is_empty() {
            repo_config.servers = None;
        } else if !servers.iter().any(|s| s.default) {
            if let Some(first) = servers.first_mut() {
                first.default = true;
            }
        }
    }
}

/// Build a `RepoConfig` from the database repository row.
pub fn build_repo_config(repo: &Repository) -> RepoConfig {
    let servers = build_servers(repo);
    let plugin_ids = parse_json_string_array(&repo.plugins);

    // Build preview_config pass-through
    let preview_config = build_preview_config(repo);

    // Build code_execution pass-through
    let code_execution = build_code_execution(repo);

    // Parse per-repo resources from JSONB
    let resources: Option<ResourceConfig> = repo
        .resources
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    RepoConfig {
        name: repo.name.clone(),
        docker_image: repo.docker_image.clone(),
        servers: if servers.is_empty() {
            None
        } else {
            Some(servers)
        },
        plugin_ids,
        system_prompt: repo.system_prompt.clone(),
        code_execution,
        preview_config,
        resources,
        plugin_repos: None,
        docker_access: None,
        docker_networks: None,
    }
}

/// Public accessor for building server configs from a repository row.
///
/// Used by `provider.rs` as a fallback when gRPC is unreachable —
/// derives server entries from DB so the frontend can display them.
pub fn build_server_configs(repo: &Repository) -> Vec<ServerConfig> {
    build_servers(repo)
}

/// Get the canonical server configs for a task.
///
/// Resolves task_id → task → repository → build_server_configs (with port assignment).
/// Returns Err on DB failures or missing rows so callers can log appropriately.
pub async fn get_server_configs(
    tasks: &dyn TaskRepository,
    repos: &dyn RepoRepository,
    task_id: &str,
) -> Result<Vec<ServerConfig>, String> {
    let task = tasks
        .find_by_id_internal(task_id)
        .await
        .map_err(|e| format!("Failed to fetch task {task_id}: {e}"))?
        .ok_or_else(|| format!("Task {task_id} not found"))?;
    let repo = repos
        .find_repository_by_id(&task.repository_id)
        .await
        .map_err(|e| format!("Failed to fetch repository {}: {e}", task.repository_id))?
        .ok_or_else(|| format!("Repository {} not found", task.repository_id))?;
    Ok(build_server_configs(&repo))
}

/// Build the servers list from the DB `servers` JSONB column.
///
/// If the `servers` JSONB column contains entries, those are used as-is
/// (data-driven). If empty or missing, a backwards-compatible "preview"
/// server is injected when the repository has start commands. The injected
/// server has `docker_image: None` so it falls through to the global
/// sandbox image at pod-build time.
fn build_servers(repo: &Repository) -> Vec<ServerConfig> {
    let mut servers: Vec<ServerConfig> = repo
        .servers
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Only inject defaults when servers list is empty (old repos without server configs)
    if servers.is_empty() {
        // Add preview server if start commands exist
        if repo.server_start_commands.is_some() {
            let start_commands = repo
                .server_start_commands
                .as_ref()
                .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok());
            servers.push(ServerConfig {
                name: "preview".to_string(),
                label: Some("Preview".to_string()),
                default: true,
                enable_inspector: repo.enable_inspector.unwrap_or(false),
                start_commands,
                docker_image: None,
                ..Default::default()
            });
        }
    }

    // Ensure at least one server is marked default
    if !servers.iter().any(|s| s.default) {
        if let Some(s) = servers.iter_mut().last() {
            s.default = true;
        }
    }

    // Assign sequential ports (starting at 3000) to servers missing one
    assign_missing_ports(&mut servers);

    servers
}

/// Assign sequential ports starting at 3000 to servers that have port 0.
fn assign_missing_ports(servers: &mut [ServerConfig]) {
    let used_ports: std::collections::HashSet<u16> = servers
        .iter()
        .filter(|s| s.port != 0)
        .map(|s| s.port)
        .collect();

    let mut next_port: u16 = 3000;
    let allocate = |next: &mut u16, used: &std::collections::HashSet<u16>| -> u16 {
        while used.contains(next) {
            *next += 1;
        }
        let port = *next;
        *next += 1;
        port
    };

    // Need to track newly-allocated ports too
    let mut all_used = used_ports;
    for server in servers.iter_mut() {
        if server.port == 0 {
            let port = allocate(&mut next_port, &all_used);
            all_used.insert(port);
            server.port = port;
        }
    }
}

/// Parse the `env_vars` JSONB column into a HashMap.
pub fn parse_env_vars(repo: &Repository) -> Option<HashMap<String, String>> {
    repo.env_vars
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Parse a JSONB value that should be an array of strings.
fn parse_json_string_array(value: &Option<serde_json::Value>) -> Option<Vec<String>> {
    value
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Build preview_config JSON pass-through from flattened DB columns.
fn build_preview_config(repo: &Repository) -> Option<serde_json::Value> {
    let has_any = repo.server_start_commands.is_some()
        || repo.build_command.is_some()
        || repo.build_output_directory.is_some()
        || repo.enable_inspector.is_some()
        || repo.startup_timeout_seconds.is_some()
        || repo.auto_reload.is_some();

    if !has_any {
        return None;
    }

    let mut config = serde_json::Map::new();

    if let Some(ref cmds) = repo.server_start_commands {
        config.insert("serverStartCommands".to_string(), cmds.clone());
    }
    if let Some(ref cmd) = repo.build_command {
        config.insert(
            "buildCommand".to_string(),
            serde_json::Value::String(cmd.clone()),
        );
    }
    if let Some(ref dir) = repo.build_output_directory {
        config.insert(
            "buildOutputDirectory".to_string(),
            serde_json::Value::String(dir.clone()),
        );
    }
    if let Some(inspector) = repo.enable_inspector {
        config.insert(
            "enableInspector".to_string(),
            serde_json::Value::Bool(inspector),
        );
    }
    if let Some(timeout) = repo.startup_timeout_seconds {
        config.insert(
            "startupTimeoutSeconds".to_string(),
            serde_json::Value::Number(serde_json::Number::from(timeout)),
        );
    }
    if let Some(reload) = repo.auto_reload {
        config.insert("autoReload".to_string(), serde_json::Value::Bool(reload));
    }

    Some(serde_json::Value::Object(config))
}

/// Build code_execution JSON pass-through from flattened DB columns.
fn build_code_execution(repo: &Repository) -> Option<serde_json::Value> {
    let has_any =
        repo.setup_commands.is_some() || repo.docker_image.is_some() || repo.env_vars.is_some();

    if !has_any {
        return None;
    }

    let mut config = serde_json::Map::new();

    if let Some(ref cmds) = repo.setup_commands {
        config.insert("setupCommands".to_string(), cmds.clone());
    }
    if let Some(ref image) = repo.docker_image {
        config.insert(
            "dockerImage".to_string(),
            serde_json::Value::String(image.clone()),
        );
    }
    if let Some(ref vars) = repo.env_vars {
        config.insert("envVars".to_string(), vars.clone());
    }

    Some(serde_json::Value::Object(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, default: bool) -> ServerConfig {
        ServerConfig {
            name: name.to_string(),
            default,
            ..Default::default()
        }
    }

    fn repo_config_with(servers: Vec<ServerConfig>) -> RepoConfig {
        RepoConfig {
            name: "repo".to_string(),
            docker_image: None,
            servers: Some(servers),
            plugin_ids: None,
            system_prompt: None,
            code_execution: None,
            preview_config: None,
            resources: None,
            plugin_repos: None,
            docker_access: None,
            docker_networks: None,
        }
    }

    #[test]
    fn filter_drops_disabled_non_default_server() {
        let mut rc = repo_config_with(vec![server("storybook", false), server("dev", true)]);
        filter_disabled_servers(&mut rc, &["storybook".to_string()]);

        let servers = rc.servers.unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "dev");
        assert!(servers[0].default);
    }

    #[test]
    fn filter_promotes_new_default_when_default_disabled() {
        let mut rc = repo_config_with(vec![server("storybook", true), server("dev", false)]);
        filter_disabled_servers(&mut rc, &["storybook".to_string()]);

        let servers = rc.servers.unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "dev");
        assert!(
            servers[0].default,
            "remaining server should be promoted to default"
        );
    }

    #[test]
    fn filter_clears_servers_when_all_disabled() {
        let mut rc = repo_config_with(vec![server("only", true)]);
        filter_disabled_servers(&mut rc, &["only".to_string()]);
        assert!(rc.servers.is_none());
    }

    #[test]
    fn filter_noop_when_disabled_empty() {
        let mut rc = repo_config_with(vec![server("dev", true)]);
        filter_disabled_servers(&mut rc, &[]);
        assert_eq!(rc.servers.unwrap().len(), 1);
    }
}
