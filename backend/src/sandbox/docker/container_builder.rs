//! Build Docker container configuration from a ProvisionPlan.
//!
//! Generates a `bollard::container::Config` with all the env vars,
//! labels, bind mounts, port mappings, and networking required for
//! a sandbox container running as a sibling container on a shared
//! Docker network.

use std::collections::HashMap;

use bollard::models::{
    HostConfig, Mount, MountTypeEnum, PortBinding, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::container::Config as ContainerConfig;

use super::naming::{generate_container_name, generate_data_host_path, generate_server_container_name, generate_workspace_host_path};
use crate::config::Config;
use crate::sandbox::models::{ProvisionPlan, ServerConfig};

/// Build a complete `bollard::container::Config` for a sandbox container.
///
/// The sandbox container runs all servers inside it (unlike K8s which uses
/// sidecar containers). Environment variables tell the sandbox binary how
/// to start servers, clone the repo, and connect back to the backend.
pub fn build_container_config(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    user_auth_token: Option<&str>,
) -> ContainerConfig<String> {
    let container_name = generate_container_name(task_id);

    // ── Image ──────────────────────────────────────────────────────
    let image = plan
        .repo_config
        .docker_image
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| config.docker.image.clone());

    // ── Environment variables ─────────────────────────────────────
    let mut env = vec![
        format!("TASK_ID={task_id}"),
        format!("GIT_CLONE_URL={}", plan.git_clone_url),
        format!(
            "BACKEND_GRPC_URL=http://{}:{}",
            config.docker.backend_hostname, config.server.grpc_port
        ),
    ];

    env.push(format!("GIT_PROVIDER={}", plan.git_provider));

    if let Some(ref branch) = plan.git_branch {
        env.push(format!("GIT_BRANCH={branch}"));
    }

    if let Some(ref token) = plan.git_provider_token {
        env.push(format!("GIT_TOKEN={token}"));
    }

    if let Some(ref token) = plan.claude_oauth_token {
        env.push(format!("CLAUDE_CODE_OAUTH_TOKEN={token}"));
    }

    if let Some(ref key) = plan.anthropic_api_key {
        env.push(format!("ANTHROPIC_API_KEY={key}"));
    }

    if let Some(token) = user_auth_token {
        env.push(format!("USER_AUTH_TOKEN={token}"));
    }

    // REPO_CONFIG env var (pass-through for sandbox)
    if let Some(repo_config_json) = build_repo_config_json(plan) {
        env.push(format!("REPO_CONFIG={repo_config_json}"));
    }

    // Custom env vars from repo config
    if let Some(ref env_vars) = plan.env_vars {
        for (key, value) in env_vars {
            env.push(format!("{key}={value}"));
        }
    }

    // ── Labels ─────────────────────────────────────────────────────
    let mut labels = HashMap::new();
    labels.insert("created-by".to_string(), "moatless".to_string());
    labels.insert(
        "moatless.service".to_string(),
        "sandbox".to_string(),
    );
    labels.insert(
        "moatless.io/task-id".to_string(),
        task_id.to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );

    // ── Exposed ports ─────────────────────────────────────────────
    // 8001: sandbox HTTP health, grpc_port: gRPC
    // bollard uses HashMap<String, HashMap<(), ()>> for exposed ports (Docker API convention)
    #[allow(clippy::zero_sized_map_values)]
    let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
    #[allow(clippy::zero_sized_map_values)]
    let empty_map: HashMap<(), ()> = HashMap::new();
    exposed_ports.insert("8001/tcp".to_string(), empty_map.clone());
    exposed_ports.insert(
        format!("{}/tcp", config.sandbox.grpc_port),
        empty_map.clone(),
    );

    // ── Port bindings (dynamic host port mapping) ─────────────────
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();

    // Map sandbox ports
    for port_key in exposed_ports.keys() {
        port_bindings.insert(
            port_key.clone(),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(String::new()), // empty = dynamic assignment
            }]),
        );
    }

    // ── Bind mounts ───────────────────────────────────────────────
    let workspace_host_path = generate_workspace_host_path(&config.docker.data_dir, task_id);
    let data_host_path = generate_data_host_path(&config.docker.data_dir, task_id);
    let mounts = vec![
        Mount {
            target: Some("/opt/moatless/repo".to_string()),
            source: Some(workspace_host_path),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        },
        Mount {
            target: Some("/opt/moatless/data".to_string()),
            source: Some(data_host_path),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        },
    ];

    // ── Host config ───────────────────────────────────────────────
    let host_config = HostConfig {
        network_mode: Some(config.docker.network.clone()),
        mounts: Some(mounts),
        port_bindings: Some(port_bindings),
        restart_policy: Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
            maximum_retry_count: None,
        }),
        ..Default::default()
    };

    // ── Container config ──────────────────────────────────────────
    ContainerConfig {
        image: Some(image),
        hostname: Some(container_name),
        env: Some(env),
        labels: Some(labels),
        exposed_ports: Some(exposed_ports),
        host_config: Some(host_config),
        cmd: Some(vec!["/usr/local/bin/sandbox".to_string()]),
        ..Default::default()
    }
}

/// Build REPO_CONFIG JSON string for the sandbox container.
///
/// This is a pass-through of the repo config fields that the sandbox
/// needs (system prompt, code execution, plugin IDs, etc.).
fn build_repo_config_json(plan: &ProvisionPlan) -> Option<String> {
    let rc = &plan.repo_config;
    if rc.system_prompt.is_none()
        && rc.code_execution.is_none()
        && rc.preview_config.is_none()
        && rc.servers.is_none()
        && rc.plugin_ids.is_none()
    {
        return None;
    }

    let mut map = serde_json::Map::new();
    if let Some(ref sp) = rc.system_prompt {
        map.insert("systemPrompt".into(), serde_json::json!(sp));
    }
    if let Some(ref ce) = rc.code_execution {
        map.insert("codeExecution".into(), ce.clone());
    }
    if let Some(ref pc) = rc.preview_config {
        map.insert("previewConfig".into(), pc.clone());
    }
    if let Some(ref servers) = rc.servers {
        let server_entries: Vec<serde_json::Value> = servers
            .iter()
            .map(|s| {
                let mut entry = serde_json::json!({"name": s.name, "port": s.port});
                if let Some(ref cmds) = s.start_commands {
                    entry["startCommands"] = serde_json::json!(cmds);
                }
                if let Some(ref env) = s.env_vars {
                    entry["envVars"] = serde_json::json!(env);
                }
                if let Some(ref img) = s.docker_image {
                    entry["dockerImage"] = serde_json::json!(img);
                }
                entry
            })
            .collect();
        map.insert("servers".into(), serde_json::json!(server_entries));
    }
    if let Some(ref pi) = rc.plugin_ids {
        map.insert("pluginIds".into(), serde_json::json!(pi));
    }

    Some(serde_json::Value::Object(map).to_string())
}

/// Default preview server image (matches K8s sidecar pattern).
const DEFAULT_SERVER_IMAGE: &str = "aorwall/moatless-vibe-preview-server:latest";

/// Build a `bollard::container::Config` for a server sidecar container.
///
/// Mirrors the K8s `build_server_containers()` pattern: each server config
/// produces a container that waits for the `.workspace-ready` sentinel,
/// then runs the start commands.
pub fn build_server_container_config(
    config: &Config,
    task_id: &str,
    server: &ServerConfig,
) -> ContainerConfig<String> {
    let container_name = generate_server_container_name(task_id, &server.name);

    // ── Image ──────────────────────────────────────────────────────
    let image = server
        .docker_image
        .clone()
        .unwrap_or_else(|| DEFAULT_SERVER_IMAGE.to_string());

    // ── Environment variables ─────────────────────────────────────
    let mut env = vec![
        format!("PORT={}", server.port),
        "WORKSPACE_PATH=/opt/moatless/repo".to_string(),
        "DATA_DIR=/opt/moatless/data".to_string(),
    ];

    if let Some(ref extra) = server.env_vars {
        for (k, v) in extra {
            env.push(format!("{k}={v}"));
        }
    }

    // ── Command ──────────────────────────────────────────────────
    // Wait for workspace-ready sentinel, then run start commands.
    // Mirrors K8s pod_builder inline wait pattern.
    let cmd = match &server.start_commands {
        Some(cmds) if !cmds.is_empty() => {
            let joined_cmd = cmds.join(" && ");
            Some(vec![
                "bash".to_string(),
                "-c".to_string(),
                format!(
                    "until [ -f /opt/moatless/data/.workspace-ready ]; do sleep 2; done && \
                     cd /opt/moatless/repo && {joined_cmd}"
                ),
            ])
        }
        _ => None,
    };

    // ── Labels ─────────────────────────────────────────────────────
    let mut labels = HashMap::new();
    labels.insert("created-by".to_string(), "moatless".to_string());
    labels.insert("moatless.service".to_string(), "server".to_string());
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());
    labels.insert(
        "moatless.io/server-name".to_string(),
        server.name.clone(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );

    // ── Exposed ports ─────────────────────────────────────────────
    #[allow(clippy::zero_sized_map_values)]
    let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
    let port_key = format!("{}/tcp", server.port);
    #[allow(clippy::zero_sized_map_values)]
    exposed_ports.insert(port_key.clone(), HashMap::new());

    // ── Port bindings (dynamic host port mapping) ─────────────────
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
    port_bindings.insert(
        port_key,
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: Some(String::new()), // empty = dynamic assignment
        }]),
    );

    // ── Bind mounts ───────────────────────────────────────────────
    let workspace_host_path = generate_workspace_host_path(&config.docker.data_dir, task_id);
    let data_host_path = generate_data_host_path(&config.docker.data_dir, task_id);
    let mounts = vec![
        Mount {
            target: Some("/opt/moatless/repo".to_string()),
            source: Some(workspace_host_path),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        },
        Mount {
            target: Some("/opt/moatless/data".to_string()),
            source: Some(data_host_path),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        },
    ];

    // ── Host config ───────────────────────────────────────────────
    let host_config = HostConfig {
        network_mode: Some(config.docker.network.clone()),
        mounts: Some(mounts),
        port_bindings: Some(port_bindings),
        restart_policy: Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
            maximum_retry_count: None,
        }),
        ..Default::default()
    };

    // ── Container config ──────────────────────────────────────────
    ContainerConfig {
        image: Some(image),
        hostname: Some(container_name),
        env: Some(env),
        labels: Some(labels),
        exposed_ports: Some(exposed_ports),
        host_config: Some(host_config),
        cmd,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::models::{ProvisionPlan, RepoConfig};

    fn test_config() -> Config {
        use crate::config::*;
        Config {
            server: crate::config::ServerConfig {
                port: 8080,
                metrics_port: 9090,
                grpc_port: 50051,
            },
            sandbox_provider: SandboxProviderType::Docker,
            sandbox: SandboxConfig {
                namespace: "test".into(),
                image: "sandbox:latest".into(),
                image_pull_policy: "Never".into(),
                grpc_port: 50052,
                resources_cpu_limit: "500m".into(),
                resources_memory_limit: "512Mi".into(),
                resources_cpu_request: "100m".into(),
                resources_memory_request: "128Mi".into(),
                storage_enabled: false,
                storage_size: "1Gi".into(),
                storage_class: None,
                pod_startup_timeout_secs: 10,
                task_token_secret: Some("secret".into()),
                file_mount_path: "/tmp".into(),
                idle_reaper_enabled: false,
                idle_timeout_minutes: 60,
                idle_check_interval_secs: 300,
                max_crash_restart_count: 5,
            },
            proxy: ProxyConfig {
                domain: None,
                protocol: "http".into(),
                traefik_auth_middleware_name: "test".into(),
                traefik_auth_middleware_namespace: "test".into(),
                traefik_entry_point: "web".into(),
            },
            docker: DockerConfig {
                network: "moatless".into(),
                image: "sandbox:latest".into(),
                data_dir: "/data".into(),
                container_data_dir: "/data".into(),
                backend_hostname: "moatless-backend".into(),
            },
            database_url: None,
            database: DatabaseConfig { url: None, max_connections: 5 },
            redis: RedisConfig { url: None, max_stream_length: 100 },
            sse: SseConfig { broadcast_capacity: 16, consumer_id: "test".into() },
            auth: AuthConfig {
                provider: None,
                github_client_id: None,
                github_client_secret: None,
                github_callback_url: None,
                github_oauth_scope: "".into(),
                auth0_domain: None,
                auth0_client_id: None,
                auth0_client_secret: None,
                auth0_callback_url: None,
                auth0_audience: None,
                auth0_groups_claim: "".into(),
                session_max_age_secs: 3600,
                frontend_url: "http://localhost:3000".into(),
                cookie_domain: None,
                session_cookie_name: "test".into(),
                is_production: false,
                cache_ttl_secs: 0,
            },
            s3: S3Config { bucket_name: None, key_prefix: "test".into(), region: "us-east-1".into() },
            slack: SlackConfig { bot_token: None, signing_secret: None, bot_user_id: None },
        }
    }

    fn test_plan() -> ProvisionPlan {
        ProvisionPlan {
            repo_config: RepoConfig {
                name: "test-repo".into(),
                docker_image: None,
                servers: None,
                plugin_ids: None,
                system_prompt: None,
                code_execution: None,
                preview_config: None,
            },
            git_clone_url: "https://github.com/test/repo.git".into(),
            git_branch: Some("main".into()),
            git_provider: "github".into(),
            git_provider_token: Some("gh_token".into()),
            claude_oauth_token: None,
            anthropic_api_key: Some("sk-ant-key".into()),
            env_vars: None,
        }
    }

    #[test]
    fn container_config_uses_default_image() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        assert_eq!(cc.image.unwrap(), "sandbox:latest");
    }

    #[test]
    fn container_config_uses_repo_image_override() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.docker_image = Some("custom:v1".into());
        let cc = build_container_config(&config, "task-1", &plan, None);
        assert_eq!(cc.image.unwrap(), "custom:v1");
    }

    #[test]
    fn container_config_includes_required_env_vars() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, Some("jwt-token"));
        let env = cc.env.unwrap();
        assert!(env.iter().any(|e| e == "TASK_ID=task-1"));
        assert!(env.iter().any(|e| e.starts_with("GIT_CLONE_URL=")));
        assert!(env.iter().any(|e| e.starts_with("BACKEND_GRPC_URL=")));
        assert!(env.iter().any(|e| e == "GIT_BRANCH=main"));
        assert!(env.iter().any(|e| e.starts_with("GIT_TOKEN=")));
        assert!(env.iter().any(|e| e.starts_with("ANTHROPIC_API_KEY=")));
        assert!(env.iter().any(|e| e == "USER_AUTH_TOKEN=jwt-token"));
    }

    #[test]
    fn container_config_has_moatless_labels() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let labels = cc.labels.unwrap();
        assert_eq!(labels.get("created-by").unwrap(), "moatless");
        assert_eq!(labels.get("moatless.io/task-id").unwrap(), "task-1");
    }

    #[test]
    fn container_config_exposes_ports() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let ports = cc.exposed_ports.unwrap();
        assert!(ports.contains_key("8001/tcp"));
        assert!(ports.contains_key("50052/tcp"));
    }

    #[test]
    fn container_config_has_bind_mounts() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let host_config = cc.host_config.unwrap();
        let mounts = host_config.mounts.unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].target.as_deref(), Some("/opt/moatless/repo"));
        assert_eq!(mounts[0].source.as_deref(), Some("/data/tasks/task-1"));
        assert_eq!(mounts[1].target.as_deref(), Some("/opt/moatless/data"));
        assert_eq!(mounts[1].source.as_deref(), Some("/data/tasks/task-1/data"));
    }

    #[test]
    fn container_config_uses_correct_network() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let host_config = cc.host_config.unwrap();
        assert_eq!(host_config.network_mode.unwrap(), "moatless");
    }

    // ── Server container config tests ──────────────────────────────

    fn test_server_config() -> ServerConfig {
        ServerConfig {
            name: "preview".into(),
            label: Some("Preview".into()),
            port: 3000,
            start_commands: Some(vec!["bun run dev".into()]),
            docker_image: None,
            env_vars: None,
            ..Default::default()
        }
    }

    #[test]
    fn server_config_uses_default_image() {
        let config = test_config();
        let server = test_server_config();
        let cc = build_server_container_config(&config, "task-1", &server);
        assert_eq!(cc.image.unwrap(), DEFAULT_SERVER_IMAGE);
    }

    #[test]
    fn server_config_uses_custom_image() {
        let config = test_config();
        let mut server = test_server_config();
        server.docker_image = Some("custom-server:v1".into());
        let cc = build_server_container_config(&config, "task-1", &server);
        assert_eq!(cc.image.unwrap(), "custom-server:v1");
    }

    #[test]
    fn server_config_has_correct_labels() {
        let config = test_config();
        let server = test_server_config();
        let cc = build_server_container_config(&config, "task-1", &server);
        let labels = cc.labels.unwrap();
        assert_eq!(labels.get("moatless.service").unwrap(), "server");
        assert_eq!(labels.get("moatless.io/task-id").unwrap(), "task-1");
        assert_eq!(labels.get("moatless.io/server-name").unwrap(), "preview");
    }

    #[test]
    fn server_config_has_workspace_ready_wait() {
        let config = test_config();
        let server = test_server_config();
        let cc = build_server_container_config(&config, "task-1", &server);
        let cmd = cc.cmd.unwrap();
        assert_eq!(cmd[0], "bash");
        assert_eq!(cmd[1], "-c");
        assert!(cmd[2].contains(".workspace-ready"));
        assert!(cmd[2].contains("bun run dev"));
    }

    #[test]
    fn server_config_exposes_server_port() {
        let config = test_config();
        let server = test_server_config();
        let cc = build_server_container_config(&config, "task-1", &server);
        let ports = cc.exposed_ports.unwrap();
        assert!(ports.contains_key("3000/tcp"));
    }

    #[test]
    fn server_config_has_bind_mounts() {
        let config = test_config();
        let server = test_server_config();
        let cc = build_server_container_config(&config, "task-1", &server);
        let host_config = cc.host_config.unwrap();
        let mounts = host_config.mounts.unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].target.as_deref(), Some("/opt/moatless/repo"));
        assert_eq!(mounts[1].target.as_deref(), Some("/opt/moatless/data"));
    }

    #[test]
    fn server_config_env_vars() {
        let config = test_config();
        let server = test_server_config();
        let cc = build_server_container_config(&config, "task-1", &server);
        let env = cc.env.unwrap();
        assert!(env.iter().any(|e| e == "PORT=3000"));
        assert!(env.iter().any(|e| e == "WORKSPACE_PATH=/opt/moatless/repo"));
        assert!(env.iter().any(|e| e == "DATA_DIR=/opt/moatless/data"));
    }
}
