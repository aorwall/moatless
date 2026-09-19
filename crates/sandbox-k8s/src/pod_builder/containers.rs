//! The containers on the pod — sandbox, servers and Docker-in-Docker — and the
//! resource envelope each compute mode admits.

use super::env::{env_var, redact_url_userinfo};
use super::placement::{resolve_sandbox_image, resolve_sandbox_size};
use super::security::{app_security_context, privileged_security_context};
use super::*;

/// Whether this repo opted into Docker access (privileged dind sidecar).
///
/// Reuses the existing `RepoConfig.docker_access` flag (already honoured by the
/// Docker provider via a socket mount); in K8s mode it means a privileged
/// `dind` sidecar instead. Defaults to `false`, so sandboxes stay hardened
/// unless a repo opts in.
pub(super) fn docker_access_enabled(plan: &ProvisionPlan) -> bool {
    plan.repo_config.docker_access.unwrap_or(false)
}

/// Build the privileged `dind` (docker-in-docker) sidecar container.
///
/// Runs `dockerd` without TLS on `tcp://0.0.0.0:2375`, reachable only inside
/// the pod's network namespace (i.e. via `localhost`), plus the default socket;
/// layers persist on the `docker-storage` volume. Only attached when a repo
/// opts into `dockerAccess`; the agent/server containers keep
/// `app_security_context`.
pub(super) fn build_dind_container(config: &Config) -> Container {
    let resources = ResourceRequirements {
        limits: Some(BTreeMap::from([
            (
                "cpu".to_string(),
                Quantity(config.sandbox.docker_dind_cpu_limit.clone()),
            ),
            (
                "memory".to_string(),
                Quantity(config.sandbox.docker_dind_memory_limit.clone()),
            ),
        ])),
        requests: Some(BTreeMap::from([
            (
                "cpu".to_string(),
                Quantity(config.sandbox.docker_dind_cpu_request.clone()),
            ),
            (
                "memory".to_string(),
                Quantity(config.sandbox.docker_dind_memory_request.clone()),
            ),
        ])),
        ..Default::default()
    };

    Container {
        name: "dind".to_string(),
        image: Some(config.sandbox.docker_dind_image.clone()),
        image_pull_policy: Some(config.sandbox.image_pull_policy.clone()),
        // Keep the image ENTRYPOINT (`dockerd-entrypoint.sh`), which performs the
        // cgroup/iptables setup before exec'ing the `dockerd` we pass as args.
        args: Some(vec![
            "dockerd".to_string(),
            "--host=tcp://0.0.0.0:2375".to_string(),
            "--host=unix:///var/run/docker.sock".to_string(),
            "--tls=false".to_string(),
            // Back to the overlay2 graphdriver: Docker 29 defaults to the
            // containerd image store, whose copier treats a refused xattr as
            // fatal, so on these SELinux-labelled nodes every layer unpack and
            // every seed of a declared `VOLUME` dies with `failed to set xattr
            // "security.selinux"`. overlay2's copier never asks for that one.
            "--feature=containerd-snapshotter=false".to_string(),
        ]),
        // Disable the entrypoint's automatic TLS cert generation so it listens
        // in plaintext on 2375.
        env: Some(vec![env_var("DOCKER_TLS_CERTDIR", "")]),
        security_context: Some(privileged_security_context()),
        volume_mounts: Some(vec![VolumeMount {
            name: "docker-storage".to_string(),
            mount_path: "/var/lib/docker".to_string(),
            ..Default::default()
        }]),
        readiness_probe: Some(Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(2375),
                ..Default::default()
            }),
            period_seconds: Some(5),
            failure_threshold: Some(6),
            initial_delay_seconds: Some(2),
            ..Default::default()
        }),
        resources: Some(resources),
        ..Default::default()
    }
}

/// Overlay a per-repo resource override onto the configured defaults, field by
/// field.
///
/// An override replaces only the `cpu`/`memory` values it names, the rest keep
/// their default; since `ResourceConfig` also carries storage fields, a repo
/// configuring only storage or only memory still gets a fully bounded container
/// instead of `BestEffort`.
pub(super) fn merge_resources(
    override_cfg: Option<&ResourceConfig>,
    default_cpu_request: &str,
    default_memory_request: &str,
    default_cpu_limit: &str,
    default_memory_limit: &str,
) -> ResourceRequirements {
    let quantities = |over: Option<&ResourceQuantities>,
                      default_cpu: &str,
                      default_memory: &str|
     -> BTreeMap<String, Quantity> {
        BTreeMap::from([
            (
                "cpu".to_string(),
                Quantity(
                    over.and_then(|q| q.cpu.as_ref())
                        .map_or_else(|| default_cpu.to_string(), Cpu::to_string),
                ),
            ),
            (
                "memory".to_string(),
                Quantity(
                    over.and_then(|q| q.memory.as_ref())
                        .map_or_else(|| default_memory.to_string(), Memory::to_string),
                ),
            ),
        ])
    };

    ResourceRequirements {
        limits: Some(quantities(
            override_cfg.map(|c| &c.limits),
            default_cpu_limit,
            default_memory_limit,
        )),
        requests: Some(quantities(
            override_cfg.map(|c| &c.requests),
            default_cpu_request,
            default_memory_request,
        )),
        ..Default::default()
    }
}

pub(super) fn validate_fargate_resources(
    resources: &ResourceRequirements,
    container: &str,
) -> Result<(), String> {
    let requests = resources.requests.as_ref().ok_or_else(|| {
        format!("EKS Fargate requires resource requests for the {container} container")
    })?;
    let limits = resources.limits.as_ref().ok_or_else(|| {
        format!("EKS Fargate requires resource limits for the {container} container")
    })?;
    for name in ["cpu", "memory"] {
        if requests.get(name) != limits.get(name) {
            return Err(format!(
                "EKS Fargate requires equal {name} requests and limits for the {container} container"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_compute_mode(
    config: &Config,
    plan: &ProvisionPlan,
    pvc_name: Option<&str>,
) -> Result<(), String> {
    if !config.sandbox.compute_mode.is_fargate() {
        return Ok(());
    }
    if docker_access_enabled(plan) {
        return Err(
            "EKS Fargate does not support dockerAccess because privileged DinD containers are unavailable"
                .to_string(),
        );
    }
    let storage = moatless_sandbox_runtime::storage::StorageBackend::from_plan(
        plan,
        config.sandbox.default_pvc_enabled,
        config.sandbox.default_s3_enabled,
    );
    if storage.uses_pvc() || !storage.syncs_sandbox() {
        return Err(
            "EKS Fargate requires resources.pvcEnabled=false and resources.s3Enabled=true"
                .to_string(),
        );
    }
    if pvc_name.is_some() {
        return Err("EKS Fargate does not support persistent workspace volume claims".to_string());
    }
    if config.sandbox.node_archive_enabled {
        return Err("EKS Fargate does not support node-backed archives".to_string());
    }
    Ok(())
}

/// Build the main sandbox container.
#[allow(clippy::unnecessary_wraps)] // Result for consistency with build_server_containers
pub(super) fn build_sandbox_container(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    tokens: Option<SandboxAuthTokens<'_>>,
    // The claim actually mounted at `/opt/moatless`, not the one the plan asks
    // for. `None` means this pod runs on an emptyDir — see the storage-switch
    // env block below, where the difference is load-bearing.
    pvc_name: Option<&str>,
) -> Result<Container, String> {
    // Resolve image: repo config override → default sandbox image
    let image = resolve_sandbox_image(config, plan);

    // Build env vars. GIT_CLONE_URL is credential-free — the sandbox clones via
    // the credential channel (GIT_TOKEN) / git credential helper, so no PAT is
    // embedded in the pod spec.
    let mut env = vec![
        env_var("TASK_ID", task_id),
        env_var("SANDBOX_ID", task_id),
        env_var("GIT_CLONE_URL", &redact_url_userinfo(&plan.git_clone_url)),
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

    // Where the sandbox writes a message's attached uploads. Passed explicitly
    // rather than left to the sandbox's own default because the backend renders
    // the same value into the prompt's "Saved to:" line: two independent
    // defaults would let the advertised path and the written path drift apart.
    env.push(env_var("UPLOAD_DIR", &config.sandbox.file_mount_path));

    // Git provider (github, gitlab, gitness)
    env.push(env_var("GIT_PROVIDER", &plan.git_provider));

    // Every repository the workspace places in the sandbox, primary first. The
    // GIT_* trio above describes the primary, and WORKSPACE_REPOS carries the
    // full checkout list. Clone URLs are redacted like GIT_CLONE_URL — no PAT
    // reaches the pod spec.
    if let Some(repos_json) = build_workspace_repos_json(plan) {
        env.push(env_var(WORKSPACE_REPOS_ENV, &repos_json));
    }

    // Workspace storage switches. With S3 on, the sandbox syncs the git working
    // tree to S3 via presigned URLs — flag it so it uploads after terminal agent
    // turns and restores on boot.
    //
    // `WORKSPACE_RESTORE_IF_EMPTY` claims the mounted volume is *durable*: the
    // sandbox then treats S3 as a fallback read only, and drops the periodic
    // upload backstop because the volume survives an involuntary kill. That has
    // to follow the volume this pod actually gets, not the one the plan asked
    // for — the caller passes `pvc_name: None` when PVC creation degraded or the
    // attach-failure fallback rebuilt onto an emptyDir, and such a pod must
    // behave like the emptyDir+S3 tier (restore every boot, keep the backstop)
    // even though its plan still reads pvc+s3.
    let storage_backend = moatless_sandbox_runtime::storage::StorageBackend::from_plan(
        plan,
        config.sandbox.default_pvc_enabled,
        config.sandbox.default_s3_enabled,
    );
    if storage_backend.syncs_sandbox() {
        env.push(env_var("WORKSPACE_SYNC_ENABLED", "true"));
    }
    if storage_backend.restore_if_empty() && pvc_name.is_some() {
        env.push(env_var("WORKSPACE_RESTORE_IF_EMPTY", "true"));
    }

    env.push(env_var(
        WORKSPACE_LIMIT_ENV,
        &resolve_sandbox_size(config, plan),
    ));

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

    // Egress forward-proxy: when this pod's network profile opts into the
    // in-cluster allowlist proxy, route all outbound HTTP(S) through it. The
    // paired per-profile NetworkPolicy blocks direct internet egress, so the
    // proxy's hostname allowlist becomes the only way out (e.g. an approved API
    // host is allowed; everything else is denied). NO_PROXY keeps in-cluster
    // traffic (backend gRPC, DNS) direct.
    if let (Some(proxy_url), Some(profile)) = (
        config.sandbox.egress_proxy_url.as_deref(),
        plan.repo_config
            .network_profile
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty()),
    ) {
        if config
            .sandbox
            .egress_proxy_profiles
            .iter()
            .any(|p| p == profile)
        {
            for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
                env.push(env_var(key, proxy_url));
            }
            env.push(env_var("NO_PROXY", &config.sandbox.egress_proxy_no_proxy));
            env.push(env_var("no_proxy", &config.sandbox.egress_proxy_no_proxy));
        }
    }

    // Default model for agent sessions
    if let Some(ref model) = config.sandbox.default_model {
        env.push(env_var("DEFAULT_CLAUDE_CODE_MODEL", model));
    }
    if let Some(ref model) = config.sandbox.default_codex_model {
        env.push(env_var("DEFAULT_CODEX_MODEL", model));
    }

    // Auth tokens. The short-lived bootstrap token is the only auth secret on
    // the pod spec; it authenticates the initial GetCredentials call, after which
    // the sandbox swaps to the long-lived token delivered over that channel. The
    // USER_AUTH_TOKEN is emitted only when SANDBOX_EMIT_LEGACY_USER_TOKEN is set.
    if let Some(tokens) = tokens {
        env.push(env_var("SANDBOX_BOOTSTRAP_TOKEN", tokens.bootstrap));
        if let Some(legacy) = tokens.legacy_user_token {
            env.push(env_var("USER_AUTH_TOKEN", legacy));
        }
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

    // Docker-in-Docker access (opt-in). Point the Docker CLI + Testcontainers at
    // the privileged `dind` sidecar over the shared pod network namespace, and
    // disable Ryuk — its socket-reaper doesn't play well with the dind topology
    // (same as the CI runners). The sidecar itself is added in `build_pod`.
    if docker_access_enabled(plan) {
        env.push(env_var("DOCKER_HOST", "tcp://localhost:2375"));
        env.push(env_var("TESTCONTAINERS_RYUK_DISABLED", "true"));
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

    // Liveness probe (opt-in via `K8S_SANDBOX_LIVENESS_ENABLED`). Hits the
    // sandbox `/livez` endpoint, which reports unhealthy only when the agent
    // actor loop is wedged — never on backend/credential/disk trouble. Settings
    // are deliberately conservative: 6 consecutive failures at 10s spacing (~60s)
    // before a restart, because a false-positive restart kills an in-flight agent
    // turn, which is worse than a rare hang. Gated off by default; the startup
    // probe on `/health` remains the startup gate regardless.
    let liveness_probe = config.sandbox.liveness_enabled.then(|| Probe {
        http_get: Some(HTTPGetAction {
            path: Some("/livez".to_string()),
            port: IntOrString::Int(8001),
            ..Default::default()
        }),
        period_seconds: Some(10),
        timeout_seconds: Some(5),
        failure_threshold: Some(6),
        ..Default::default()
    });

    // Readiness probe — TCP connect on the gRPC control port. The backend
    // reaches this container only over gRPC (via the K8s Service), so pod
    // readiness must track the gRPC listener, not the HTTP `/health` startup
    // gate: `/health` on :8001 can answer before the gRPC socket on :50052 is
    // bound (or after it dies while HTTP still serves), and without this probe
    // the pod stays in the Service's endpoints through that window, so real
    // gRPC calls hit a dead socket and surface as connect timeouts / 503s.
    let readiness_probe = Probe {
        tcp_socket: Some(TCPSocketAction {
            port: IntOrString::Int(config.sandbox.grpc_port as i32),
            ..Default::default()
        }),
        period_seconds: Some(5),
        timeout_seconds: Some(3),
        failure_threshold: Some(3),
        ..Default::default()
    };

    // Volume mounts
    //
    // `workspace-data` (PVC or emptyDir) is always present and mounted at
    // `/opt/moatless`. `/tmp` is backed by its `tmp` subPath — the same volume, a
    // separate directory — so `/tmp` shares the workspace headroom and never charges
    // against container memory.
    let mut volume_mounts = vec![
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
        VolumeMount {
            name: "run-moatless".to_string(),
            mount_path: "/run/moatless".to_string(),
            ..Default::default()
        },
    ];

    // Persist cargo's downloaded state across pod replacement (opt-in via
    // `K8S_SANDBOX_PERSIST_TOOLCHAIN`). Only the registry and git caches ride
    // the workspace volume: mounting an empty subPath over all of `/opt/cargo`
    // (or `/opt/rust`) would shadow the toolchain baked into the image — K8s
    // subPath mounts don't seed from image content — whereas cargo repopulates
    // an empty registry/git cache transparently. Without a PVC the
    // subPaths land in the emptyDir and die with the pod (harmless no-op).
    if config.sandbox.persist_toolchain {
        volume_mounts.push(VolumeMount {
            name: "workspace-data".to_string(),
            mount_path: "/opt/cargo/registry".to_string(),
            sub_path: Some("cargo-registry".to_string()),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: "workspace-data".to_string(),
            mount_path: "/opt/cargo/git".to_string(),
            sub_path: Some("cargo-git".to_string()),
            ..Default::default()
        });
    }

    // Resource requests + limits for the agent container. A per-repo override
    // replaces the keys it names; the rest fall back to the global
    // `K8S_RESOURCE_*` config. Every sandbox container is bounded — a BestEffort
    // one is the first OOM/eviction target on spot nodes.
    let resources = merge_resources(
        plan.repo_config.resources.as_ref(),
        &config.sandbox.sandbox_resources_cpu_request,
        &config.sandbox.sandbox_resources_memory_request,
        &config.sandbox.sandbox_resources_cpu_limit,
        &config.sandbox.sandbox_resources_memory_limit,
    );
    if config.sandbox.compute_mode.is_fargate() {
        validate_fargate_resources(&resources, "sandbox")?;
    }
    let mut resources = resources;
    if let Some(size) = &config.sandbox.ephemeral_storage {
        resources
            .requests
            .get_or_insert_with(BTreeMap::new)
            .insert("ephemeral-storage".to_string(), Quantity(size.clone()));
        resources
            .limits
            .get_or_insert_with(BTreeMap::new)
            .insert("ephemeral-storage".to_string(), Quantity(size.clone()));
    }

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
            ContainerPort {
                container_port: i32::from(SANDBOX_PREVIEW_PROXY_PORT),
                name: Some("preview".to_string()),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ]),
        env: Some(env),
        startup_probe: Some(startup_probe),
        readiness_probe: Some(readiness_probe),
        liveness_probe,
        volume_mounts: Some(volume_mounts),
        resources: Some(resources),
        security_context: Some(app_security_context(config)),
        ..Default::default()
    })
}

/// Build server containers from repo config.
///
/// Each container's image resolves from `server.docker_image`, falling back to
/// `config.sandbox.image`; start commands come from `server.start_commands`, or
/// the image's default CMD if absent.
#[allow(clippy::unnecessary_wraps)] // Result for consistency with build_sandbox_container
pub(super) fn build_server_containers(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
) -> Result<Vec<Container>, String> {
    let servers = match &plan.repo_config.servers {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(vec![]),
    };

    let mut containers = Vec::new();

    for server in servers {
        let Some(port) = allocated_port(server, task_id) else {
            continue;
        };
        let container_name = format!("{}-server", server.name);

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
            env_var("WORKSPACE_DIR", "/opt/moatless/workspace"),
            env_var("WORKSPACE_PATH", "/opt/moatless/workspace"),
            env_var("DATA_DIR", "/opt/moatless/data"),
        ];

        // Per-server env vars from config
        for (k, v) in &server.env_vars {
            env.push(env_var(k.as_str(), v));
        }

        // Command: always set command to override the image ENTRYPOINT.
        // Server containers reuse the sandbox image (ENTRYPOINT: runs sandbox
        // binary), which we must override.
        // The script waits for the workspace-ready sentinel, sources the shared
        // env files, and supervises the child so a live config override can
        // restart it without rebuilding the pod — see `server_runner`.
        let (command, args) = if server.start_commands.is_empty() {
            (None, None)
        } else {
            (
                Some(vec!["bash".to_string(), "-c".to_string()]),
                Some(vec![build_server_start_script(
                    &server.name,
                    &server.start_commands,
                )]),
            )
        };

        // Volume mounts — server containers share workspace-data + shared-data
        // with the sandbox container. `workspace-data` (PVC or emptyDir) is
        // always present, so the server sees the same `/opt/moatless`.
        let volume_mounts = vec![
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

        // Resource limits: per-server override wins, then per-repo, then global
        // defaults. Servers are typically far lighter than the agent container,
        // so sizing them separately is what keeps pod density honest.
        let server_resources = server
            .resources
            .as_ref()
            .or(plan.repo_config.resources.as_ref());
        let resources = merge_resources(
            server_resources,
            &config.sandbox.resources_cpu_request,
            &config.sandbox.resources_memory_request,
            &config.sandbox.resources_cpu_limit,
            &config.sandbox.resources_memory_limit,
        );
        if config.sandbox.compute_mode.is_fargate() {
            validate_fargate_resources(&resources, &container_name)?;
        }

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
            security_context: Some(app_security_context(config)),
            ..Default::default()
        });
    }

    Ok(containers)
}

/// Serialize the plan's repository checkouts for the `WORKSPACE_REPOS` env.
///
/// Clone URLs are redacted exactly like `GIT_CLONE_URL`, so no credential
/// reaches the pod spec — the sandbox authenticates each clone via the
/// credential channel instead; `None` when the plan carries no checkouts.
pub(super) fn build_workspace_repos_json(plan: &ProvisionPlan) -> Option<String> {
    if plan.repos.is_empty() {
        return None;
    }
    let redacted: Vec<RepoCheckout> = plan
        .repos
        .iter()
        .map(|repo| RepoCheckout {
            uses_ssh: repo.uses_ssh,
            clone_url: redact_url_userinfo(&repo.clone_url),
            ..repo.clone()
        })
        .collect();
    match serde_json::to_string(&redacted) {
        Ok(json) => Some(json),
        Err(e) => {
            warn!(
                error = %e,
                "Failed to serialize WORKSPACE_REPOS; sandbox will clone the primary repo only"
            );
            None
        }
    }
}

pub(super) fn build_repo_config_json(plan: &ProvisionPlan) -> Option<String> {
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
            fields.insert("autoReload".into(), serde_json::json!(server.auto_reload));
            if let Some(label) = &server.label {
                fields.insert("label".into(), serde_json::json!(label));
            }
            fields
        },
    )
}
