//! Auto-provisioning logic: reads task + repository from DB, resolves
//! credentials, and builds a `ProvisionRequest` so `start()` can
//! transparently provision a workspace that was never provisioned before.

use std::collections::HashMap;
use std::env;
use tracing::{info, warn};

use super::client::K8sClient;
use crate::auth::provider_token_dao;
use crate::db::DbPool;
use crate::repository::dao as repository_dao;
use crate::repository::models::Repository;
use crate::task::dao as task_dao;
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::{ProvisionPlan, ProvisionRequest, RepoConfig, ServerConfig};

/// Build a `ProvisionRequest` from database state + recovered credentials.
///
/// Steps:
/// 1. Read the task row to get repository_id + branch
/// 2. Read the repository row to get repo config
/// 3. Resolve credentials (K8s Secret → env var fallback)
/// 4. Assemble the ProvisionRequest
pub async fn build_provision_request(
    pool: &DbPool,
    k8s: &K8sClient,
    task_id: &str,
) -> Result<ProvisionRequest, SandboxError> {
    // 1. Read task from DB
    let task = task_dao::find_by_id_internal(pool, task_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("DB error reading task: {e}")))?
        .ok_or_else(|| {
            SandboxError::Internal(format!("Task '{task_id}' not found in database"))
        })?;

    info!(
        task_id = %task_id,
        repository_id = %task.repository_id,
        "Auto-provision: task found in DB"
    );

    // 2. Read repository from DB
    let repo = repository_dao::find_repository_by_id(pool, &task.repository_id)
        .await
        .map_err(|e| SandboxError::Internal(format!("DB error reading repository: {e}")))?
        .ok_or_else(|| {
            SandboxError::Internal(format!(
                "Repository '{}' not found in database",
                task.repository_id
            ))
        })?;

    // 3. Resolve credentials
    let (git_provider_token, anthropic_api_key, claude_oauth_token) =
        resolve_credentials(k8s, task_id, pool, task.created_by.as_deref()).await;

    // Ensure we have at least some auth
    if anthropic_api_key.is_none() && claude_oauth_token.is_none() {
        warn!(
            task_id = %task_id,
            "Auto-provision: no Anthropic API key or Claude OAuth token available"
        );
    }

    // 4. Build the ProvisionRequest
    let git_clone_url = repo.remote_url.clone().unwrap_or_default();
    let git_branch = task.branch.or(repo.default_branch.clone());

    let repo_config = build_repo_config(&repo);
    let env_vars = parse_env_vars(&repo);

    let plan = ProvisionPlan {
        repo_config,
        git_clone_url,
        git_branch,
        git_provider_token,
        claude_oauth_token,
        anthropic_api_key,
        env_vars,
    };

    let user_id = task.created_by.unwrap_or_else(|| "system".to_string());

    Ok(ProvisionRequest { plan, user_id })
}

/// Resolve credentials with fallback chain:
/// 1. Query `provider_tokens` table (user-linked tokens)
/// 2. Fall back to environment variables
///
/// Note: K8s Secret reading was removed — credentials are now fetched
/// on-demand by the sandbox via gRPC `GetCredentials`.
async fn resolve_credentials(
    _k8s: &K8sClient,
    task_id: &str,
    pool: &DbPool,
    user_id: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut git_token: Option<String> = None;
    let mut anthropic: Option<String> = None;
    let mut claude: Option<String> = None;

    // Step 1: Fill from provider_tokens table
    if let Some(uid) = user_id {
        match provider_token_dao::find_provider_token(pool, uid, "github").await {
            Ok(Some(row)) => {
                info!(task_id = %task_id, "Auto-provision: GitHub token resolved from provider_tokens");
                git_token = Some(row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Auto-provision: failed to query provider_tokens for github");
            }
        }

        match provider_token_dao::find_provider_token(pool, uid, "claude").await {
            Ok(Some(row)) => {
                info!(task_id = %task_id, "Auto-provision: Claude token resolved from provider_tokens");
                claude = Some(row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Auto-provision: failed to query provider_tokens for claude");
            }
        }
    }

    // Step 2: Fill remaining gaps from environment variables
    if git_token.is_none() {
        git_token = env::var("GITHUB_PAT").ok();
    }
    if anthropic.is_none() {
        anthropic = env::var("ANTHROPIC_API_KEY").ok();
    }
    if claude.is_none() {
        claude = env::var("CLAUDE_CODE_OAUTH_TOKEN").ok();
    }

    info!(
        task_id = %task_id,
        has_git_token = git_token.is_some(),
        has_anthropic = anthropic.is_some(),
        has_claude_oauth = claude.is_some(),
        "Auto-provision: credential resolution complete"
    );

    (git_token, anthropic, claude)
}

/// Build a `RepoConfig` from the database repository row.
fn build_repo_config(repo: &Repository) -> RepoConfig {
    let servers = build_servers(repo);
    let plugin_ids = parse_json_string_array(&repo.plugins);

    // Build preview_config pass-through
    let preview_config = build_preview_config(repo);

    // Build code_execution pass-through
    let code_execution = build_code_execution(repo);

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
pub async fn get_server_configs(pool: &DbPool, task_id: &str) -> Result<Vec<ServerConfig>, String> {
    let task = task_dao::find_by_id_internal(pool, task_id)
        .await
        .map_err(|e| format!("Failed to fetch task {task_id}: {e}"))?
        .ok_or_else(|| format!("Task {task_id} not found"))?;
    let repo = repository_dao::find_repository_by_id(pool, &task.repository_id)
        .await
        .map_err(|e| format!("Failed to fetch repository {}: {e}", task.repository_id))?
        .ok_or_else(|| format!("Repository {} not found", task.repository_id))?;
    Ok(build_server_configs(&repo))
}

/// Build the servers list from the DB `servers` JSONB column.
///
/// If the `servers` JSONB column contains entries, those are used as-is
/// (data-driven). If empty or missing, backwards-compatible defaults are
/// injected: a "files" server and optionally a "preview" server.
fn build_servers(repo: &Repository) -> Vec<ServerConfig> {
    let mut servers: Vec<ServerConfig> = repo
        .servers
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Only inject defaults when servers list is empty (old repos without server configs)
    if servers.is_empty() {
        // Always add file explorer
        servers.push(ServerConfig {
            name: "files".to_string(),
            label: Some("Files".to_string()),
            port: 4000,
            docker_image: Some("aorwall/moatless-vibe-files-server:latest".to_string()),
            env_vars: Some(HashMap::from([
                ("FILE_EXPLORER_PORT".into(), "4000".into()),
            ])),
            ..Default::default()
        });

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
                docker_image: Some(
                    "aorwall/moatless-vibe-preview-server:latest".to_string(),
                ),
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
fn parse_env_vars(repo: &Repository) -> Option<HashMap<String, String>> {
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
        config.insert(
            "autoReload".to_string(),
            serde_json::Value::Bool(reload),
        );
    }

    Some(serde_json::Value::Object(config))
}

/// Build code_execution JSON pass-through from flattened DB columns.
fn build_code_execution(repo: &Repository) -> Option<serde_json::Value> {
    let has_any = repo.setup_commands.is_some()
        || repo.docker_image.is_some()
        || repo.env_vars.is_some();

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
