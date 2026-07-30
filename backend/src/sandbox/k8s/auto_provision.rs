//! Workspace-sourced run-config builders for provisioning.
//!
//! A task's run-config comes from its workspace (`build_repo_config_from_workspace`
//! and friends). The repository row carries no run-config — it is only a git
//! connection.

use std::collections::HashMap;

use crate::repository::models::ResourceConfig;
use crate::sandbox::models::{RepoConfig, ServerConfig};
use crate::task::models::Task;
use crate::workspace::models::Workspace;

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
        filter_disabled_server_list(servers, disabled);
        if servers.is_empty() {
            repo_config.servers = None;
        }
    }
}

/// The list half of [`filter_disabled_servers`], shared with the status path so
/// what the UI lists matches what provisioning actually builds. Without it a
/// disabled server keeps a container-less entry that reads `starting` forever.
pub fn filter_disabled_server_list(servers: &mut Vec<ServerConfig>, disabled: &[String]) {
    if disabled.is_empty() {
        return;
    }
    servers.retain(|s| !disabled.contains(&s.name));
    if !servers.is_empty() && !servers.iter().any(|s| s.default) {
        if let Some(first) = servers.first_mut() {
            first.default = true;
        }
    }
}

/// Build a `RepoConfig` from a workspace row — the runtime config source once a
/// task references a workspace. Reads the workspace's JSONB run-config columns
/// (`resources`, `network_profile`, `servers`, …). The workspace has no
/// flattened preview columns, so `preview_config` is `None`; server entries come
/// from the workspace's `servers` JSONB (data-driven).
pub fn build_repo_config_from_workspace(ws: &Workspace) -> RepoConfig {
    let servers = build_server_configs_from_workspace(ws);
    let resources: Option<ResourceConfig> = ws
        .resources
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let code_execution = build_code_execution_from_workspace(ws);

    RepoConfig {
        name: ws.name.clone(),
        docker_image: ws.docker_image.clone(),
        servers: if servers.is_empty() {
            None
        } else {
            Some(servers)
        },
        system_prompt: ws.system_prompt.clone(),
        code_execution,
        preview_config: None,
        resources,
        plugins: None,
        docker_access: ws.docker_access,
        docker_networks: None,
        sandbox_service_account: ws.sandbox_service_account.clone(),
        network_profile: ws.network_profile.clone(),
        // Layered on separately by `auto_provision` — this is task state, not
        // workspace config.
    }
}

/// Build the server list from a workspace's `servers` JSONB column, ensuring a
/// default is elected and every server has a port assigned.
pub fn build_server_configs_from_workspace(ws: &Workspace) -> Vec<ServerConfig> {
    let mut servers: Vec<ServerConfig> = ws
        .servers
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Ensure at least one default among any present servers, then assign ports.
    if !servers.iter().any(|s| s.default) {
        if let Some(s) = servers.iter_mut().last() {
            s.default = true;
        }
    }
    assign_missing_ports(&mut servers);
    servers
}

/// Parse a workspace's `env_vars` JSONB column into a HashMap.
pub fn parse_env_vars_from_workspace(ws: &Workspace) -> Option<HashMap<String, String>> {
    ws.env_vars
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Build code_execution JSON pass-through from a workspace's run-config columns.
fn build_code_execution_from_workspace(ws: &Workspace) -> Option<serde_json::Value> {
    let has_any = ws.setup_commands.is_some() || ws.docker_image.is_some() || ws.env_vars.is_some();
    if !has_any {
        return None;
    }

    let mut config = serde_json::Map::new();
    if let Some(ref cmds) = ws.setup_commands {
        config.insert("setupCommands".to_string(), cmds.clone());
    }
    if let Some(ref image) = ws.docker_image {
        config.insert(
            "dockerImage".to_string(),
            serde_json::Value::String(image.clone()),
        );
    }
    if let Some(ref vars) = ws.env_vars {
        config.insert("envVars".to_string(), vars.clone());
    }
    Some(serde_json::Value::Object(config))
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
            system_prompt: None,
            code_execution: None,
            preview_config: None,
            resources: None,
            plugins: None,
            docker_access: None,
            docker_networks: None,
            sandbox_service_account: None,
            network_profile: None,
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

    /// The status path filters the bare list, and it has to drop the same
    /// entries provisioning does — otherwise a disabled server keeps a
    /// container-less entry the resolver reports as `starting` forever.
    #[test]
    fn list_filter_matches_repo_config_filter() {
        let mut servers = vec![server("storybook", true), server("dev", false)];
        filter_disabled_server_list(&mut servers, &["storybook".to_string()]);

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "dev");
        assert!(servers[0].default);
    }

    #[test]
    fn list_filter_empties_when_all_disabled() {
        let mut servers = vec![server("only", true)];
        filter_disabled_server_list(&mut servers, &["only".to_string()]);
        assert!(servers.is_empty());
    }
}
