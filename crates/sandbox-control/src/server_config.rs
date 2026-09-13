//! Preview server configuration and restart operations.

use super::{
    access::{authorize_task_control, authorize_task_read, authorize_task_write},
    lifecycle::TaskIntent,
    state::SandboxApiState,
};
use moatless_sandbox_runtime::{
    error::SandboxError,
    schemas::{
        ServerConfigResponse, ServerOverrideEntry, ServerOverridesResponse,
        UpdateServerConfigRequest,
    },
};
use moatless_types::{access_control::ScopeContext, workspace::ServerOverride};
use tracing::info;

/// List server config after caller authorization.
pub async fn list_server_config(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: String,
) -> Result<ServerOverridesResponse, SandboxError> {
    authorize_task_read(state, scope, &task_id).await?;
    let client = state.connector.sandbox_client(&task_id).await?;
    let overrides = client
        .get_server_config(&task_id)
        .await
        .map_err(|e| moatless_sandbox_runtime::client::classify_grpc_error(e, "GetServerConfig"))?;

    Ok(ServerOverridesResponse {
        overrides: overrides
            .into_iter()
            .map(
                |(server_name, (config_override, generation))| ServerOverrideEntry {
                    server_name,
                    config_override,
                    generation,
                },
            )
            .collect(),
    })
}

/// Update server config after caller authorization.
pub async fn update_server_config(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: String,
    server_name: String,
    req: UpdateServerConfigRequest,
) -> Result<ServerConfigResponse, SandboxError> {
    info!(task_id = %task_id, server = %server_name, "PATCH /servers/{server_name}");
    let intent = authorize_task_write(state, scope, &task_id).await?;

    if req.port.is_some() {
        return Err(SandboxError::Conflict(
            "changing a server's port requires a redeploy — it is wired into the Service and \
             IngressRoute at provision time (POST /redeploy)"
                .into(),
        ));
    }
    if req.docker_image.is_some() {
        return Err(SandboxError::Conflict(
            "changing a server's image requires a redeploy (POST /redeploy)".into(),
        ));
    }

    require_known_server(state, &intent, &task_id, &server_name).await?;

    let client = state.connector.sandbox_client(&task_id).await?;
    let mut entry = read_override(&client, &task_id, &server_name).await?;

    if let Some(cmds) = req.start_commands {
        entry.start_command = if cmds.is_empty() {
            None
        } else {
            Some(cmds.join(" && "))
        };
    }
    if let Some(env) = req.env {
        for (key, value) in env {
            validate_env_key(&key)?;
            // A key can't be both exported and unset — the later `set` wins.
            entry.unset_env.retain(|k| k != &key);
            entry.env.insert(key, value);
        }
    }
    if let Some(unset) = req.unset_env {
        for key in unset {
            validate_env_key(&key)?;
            entry.env.remove(&key);
            if !entry.unset_env.contains(&key) {
                entry.unset_env.push(key);
            }
        }
    }

    // An emptied entry writes as "no override" — `apply` removes the control
    // files rather than leaving empty ones, so it has one representation.
    apply_override(&client, &task_id, &server_name, entry, req.restart).await
}

/// Restart server after caller authorization.
pub async fn restart_server(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: String,
    server_name: String,
) -> Result<ServerConfigResponse, SandboxError> {
    info!(task_id = %task_id, server = %server_name, "POST /servers/{server_name}/restart");
    let intent = authorize_task_control(state, scope, &task_id).await?;
    require_known_server(state, &intent, &task_id, &server_name).await?;

    // Re-send the override the sandbox already holds: `apply` is idempotent, and
    // the generation bump is the only part the sidecar reacts to.
    let client = state.connector.sandbox_client(&task_id).await?;
    let current = read_override(&client, &task_id, &server_name).await?;
    apply_override(&client, &task_id, &server_name, current, true).await
}

/// Reset server config after caller authorization.
pub async fn reset_server_config(
    state: &SandboxApiState,
    scope: &Option<ScopeContext>,
    task_id: String,
    server_name: String,
) -> Result<ServerConfigResponse, SandboxError> {
    info!(task_id = %task_id, server = %server_name, "DELETE /servers/{server_name}/override");
    let intent = authorize_task_write(state, scope, &task_id).await?;
    require_known_server(state, &intent, &task_id, &server_name).await?;

    // An empty override is exactly "reset": `apply` removes the control files.
    let client = state.connector.sandbox_client(&task_id).await?;
    apply_override(
        &client,
        &task_id,
        &server_name,
        ServerOverride::default(),
        true,
    )
    .await
}

/// Read one server's current override off the sandbox's workspace volume.
///
/// Absent means no override, which is the same value as an empty one — callers
/// merge into it either way.
async fn read_override(
    client: &moatless_sandbox_runtime::client::SandboxClient,
    task_id: &str,
    server_name: &str,
) -> Result<ServerOverride, SandboxError> {
    let mut overrides = client
        .get_server_config(task_id)
        .await
        .map_err(|e| moatless_sandbox_runtime::client::classify_grpc_error(e, "GetServerConfig"))?;
    Ok(overrides
        .remove(server_name)
        .map(|(config, _generation)| config)
        .unwrap_or_default())
}

/// Push one server's complete override to the sandbox, optionally restarting
/// it.
async fn apply_override(
    client: &moatless_sandbox_runtime::client::SandboxClient,
    task_id: &str,
    server_name: &str,
    effective: ServerOverride,
    restart: bool,
) -> Result<ServerConfigResponse, SandboxError> {
    let generation = client
        .apply_server_config(task_id, server_name, &effective, restart)
        .await
        .map_err(|e| {
            moatless_sandbox_runtime::client::classify_grpc_error(e, "ApplyServerConfig")
        })?;

    info!(
        task_id = %task_id,
        server = %server_name,
        generation,
        restart,
        "Applied live server config"
    );
    Ok(ServerConfigResponse {
        server_name: server_name.to_string(),
        config_override: effective,
        generation,
        restarted: restart,
    })
}

/// 404 unless the task actually has a server by this name.
///
/// Resolved from live status, which is config-first, so a server that has not
/// started yet still counts.
async fn require_known_server(
    state: &SandboxApiState,
    intent: &TaskIntent,
    task_id: &str,
    server_name: &str,
) -> Result<(), SandboxError> {
    let status = state
        .observer
        .sandbox_status(task_id, intent.status, intent.desired_state)
        .await?;
    if status.servers.iter().any(|s| s.name == server_name) {
        return Ok(());
    }
    Err(SandboxError::NotFound(format!(
        "no server '{server_name}' on task {task_id}; available: {}",
        status
            .servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Env keys must be shell-legal and must not collide with names the platform
/// controls — the runtime trio the container spec injects, plus the reserved
/// secret keys.
fn validate_env_key(key: &str) -> Result<(), SandboxError> {
    if !moatless_core::secret_keys::is_valid_secret_key(key) {
        return Err(SandboxError::BadRequest(format!(
            "invalid env key '{key}': expected ^[A-Z_][A-Z0-9_]*$"
        )));
    }
    if moatless_core::secret_keys::is_reserved_key(key)
        || matches!(
            key,
            "PORT" | "WORKSPACE_DIR" | "WORKSPACE_PATH" | "DATA_DIR"
        )
    {
        return Err(SandboxError::BadRequest(format!(
            "env key '{key}' is reserved by the platform"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_env_key;
    use moatless_sandbox_runtime::error::SandboxError;

    #[test]
    fn server_env_overrides_reject_runtime_paths() {
        for key in ["PORT", "WORKSPACE_DIR", "WORKSPACE_PATH", "DATA_DIR"] {
            assert!(
                matches!(validate_env_key(key), Err(SandboxError::BadRequest(_))),
                "{key} should be reserved"
            );
        }
        assert!(validate_env_key("EXTRA_FLAG").is_ok());
    }
}

// =============================================================================
// Authorization helpers
// =============================================================================
