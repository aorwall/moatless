//! Build Docker container configuration from a `ProvisionPlan`.
//!
//! Generates a `bollard::container::Config` with all the env vars,
//! labels, bind mounts, port mappings, and networking required for
//! a sandbox container running as a sibling container on a shared
//! Docker network.

use std::collections::HashMap;

use bollard::models::{
    ContainerCreateBody as ContainerConfig, HostConfig, Mount, MountType, PortBinding,
    RestartPolicy, RestartPolicyNameEnum,
};

use super::naming::{
    generate_claude_config_host_path, generate_container_name, generate_data_host_path,
    generate_sandbox_host_path, generate_server_container_name,
};
use moatless_core::config::Config;
use moatless_sandbox_runtime::provisioning::ProvisionPlan;
use moatless_sandbox_runtime::runtime::{OPERATION_GENERATION_LABEL, RuntimeOperationContext};
use moatless_sandbox_runtime::server_runner::build_server_start_script;
use moatless_types::workspace::{Memory, Port, ServerConfig, WORKSPACE_REPOS_ENV};

pub(crate) const CREATED_BY_LABEL: &str = "created-by";
pub(crate) const CREATED_BY_VALUE: &str = "moatless";
pub(crate) const SERVICE_LABEL: &str = "moatless.service";
pub(crate) const SANDBOX_SERVICE: &str = "sandbox";
pub(crate) const SERVER_SERVICE: &str = "server";

/// Build a complete `bollard::container::Config` for a sandbox container.
///
/// The sandbox container runs all servers inside it (unlike K8s which uses
/// sidecar containers). Environment variables tell the sandbox binary how
/// to start servers, clone the repo, and connect back to the backend.
/// Resolve the sandbox image for a plan: per-repo override → default docker image.
///
/// An empty override string is treated as unset. This is the single source of
/// truth for the "expected" image, so the start path can diff it against the
/// image baked into a stopped container.
pub fn resolve_sandbox_image(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .docker_image
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| config.docker.image.clone())
}

pub fn build_container_config(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    user_auth_token: Option<&str>,
) -> ContainerConfig {
    let container_name = generate_container_name(task_id);

    // ── Image ──────────────────────────────────────────────────────
    let image = resolve_sandbox_image(config, plan);

    // ── Environment variables ─────────────────────────────────────
    let mut env = vec![
        format!("TASK_ID={task_id}"),
        format!("SANDBOX_ID={task_id}"),
        format!("GIT_CLONE_URL={}", plan.git_clone_url),
        format!(
            "BACKEND_GRPC_URL=http://{}:{}",
            config.docker.backend_hostname, config.server.grpc_port
        ),
        format!(
            "BACKEND_API_URL=http://{}:{}",
            config.docker.backend_hostname, config.server.port
        ),
    ];

    env.push(format!("GIT_PROVIDER={}", plan.git_provider));

    if let Some(ref branch) = plan.git_branch {
        env.push(format!("GIT_BRANCH={branch}"));
    }

    // Where the sandbox writes a message's attached uploads. Passed explicitly
    // rather than left to the sandbox's own default because the backend renders
    // the same value into the prompt's "Saved to:" line: two independent
    // defaults would let the advertised path and the written path drift apart.
    env.push(format!("UPLOAD_DIR={}", config.sandbox.file_mount_path));

    // Every repository the workspace places in the sandbox, primary first. The
    // GIT_* trio above still describes the primary, so a sandbox image predating
    // multi-repo support keeps cloning it. URLs are passed verbatim like
    // GIT_CLONE_URL — local Docker remotes may embed their own credentials.
    if !plan.repos.is_empty() {
        match serde_json::to_string(&plan.repos) {
            Ok(json) => env.push(format!("{WORKSPACE_REPOS_ENV}={json}")),
            Err(e) => tracing::warn!(
                error = %e,
                "Failed to serialize WORKSPACE_REPOS; sandbox will clone the primary repo only"
            ),
        }
    }

    if let Some(token) = user_auth_token {
        env.push(format!("USER_AUTH_TOKEN={token}"));
    }

    // REPO_CONFIG env var (pass-through for sandbox)
    if let Some(repo_config_json) = build_repo_config_json(plan) {
        env.push(format!("REPO_CONFIG={repo_config_json}"));
    }

    // Proxy domain and protocol for constructing preview URLs
    if let Some(ref domain) = config.proxy.domain {
        env.push(format!("PROXY_DOMAIN={domain}"));
        env.push(format!("PROXY_PROTOCOL={}", config.proxy.protocol));
    }

    // Frontend URL for constructing task URLs in the agent system prompt
    env.push(format!("FRONTEND_URL={}", config.auth.frontend_url));

    // Default model for agent sessions
    if let Some(ref model) = config.sandbox.default_model {
        env.push(format!("DEFAULT_CLAUDE_CODE_MODEL={model}"));
    }
    if let Some(ref model) = config.sandbox.default_codex_model {
        env.push(format!("DEFAULT_CODEX_MODEL={model}"));
    }

    // Custom env vars from repo config
    if let Some(ref env_vars) = plan.env_vars {
        for (key, value) in env_vars {
            env.push(format!("{key}={value}"));
        }
    }

    // ── Labels ─────────────────────────────────────────────────────
    let mut labels = HashMap::new();
    labels.insert(CREATED_BY_LABEL.to_string(), CREATED_BY_VALUE.to_string());
    labels.insert(SERVICE_LABEL.to_string(), SANDBOX_SERVICE.to_string());
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );

    // ── Exposed ports ─────────────────────────────────────────────
    // 8001: sandbox HTTP health, grpc_port: gRPC
    // bollard's ContainerCreateBody takes exposed ports as a `Vec<String>`.
    let exposed_ports: Vec<String> = vec![
        "8001/tcp".to_string(),
        format!("{}/tcp", config.sandbox.grpc_port),
    ];

    // ── Port bindings (dynamic host port mapping) ─────────────────
    let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();

    // Map sandbox ports
    for port_key in &exposed_ports {
        port_bindings.insert(
            port_key.clone(),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(String::new()), // empty = dynamic assignment
            }]),
        );
    }

    // ── Bind mounts ───────────────────────────────────────────────
    let workspace_host_path = generate_sandbox_host_path(&config.docker.data_dir, task_id);
    let data_host_path = generate_data_host_path(&config.docker.data_dir, task_id);
    let claude_config_host_path =
        generate_claude_config_host_path(&config.docker.data_dir, task_id);
    let mut mounts = vec![
        Mount {
            target: Some("/opt/moatless/workspace".to_string()),
            source: Some(workspace_host_path),
            typ: Some(MountType::BIND),
            read_only: Some(false),
            ..Default::default()
        },
        Mount {
            target: Some("/opt/moatless/data".to_string()),
            source: Some(data_host_path),
            typ: Some(MountType::BIND),
            read_only: Some(false),
            ..Default::default()
        },
        Mount {
            target: Some("/opt/moatless/.claude".to_string()),
            source: Some(claude_config_host_path),
            typ: Some(MountType::BIND),
            read_only: Some(false),
            ..Default::default()
        },
    ];

    // ── Docker socket mount (opt-in) ────────────────────────────
    // Repo-level `docker_access` overrides global `enable_socket_mount`.
    let docker_access = plan
        .repo_config
        .docker_access
        .unwrap_or(config.docker.enable_socket_mount);
    if docker_access {
        mounts.push(Mount {
            target: Some("/var/run/docker.sock".to_string()),
            source: Some(config.docker.socket_path.clone()),
            typ: Some(MountType::BIND),
            read_only: Some(false),
            ..Default::default()
        });
    }

    // ── Host config ───────────────────────────────────────────────
    // Per-repo resource limits (K8s quantity strings, e.g. "8Gi" / "4"). The
    // K8s provider applies the whole ResourceConfig; Docker has no
    // requests/eviction concept, so only the limits map onto the container
    // cgroup; unset limits leave the container unconstrained.
    let (memory_limit, nano_cpus) = sandbox_resource_limits(plan);
    let host_config = HostConfig {
        network_mode: Some(config.docker.network.clone()),
        mounts: Some(mounts),
        port_bindings: Some(port_bindings),
        restart_policy: Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
            maximum_retry_count: None,
        }),
        memory: memory_limit,
        nano_cpus,
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

/// Map the plan's `resources.limits` to Docker cgroup limits: memory in bytes
/// and CPU in nanocpus. An unset limit yields `None` (unconstrained).
fn sandbox_resource_limits(plan: &ProvisionPlan) -> (Option<i64>, Option<i64>) {
    let limits = plan.repo_config.resources.as_ref().map(|r| &r.limits);
    let memory = limits
        .and_then(|l| l.memory.as_ref())
        .map(Memory::bytes)
        .filter(|b| *b > 0)
        .map(|b| b as i64);
    let nano_cpus = limits
        .and_then(|l| l.cpu.as_ref())
        .map(|cpu| cpu.millicores().saturating_mul(1_000_000))
        .filter(|n| *n > 0)
        .map(|n| n as i64);
    (memory, nano_cpus)
}

/// Stamp lifecycle's operation identity onto a sandbox container before
/// creation. Docker labels are immutable, so every replacing operation creates
/// a new container carrying its own generation.
pub fn stamp_operation_generation(config: &mut ContainerConfig, ctx: RuntimeOperationContext) {
    config.labels.get_or_insert_with(HashMap::new).insert(
        OPERATION_GENERATION_LABEL.to_string(),
        ctx.generation_label(),
    );
}

fn build_repo_config_json(plan: &ProvisionPlan) -> Option<String> {
    moatless_sandbox_runtime::repo_config_encoding::encode_repo_config(
        &plan.repo_config,
        |server| {
            let mut fields = serde_json::Map::new();
            if !server.start_commands.is_empty() {
                fields.insert(
                    "startCommands".into(),
                    serde_json::json!(server.start_commands),
                );
            }
            if !server.env_vars.is_empty() {
                fields.insert("envVars".into(), serde_json::json!(server.env_vars));
            }
            if let Some(image) = &server.docker_image {
                fields.insert("dockerImage".into(), serde_json::json!(image));
            }
            fields
        },
    )
}

/// Validate a server's configuration before we try to launch it.
///
/// Server containers reuse the sandbox image by default, whose ENTRYPOINT
/// runs the sandbox binary — useless for a server. So when no per-server
/// `docker_image` is set, we require `start_commands` to override the
/// entrypoint. If a custom `docker_image` is set, we trust it has its own
/// usable CMD.
///
/// A server also needs a port: the container publishes one, so a server that
/// reached the provider without an allocated port cannot be launched.
///
/// Returns the validated port, or a human-readable message on failure.
pub(super) fn validate_server_config(server: &ServerConfig) -> Result<u16, String> {
    let has_start_commands = !server.start_commands.is_empty();
    let has_per_server_image = server.docker_image.is_some();

    if !has_start_commands && !has_per_server_image {
        return Err(format!(
            "Server '{}' has no start_commands and no docker_image override. \
             Set repositories.servers[].startCommands (camelCase) in the repo config, \
             or specify a docker_image with a baked-in CMD.",
            server.name
        ));
    }

    server.port.map(Port::get).ok_or_else(|| {
        format!(
            "Server '{}' has no allocated port; declare one in the repo config.",
            server.name
        )
    })
}

/// Build a `bollard::container::Config` for a server sidecar container.
///
/// Mirrors the K8s `build_server_containers()` pattern: each server config
/// produces a container that waits for the `.workspace-ready` sentinel,
/// then runs the start commands.
pub fn build_server_container_config(
    config: &Config,
    task_id: &str,
    server: &ServerConfig,
    port: u16,
    sandbox_image: &str,
) -> ContainerConfig {
    let container_name = generate_server_container_name(task_id, &server.name);

    // ── Image: per-server override → sandbox image fallback.
    // Treat empty strings as unset so cleared-but-non-null DB values fall through.
    let image = server
        .docker_image
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| sandbox_image.to_string());

    // ── Environment variables ─────────────────────────────────────
    let mut env = vec![
        format!("PORT={port}"),
        "WORKSPACE_DIR=/opt/moatless/workspace".to_string(),
        "WORKSPACE_PATH=/opt/moatless/workspace".to_string(),
        "DATA_DIR=/opt/moatless/data".to_string(),
    ];
    // Note: server.env (sourced below at startup) overrides WORKSPACE_DIR and
    // WORKSPACE_PATH with sandbox-derived paths; these defaults only apply
    // before init.

    for (k, v) in &server.env_vars {
        env.push(format!("{k}={v}"));
    }

    // ── Command ──────────────────────────────────────────────────
    // The supervisor script waits for the workspace-ready sentinel, sources the
    // shared env files, and restarts its child on a live config override — see
    // `server_runner`.
    //
    // This must be set as the container *entrypoint*: Docker `Cmd` only
    // supplies arguments to the image ENTRYPOINT, and the sandbox image's
    // docker-entrypoint-sandbox.sh ignores its args and execs the sandbox
    // binary (which crash-loops without BACKEND_GRPC_URL). K8s `command:`
    // overrides the entrypoint — mirror that here with `entrypoint`.
    let entrypoint = (!server.start_commands.is_empty()).then(|| {
        vec![
            "bash".to_string(),
            "-c".to_string(),
            build_server_start_script(&server.name, &server.start_commands),
        ]
    });

    // ── Labels ─────────────────────────────────────────────────────
    let mut labels = HashMap::new();
    labels.insert("created-by".to_string(), "moatless".to_string());
    labels.insert(SERVICE_LABEL.to_string(), SERVER_SERVICE.to_string());
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());
    labels.insert("moatless.io/server-name".to_string(), server.name.clone());
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );

    // ── Exposed ports ─────────────────────────────────────────────
    let port_key = format!("{port}/tcp");
    let exposed_ports: Vec<String> = vec![port_key.clone()];

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
    let workspace_host_path = generate_sandbox_host_path(&config.docker.data_dir, task_id);
    let data_host_path = generate_data_host_path(&config.docker.data_dir, task_id);
    let mounts = vec![
        Mount {
            target: Some("/opt/moatless/workspace".to_string()),
            source: Some(workspace_host_path),
            typ: Some(MountType::BIND),
            read_only: Some(false),
            ..Default::default()
        },
        Mount {
            target: Some("/opt/moatless/data".to_string()),
            source: Some(data_host_path),
            typ: Some(MountType::BIND),
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
        entrypoint,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_sandbox_runtime::provisioning::{ProvisionPlan, RepoConfig};

    fn test_config() -> Config {
        use moatless_core::config::*;
        use moatless_types::auth::*;
        use moatless_types::sandbox::*;
        Config {
            server: moatless_core::config::ServerConfig {
                port: 8080,
                metrics_port: 9090,
                grpc_port: 50051,
            },
            sandbox_provider: SandboxProviderType::Docker,
            sandbox: SandboxConfig {
                workspace_repository_secret: None,
                workspace_repository_role_arn: None,
                compute_mode: KubernetesComputeMode::Standard,
                namespace: "test".into(),
                image: "sandbox:latest".into(),
                image_pull_policy: "Never".into(),
                grpc_port: 50052,
                resources_cpu_limit: "500m".into(),
                resources_memory_limit: "512Mi".into(),
                resources_cpu_request: "100m".into(),
                resources_memory_request: "128Mi".into(),
                sandbox_resources_cpu_request: "1".into(),
                sandbox_resources_cpu_limit: "4".into(),
                sandbox_resources_memory_request: "2Gi".into(),
                sandbox_resources_memory_limit: "8Gi".into(),
                ephemeral_storage: None,
                run_as_non_root: false,
                shared_data_size: "2Gi".into(),
                persist_toolchain: false,
                storage_enabled: false,
                storage_size: "1Gi".into(),
                storage_class: None,
                default_pvc_enabled: true,
                default_s3_enabled: true,
                node_archive_enabled: false,
                retain_storage_on_close: false,
                pod_startup_timeout_secs: 10,
                task_token_secret: Some("secret".into()),
                user_token_ttl_secs: 7 * 24 * 3600,
                bootstrap_token_ttl_secs: 1800,
                emit_legacy_user_token_env: true,
                accept_claimless_tokens: true,
                file_mount_path: "/tmp".into(),
                idle_reaper_enabled: false,
                cron_scheduler_enabled: false,
                liveness_enabled: false,
                idle_timeout_minutes: 15,
                idle_check_interval_secs: 300,
                inactive_task_timeout_days: 0,
                cold_after_hours: 0,
                orphan_gc_enabled: false,
                codex_refresher_interval_secs: 0,
                max_crash_restart_count: 5,
                node_selector_json: None,
                tolerations_json: None,
                service_account_name: None,
                default_model: None,
                default_codex_model: None,
                extra_env: vec![],
                cache_volumes: vec![],
                cache_host_root: "/mnt/cache".into(),
                cache_init_image: "docker:dind".into(),
                docker_dind_image: "docker:dind".into(),
                docker_dind_storage_size: "10Gi".into(),
                docker_dind_cpu_request: "500m".into(),
                docker_dind_cpu_limit: "2".into(),
                docker_dind_memory_request: "1Gi".into(),
                docker_dind_memory_limit: "4Gi".into(),
                egress_proxy_url: None,
                egress_proxy_profiles: vec![],
                egress_proxy_no_proxy: "localhost,.svc.cluster.local".into(),
            },
            proxy: ProxyConfig {
                domain: None,
                protocol: "http".into(),
                traefik_auth_middleware_name: "test".into(),
                traefik_auth_middleware_namespace: "test".into(),
                traefik_entry_point: "web".into(),
                traefik_ingress_namespaces: vec!["test".into()],
            },
            docker: DockerConfig {
                network: "moatless".into(),
                image: "sandbox:latest".into(),
                data_dir: "/data".into(),
                container_data_dir: "/data".into(),
                backend_hostname: "moatless-backend".into(),
                enable_socket_mount: false,
                socket_path: "/var/run/docker.sock".into(),
                extra_networks: vec![],
            },
            database_url: None,
            database: DatabaseConfig {
                url: None,
                max_connections: 5,
                run_migrations: true,
            },
            redis: RedisConfig {
                url: None,
                max_stream_length: 100,
                event_handler_enabled: true,
            },
            sse: SseConfig {
                broadcast_capacity: 16,
                consumer_id: "test".into(),
            },
            auth: AuthConfig {
                provider: moatless_types::auth::AuthProvider::Local,
                github_required_org: None,
                auth0_domain: None,
                auth0_client_id: None,
                auth0_client_secret: None,
                auth0_callback_url: None,
                auth0_audience: None,
                auth0_groups_claim: String::new(),
                oidc: None,
                default_team_slug: None,
                session_max_age_secs: 3600,
                session_absolute_age_secs: 3600,
                frontend_url: "http://localhost:3000".into(),
                csrf_trusted_origins: Vec::new(),
                cookie_domain: None,
                session_cookie_name: "test".into(),
                is_production: false,
                admin_username: "admin".into(),
                admin_email: Some("admin@example.com".into()),
                admin_password: None,
                break_glass_enabled: false,
                cache_ttl_secs: 0,
                session_jwt_secret: None,
                test_api_key: None,
                github_app_client_id: None,
                github_app_client_secret: None,
                github_app_callback_url: None,
                github_app_id: None,
                github_app_private_key: None,
                github_app_default_installation_id: None,
                github_apps: vec![],
            },
            s3: S3Config {
                bucket_name: None,
                key_prefix: "test".into(),
                transcript_prefix: "transcripts".into(),
                workspace_prefix: "workspaces".into(),
                region: "us-east-1".into(),
            },
            slack: SlackConfig {
                bot_token: None,
                signing_secret: None,
                bot_user_id: None,
            },
            telegram: TelegramConfig {
                bot_token: None,
                mode: "webhook".into(),
            },
            github_pr: moatless_core::config::GithubPrConfig {
                webhook_secret: None,
            },
            linear: moatless_types::adapter::LinearConfig {
                webhook_secret: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                oauth_callback_url: None,
                api_base_url: None,
            },
            skill: moatless_core::config::SkillConfig { cache_ttl_secs: 60 },
            secrets: moatless_types::secret::SecretsConfig {
                encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
                kms_key_id: None,
            },
            codex_oauth: moatless_types::agent::CodexOAuthConfig {
                issuer: "https://auth.openai.com".into(),
                client_id: "test-codex-client".into(),
            },
            otel: OtelConfig {
                otlp_endpoint: None,
                sampling_ratio: 0.1,
                service_name: "test".into(),
            },
        }
    }

    #[test]
    fn repo_config_preserves_docker_server_projection() {
        let mut plan = test_plan();
        plan.repo_config.servers = Some(vec![ServerConfig {
            name: "web".into(),
            port: Port::new(3000),
            start_commands: vec!["run".into()],
            env_vars: serde_json::from_value(serde_json::json!({"KEY": "value"}))
                .expect("env vars"),
            auto_reload: true,
            docker_image: Some("image".into()),
            label: Some("Web".into()),
            ..Default::default()
        }]);
        let encoded = build_repo_config_json(&plan).expect("config");
        let value: serde_json::Value = serde_json::from_str(&encoded).expect("json");
        assert_eq!(
            value["servers"],
            serde_json::json!([{
                "name": "web", "port": 3000, "startCommands": ["run"],
                "envVars": {"KEY": "value"}, "dockerImage": "image"
            }])
        );
    }

    fn test_plan() -> ProvisionPlan {
        ProvisionPlan {
            repo_config: RepoConfig {
                name: "test-repo".into(),
                docker_image: None,
                servers: None,
                system_prompt: None,
                code_execution: None,
                preview_config: None,
                resources: None,
                plugins: None,
                docker_access: None,
                docker_networks: None,
                sandbox_service_account: None,
                network_profile: None,
            },
            git_clone_url: "https://github.com/test/repo.git".into(),
            git_branch: Some("main".into()),
            git_provider: "github".into(),
            env_vars: None,
            repos: Vec::new(),
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
    fn container_config_treats_empty_repo_image_as_unset() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.docker_image = Some(String::new());
        let cc = build_container_config(&config, "task-1", &plan, None);
        assert_eq!(cc.image.unwrap(), "sandbox:latest");
    }

    /// The prompt's "Saved to:" line is rendered from `file_mount_path`, so the
    /// container has to be told the same path rather than falling back to a
    /// default of its own.
    #[test]
    fn container_config_takes_upload_dir_from_file_mount_path() {
        let mut config = test_config();
        config.sandbox.file_mount_path = "/opt/moatless/workspace/uploads".into();
        let plan = test_plan();

        let cc = build_container_config(&config, "task-1", &plan, None);

        let env = cc.env.unwrap();
        assert!(
            env.iter()
                .any(|e| e == "UPLOAD_DIR=/opt/moatless/workspace/uploads"),
            "expected UPLOAD_DIR from file_mount_path, got: {env:?}"
        );
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
        assert!(env.iter().any(|e| e == "USER_AUTH_TOKEN=jwt-token"));
        assert!(!env.iter().any(|e| e.starts_with("GIT_TOKEN=")));
        assert!(!env.iter().any(|e| e.starts_with("ANTHROPIC_API_KEY=")));
    }

    /// A multi-repo workspace reaches the container as the `WORKSPACE_REPOS`
    /// list; a plan without checkouts omits the env entirely.
    #[test]
    fn container_config_carries_every_workspace_repo() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repos = vec![
            moatless_types::workspace::RepoCheckout {
                repository_id: "repo-1".into(),
                clone_url: "https://github.com/test/repo.git".into(),
                branch: Some("main".into()),
                provider: "github".into(),
                mount_name: "repo".into(),
                is_primary: true,
            },
            moatless_types::workspace::RepoCheckout {
                repository_id: "repo-2".into(),
                clone_url: "https://github.com/test/shared.git".into(),
                branch: None,
                provider: "github".into(),
                mount_name: "shared".into(),
                is_primary: false,
            },
        ];

        let cc = build_container_config(&config, "task-1", &plan, None);
        let env = cc.env.unwrap();
        let raw = env
            .iter()
            .find_map(|e| e.strip_prefix(&format!("{WORKSPACE_REPOS_ENV}=")))
            .expect("WORKSPACE_REPOS present");
        let parsed: Vec<moatless_types::workspace::RepoCheckout> =
            serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].mount_name, "shared");

        plan.repos.clear();
        let cc = build_container_config(&config, "task-1", &plan, None);
        assert!(
            cc.env
                .unwrap()
                .iter()
                .all(|e| !e.starts_with(&format!("{WORKSPACE_REPOS_ENV}=")))
        );
    }

    #[test]
    fn container_config_applies_resource_limits_from_repo_config() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.resources = Some(moatless_types::workspace::ResourceConfig {
            limits: moatless_types::workspace::ResourceQuantities {
                cpu: Some("4".parse().expect("cpu")),
                memory: Some("8Gi".parse().expect("memory")),
            },
            ..Default::default()
        });
        let cc = build_container_config(&config, "task-1", &plan, None);
        let hc = cc.host_config.unwrap();
        assert_eq!(hc.memory, Some(8 * 1024 * 1024 * 1024));
        assert_eq!(hc.nano_cpus, Some(4_000_000_000));
    }

    #[test]
    fn container_config_without_resources_leaves_limits_unset() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let hc = cc.host_config.unwrap();
        assert_eq!(hc.memory, None);
        assert_eq!(hc.nano_cpus, None);
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
    fn sandbox_container_carries_operation_generation() {
        let config = test_config();
        let plan = test_plan();
        let mut cc = build_container_config(&config, "task-1", &plan, None);
        stamp_operation_generation(
            &mut cc,
            RuntimeOperationContext::new(moatless_sandbox_runtime::operation::OperationGeneration(
                17,
            )),
        );
        assert_eq!(
            cc.labels
                .as_ref()
                .and_then(|labels| labels.get(OPERATION_GENERATION_LABEL))
                .map(String::as_str),
            Some("17")
        );
    }

    #[test]
    fn container_config_exposes_ports() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let ports = cc.exposed_ports.unwrap();
        assert!(ports.iter().any(|p| p == "8001/tcp"));
        assert!(ports.iter().any(|p| p == "50052/tcp"));
    }

    #[test]
    fn container_config_has_bind_mounts() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let host_config = cc.host_config.unwrap();
        let mounts = host_config.mounts.unwrap();
        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[0].target.as_deref(), Some("/opt/moatless/workspace"));
        assert_eq!(mounts[0].source.as_deref(), Some("/data/tasks/task-1/repo"));
        assert_eq!(mounts[1].target.as_deref(), Some("/opt/moatless/data"));
        assert_eq!(mounts[1].source.as_deref(), Some("/data/tasks/task-1/data"));
        assert_eq!(mounts[2].target.as_deref(), Some("/opt/moatless/.claude"));
        assert_eq!(
            mounts[2].source.as_deref(),
            Some("/data/tasks/task-1/claude-config")
        );
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
            port: Port::new(3000),
            start_commands: vec!["bun run dev".into()],
            docker_image: None,
            ..Default::default()
        }
    }

    const TEST_SANDBOX_IMAGE: &str = "aorwall/moatless-sandbox:latest";

    #[test]
    fn server_config_uses_sandbox_image_fallback() {
        let config = test_config();
        let server = test_server_config();
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        assert_eq!(cc.image.unwrap(), TEST_SANDBOX_IMAGE);
    }

    #[test]
    fn server_config_uses_custom_image() {
        let config = test_config();
        let mut server = test_server_config();
        server.docker_image = Some("custom-server:v1".into());
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        assert_eq!(cc.image.unwrap(), "custom-server:v1");
    }

    #[test]
    fn server_config_treats_empty_image_as_unset() {
        let config = test_config();
        let mut server = test_server_config();
        server.docker_image = Some(String::new());
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        assert_eq!(cc.image.unwrap(), TEST_SANDBOX_IMAGE);
    }

    #[test]
    fn server_config_per_server_image_overrides_sandbox_fallback() {
        let config = test_config();
        let mut server = test_server_config();
        server.docker_image = Some("custom-server:v1".into());
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        assert_eq!(cc.image.unwrap(), "custom-server:v1");
    }

    #[test]
    fn server_config_has_correct_labels() {
        let config = test_config();
        let server = test_server_config();
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        let labels = cc.labels.unwrap();
        assert_eq!(labels.get(SERVICE_LABEL).unwrap(), SERVER_SERVICE);
        assert_eq!(labels.get("moatless.io/task-id").unwrap(), "task-1");
        assert_eq!(labels.get("moatless.io/server-name").unwrap(), "preview");
    }

    #[test]
    fn server_config_has_workspace_ready_wait() {
        let config = test_config();
        let server = test_server_config();
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        // Start commands must land in `entrypoint` (not `cmd`): Docker Cmd
        // only feeds args to the image ENTRYPOINT, which ignores them.
        assert!(cc.cmd.is_none());
        let entrypoint = cc.entrypoint.unwrap();
        assert_eq!(entrypoint[0], "bash");
        assert_eq!(entrypoint[1], "-c");
        assert!(entrypoint[2].contains(".workspace-ready"));
        assert!(entrypoint[2].contains("bun run dev"));
    }

    #[test]
    fn server_config_sources_shared_server_env_file() {
        let config = test_config();
        let server = test_server_config();
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        let env = cc.env.unwrap();
        assert!(
            !env.iter()
                .any(|e| e.starts_with("MOATLESS_SERVER_ENV_FILE="))
        );
        assert!(!env.iter().any(|e| e.starts_with("BASH_ENV=")));

        let entrypoint = cc.entrypoint.unwrap();
        assert!(entrypoint[2].contains(". /opt/moatless/data/server.env"));
        assert!(entrypoint[2].contains("bun run dev"));
    }

    #[test]
    fn server_config_exposes_server_port() {
        let config = test_config();
        let server = test_server_config();
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        let ports = cc.exposed_ports.unwrap();
        assert!(ports.iter().any(|p| p == "3000/tcp"));
    }

    #[test]
    fn server_config_has_bind_mounts() {
        let config = test_config();
        let server = test_server_config();
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        let host_config = cc.host_config.unwrap();
        let mounts = host_config.mounts.unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].target.as_deref(), Some("/opt/moatless/workspace"));
        assert_eq!(mounts[1].target.as_deref(), Some("/opt/moatless/data"));
    }

    #[test]
    fn server_config_env_vars() {
        let config = test_config();
        let server = test_server_config();
        let cc =
            build_server_container_config(&config, "task-1", &server, 3000, TEST_SANDBOX_IMAGE);
        let env = cc.env.unwrap();
        assert!(env.iter().any(|e| e == "PORT=3000"));
        assert!(
            env.iter()
                .any(|e| e == "WORKSPACE_DIR=/opt/moatless/workspace")
        );
        assert!(
            env.iter()
                .any(|e| e == "WORKSPACE_PATH=/opt/moatless/workspace")
        );
        assert!(env.iter().any(|e| e == "DATA_DIR=/opt/moatless/data"));
    }

    // ── Docker socket access tests ──────────────────────────────

    #[test]
    fn no_docker_socket_by_default() {
        let config = test_config();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let mounts = cc.host_config.unwrap().mounts.unwrap();
        assert_eq!(mounts.len(), 3);
        assert!(
            !mounts
                .iter()
                .any(|m| m.target.as_deref() == Some("/var/run/docker.sock"))
        );
    }

    #[test]
    fn docker_socket_via_repo_config() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.docker_access = Some(true);
        let cc = build_container_config(&config, "task-1", &plan, None);
        let mounts = cc.host_config.unwrap().mounts.unwrap();
        assert_eq!(mounts.len(), 4);
        let socket_mount = mounts
            .iter()
            .find(|m| m.target.as_deref() == Some("/var/run/docker.sock"));
        assert!(socket_mount.is_some());
        assert_eq!(
            socket_mount.unwrap().source.as_deref(),
            Some("/var/run/docker.sock")
        );
    }

    #[test]
    fn docker_socket_via_global_config() {
        let mut config = test_config();
        config.docker.enable_socket_mount = true;
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let mounts = cc.host_config.unwrap().mounts.unwrap();
        assert_eq!(mounts.len(), 4);
        assert!(
            mounts
                .iter()
                .any(|m| m.target.as_deref() == Some("/var/run/docker.sock"))
        );
    }

    #[test]
    fn repo_config_false_overrides_global_true() {
        let mut config = test_config();
        config.docker.enable_socket_mount = true;
        let mut plan = test_plan();
        plan.repo_config.docker_access = Some(false);
        let cc = build_container_config(&config, "task-1", &plan, None);
        let mounts = cc.host_config.unwrap().mounts.unwrap();
        assert_eq!(mounts.len(), 3);
        assert!(
            !mounts
                .iter()
                .any(|m| m.target.as_deref() == Some("/var/run/docker.sock"))
        );
    }

    #[test]
    fn custom_socket_path() {
        let mut config = test_config();
        config.docker.enable_socket_mount = true;
        config.docker.socket_path = "/custom/docker.sock".into();
        let plan = test_plan();
        let cc = build_container_config(&config, "task-1", &plan, None);
        let mounts = cc.host_config.unwrap().mounts.unwrap();
        let socket_mount = mounts
            .iter()
            .find(|m| m.target.as_deref() == Some("/var/run/docker.sock"))
            .unwrap();
        assert_eq!(socket_mount.source.as_deref(), Some("/custom/docker.sock"));
    }

    // ── Server config validation tests ──────────────────────────────

    #[test]
    fn validate_ok_with_start_commands() {
        let server = test_server_config();
        assert!(validate_server_config(&server).is_ok());
    }

    #[test]
    fn validate_rejects_no_image_no_start_commands() {
        let mut server = test_server_config();
        server.start_commands = Vec::new();
        let err = validate_server_config(&server).unwrap_err();
        assert!(err.contains("preview"));
        assert!(err.contains("startCommands"));
    }

    #[test]
    fn validate_rejects_a_server_without_a_port() {
        let mut server = test_server_config();
        server.port = None;
        let err = validate_server_config(&server).unwrap_err();
        assert!(err.contains("preview"));
        assert!(err.contains("port"));
    }

    #[test]
    fn validate_allows_custom_image_without_start_commands() {
        let mut server = test_server_config();
        server.start_commands = Vec::new();
        server.docker_image = Some("my-server:v1".into());
        assert!(validate_server_config(&server).is_ok());
    }
}
