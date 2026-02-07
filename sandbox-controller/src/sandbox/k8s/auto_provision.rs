//! Auto-provisioning logic: reads task + repository from DB, resolves
//! credentials, and builds a `ProvisionRequest` so `start()` can
//! transparently provision a workspace that was never provisioned before.

use std::collections::HashMap;
use std::env;
use tracing::{info, warn};

use super::client::K8sClient;
use super::secret;
use crate::db::models::RepositoryRow;
use crate::db::{provider_token_repo, repository_repo, task_repo, DbPool};
use crate::sandbox::error::SandboxError;
use crate::sandbox::types::{ProvisionPlan, ProvisionRequest, RepoConfig, ServerConfig};

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
    let task = task_repo::find_task_by_id(pool, task_id)
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
    let repo = repository_repo::find_repository_by_id(pool, &task.repository_id)
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
/// 1. Read from existing K8s Secret (from a previous provision)
/// 2. Query `provider_tokens` table (user-linked tokens)
/// 3. Fall back to environment variables
async fn resolve_credentials(
    k8s: &K8sClient,
    task_id: &str,
    pool: &DbPool,
    user_id: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    // Step 1: Try K8s Secret first
    let k8s_creds = secret::read_task_credentials(k8s, task_id).await;
    let mut git_token = k8s_creds.as_ref().and_then(|c| c.github_token.clone());
    let mut anthropic = k8s_creds.as_ref().and_then(|c| c.anthropic_api_key.clone());
    let mut claude = k8s_creds.as_ref().and_then(|c| c.claude_oauth_token.clone());

    // Step 2: Fill gaps from provider_tokens table
    if let Some(uid) = user_id {
        if git_token.is_none() {
            match provider_token_repo::find_provider_token(pool, uid, "github").await {
                Ok(Some(row)) => {
                    info!(task_id = %task_id, "Auto-provision: GitHub token resolved from provider_tokens");
                    git_token = Some(row.access_token);
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "Auto-provision: failed to query provider_tokens for github");
                }
            }
        }

        if anthropic.is_none() && claude.is_none() {
            match provider_token_repo::find_provider_token(pool, uid, "claude").await {
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
    }

    // Step 3: Fill remaining gaps from environment variables
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
fn build_repo_config(repo: &RepositoryRow) -> RepoConfig {
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

/// Build the servers list from the DB `servers` JSONB column.
///
/// Mirrors the TS `autoPopulateServers` + `assignMissingPorts` logic:
/// - Always includes a "files" entry on port 4000 if not present
/// - Auto-populates "preview" from previewConfig if not present
/// - Assigns sequential ports (starting at 3000) to servers without one
fn build_servers(repo: &RepositoryRow) -> Vec<ServerConfig> {
    let mut servers: Vec<ServerConfig> = repo
        .servers
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Always include file explorer if not present (port 4000)
    if !servers.iter().any(|s| s.name == "files") {
        servers.insert(
            0,
            ServerConfig {
                name: "files".to_string(),
                port: 4000,
            },
        );
    }

    // Auto-populate preview from previewConfig if no preview server exists
    if !servers.iter().any(|s| s.name == "preview")
        && repo.server_start_commands.is_some() {
            servers.push(ServerConfig {
                name: "preview".to_string(),
                port: 0, // Will be assigned below
            });
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
fn parse_env_vars(repo: &RepositoryRow) -> Option<HashMap<String, String>> {
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
fn build_preview_config(repo: &RepositoryRow) -> Option<serde_json::Value> {
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
fn build_code_execution(repo: &RepositoryRow) -> Option<serde_json::Value> {
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
