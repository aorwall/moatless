//! The containers on the pod — sandbox and Docker-in-Docker — and the
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

/// Seeding an image's `VOLUME` copies `security.selinux`, which the dind's
/// Bottlerocket `control_t` may not relabel (`EACCES`, fatal to moby). A
/// `context=` overlay on `volumes` answers `ENOTSUP`, which moby skips; a node
/// lacking `local_t` refuses the mount and dockerd starts without it.
const DIND_START: &str = r#"D=/var/lib/docker; O="$D/.volumes-overlay"
mkdir -p "$D/volumes" "$O/upper" "$O/work"
mountpoint -q "$D/volumes" || mount -t overlay overlay \
  -o "lowerdir=$D/volumes,upperdir=$O/upper,workdir=$O/work,context=system_u:object_r:local_t:s0" \
  "$D/volumes" || echo "dind: no SELinux volumes overlay; seeding a VOLUME may fail" >&2
exec dockerd-entrypoint.sh "$@""#;

/// Build the privileged `dind` (docker-in-docker) sidecar container.
///
/// Runs `dockerd` without TLS on `tcp://0.0.0.0:2375`, reachable only inside
/// the pod's network namespace, plus the default socket; layers persist on the
/// `docker-storage` volume. The agent container keeps `app_security_context`.
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
        // `DIND_START` ends in the image's `dockerd-entrypoint.sh`, which does
        // the cgroup/iptables setup before exec'ing the `dockerd` in args.
        command: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            DIND_START.to_string(),
            "dind".to_string(),
        ]),
        args: Some(vec![
            "dockerd".to_string(),
            "--host=tcp://0.0.0.0:2375".to_string(),
            "--host=unix:///var/run/docker.sock".to_string(),
            "--tls=false".to_string(),
            // Back to the overlay2 graphdriver: Docker 29 defaults to the
            // containerd image store, whose copier treats a refused xattr as
            // fatal, so on these SELinux-labelled nodes every layer unpack dies
            // with `failed to set xattr "security.selinux"`. A `VOLUME` seed
            // still copies it whatever the driver: see `DIND_START`.
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

/// Overlay a per-repo resource override onto the configured defaults, field
/// by field: an override replaces only the `cpu`/`memory` it names.
///
/// `ResourceConfig` also carries storage fields, so a repo configuring only
/// storage still gets a fully bounded container instead of `BestEffort`.
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

    env.push(env_var("GIT_PROVIDER", &plan.git_provider));

    // Every repository the workspace places in the sandbox, primary first. The
    // GIT_* trio above describes the primary, and WORKSPACE_REPOS carries the
    // full checkout list. Clone URLs are redacted like GIT_CLONE_URL — no PAT
    // reaches the pod spec.
    if let Some(repos_json) = build_workspace_repos_json(plan) {
        env.push(env_var(WORKSPACE_REPOS_ENV, &repos_json));
    }

    // WORKSPACE_SYNC_ENABLED: sync the git tree to S3 on terminal agent turns
    // and restore it on boot. WORKSPACE_RESTORE_IF_EMPTY: the volume is
    // durable, so the periodic backstop upload is off — gated on
    // `pvc_name`, the volume this pod actually got, not the plan's request,
    // since a degraded/fallback pod runs emptyDir despite its plan.
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

    if let Some(repo_config_json) = build_repo_config_json(plan) {
        env.push(env_var("REPO_CONFIG", &repo_config_json));
    }

    if let Some(ref domain) = config.proxy.domain {
        env.push(env_var("PROXY_DOMAIN", domain));
        env.push(env_var("PROXY_PROTOCOL", &config.proxy.protocol));
    }

    env.push(env_var("FRONTEND_URL", &config.auth.frontend_url));

    // When this pod's network profile opts in, route outbound HTTP(S) through
    // the in-cluster allowlist proxy. The paired per-profile NetworkPolicy
    // blocks direct internet egress, so the proxy's hostname allowlist is the
    // only way out. NO_PROXY keeps in-cluster traffic (backend gRPC, DNS) direct.
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

    if let Some(ref model) = config.sandbox.default_model {
        env.push(env_var("DEFAULT_CLAUDE_CODE_MODEL", model));
    }

    // The bootstrap token is the only auth secret in the pod spec; it
    // authenticates the initial GetCredentials call, which hands back the
    // long-lived token. USER_AUTH_TOKEN is legacy, emitted only when
    // SANDBOX_EMIT_LEGACY_USER_TOKEN is set.
    for (key, value) in moatless_sandbox_runtime::client::grpc_auth_env(&config.sandbox, task_id) {
        env.push(env_var(key, &value));
    }
    if let Some(tokens) = tokens {
        env.push(env_var("SANDBOX_BOOTSTRAP_TOKEN", tokens.bootstrap));
        if let Some(legacy) = tokens.legacy_user_token {
            env.push(env_var("USER_AUTH_TOKEN", legacy));
        }
    }

    if let Some(ref tools) = config.sandbox.claude_disallowed_tools {
        env.push(env_var("SANDBOX_CLAUDE_DISALLOWED_TOOLS", tools));
    }

    for (key, value) in &config.sandbox.extra_env {
        env.push(env_var(key, value));
    }

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

    let startup_timeout = config.sandbox.pod_startup_timeout_secs;
    let startup_probe_period: i32 = 5;
    let failure_threshold = ((startup_timeout / startup_probe_period as u64) + 6) as i32;

    if let Some(stall_secs) = config.sandbox.git_clone_stall_timeout_secs {
        let window_secs = failure_threshold as u64 * startup_probe_period as u64;
        env.push(env_var(
            "GIT_CLONE_STALL_TIMEOUT_SECS",
            &clone_stall_within_startup_window(stall_secs, window_secs).to_string(),
        ));
    }

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

    // Opt-in (`K8S_SANDBOX_LIVENESS_ENABLED`). `/livez` is unhealthy only when
    // the agent actor loop is wedged, never for backend/credential/disk
    // trouble. A sandbox starved of CPU on a busy node times `/livez` out
    // without being wedged, so the default window is five minutes (20 failures
    // at 15s) rather than one: a false-positive restart kills an agent turn.
    let liveness_probe = config.sandbox.liveness_enabled.then(|| Probe {
        http_get: Some(HTTPGetAction {
            path: Some("/livez".to_string()),
            port: IntOrString::Int(8001),
            ..Default::default()
        }),
        period_seconds: Some(config.sandbox.liveness_period_secs),
        timeout_seconds: Some(config.sandbox.liveness_timeout_secs),
        failure_threshold: Some(config.sandbox.liveness_failure_threshold),
        ..Default::default()
    });

    // TCP probe on the gRPC port, not `/health`: `/health` on :8001 can answer
    // before the gRPC socket on :50052 binds (or after it dies). Without this,
    // the pod stays in the Service's endpoints through that window, and real
    // gRPC calls hit a dead socket as connect timeouts / 503s.
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

    // `workspace-data` (PVC or emptyDir) mounts at `/opt/moatless`; `/tmp` is
    // its `tmp` subPath, not a separate volume, so `/tmp` shares the workspace
    // headroom and never charges against container memory.
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

    // Opt-in (`K8S_SANDBOX_PERSIST_TOOLCHAIN`). Only cache dirs ride the
    // volume: subPath mounts don't seed from image content, so mounting an
    // empty subPath over all of `/opt/cargo` would shadow the baked-in
    // toolchain. Without a PVC these subPaths land in emptyDir (no-op).
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
    let request = config.sandbox.ephemeral_storage.as_ref();
    if let Some(size) = request {
        resources
            .requests
            .get_or_insert_with(BTreeMap::new)
            .insert("ephemeral-storage".to_string(), Quantity(size.clone()));
    }
    let limit = if config.sandbox.compute_mode.is_fargate() {
        request
    } else {
        config.sandbox.ephemeral_storage_limit.as_ref()
    };
    if let Some(size) = limit {
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

/// Headroom the startup probe window keeps for init's work before the clone.
const CLONE_STALL_STARTUP_HEADROOM_SECS: u64 = 30;

/// `stall_secs`, capped so a stalled clone fails inside the startup probe
/// window. Past it the probe SIGKILLs init first, and the clone error never
/// reaches the termination log.
fn clone_stall_within_startup_window(stall_secs: u64, window_secs: u64) -> u64 {
    let capped = stall_secs
        .min(window_secs.saturating_sub(CLONE_STALL_STARTUP_HEADROOM_SECS))
        .max(1);
    if capped < stall_secs {
        warn!(
            configured_secs = stall_secs,
            capped_secs = capped,
            startup_window_secs = window_secs,
            "Clone stall timeout lowered to fit the sandbox startup probe window; raise podStartupTimeout to allow more"
        );
    }
    capped
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
    moatless_sandbox_runtime::repo_config_encoding::encode_repo_config(&plan.repo_config)
}
