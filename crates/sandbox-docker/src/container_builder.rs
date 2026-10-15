//! Build Docker container configuration from a `ProvisionPlan`.
//!
//! Generates a `bollard::container::Config` with all the env vars, labels, bind
//! mounts, port mappings, and networking required for a sandbox container
//! running as a sibling container on a shared Docker network.

use std::collections::HashMap;

use bollard::models::{
    ContainerCreateBody as ContainerConfig, HostConfig, Mount, MountType, PortBinding,
    RestartPolicy, RestartPolicyNameEnum,
};

use super::naming::{
    generate_claude_config_host_path, generate_container_name, generate_data_host_path,
    generate_sandbox_host_path,
};
use moatless_core::config::Config;
use moatless_sandbox_runtime::provisioning::ProvisionPlan;
use moatless_sandbox_runtime::runtime::{OPERATION_GENERATION_LABEL, RuntimeOperationContext};
use moatless_workspace_types::workspace::{Memory, WORKSPACE_REPOS_ENV};

pub(crate) const CREATED_BY_LABEL: &str = "created-by";
pub(crate) const CREATED_BY_VALUE: &str = "moatless";
pub(crate) const SERVICE_LABEL: &str = "moatless.service";
pub(crate) const SANDBOX_SERVICE: &str = "sandbox";

/// Resolve the sandbox image for a plan: per-repo override → default docker
/// image.
///
/// An empty override string is treated as unset. The start path diffs this
/// against the image baked into a stopped container.
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

    if let Some(ref model) = config.sandbox.default_model {
        env.push(format!("DEFAULT_CLAUDE_CODE_MODEL={model}"));
    }

    if let Some(ref env_vars) = plan.env_vars {
        for (key, value) in env_vars {
            env.push(format!("{key}={value}"));
        }
    }

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
    moatless_sandbox_runtime::repo_config_encoding::encode_repo_config(&plan.repo_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_sandbox_runtime::provisioning::{ProvisionPlan, RepoConfig};

    fn test_config() -> Config {
        use moatless_types::sandbox::SandboxProviderType;
        let mut config = Config::test_minimal();
        config.sandbox_provider = SandboxProviderType::Docker;
        config.docker.image = "sandbox:latest".into();
        config.docker.network = "moatless".into();
        config.sandbox.task_token_secret = Some("secret".into());
        config
    }

    fn test_plan() -> ProvisionPlan {
        ProvisionPlan {
            repo_config: RepoConfig {
                name: "test-repo".into(),
                docker_image: None,
                system_prompt: None,
                code_execution: None,
                resources: None,
                plugins: None,
                language_servers: None,
                docker_access: None,
                docker_networks: None,
                sandbox_service_account: None,
                network_profile: None,
                node_selector: None,
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
            moatless_workspace_types::workspace::RepoCheckout {
                uses_ssh: false,
                repository_id: "repo-1".into(),
                clone_url: "https://github.com/test/repo.git".into(),
                branch: Some("main".into()),
                provider: "github".into(),
                mount_name: "repo".into(),
                is_primary: true,
            },
            moatless_workspace_types::workspace::RepoCheckout {
                uses_ssh: false,
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
        let parsed: Vec<moatless_workspace_types::workspace::RepoCheckout> =
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
        plan.repo_config.resources = Some(moatless_workspace_types::workspace::ResourceConfig {
            limits: moatless_workspace_types::workspace::ResourceQuantities {
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
}
