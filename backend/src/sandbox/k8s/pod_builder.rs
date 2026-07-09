//! Code-generated Pod spec with dynamic server containers.
//!
//! Generates the full Pod spec in Rust code — no YAML template needed.
//! Server containers are created dynamically from the repo config.

use k8s_openapi::api::core::v1::{
    Capabilities, Container, ContainerPort, EmptyDirVolumeSource, EnvVar, HTTPGetAction,
    HostPathVolumeSource, PersistentVolumeClaimVolumeSource, Pod, PodSpec, Probe,
    ResourceRequirements, SecurityContext, TCPSocketAction, Toleration, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use std::collections::{BTreeMap, HashMap};
use tracing::{debug, info};

use super::naming::generate_pod_name;
use crate::config::Config;
use crate::sandbox::models::ProvisionPlan;

/// Build a complete Pod spec from config + plan (no template).
///
/// Generates the sandbox container, dynamic server containers, volumes,
/// probes, and security contexts entirely in code.
pub fn build_pod(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    pvc_name: Option<&str>,
    user_auth_token: Option<&str>,
) -> Result<Pod, String> {
    let pod_name = generate_pod_name(&plan.repo_config.name, task_id);
    let namespace = config.sandbox.namespace.clone();

    // ── Labels ──────────────────────────────────────────────────────
    let mut labels = BTreeMap::new();
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-sandbox".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "sandbox".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );

    // ── Annotations ─────────────────────────────────────────────────
    let mut annotations = BTreeMap::new();
    annotations.insert(
        "moatless.io/repository-url".to_string(),
        plan.git_clone_url.clone(),
    );
    annotations.insert("moatless.io/created-at".to_string(), chrono_now_iso());
    // Prevent Karpenter from evicting running sandbox pods during node consolidation.
    annotations.insert(
        "karpenter.sh/do-not-disrupt".to_string(),
        "true".to_string(),
    );

    // ── Volumes ─────────────────────────────────────────────────────
    //
    // `/opt/moatless` (the workspace) is backed by the `workspace-data` volume:
    // a per-task EBS PVC for the default tier, or a node-local emptyDir for the
    // ephemeral tier (and any time the PVC is unavailable). Either way it is a
    // single volume shared across the sandbox and server containers, with `/tmp`
    // served from its `tmp` subPath so `/tmp` never charges against the pod's
    // memory budget.
    let mut volumes = vec![
        Volume {
            name: "build-cache".to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity("1536Mi".to_string())),
                ..Default::default()
            }),
            ..Default::default()
        },
        Volume {
            name: "shared-data".to_string(),
            empty_dir: Some(EmptyDirVolumeSource::default()),
            ..Default::default()
        },
    ];

    if let Some(pvc) = pvc_name {
        volumes.push(Volume {
            name: "workspace-data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: pvc.to_string(),
                read_only: Some(false),
            }),
            ..Default::default()
        });
    } else {
        // Ephemeral (no-EBS) tier — a node-local emptyDir capped at the repo's
        // workspace size (`resources.storage`, default `K8S_SANDBOX_STORAGE_SIZE`).
        // No `medium: Memory`, so it lives on node disk, not the pod memory budget.
        // This also replaces the old uncapped rootfs `/opt/moatless`, so the
        // workspace is bounded and shared with server containers like the PVC tier.
        volumes.push(Volume {
            name: "workspace-data".to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity(resolve_workspace_size(config, plan))),
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    // ── Package-manager caches (node-local NVMe) ────────────────────
    //
    // When `cache_volumes` is configured, all caches share ONE node-local
    // hostPath volume rooted at `cache_host_root` (`/mnt/cache`), mounted
    // per-entry via subPath in the sandbox container. A privileged `nvme-setup`
    // init container formats and mounts the instance-store NVMe under it (and
    // ensures a world-writable subdir per cache) — see `build_nvme_setup_container`.
    // The whole block is gated on a non-empty list, so default sandboxes stay
    // hardened (no privileged init container, no hostPath).
    let mut init_containers: Vec<Container> = Vec::new();
    if !config.sandbox.cache_volumes.is_empty() {
        volumes.push(Volume {
            name: "pkg-cache".to_string(),
            host_path: Some(HostPathVolumeSource {
                path: config.sandbox.cache_host_root.clone(),
                type_: Some("DirectoryOrCreate".to_string()),
            }),
            ..Default::default()
        });
        // The init container needs the PARENT of the cache root so its NVMe
        // mount propagates back to the host namespace (bidirectional), where
        // the sibling `pkg-cache` hostPath mount then picks it up.
        volumes.push(Volume {
            name: "host-mnt".to_string(),
            host_path: Some(HostPathVolumeSource {
                path: cache_host_parent(&config.sandbox.cache_host_root).to_string(),
                type_: Some("DirectoryOrCreate".to_string()),
            }),
            ..Default::default()
        });
        init_containers.push(build_nvme_setup_container(config));
    }

    // ── Sandbox container ───────────────────────────────────────────
    let sandbox_container = build_sandbox_container(config, task_id, plan, user_auth_token)?;

    // ── Server containers ───────────────────────────────────────────
    let server_containers = build_server_containers(config, plan)?;

    // ── Assemble Pod ────────────────────────────────────────────────
    let mut all_containers = vec![sandbox_container];
    all_containers.extend(server_containers);

    let pod = Pod {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(pod_name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels),
            annotations: Some(annotations),
            ..Default::default()
        },
        spec: Some({
            let tolerations = config
                .sandbox
                .tolerations_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<Vec<Toleration>>(json).ok());
            let node_selector = config
                .sandbox
                .node_selector_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<BTreeMap<String, String>>(json).ok());
            PodSpec {
                containers: all_containers,
                init_containers: (!init_containers.is_empty()).then_some(init_containers),
                volumes: Some(volumes),
                restart_policy: Some("Always".to_string()),
                termination_grace_period_seconds: Some(30),
                tolerations,
                node_selector,
                service_account_name: config.sandbox.service_account_name.clone(),
                ..Default::default()
            }
        }),
        ..Default::default()
    };

    let server_count = plan
        .repo_config
        .servers
        .as_ref()
        .map_or(0, std::vec::Vec::len);
    info!(
        task_id,
        pod_name = %pod_name,
        namespace = %namespace,
        server_count,
        "Pod spec built (code-generated)"
    );
    debug!(
        task_id,
        "Pod spec: {:?}",
        pod.spec
            .as_ref()
            .map(|s| s.containers.iter().map(|c| &c.name).collect::<Vec<_>>())
    );

    Ok(pod)
}

/// Resolve the sandbox image for a plan: per-repo override → default sandbox image.
///
/// An empty override string is treated as unset. This is the single source of
/// truth for the "expected" image, so the start path can diff it against the
/// image baked into a stopped Deployment.
pub fn resolve_sandbox_image(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .docker_image
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| config.sandbox.image.clone())
}

/// Resolve the workspace size for the ephemeral (emptyDir) tier: a per-repo
/// `resources.storage` override wins over the global `K8S_SANDBOX_STORAGE_SIZE`.
/// One knob sizes both tiers — the PVC tier reads the same value in `pvc.rs`.
fn resolve_workspace_size(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.storage.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| config.sandbox.storage_size.clone(), ToString::to_string)
}

/// Parent directory of the package-cache root (e.g. `/mnt` for `/mnt/cache`).
///
/// The `nvme-setup` init container bind-mounts this parent with bidirectional
/// propagation so that the NVMe it mounts at `<parent>/<basename>` propagates
/// back to the host and is visible to the sibling `pkg-cache` hostPath mount.
/// Falls back to `/` if the root has no parent component.
fn cache_host_parent(root: &str) -> &str {
    let trimmed = root.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => "/",
    }
}

/// Build the privileged `nvme-setup` init container.
///
/// Formats (idempotently — only when unformatted) and mounts the node-local
/// instance-store NVMe (`/dev/nvme1n1`) under the package-cache root, then
/// ensures a world-writable subdir exists for each configured cache. Mirrors the
/// GitHub Actions runner pattern (soap-platform-gitops `java-build-runner-set`):
/// mount via bidirectional propagation so the host — and thus the sibling
/// `pkg-cache` hostPath mounts in the app containers — see the NVMe-backed dirs.
///
/// On a node without the NVMe device the format/mount is skipped and the caches
/// fall back to the node root disk (still node-local) — the feature degrades
/// gracefully rather than failing the pod.
///
/// Deliberately does NOT use `drop_all_security_context()`: `mkfs.ext4` / `mount`
/// of the raw block device require full privileges. Scoped to this short-lived
/// init container; the app containers stay hardened.
fn build_nvme_setup_container(config: &Config) -> Container {
    // Inside the container the parent is bind-mounted at `/host-mnt`, so the
    // NVMe mount target is `/host-mnt/<basename-of-cache-root>`.
    let root = config.sandbox.cache_host_root.trim_end_matches('/');
    let basename = root.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("cache");
    let cache_dir = format!("/host-mnt/{basename}");

    // mkdir/chmod each cache subdir AFTER the mount block so it runs whether the
    // cache landed on NVMe or the root-disk fallback.
    let mkdirs = config
        .sandbox
        .cache_volumes
        .iter()
        .map(|c| {
            format!(
                "mkdir -p {cache_dir}/{name} && chmod 0777 {cache_dir}/{name}",
                name = c.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!(
        "if [ -b /dev/nvme1n1 ]; then\n\
         \x20 if ! blkid /dev/nvme1n1 >/dev/null 2>&1; then\n\
         \x20   echo \"NVMe unformatted, creating ext4...\"\n\
         \x20   mkfs.ext4 -F /dev/nvme1n1\n\
         \x20 fi\n\
         \x20 mkdir -p {cache_dir}\n\
         \x20 mountpoint -q {cache_dir} || mount /dev/nvme1n1 {cache_dir}\n\
         fi\n\
         mkdir -p {cache_dir}\n\
         {mkdirs}\n"
    );

    Container {
        name: "nvme-setup".to_string(),
        image: Some(config.sandbox.cache_init_image.clone()),
        image_pull_policy: Some(config.sandbox.image_pull_policy.clone()),
        command: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
        args: Some(vec![script]),
        security_context: Some(privileged_security_context()),
        volume_mounts: Some(vec![VolumeMount {
            name: "host-mnt".to_string(),
            mount_path: "/host-mnt".to_string(),
            mount_propagation: Some("Bidirectional".to_string()),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

/// Build the main sandbox container.
#[allow(clippy::unnecessary_wraps)] // Result for consistency with build_server_containers
fn build_sandbox_container(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    user_auth_token: Option<&str>,
) -> Result<Container, String> {
    // Resolve image: repo config override → default sandbox image
    let image = resolve_sandbox_image(config, plan);

    // Build env vars
    let mut env = vec![
        env_var("TASK_ID", task_id),
        env_var("GIT_CLONE_URL", &plan.git_clone_url),
        env_var(
            "BACKEND_GRPC_URL",
            &format!(
                "http://moatless-vibe-backend.{}.svc.cluster.local:{}",
                config.sandbox.namespace, config.server.grpc_port
            ),
        ),
        env_var(
            "BACKEND_API_URL",
            &format!(
                "http://moatless-vibe-backend.{}.svc.cluster.local:{}",
                config.sandbox.namespace, config.server.port
            ),
        ),
    ];

    if let Some(ref branch) = plan.git_branch {
        env.push(env_var("GIT_BRANCH", branch));
    }

    // Git provider (github, gitlab, gitness)
    env.push(env_var("GIT_PROVIDER", &plan.git_provider));

    // Workspace storage backend. The `s3` backend runs on a node-local emptyDir
    // (no PVC) and syncs the git working tree to S3 via presigned URLs — flag the
    // sandbox so it starts its workspace-sync loop and restores on boot. `ebs`
    // and `ephemeral` leave this unset (loop stays off).
    if super::storage::StorageBackend::from_plan(plan).syncs_workspace() {
        env.push(env_var("WORKSPACE_SYNC_ENABLED", "true"));
    }

    // REPO_CONFIG env var (pass-through for sandbox)
    if let Some(repo_config_json) = build_repo_config_json(plan) {
        env.push(env_var("REPO_CONFIG", &repo_config_json));
    }

    // Proxy domain and protocol for constructing preview URLs
    if let Some(ref domain) = config.proxy.domain {
        env.push(env_var("PROXY_DOMAIN", domain));
        env.push(env_var("PROXY_PROTOCOL", &config.proxy.protocol));
    }

    // Frontend URL for constructing task URLs in the agent system prompt
    env.push(env_var("FRONTEND_URL", &config.auth.frontend_url));

    // Default model for agent sessions
    if let Some(ref model) = config.sandbox.default_model {
        env.push(env_var("DEFAULT_CLAUDE_CODE_MODEL", model));
    }
    if let Some(ref model) = config.sandbox.default_codex_model {
        env.push(env_var("DEFAULT_CODEX_MODEL", model));
    }

    if let Some(token) = user_auth_token {
        env.push(env_var("USER_AUTH_TOKEN", token));
    }

    // Extra env vars from backend config (K8S_SANDBOX_EXTRA_ENV)
    for (key, value) in &config.sandbox.extra_env {
        env.push(env_var(key, value));
    }

    // Custom env vars from repo config
    if let Some(ref env_vars) = plan.env_vars {
        for (key, value) in env_vars {
            env.push(env_var(key, value));
        }
    }

    // Package-manager cache env — point each tool at its mounted cache dir
    // (e.g. MAVEN_OPTS=-Dmaven.repo.local=/maven-cache, BUN_INSTALL_CACHE_DIR).
    for cache in &config.sandbox.cache_volumes {
        for (key, value) in &cache.env {
            env.push(env_var(key, value));
        }
    }

    // Startup probe
    let startup_timeout = config.sandbox.pod_startup_timeout_secs;
    let startup_probe_period: i32 = 5;
    let failure_threshold = ((startup_timeout / startup_probe_period as u64) + 6) as i32;

    let startup_probe = Probe {
        http_get: Some(HTTPGetAction {
            path: Some("/health".to_string()),
            port: IntOrString::Int(8001),
            ..Default::default()
        }),
        period_seconds: Some(startup_probe_period),
        timeout_seconds: Some(5),
        failure_threshold: Some(failure_threshold),
        ..Default::default()
    };

    // Volume mounts
    //
    // `workspace-data` (PVC or ephemeral emptyDir) is always present and mounted at
    // `/opt/moatless`. `/tmp` is backed by its `tmp` subPath — the same volume, a
    // separate directory — so `/tmp` shares the workspace headroom and never charges
    // against container memory.
    let mut volume_mounts = vec![
        VolumeMount {
            name: "build-cache".to_string(),
            mount_path: "/opt/moatless/build-cache".to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "shared-data".to_string(),
            mount_path: "/opt/moatless/data".to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "workspace-data".to_string(),
            mount_path: "/opt/moatless".to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "workspace-data".to_string(),
            mount_path: "/tmp".to_string(),
            sub_path: Some("tmp".to_string()),
            ..Default::default()
        },
    ];

    // Package-manager cache mounts — each entry is a subPath (`<name>`) on the
    // single shared `pkg-cache` hostPath volume, surfaced at its `mount_path`.
    for cache in &config.sandbox.cache_volumes {
        volume_mounts.push(VolumeMount {
            name: "pkg-cache".to_string(),
            mount_path: cache.mount_path.clone(),
            sub_path: Some(cache.name.clone()),
            ..Default::default()
        });
    }

    // Resource requests + limits for the agent container. A per-repo override
    // takes precedence; otherwise the global `K8S_RESOURCE_*` config applies.
    // Without this the sandbox container is BestEffort and the first OOM/eviction
    // target on spot nodes — see the `sandbox_resources_*` config fields.
    let resources = if let Some(ref res) = plan.repo_config.resources {
        ResourceRequirements {
            limits: res.limits.as_ref().map(|l| {
                l.iter()
                    .map(|(k, v)| (k.clone(), Quantity(v.clone())))
                    .collect()
            }),
            requests: res.requests.as_ref().map(|r| {
                r.iter()
                    .map(|(k, v)| (k.clone(), Quantity(v.clone())))
                    .collect()
            }),
            ..Default::default()
        }
    } else {
        ResourceRequirements {
            limits: Some(BTreeMap::from([
                (
                    "cpu".to_string(),
                    Quantity(config.sandbox.sandbox_resources_cpu_limit.clone()),
                ),
                (
                    "memory".to_string(),
                    Quantity(config.sandbox.sandbox_resources_memory_limit.clone()),
                ),
            ])),
            requests: Some(BTreeMap::from([
                (
                    "cpu".to_string(),
                    Quantity(config.sandbox.sandbox_resources_cpu_request.clone()),
                ),
                (
                    "memory".to_string(),
                    Quantity(config.sandbox.sandbox_resources_memory_request.clone()),
                ),
            ])),
            ..Default::default()
        }
    };

    Ok(Container {
        name: "sandbox".to_string(),
        image: Some(image),
        image_pull_policy: Some(config.sandbox.image_pull_policy.clone()),
        command: Some(vec!["/usr/local/bin/sandbox".to_string()]),
        ports: Some(vec![
            ContainerPort {
                container_port: 8001,
                name: Some("http".to_string()),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            ContainerPort {
                container_port: config.sandbox.grpc_port as i32,
                name: Some("grpc".to_string()),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ]),
        env: Some(env),
        startup_probe: Some(startup_probe),
        volume_mounts: Some(volume_mounts),
        resources: Some(resources),
        security_context: Some(drop_all_security_context()),
        ..Default::default()
    })
}

/// Build server containers from repo config.
///
/// Each server container's image is resolved from `server.docker_image`,
/// falling back to `config.sandbox.image`. Start commands come from
/// `server.start_commands`; if absent the image's default CMD is used.
#[allow(clippy::unnecessary_wraps)] // Result for consistency with build_sandbox_container
fn build_server_containers(
    config: &Config,
    plan: &ProvisionPlan,
) -> Result<Vec<Container>, String> {
    let servers = match &plan.repo_config.servers {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(vec![]),
    };

    let mut containers = Vec::new();

    for server in servers {
        let container_name = format!("{}-server", server.name);
        let port = server.port;

        // Image: per-server override → repo-level override → global fallback.
        // Treat empty strings as unset so cleared-but-non-null DB values fall through.
        let image = server
            .docker_image
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| {
                plan.repo_config
                    .docker_image
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .cloned()
            })
            .unwrap_or_else(|| config.sandbox.image.clone());

        // Standard env vars for all server containers
        let mut env = vec![
            env_var("PORT", &port.to_string()),
            env_var("WORKSPACE_PATH", "/opt/moatless/repo"),
            env_var("DATA_DIR", "/opt/moatless/data"),
        ];

        // Per-server env vars from config
        if let Some(ref extra) = server.env_vars {
            for (k, v) in extra {
                env.push(env_var(k, v));
            }
        }

        // Command: always set command to override the image ENTRYPOINT.
        // Server containers reuse the sandbox image (ENTRYPOINT: runs sandbox
        // binary), which we must override.
        // The inline wait polls for the workspace-ready sentinel written by
        // the sandbox container after setup commands complete, then sources
        // the shared server.env file written by the sandbox init.
        let (command, args) = match &server.start_commands {
            Some(cmds) if !cmds.is_empty() => {
                let joined_cmd = cmds.join(" && ");
                let full_cmd = format!(
                    "until [ -f /opt/moatless/data/.workspace-ready ]; do sleep 2; done && \
                     if [ -f /opt/moatless/data/server.env ]; then set -a && . /opt/moatless/data/server.env && set +a; fi && \
                     cd /opt/moatless/repo && {joined_cmd}"
                );
                (
                    Some(vec!["bash".to_string(), "-c".to_string()]),
                    Some(vec![full_cmd]),
                )
            }
            _ => (None, None),
        };

        // Volume mounts — server containers share workspace-data + shared-data +
        // build-cache with the sandbox container. `workspace-data` (PVC or ephemeral
        // emptyDir) is always present, so the server sees the same `/opt/moatless`.
        let volume_mounts = vec![
            VolumeMount {
                name: "shared-data".to_string(),
                mount_path: "/opt/moatless/data".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "build-cache".to_string(),
                mount_path: "/opt/moatless/build-cache".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "workspace-data".to_string(),
                mount_path: "/opt/moatless".to_string(),
                ..Default::default()
            },
        ];

        // Readiness probe — TCP check on server port
        let readiness_probe = Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(port as i32),
                ..Default::default()
            }),
            period_seconds: Some(5),
            failure_threshold: Some(3),
            initial_delay_seconds: Some(5),
            ..Default::default()
        };

        // Resource limits: use per-repo override if set, otherwise global defaults
        let resources = if let Some(ref res) = plan.repo_config.resources {
            ResourceRequirements {
                limits: res.limits.as_ref().map(|l| {
                    l.iter()
                        .map(|(k, v)| (k.clone(), Quantity(v.clone())))
                        .collect()
                }),
                requests: res.requests.as_ref().map(|r| {
                    r.iter()
                        .map(|(k, v)| (k.clone(), Quantity(v.clone())))
                        .collect()
                }),
                ..Default::default()
            }
        } else {
            ResourceRequirements {
                limits: Some(BTreeMap::from([
                    (
                        "cpu".to_string(),
                        Quantity(config.sandbox.resources_cpu_limit.clone()),
                    ),
                    (
                        "memory".to_string(),
                        Quantity(config.sandbox.resources_memory_limit.clone()),
                    ),
                ])),
                requests: Some(BTreeMap::from([
                    (
                        "cpu".to_string(),
                        Quantity(config.sandbox.resources_cpu_request.clone()),
                    ),
                    (
                        "memory".to_string(),
                        Quantity(config.sandbox.resources_memory_request.clone()),
                    ),
                ])),
                ..Default::default()
            }
        };

        containers.push(Container {
            name: container_name,
            image: Some(image),
            image_pull_policy: Some(config.sandbox.image_pull_policy.clone()),
            command,
            args,
            ports: Some(vec![ContainerPort {
                container_port: port as i32,
                name: Some("http".to_string()),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            }]),
            env: Some(env),
            readiness_probe: Some(readiness_probe),
            volume_mounts: Some(volume_mounts),
            resources: Some(resources),
            security_context: Some(drop_all_security_context()),
            ..Default::default()
        });
    }

    Ok(containers)
}

/// Build REPO_CONFIG JSON string for the sandbox container.
///
/// This is a pass-through of the repo config fields that the sandbox
/// needs (system prompt, code execution, plugin IDs, etc.).
/// Server start commands are NOT included — they're handled by K8s command/args.
fn build_repo_config_json(plan: &ProvisionPlan) -> Option<String> {
    let rc = &plan.repo_config;
    if rc.system_prompt.is_none()
        && rc.code_execution.is_none()
        && rc.preview_config.is_none()
        && rc.servers.is_none()
        && rc.plugin_ids.is_none()
        && rc.plugin_repos.is_none()
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
                let mut entry = serde_json::json!({
                    "name": s.name,
                    "port": s.port,
                    "autoReload": s.auto_reload,
                });
                if let Some(ref label) = s.label {
                    entry["label"] = serde_json::json!(label);
                }
                if let Some(ref cmds) = s.start_commands {
                    entry["startCommands"] = serde_json::json!(cmds);
                }
                entry
            })
            .collect();
        map.insert("servers".into(), serde_json::json!(server_entries));
    }
    if let Some(ref pi) = rc.plugin_ids {
        map.insert("pluginIds".into(), serde_json::json!(pi));
    }
    if let Some(ref pr) = rc.plugin_repos {
        map.insert("pluginRepos".into(), serde_json::json!(pr));
    }

    Some(serde_json::Value::Object(map).to_string())
}

/// Security context that drops all capabilities except those needed at runtime.
///
/// DAC_OVERRIDE is required because the container runs as root (UID 0) but
/// the home directory (/home/moatless) is owned by the moatless user (UID 1000).
/// Without DAC_OVERRIDE, root cannot write to those directories (e.g. gh CLI
/// config, Claude onboarding flag), causing sandbox init to fail.
///
/// TODO: Harden by switching to runAsUser: 1000 (moatless) in the pod security
/// context and removing DAC_OVERRIDE. This requires ensuring all workspace dirs,
/// volume mounts, and build caches are accessible to UID 1000 (via fsGroup or
/// proper ownership in the Dockerfile).
fn drop_all_security_context() -> SecurityContext {
    SecurityContext {
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".to_string()]),
            add: Some(vec![
                "NET_BIND_SERVICE".to_string(),
                "DAC_OVERRIDE".to_string(),
            ]),
        }),
        ..Default::default()
    }
}

/// Privileged security context for the `nvme-setup` init container.
///
/// Required to `mkfs.ext4` / `mount` the raw instance-store NVMe device. Used
/// ONLY by the short-lived init container — never by the agent/server
/// containers, which keep `drop_all_security_context()`.
fn privileged_security_context() -> SecurityContext {
    SecurityContext {
        privileged: Some(true),
        ..Default::default()
    }
}

/// Create an EnvVar.
fn env_var(name: &str, value: &str) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value: Some(value.to_string()),
        ..Default::default()
    }
}

/// Get current time as ISO 8601 string (no chrono dependency — use simple format).
fn chrono_now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{now}Z")
}

/// Build servers name→port map from plan.
pub fn build_servers_map(plan: &ProvisionPlan) -> HashMap<String, u16> {
    let mut servers = HashMap::new();
    if let Some(ref server_configs) = plan.repo_config.servers {
        for s in server_configs {
            servers.insert(s.name.clone(), s.port);
        }
    }
    servers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::models::{RepoConfig, ServerConfig};

    fn test_config() -> Config {
        use crate::config::*;
        Config {
            server: crate::config::ServerConfig {
                port: 8080,
                metrics_port: 9090,
                grpc_port: 50051,
            },
            sandbox_provider: SandboxProviderType::Kubernetes,
            sandbox: SandboxConfig {
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
                storage_enabled: false,
                storage_size: "1Gi".into(),
                storage_class: None,
                pod_startup_timeout_secs: 10,
                task_token_secret: Some("secret".into()),
                file_mount_path: "/tmp".into(),
                idle_reaper_enabled: false,
                idle_timeout_minutes: 15,
                idle_check_interval_secs: 300,
                inactive_task_timeout_days: 0,
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
                enable_socket_mount: false,
                socket_path: "/var/run/docker.sock".into(),
                extra_networks: vec![],
            },
            database_url: None,
            database: DatabaseConfig {
                url: None,
                max_connections: 5,
            },
            redis: RedisConfig {
                url: None,
                max_stream_length: 100,
            },
            sse: SseConfig {
                broadcast_capacity: 16,
                consumer_id: "test".into(),
            },
            auth: AuthConfig {
                provider: None,
                github_client_id: None,
                github_client_secret: None,
                github_callback_url: None,
                github_oauth_scope: "".into(),
                github_required_org: None,
                auth0_domain: None,
                auth0_client_id: None,
                auth0_client_secret: None,
                auth0_callback_url: None,
                auth0_audience: None,
                auth0_groups_claim: "".into(),
                default_team_slug: None,
                session_max_age_secs: 3600,
                frontend_url: "http://localhost:3000".into(),
                cookie_domain: None,
                session_cookie_name: "test".into(),
                is_production: false,
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
            github_pr: crate::config::GithubPrConfig {
                webhook_secret: None,
                bot_token: None,
            },
            linear: crate::config::LinearConfig {
                webhook_secret: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                oauth_callback_url: None,
            },
            skill: crate::config::SkillConfig { cache_ttl_secs: 60 },
            secrets: crate::config::SecretsConfig {
                encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
                kms_key_id: None,
            },
            codex_oauth: crate::config::CodexOAuthConfig {
                issuer: "https://auth.openai.com".into(),
                client_id: "test-codex-client".into(),
            },
            default_run_as_user_id: None,
        }
    }

    fn test_plan() -> ProvisionPlan {
        ProvisionPlan {
            repo_config: RepoConfig {
                name: "repo".into(),
                docker_image: None,
                servers: Some(vec![ServerConfig {
                    name: "web".into(),
                    port: 3000,
                    start_commands: Some(vec!["bun run dev".into()]),
                    ..Default::default()
                }]),
                plugin_ids: None,
                system_prompt: None,
                code_execution: None,
                preview_config: None,
                resources: None,
                plugin_repos: None,
                docker_access: None,
                docker_networks: None,
            },
            git_clone_url: "https://github.com/test/repo.git".into(),
            git_branch: None,
            git_provider: "github".into(),
            env_vars: None,
        }
    }

    #[test]
    fn server_container_sources_server_env_in_startup() {
        let config = test_config();
        let plan = test_plan();

        let containers = build_server_containers(&config, &plan).unwrap();
        let web = containers
            .iter()
            .find(|container| container.name == "web-server")
            .unwrap();
        let args = web.args.as_ref().unwrap();
        assert_eq!(args.len(), 1);
        let startup = &args[0];
        assert!(
            startup.contains(". /opt/moatless/data/server.env"),
            "expected server.env sourcing in startup command, got: {startup}"
        );
        assert!(
            startup.contains(".workspace-ready"),
            "expected workspace-ready gate in startup command, got: {startup}"
        );
    }

    #[test]
    fn server_container_treats_empty_repo_image_as_unset() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.docker_image = Some(String::new());

        let containers = build_server_containers(&config, &plan).unwrap();
        let web = containers
            .iter()
            .find(|container| container.name == "web-server")
            .unwrap();
        assert_eq!(web.image.as_deref(), Some("sandbox:latest"));
    }

    #[test]
    fn server_container_treats_empty_server_image_as_unset() {
        let config = test_config();
        let mut plan = test_plan();
        if let Some(ref mut servers) = plan.repo_config.servers {
            servers[0].docker_image = Some(String::new());
        }

        let containers = build_server_containers(&config, &plan).unwrap();
        let web = containers
            .iter()
            .find(|container| container.name == "web-server")
            .unwrap();
        assert_eq!(web.image.as_deref(), Some("sandbox:latest"));
    }

    #[test]
    fn resolve_sandbox_image_prefers_repo_override() {
        let config = test_config();

        // case: no override → config default
        let plan = test_plan();
        assert_eq!(resolve_sandbox_image(&config, &plan), "sandbox:latest");

        // case: empty override → treated as unset → config default
        let mut plan = test_plan();
        plan.repo_config.docker_image = Some(String::new());
        assert_eq!(resolve_sandbox_image(&config, &plan), "sandbox:latest");

        // case: non-empty override → wins over config default
        let mut plan = test_plan();
        plan.repo_config.docker_image = Some("custom:v2".into());
        assert_eq!(resolve_sandbox_image(&config, &plan), "custom:v2");
    }

    #[test]
    fn sandbox_container_has_memory_request_and_limit() {
        let config = test_config();
        let plan = test_plan();

        let container =
            build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        let resources = container
            .resources
            .as_ref()
            .expect("sandbox container must have resources populated");

        let requests = resources.requests.as_ref().expect("requests set");
        assert_eq!(requests.get("memory").map(|q| q.0.as_str()), Some("2Gi"));
        assert_eq!(requests.get("cpu").map(|q| q.0.as_str()), Some("1"));

        let limits = resources.limits.as_ref().expect("limits set");
        assert_eq!(limits.get("memory").map(|q| q.0.as_str()), Some("8Gi"));
        assert_eq!(limits.get("cpu").map(|q| q.0.as_str()), Some("4"));
    }

    #[test]
    fn sandbox_container_honors_per_repo_resource_override() {
        use crate::repository::models::ResourceConfig;

        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.resources = Some(ResourceConfig {
            requests: Some(HashMap::from([("memory".to_string(), "4Gi".to_string())])),
            limits: Some(HashMap::from([("memory".to_string(), "16Gi".to_string())])),
            storage: None,
            storage_mode: None,
        });

        let container =
            build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        let resources = container.resources.as_ref().unwrap();
        assert_eq!(
            resources
                .requests
                .as_ref()
                .and_then(|r| r.get("memory"))
                .map(|q| q.0.as_str()),
            Some("4Gi")
        );
        assert_eq!(
            resources
                .limits
                .as_ref()
                .and_then(|l| l.get("memory"))
                .map(|q| q.0.as_str()),
            Some("16Gi")
        );
    }

    #[test]
    fn ephemeral_tier_mounts_capped_workspace_empty_dir() {
        use crate::repository::models::ResourceConfig;

        // No PVC (ephemeral tier) → workspace-data is a node-local emptyDir sized
        // from the repo's resources.storage, mounted at /opt/moatless everywhere.
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.resources = Some(ResourceConfig {
            requests: None,
            limits: None,
            storage: Some("7Gi".into()),
            storage_mode: Some("ephemeral".into()),
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let ws = volumes
            .iter()
            .find(|v| v.name == "workspace-data")
            .expect("workspace-data volume present");
        let empty = ws
            .empty_dir
            .as_ref()
            .expect("workspace-data is an emptyDir in the ephemeral tier");
        // Node disk, not RAM: medium must be unset (None/empty).
        assert!(
            empty.medium.as_deref().unwrap_or("").is_empty(),
            "ephemeral workspace must live on node disk, not memory"
        );
        assert_eq!(empty.size_limit.as_ref().map(|q| q.0.as_str()), Some("7Gi"));

        // No PVC volume should be present.
        assert!(
            volumes
                .iter()
                .all(|v| v.persistent_volume_claim.is_none()),
            "ephemeral tier must not reference a PVC"
        );

        // The sandbox container mounts the workspace emptyDir at /opt/moatless and /tmp.
        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        let mounts = container.volume_mounts.as_ref().unwrap();
        assert!(
            mounts
                .iter()
                .any(|m| m.name == "workspace-data" && m.mount_path == "/opt/moatless"),
            "sandbox must mount workspace-data at /opt/moatless"
        );
        assert!(
            mounts.iter().any(|m| m.name == "workspace-data"
                && m.mount_path == "/tmp"
                && m.sub_path.as_deref() == Some("tmp")),
            "/tmp must be a subPath of workspace-data"
        );

        // Server containers share the same workspace volume.
        let servers = build_server_containers(&config, &plan).unwrap();
        let web = servers
            .iter()
            .find(|c| c.name == "web-server")
            .expect("web-server present");
        assert!(
            web.volume_mounts
                .as_ref()
                .unwrap()
                .iter()
                .any(|m| m.name == "workspace-data" && m.mount_path == "/opt/moatless"),
            "server containers must share workspace-data"
        );
    }

    #[test]
    fn ephemeral_workspace_size_falls_back_to_global_default() {
        // No per-repo storage override → emptyDir uses the global storage_size.
        let mut config = test_config();
        config.sandbox.storage_size = "3Gi".into();
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let ws = volumes
            .iter()
            .find(|v| v.name == "workspace-data")
            .expect("workspace-data volume present");
        assert_eq!(
            ws.empty_dir
                .as_ref()
                .and_then(|e| e.size_limit.as_ref())
                .map(|q| q.0.as_str()),
            Some("3Gi")
        );
    }

    /// Two cache entries → ONE shared `pkg-cache` hostPath volume + the
    /// `host-mnt` parent volume + a privileged `nvme-setup` init container + a
    /// per-entry subPath mount and per-entry env on the sandbox container.
    #[test]
    fn cache_volumes_wire_volume_mounts_env_and_init() {
        use crate::config::SandboxCacheVolume;

        let mut config = test_config();
        config.sandbox.cache_volumes = vec![
            SandboxCacheVolume {
                name: "maven".into(),
                mount_path: "/maven-cache".into(),
                env: vec![(
                    "MAVEN_OPTS".into(),
                    "-Dmaven.repo.local=/maven-cache".into(),
                )],
            },
            SandboxCacheVolume {
                name: "bun".into(),
                mount_path: "/bun-cache".into(),
                env: vec![("BUN_INSTALL_CACHE_DIR".into(), "/bun-cache".into())],
            },
        ];
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();

        // Exactly one shared pkg-cache hostPath volume rooted at the cache root,
        // plus the host-mnt parent volume for the init container.
        let volumes = spec.volumes.as_ref().unwrap();
        let pkg: Vec<_> = volumes.iter().filter(|v| v.name == "pkg-cache").collect();
        assert_eq!(pkg.len(), 1, "exactly one shared pkg-cache volume for N caches");
        let hp = pkg[0]
            .host_path
            .as_ref()
            .expect("pkg-cache must be a hostPath volume");
        assert_eq!(hp.path, "/mnt/cache");
        assert_eq!(hp.type_.as_deref(), Some("DirectoryOrCreate"));
        let host_mnt = volumes
            .iter()
            .find(|v| v.name == "host-mnt")
            .expect("host-mnt parent volume present");
        assert_eq!(host_mnt.host_path.as_ref().unwrap().path, "/mnt");

        // Privileged nvme-setup init container with a bidirectional host-mnt mount
        // and a script that formats the NVMe and creates each cache subdir.
        let inits = spec
            .init_containers
            .as_ref()
            .expect("init containers present when caches configured");
        let nvme = inits
            .iter()
            .find(|c| c.name == "nvme-setup")
            .expect("nvme-setup init container present");
        assert_eq!(
            nvme.security_context.as_ref().and_then(|s| s.privileged),
            Some(true),
            "nvme-setup must be privileged"
        );
        let init_mount = nvme
            .volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .find(|m| m.name == "host-mnt")
            .unwrap();
        assert_eq!(init_mount.mount_propagation.as_deref(), Some("Bidirectional"));
        let init_script = nvme.args.as_ref().unwrap().join("\n");
        assert!(init_script.contains("mkfs.ext4 -F /dev/nvme1n1"));
        assert!(init_script.contains("/host-mnt/cache/maven"));
        assert!(init_script.contains("/host-mnt/cache/bun"));

        // Sandbox container: one subPath mount per cache + the matching env.
        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        let mounts = container.volume_mounts.as_ref().unwrap();
        assert!(
            mounts.iter().any(|m| m.name == "pkg-cache"
                && m.mount_path == "/maven-cache"
                && m.sub_path.as_deref() == Some("maven")),
            "maven cache must mount via subPath"
        );
        assert!(
            mounts.iter().any(|m| m.name == "pkg-cache"
                && m.mount_path == "/bun-cache"
                && m.sub_path.as_deref() == Some("bun")),
            "bun cache must mount via subPath"
        );
        let env = container.env.as_ref().unwrap();
        assert!(env.iter().any(|e| e.name == "MAVEN_OPTS"
            && e.value.as_deref() == Some("-Dmaven.repo.local=/maven-cache")));
        assert!(env.iter().any(|e| e.name == "BUN_INSTALL_CACHE_DIR"
            && e.value.as_deref() == Some("/bun-cache")));

        // Caches are scoped to the agent container — server containers must not
        // mount the shared package cache.
        let servers = build_server_containers(&config, &plan).unwrap();
        for s in &servers {
            assert!(
                s.volume_mounts
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|m| m.name != "pkg-cache"),
                "server containers must not mount pkg-cache"
            );
        }
    }

    /// No cache list → hardened default: no init container, no hostPath volume.
    #[test]
    fn no_cache_volumes_means_no_init_or_hostpath() {
        let config = test_config(); // cache_volumes empty by default
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();
        assert!(
            spec.init_containers.is_none(),
            "no init containers without configured caches"
        );
        assert!(
            spec.volumes
                .as_ref()
                .unwrap()
                .iter()
                .all(|v| v.host_path.is_none()),
            "no hostPath volumes without configured caches"
        );
    }

    #[test]
    fn cache_host_parent_derives_mount_propagation_root() {
        assert_eq!(cache_host_parent("/mnt/cache"), "/mnt");
        assert_eq!(cache_host_parent("/mnt/cache/"), "/mnt");
        assert_eq!(cache_host_parent("/var/lib/pkg-cache"), "/var/lib");
        assert_eq!(cache_host_parent("/mnt"), "/");
    }
}
