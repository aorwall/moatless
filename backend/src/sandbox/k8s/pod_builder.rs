//! Code-generated Pod spec with dynamic server containers.
//!
//! Generates the full Pod spec in Rust code — no YAML template needed.
//! Server containers are created dynamically from the repo config.

use k8s_openapi::api::core::v1::{
    Capabilities, Container, ContainerPort, EmptyDirVolumeSource, EnvVar, HTTPGetAction,
    HostPathVolumeSource, PersistentVolumeClaimVolumeSource, Pod, PodSecurityContext, PodSpec,
    Probe, ResourceRequirements, SeccompProfile, SecurityContext, TCPSocketAction, Toleration,
    Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use std::collections::{BTreeMap, HashMap};
use tracing::{debug, info, warn};

use super::naming::generate_pod_name;
use crate::config::Config;
use crate::sandbox::error::SandboxError;
use crate::sandbox::jwt::{self, MintedSandboxTokens};
use crate::sandbox::lifecycle::runtime::{OPERATION_GENERATION_LABEL, RuntimeOperationContext};
use crate::sandbox::models::{ProvisionPlan, RepoCheckout, WORKSPACE_REPOS_ENV};
use crate::sandbox::server_runner::build_server_start_script;

/// Auth tokens delivered to a sandbox pod.
///
/// The `bootstrap` token is placed on the pod spec (env `SANDBOX_BOOTSTRAP_TOKEN`)
/// and authenticates only the sandbox's initial `GetCredentials` call. The
/// long-lived user token is delivered over that credential channel instead of the
/// pod spec — see `secrets.rs`. `legacy_user_token`, when present, additionally
/// emits the `USER_AUTH_TOKEN` env (gated by `SANDBOX_EMIT_LEGACY_USER_TOKEN`).
#[derive(Clone, Copy)]
pub struct SandboxAuthTokens<'a> {
    pub bootstrap: &'a str,
    pub legacy_user_token: Option<&'a str>,
}

impl<'a> SandboxAuthTokens<'a> {
    /// Borrow the owned [`MintedSandboxTokens`] into the pod-spec view.
    pub fn from_minted(minted: &'a MintedSandboxTokens) -> Self {
        Self {
            bootstrap: minted.bootstrap.as_str(),
            legacy_user_token: minted.legacy_user_token.as_deref(),
        }
    }
}

/// Mint the auth tokens a sandbox pod needs from config.
///
/// Produces a short-lived bootstrap token plus, while
/// `SANDBOX_EMIT_LEGACY_USER_TOKEN` is set, the long-lived user token emitted as
/// the `USER_AUTH_TOKEN` env. Callers borrow the result into
/// [`SandboxAuthTokens`] via [`SandboxAuthTokens::from_minted`].
pub fn mint_sandbox_tokens(
    config: &Config,
    task_id: &str,
    user_id: &str,
) -> Result<MintedSandboxTokens, SandboxError> {
    let secret = config
        .sandbox
        .task_token_secret
        .as_deref()
        .ok_or_else(|| SandboxError::ConfigMissing("TASK_TOKEN_SECRET".to_string()))?;
    jwt::mint_sandbox_tokens(
        user_id,
        user_id,
        task_id,
        secret,
        config.sandbox.bootstrap_token_ttl_secs,
        config.sandbox.user_token_ttl_secs,
        config.sandbox.emit_legacy_user_token_env,
    )
    .map_err(|source| SandboxError::JwtSigning {
        purpose: "the sandbox task token",
        source,
    })
}

/// Build a complete Pod spec from config + plan (no template).
///
/// Generates the sandbox container, dynamic server containers, volumes,
/// probes, and security contexts entirely in code.
pub fn build_pod(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    pvc_name: Option<&str>,
    tokens: Option<SandboxAuthTokens<'_>>,
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
    // Per-repo network profile (opt-in): tag the pod so a static, GitOps-owned
    // NetworkPolicy with a matching podSelector governs its egress/ingress. The
    // label only selects a policy — the policy itself lives outside moatless.
    if let Some(profile) = plan
        .repo_config
        .network_profile
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        labels.insert(
            "moatless.io/network-profile".to_string(),
            profile.to_string(),
        );
    }

    // ── Annotations ─────────────────────────────────────────────────
    let mut annotations = BTreeMap::new();
    // Redact any embedded credentials (userinfo) so a PAT/password in the clone
    // URL never lands in the pod spec / etcd. Authentication happens at runtime
    // via the credential channel + git credential helper, not the URL.
    annotations.insert(
        "moatless.io/repository-url".to_string(),
        redact_url_userinfo(&plan.git_clone_url),
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
    // a per-task EBS PVC, a node-local emptyDir for the ephemeral/s3 tiers, or a
    // per-task hostPath for the best-effort node tier. Either way it is a single
    // volume shared across the sandbox and server containers, with `/tmp` served
    // from its `tmp` subPath so `/tmp` never charges against the pod's memory
    // budget.
    let build_cache_backend = resolve_build_cache_backend(plan);
    let mut volumes = vec![
        build_cache_volume(config, task_id, plan, build_cache_backend),
        Volume {
            name: "shared-data".to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                // Bound the shared `/opt/moatless/data` emptyDir so server
                // artifacts/logs can't grow unbounded and evict the node.
                size_limit: Some(Quantity(config.sandbox.shared_data_size.clone())),
                ..Default::default()
            }),
            ..Default::default()
        },
        Volume {
            name: "run-moatless".to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                // Memory-backed home for the credential-broker Unix socket at
                // `/run/moatless`. `/run` itself is root-owned tmpfs, so under
                // `run_as_non_root` the sandbox (uid 1000) can't mkdir there and
                // the broker degrades to the on-disk hosts.yml fallback.
                medium: Some("Memory".to_string()),
                size_limit: Some(Quantity("1Mi".to_string())),
            }),
            ..Default::default()
        },
    ];

    let storage_backend =
        super::storage::StorageBackend::from_plan(plan, &config.sandbox.default_storage_mode);

    if let Some(pvc) = pvc_name {
        volumes.push(Volume {
            name: "workspace-data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: pvc.to_string(),
                read_only: Some(false),
            }),
            ..Default::default()
        });
    } else if storage_backend == super::storage::StorageBackend::Node {
        // Node-local tier — a hostPath keyed by task id. Lifecycle records the
        // scheduled node and pins retained replacements back to the newest
        // surviving archive location instead of silently mounting an empty tree
        // elsewhere.
        volumes.push(Volume {
            name: "workspace-data".to_string(),
            host_path: Some(HostPathVolumeSource {
                path: node_sandbox_host_path(config, task_id),
                type_: Some("DirectoryOrCreate".to_string()),
            }),
            ..Default::default()
        });
    } else {
        // Ephemeral/S3 (no-EBS) tiers — a node-local emptyDir capped at the
        // repo's workspace size (`resources.storage`, default
        // `K8S_SANDBOX_STORAGE_SIZE`). No `medium: Memory`, so it lives on node
        // disk, not the pod memory budget.
        volumes.push(Volume {
            name: "workspace-data".to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity(resolve_sandbox_size(config, plan))),
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
    if build_cache_backend.is_node() {
        init_containers.push(build_node_build_cache_setup_container(config, plan));
    }
    if storage_backend == super::storage::StorageBackend::Node {
        init_containers.push(build_node_sandbox_setup_container(config, plan));
    }
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
    let sandbox_container = build_sandbox_container(config, task_id, plan, tokens)?;

    // ── Server containers ───────────────────────────────────────────
    let server_containers = build_server_containers(config, plan)?;

    // ── Docker-in-Docker sidecar (opt-in via repo config `dockerAccess`) ──
    //
    // When a repo opts in, attach a privileged `docker:dind` sidecar. It shares
    // the pod network namespace, so the sandbox reaches dockerd at
    // `tcp://localhost:2375` — which is what lets Testcontainers spin up real
    // containers (Postgres, Redis, …). dind's image/container storage lives on a
    // node-local hostPath by default (`resources.dockerStorageMode: "node"`), or
    // a capped emptyDir when explicitly requested. Gated on the opt-in so default
    // sandboxes stay hardened with no privileged container. Mirrors the CI runner dind pattern
    // (soap-platform-gitops `java-build-runner-set`).
    let docker_access = docker_access_enabled(plan);
    if docker_access {
        let mut volume = Volume {
            name: "docker-storage".to_string(),
            ..Default::default()
        };
        match resolve_dind_storage_backend(plan) {
            DockerStorageBackend::Node => {
                volume.host_path = Some(HostPathVolumeSource {
                    path: node_docker_host_path(config, task_id),
                    type_: Some("DirectoryOrCreate".to_string()),
                });
            }
            DockerStorageBackend::EmptyDir => {
                volume.empty_dir = Some(EmptyDirVolumeSource {
                    size_limit: Some(Quantity(resolve_dind_storage_size(config, plan))),
                    ..Default::default()
                });
            }
        }
        volumes.push(volume);
    }

    // ── Assemble Pod ────────────────────────────────────────────────
    let mut all_containers = vec![sandbox_container];
    all_containers.extend(server_containers);
    if docker_access {
        all_containers.push(build_dind_container(config));
    }

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
                // Per-repo override (opt-in) wins over the global default SA.
                // Enables shadow dev sandboxes to run under an SA bound to the
                // backend's RBAC without elevating every other repo.
                service_account_name: plan
                    .repo_config
                    .sandbox_service_account
                    .clone()
                    .or_else(|| config.sandbox.service_account_name.clone()),
                security_context: pod_security_context(config),
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

/// Stamp lifecycle's operation identity onto a pod before it becomes a
/// Deployment template. Kubernetes copies this label to every replacement pod.
pub fn stamp_operation_generation(pod: &mut Pod, ctx: RuntimeOperationContext) {
    pod.metadata
        .labels
        .get_or_insert_with(BTreeMap::new)
        .insert(
            OPERATION_GENERATION_LABEL.to_string(),
            ctx.generation_label(),
        );
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

/// Resolve the sandbox storage size for the ephemeral (emptyDir) tier: a per-repo
/// `resources.storage` override wins over the global `K8S_SANDBOX_STORAGE_SIZE`.
/// One knob sizes both tiers — the PVC tier reads the same value in `pvc.rs`.
fn resolve_sandbox_size(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.storage.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| config.sandbox.storage_size.clone(), ToString::to_string)
}

/// Resolve the transient `build-cache` emptyDir size: a per-repo
/// `resources.buildCache` override wins over `K8S_SANDBOX_BUILD_CACHE_SIZE`.
fn resolve_build_cache_size(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.build_cache.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(
            || config.sandbox.build_cache_size.clone(),
            ToString::to_string,
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildCacheBackend {
    EmptyDir,
    NodeTask,
    NodeShared,
}

impl BuildCacheBackend {
    fn is_node(self) -> bool {
        matches!(self, Self::NodeTask | Self::NodeShared)
    }
}

fn resolve_build_cache_backend(plan: &ProvisionPlan) -> BuildCacheBackend {
    let mode = plan
        .repo_config
        .resources
        .as_ref()
        .and_then(|r| r.build_cache_mode.as_deref())
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    match mode.as_deref() {
        Some("node") => {
            let scope = plan
                .repo_config
                .resources
                .as_ref()
                .and_then(|r| r.build_cache_scope.as_deref())
                .map(str::trim)
                .map(str::to_ascii_lowercase);
            match scope.as_deref() {
                Some("node") => BuildCacheBackend::NodeShared,
                _ => BuildCacheBackend::NodeTask,
            }
        }
        _ => BuildCacheBackend::EmptyDir,
    }
}

fn build_cache_volume(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    backend: BuildCacheBackend,
) -> Volume {
    let mut volume = Volume {
        name: "build-cache".to_string(),
        ..Default::default()
    };
    match backend {
        BuildCacheBackend::EmptyDir => {
            volume.empty_dir = Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity(resolve_build_cache_size(config, plan))),
                ..Default::default()
            });
        }
        BuildCacheBackend::NodeTask => {
            volume.host_path = Some(HostPathVolumeSource {
                path: node_task_build_cache_host_path(config, task_id),
                type_: Some("DirectoryOrCreate".to_string()),
            });
        }
        BuildCacheBackend::NodeShared => {
            volume.host_path = Some(HostPathVolumeSource {
                path: node_shared_build_cache_host_path(config),
                type_: Some("DirectoryOrCreate".to_string()),
            });
        }
    }
    volume
}

fn node_sandbox_host_path(config: &Config, task_id: &str) -> String {
    let root = config.sandbox.cache_host_root.trim_end_matches('/');
    format!("{root}/workspaces/{task_id}")
}

fn node_task_build_cache_host_path(config: &Config, task_id: &str) -> String {
    let root = config.sandbox.cache_host_root.trim_end_matches('/');
    format!("{root}/build-cache/tasks/{task_id}")
}

fn node_shared_build_cache_host_path(config: &Config) -> String {
    let root = config.sandbox.cache_host_root.trim_end_matches('/');
    format!("{root}/build-cache/shared")
}

fn node_docker_host_path(config: &Config, task_id: &str) -> String {
    let root = config.sandbox.cache_host_root.trim_end_matches('/');
    format!("{root}/docker/{task_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerStorageBackend {
    EmptyDir,
    Node,
}

fn resolve_dind_storage_backend(plan: &ProvisionPlan) -> DockerStorageBackend {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.docker_storage_mode.as_deref())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .and_then(|mode| match mode.as_str() {
            "emptydir" => Some(DockerStorageBackend::EmptyDir),
            "node" => Some(DockerStorageBackend::Node),
            _ => None,
        })
        .unwrap_or(DockerStorageBackend::Node)
}

fn build_node_sandbox_setup_container(config: &Config, plan: &ProvisionPlan) -> Container {
    // Reuse the sandbox image (already pulled for the sandbox container) rather
    // than the privileged dind `cache_init_image`: this is only a mkdir/chown, and
    // reusing it keeps `node` mode pullable on any cluster with no extra pull.
    Container {
        name: "node-sandbox-setup".to_string(),
        image: Some(resolve_sandbox_image(config, plan)),
        image_pull_policy: Some(config.sandbox.image_pull_policy.clone()),
        command: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
        args: Some(vec![
            "mkdir -p /workspace /workspace/tmp && chown 1000:1000 /workspace /workspace/tmp && chmod 0775 /workspace /workspace/tmp".to_string(),
        ]),
        security_context: Some(root_init_security_context()),
        resources: Some(ResourceRequirements {
            requests: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity("25m".to_string())),
                ("memory".to_string(), Quantity("32Mi".to_string())),
            ])),
            limits: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity("100m".to_string())),
                ("memory".to_string(), Quantity("128Mi".to_string())),
            ])),
            ..Default::default()
        }),
        volume_mounts: Some(vec![VolumeMount {
            name: "workspace-data".to_string(),
            mount_path: "/workspace".to_string(),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

fn build_node_build_cache_setup_container(config: &Config, plan: &ProvisionPlan) -> Container {
    Container {
        name: "build-cache-setup".to_string(),
        image: Some(resolve_sandbox_image(config, plan)),
        image_pull_policy: Some(config.sandbox.image_pull_policy.clone()),
        command: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
        args: Some(vec![
            "mkdir -p /build-cache && chown 1000:1000 /build-cache && chmod 0775 /build-cache"
                .to_string(),
        ]),
        security_context: Some(root_init_security_context()),
        resources: Some(ResourceRequirements {
            requests: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity("25m".to_string())),
                ("memory".to_string(), Quantity("32Mi".to_string())),
            ])),
            limits: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity("100m".to_string())),
                ("memory".to_string(), Quantity("128Mi".to_string())),
            ])),
            ..Default::default()
        }),
        volume_mounts: Some(vec![VolumeMount {
            name: "build-cache".to_string(),
            mount_path: "/build-cache".to_string(),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

/// Resolve the `docker-storage` emptyDir cap for the dind sidecar: a per-repo
/// `resources.dockerStorage` override wins over the global
/// `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE`. Exceeding the cap gets the pod
/// evicted by the kubelet (wiping dind data), so DB-heavy repos need headroom.
fn resolve_dind_storage_size(config: &Config, plan: &ProvisionPlan) -> String {
    plan.repo_config
        .resources
        .as_ref()
        .and_then(|r| r.docker_storage.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(
            || config.sandbox.docker_dind_storage_size.clone(),
            ToString::to_string,
        )
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
/// Deliberately does NOT use `app_security_context`: `mkfs.ext4` / `mount`
/// of the raw block device require full privileges. Scoped to this short-lived
/// init container; the app containers stay hardened.
fn build_nvme_setup_container(config: &Config) -> Container {
    // Inside the container the parent is bind-mounted at `/host-mnt`, so the
    // NVMe mount target is `/host-mnt/<basename-of-cache-root>`.
    let root = config.sandbox.cache_host_root.trim_end_matches('/');
    let basename = root
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("cache");
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
        // Modest requests/limits so this short-lived, privileged init container is
        // schedulable and bounded rather than BestEffort.
        resources: Some(ResourceRequirements {
            requests: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity("50m".to_string())),
                ("memory".to_string(), Quantity("64Mi".to_string())),
            ])),
            limits: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity("200m".to_string())),
                ("memory".to_string(), Quantity("256Mi".to_string())),
            ])),
            ..Default::default()
        }),
        volume_mounts: Some(vec![VolumeMount {
            name: "host-mnt".to_string(),
            mount_path: "/host-mnt".to_string(),
            mount_propagation: Some("Bidirectional".to_string()),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

/// Whether this repo opted into Docker access (privileged dind sidecar).
///
/// Reuses the existing `RepoConfig.docker_access` flag (already honoured by the
/// Docker provider via a socket mount); in K8s mode "docker access" means a
/// privileged `dind` sidecar rather than a host socket. Defaults to `false`, so
/// sandboxes stay hardened unless a repo explicitly opts in.
fn docker_access_enabled(plan: &ProvisionPlan) -> bool {
    plan.repo_config.docker_access.unwrap_or(false)
}

/// Build the privileged `dind` (docker-in-docker) sidecar container.
///
/// Runs `dockerd` without TLS on `tcp://0.0.0.0:2375` (reachable only from
/// within the pod's network namespace, i.e. by the sandbox container via
/// `localhost`) plus the default unix socket. Image/container layers live on the
/// `docker-storage` volume mounted at `/var/lib/docker`. Mirrors the CI runner
/// dind topology (soap-platform-gitops `java-build-runner-set`).
///
/// Deliberately privileged — dockerd needs it to manage cgroups, overlay mounts,
/// and iptables. Scoped to this sidecar and only attached when a repo opts into
/// `dockerAccess`; the agent/server containers keep `app_security_context`.
fn build_dind_container(config: &Config) -> Container {
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

/// Build the main sandbox container.
#[allow(clippy::unnecessary_wraps)] // Result for consistency with build_server_containers
fn build_sandbox_container(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    tokens: Option<SandboxAuthTokens<'_>>,
) -> Result<Container, String> {
    // Resolve image: repo config override → default sandbox image
    let image = resolve_sandbox_image(config, plan);

    // Build env vars. GIT_CLONE_URL is credential-free — the sandbox clones via
    // the credential channel (GIT_TOKEN) / git credential helper, so no PAT is
    // embedded in the pod spec.
    let mut env = vec![
        env_var("TASK_ID", task_id),
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

    // Git provider (github, gitlab, gitness)
    env.push(env_var("GIT_PROVIDER", &plan.git_provider));

    // Every repository the workspace places in the sandbox, primary first. The
    // GIT_* trio above describes the primary, and WORKSPACE_REPOS carries the
    // full checkout list. Clone URLs are redacted like GIT_CLONE_URL — no PAT
    // reaches the pod spec.
    if let Some(repos_json) = build_workspace_repos_json(plan) {
        env.push(env_var(WORKSPACE_REPOS_ENV, &repos_json));
    }

    // Workspace storage backend. The `s3` backend runs on a node-local emptyDir
    // (no PVC) and syncs the git working tree to S3 via presigned URLs — flag the
    // sandbox so it syncs after terminal agent turns and restores on boot. `ebs`,
    // `node`, and `ephemeral` leave this unset.
    if super::storage::StorageBackend::from_plan(plan, &config.sandbox.default_storage_mode)
        .syncs_sandbox()
    {
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

    // Incremental Rust compiles avoid the workspace volume when the target dir
    // rides the `build-cache` volume. With the default emptyDir backend, its
    // size limit must fit the repo's target dir or the kubelet evicts the pod;
    // with the node hostPath backend, capacity is the node filesystem instead.
    if config.sandbox.cargo_target_on_build_cache {
        env.push(env_var(
            "CARGO_TARGET_DIR",
            "/opt/moatless/build-cache/target",
        ));
    }

    // Package-manager cache env — point each tool at its mounted cache dir
    // (e.g. MAVEN_OPTS=-Dmaven.repo.local=/maven-cache, BUN_INSTALL_CACHE_DIR).
    for cache in &config.sandbox.cache_volumes {
        for (key, value) in &cache.env {
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
    // probe on `/health` remains the readiness/startup gate regardless.
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
    // an empty registry/git cache transparently. On the ephemeral tier the
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
        liveness_probe,
        volume_mounts: Some(volume_mounts),
        resources: Some(resources),
        security_context: Some(app_security_context(config)),
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
            env_var("WORKSPACE_DIR", "/opt/moatless/workspace"),
            env_var("WORKSPACE_PATH", "/opt/moatless/workspace"),
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
        // The script waits for the workspace-ready sentinel, sources the shared
        // env files, and supervises the child so a live config override can
        // restart it without rebuilding the pod — see `server_runner`.
        let (command, args) = match &server.start_commands {
            Some(cmds) if !cmds.is_empty() => (
                Some(vec!["bash".to_string(), "-c".to_string()]),
                Some(vec![build_server_start_script(&server.name, cmds)]),
            ),
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

        // Resource limits: per-server override wins, then per-repo, then global
        // defaults. Servers are typically far lighter than the agent container,
        // so sizing them separately is what keeps pod density honest.
        let server_resources = server
            .resources
            .as_ref()
            .or(plan.repo_config.resources.as_ref());
        let resources = if let Some(res) = server_resources {
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
            security_context: Some(app_security_context(config)),
            ..Default::default()
        });
    }

    Ok(containers)
}

/// Serialize the plan's repository checkouts for the `WORKSPACE_REPOS` env.
///
/// Clone URLs are redacted exactly like `GIT_CLONE_URL`, so no credential
/// reaches the pod spec — the sandbox authenticates each clone through the
/// credential channel using the checkout's provider. `None` when the plan
/// carries no checkouts.
fn build_workspace_repos_json(plan: &ProvisionPlan) -> Option<String> {
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
        && rc.plugins.is_none()
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
    if let Some(ref plugins) = rc.plugins {
        map.insert("plugins".into(), serde_json::json!(plugins));
    }

    Some(serde_json::Value::Object(map).to_string())
}

/// Container-level security context for the agent + server containers.
///
/// Both modes drop ALL capabilities and add back only what the workload needs.
///
/// - Non-root (`K8S_SANDBOX_RUN_AS_NON_ROOT=true`): the pod-level securityContext
///   (see [`pod_security_context`]) runs these containers as UID/GID 1000, and
///   the image chowns every runtime path to that user (plus `fsGroup: 1000` for
///   the mounted volumes). Ownership is therefore correct without `DAC_OVERRIDE`.
/// - Root (default, flag off): the container runs as UID 0 while `/home/moatless`
///   and friends are owned by UID 1000, so `DAC_OVERRIDE` is required for root to
///   write there (gh CLI config, Claude onboarding flag) during init.
///
/// `NET_BIND_SERVICE` is kept in both modes so servers can bind privileged ports
/// if a repo configures one.
fn app_security_context(config: &Config) -> SecurityContext {
    let mut add = vec!["NET_BIND_SERVICE".to_string()];
    if !config.sandbox.run_as_non_root {
        add.push("DAC_OVERRIDE".to_string());
    }
    SecurityContext {
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".to_string()]),
            add: Some(add),
        }),
        ..Default::default()
    }
}

/// Pod-level securityContext, populated only when `K8S_SANDBOX_RUN_AS_NON_ROOT`
/// is enabled.
///
/// Runs the pod's containers as the non-root `moatless` user (UID/GID 1000) with
/// the RuntimeDefault seccomp profile, and sets `fsGroup: 1000` so mounted
/// volumes (workspace PVC/emptyDir, shared-data, build-cache) are group-writable
/// by that user. The privileged `dind` sidecar and `nvme-setup` init container
/// override this back to root at the container level (see
/// [`privileged_security_context`]).
///
/// Returns `None` when the flag is off, leaving the pod at its previous behaviour
/// (containers run as root).
fn pod_security_context(config: &Config) -> Option<PodSecurityContext> {
    config.sandbox.run_as_non_root.then(|| PodSecurityContext {
        run_as_user: Some(1000),
        run_as_group: Some(1000),
        run_as_non_root: Some(true),
        fs_group: Some(1000),
        // Only recursively chown/chmod a volume when its root doesn't already
        // have the right ownership. Without this the kubelet walks the ENTIRE
        // workspace PVC (node_modules, build caches, many small files) on every
        // mount, adding large pod-start latency on each start/restart. After the
        // first mount the root already matches, so the walk is skipped.
        fs_group_change_policy: Some("OnRootMismatch".to_string()),
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    })
}

/// Privileged security context for the `nvme-setup` init container and the `dind`
/// sidecar.
///
/// Required to `mkfs.ext4` / `mount` the raw instance-store NVMe device (init) and
/// to run `dockerd` (sidecar). Explicitly pins `runAsUser: 0` / `runAsNonRoot:
/// false` so these containers keep running as root even when the pod-level
/// securityContext enables `runAsNonRoot` — otherwise the kubelet would reject
/// them. Used ONLY by these containers — never by the agent/server containers,
/// which use [`app_security_context`].
fn privileged_security_context() -> SecurityContext {
    SecurityContext {
        privileged: Some(true),
        run_as_user: Some(0),
        run_as_non_root: Some(false),
        ..Default::default()
    }
}

fn root_init_security_context() -> SecurityContext {
    SecurityContext {
        run_as_user: Some(0),
        run_as_non_root: Some(false),
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

/// Strip any embedded userinfo (credentials) from an HTTP(S) URL.
///
/// `https://user:pat@host/owner/repo.git` → `https://host/owner/repo.git`. Keeps
/// non-HTTP(S) forms unchanged: an scp-like SSH URL's `git@host:path` user is not
/// a secret, and there is nothing to strip. Used so a clone URL that embeds a
/// PAT/password never reaches the pod spec (annotation + `GIT_CLONE_URL` env),
/// and thus never lands in etcd.
fn redact_url_userinfo(url: &str) -> String {
    for scheme in ["https://", "http://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            // Look only within the authority (before the first '/') for userinfo.
            let authority_end = rest.find('/').unwrap_or(rest.len());
            if let Some(at) = rest[..authority_end].rfind('@') {
                return format!("{scheme}{}", &rest[at + 1..]);
            }
            return url.to_string();
        }
    }
    url.to_string()
}

/// Get the current time as an RFC3339 timestamp (e.g. `2026-07-12T10:20:30Z`).
fn chrono_now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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

    #[test]
    fn pod_template_carries_operation_generation() {
        let mut pod = Pod::default();
        stamp_operation_generation(
            &mut pod,
            RuntimeOperationContext::new(crate::sandbox::lifecycle::OperationGeneration(31)),
        );
        assert_eq!(
            pod.metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(OPERATION_GENERATION_LABEL))
                .map(String::as_str),
            Some("31")
        );
    }

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
                run_as_non_root: false,
                shared_data_size: "2Gi".into(),
                build_cache_size: "1536Mi".into(),
                persist_toolchain: false,
                cargo_target_on_build_cache: false,
                storage_enabled: false,
                storage_size: "1Gi".into(),
                storage_class: None,
                default_storage_mode: "ebs".into(),
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
                busy_extension_max_hours: 4,
                idle_check_interval_secs: 300,
                inactive_task_timeout_days: 0,
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
                provider: crate::config::AuthProvider::Local,
                github_client_id: None,
                github_client_secret: None,
                github_callback_url: None,
                github_oauth_scope: String::new(),
                github_required_org: None,
                auth0_domain: None,
                auth0_client_id: None,
                auth0_client_secret: None,
                auth0_callback_url: None,
                auth0_audience: None,
                auth0_groups_claim: String::new(),
                default_team_slug: None,
                session_max_age_secs: 3600,
                frontend_url: "http://localhost:3000".into(),
                cookie_domain: None,
                session_cookie_name: "test".into(),
                is_production: false,
                admin_username: "admin".into(),
                admin_password: None,
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
            github_pr: crate::config::GithubPrConfig {
                webhook_secret: None,
                bot_token: None,
            },
            linear: crate::config::LinearConfig {
                webhook_secret: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                oauth_callback_url: None,
                api_base_url: None,
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
            git_branch: None,
            git_provider: "github".into(),
            env_vars: None,
            repos: vec![RepoCheckout {
                clone_url: "https://github.com/test/repo.git".into(),
                branch: None,
                provider: "github".into(),
                mount_name: "repo".into(),
                is_primary: true,
            }],
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
        let env = web.env.as_ref().unwrap();
        assert!(env.iter().any(|var| {
            var.name == "WORKSPACE_DIR" && var.value.as_deref() == Some("/opt/moatless/workspace")
        }));
        assert!(env.iter().any(|var| {
            var.name == "WORKSPACE_PATH" && var.value.as_deref() == Some("/opt/moatless/workspace")
        }));
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
    fn chrono_now_iso_is_valid_rfc3339() {
        let ts = chrono_now_iso();
        // The created-at annotation must be valid RFC3339.
        assert!(
            chrono::DateTime::parse_from_rfc3339(&ts).is_ok(),
            "created-at annotation must be valid RFC3339, got {ts:?}"
        );
        assert!(ts.ends_with('Z'), "expected UTC 'Z' suffix, got {ts:?}");
    }

    #[test]
    fn sandbox_container_has_memory_request_and_limit() {
        let config = test_config();
        let plan = test_plan();

        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
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
            ..Default::default()
        });

        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
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
    fn liveness_probe_absent_when_flag_disabled() {
        // Default config has `liveness_enabled: false` — no liveness probe is
        // attached, and the startup probe on /health is untouched.
        let config = test_config();
        let plan = test_plan();

        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        assert!(
            container.liveness_probe.is_none(),
            "liveness probe must be absent when K8S_SANDBOX_LIVENESS_ENABLED is off"
        );
        assert!(
            container.startup_probe.is_some(),
            "startup probe must remain the readiness gate regardless of the flag"
        );
    }

    #[test]
    fn liveness_probe_present_with_conservative_settings_when_flag_enabled() {
        let mut config = test_config();
        config.sandbox.liveness_enabled = true;
        let plan = test_plan();

        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        let probe = container
            .liveness_probe
            .as_ref()
            .expect("liveness probe present when flag enabled");

        let http = probe.http_get.as_ref().expect("http_get action set");
        assert_eq!(http.path.as_deref(), Some("/livez"));
        assert_eq!(http.port, IntOrString::Int(8001));

        // Conservative timings: ~60s of failures before a restart.
        assert_eq!(probe.period_seconds, Some(10));
        assert_eq!(probe.timeout_seconds, Some(5));
        assert_eq!(probe.failure_threshold, Some(6));

        // The startup probe still targets /health — liveness is additive.
        let startup = container.startup_probe.as_ref().expect("startup probe set");
        assert_eq!(
            startup.http_get.as_ref().and_then(|h| h.path.as_deref()),
            Some("/health")
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
            storage: Some("7Gi".into()),
            storage_mode: Some("ephemeral".into()),
            ..Default::default()
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
            volumes.iter().all(|v| v.persistent_volume_claim.is_none()),
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
        assert!(
            mounts
                .iter()
                .any(|m| m.name == "run-moatless" && m.mount_path == "/run/moatless"),
            "credential-broker socket dir must be a writable mount for non-root"
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
    fn node_tier_remains_available_without_archive_management() {
        use crate::repository::models::ResourceConfig;

        let mut config = test_config();
        assert!(!config.sandbox.node_archive_enabled);
        config.sandbox.cache_host_root = "/mnt/cache".into();
        let mut plan = test_plan();
        plan.repo_config.resources = Some(ResourceConfig {
            storage: Some("30Gi".into()),
            storage_mode: Some("node".into()),
            ..Default::default()
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let ws = volumes
            .iter()
            .find(|v| v.name == "workspace-data")
            .expect("workspace-data volume present");

        let host_path = ws
            .host_path
            .as_ref()
            .expect("node tier uses hostPath for workspace-data");
        assert_eq!(host_path.path, "/mnt/cache/workspaces/task-1");
        assert_eq!(host_path.type_.as_deref(), Some("DirectoryOrCreate"));
        assert!(ws.empty_dir.is_none(), "node tier must not use emptyDir");
        assert!(
            volumes.iter().all(|v| v.persistent_volume_claim.is_none()),
            "node tier must not reference a PVC"
        );

        let init = pod
            .spec
            .as_ref()
            .and_then(|s| s.init_containers.as_ref())
            .and_then(|containers| containers.iter().find(|c| c.name == "node-sandbox-setup"))
            .expect("node tier must prepare hostPath ownership");
        // Reuses the sandbox image (already pulled), not the private dind
        // `cache_init_image` — so `node` mode is pullable on any cluster.
        assert_eq!(init.image.as_deref(), Some("sandbox:latest"));
        let security = init
            .security_context
            .as_ref()
            .expect("node workspace setup must set security context");
        assert_eq!(security.run_as_user, Some(0));
        assert_eq!(security.run_as_non_root, Some(false));
        assert_ne!(security.privileged, Some(true));
        assert!(
            init.volume_mounts.as_ref().unwrap().iter().any(|m| {
                m.name == "workspace-data"
                    && m.mount_path == "/workspace"
                    && m.sub_path.as_deref().unwrap_or("").is_empty()
            }),
            "node workspace setup must mount workspace-data"
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
        assert_eq!(
            pkg.len(),
            1,
            "exactly one shared pkg-cache volume for N caches"
        );
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
        assert_eq!(
            init_mount.mount_propagation.as_deref(),
            Some("Bidirectional")
        );
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
        assert!(
            env.iter()
                .any(|e| e.name == "BUN_INSTALL_CACHE_DIR"
                    && e.value.as_deref() == Some("/bun-cache"))
        );

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

    /// `dockerAccess: true` → a privileged `dind` sidecar + `docker-storage`
    /// volume + `DOCKER_HOST`/`TESTCONTAINERS_RYUK_DISABLED` on the agent
    /// container, while the agent container itself stays unprivileged.
    #[test]
    fn docker_access_adds_privileged_dind_sidecar_and_env() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.docker_access = Some(true);

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();

        // A privileged `dind` sidecar using the configured image, running dockerd.
        let dind = spec
            .containers
            .iter()
            .find(|c| c.name == "dind")
            .expect("dind sidecar present when dockerAccess is set");
        assert_eq!(
            dind.security_context.as_ref().and_then(|s| s.privileged),
            Some(true),
            "dind must be privileged"
        );
        assert_eq!(dind.image.as_deref(), Some("docker:dind"));
        let args = dind.args.as_ref().unwrap().join(" ");
        assert!(args.contains("dockerd"), "dind runs dockerd, got: {args}");
        assert!(args.contains("--tls=false"), "dind runs without TLS");
        assert!(
            dind.volume_mounts
                .as_ref()
                .unwrap()
                .iter()
                .any(|m| m.name == "docker-storage" && m.mount_path == "/var/lib/docker"),
            "dind must mount docker-storage at /var/lib/docker"
        );

        // The default node-local docker-storage hostPath backs /var/lib/docker.
        let ds = spec
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "docker-storage")
            .expect("docker-storage volume present");
        assert!(ds.host_path.is_some(), "docker-storage must be a hostPath");
        assert!(
            ds.empty_dir.is_none(),
            "docker-storage must not be an emptyDir"
        );

        // The sandbox container is pointed at the dind sidecar and disables Ryuk.
        let sandbox = spec
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap();
        let env = sandbox.env.as_ref().unwrap();
        assert!(
            env.iter()
                .any(|e| e.name == "DOCKER_HOST"
                    && e.value.as_deref() == Some("tcp://localhost:2375")),
            "sandbox must point DOCKER_HOST at the dind sidecar"
        );
        assert!(
            env.iter()
                .any(|e| e.name == "TESTCONTAINERS_RYUK_DISABLED"
                    && e.value.as_deref() == Some("true")),
            "Ryuk must be disabled for the dind topology"
        );

        // The agent container itself stays hardened (not privileged).
        assert_ne!(
            sandbox.security_context.as_ref().and_then(|s| s.privileged),
            Some(true),
            "sandbox container must not be privileged"
        );
    }

    /// `resources.dockerStorage` overrides the global dind storage cap when the
    /// repo explicitly selects the emptyDir Docker storage tier.
    #[test]
    fn docker_storage_override_sizes_dind_empty_dir() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.docker_access = Some(true);
        plan.repo_config.resources = Some(crate::repository::models::ResourceConfig {
            docker_storage: Some("50Gi".into()),
            docker_storage_mode: Some("emptyDir".into()),
            ..Default::default()
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let ds_limit = pod
            .spec
            .as_ref()
            .unwrap()
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "docker-storage")
            .and_then(|v| v.empty_dir.as_ref())
            .and_then(|e| e.size_limit.as_ref())
            .map(|q| q.0.as_str().to_string());
        assert_eq!(ds_limit.as_deref(), Some("50Gi"));

        // EmptyDir mode with no override → global default from config.
        plan.repo_config.resources = Some(crate::repository::models::ResourceConfig {
            docker_storage_mode: Some("emptyDir".into()),
            ..Default::default()
        });
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let ds_limit = pod
            .spec
            .as_ref()
            .unwrap()
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "docker-storage")
            .and_then(|v| v.empty_dir.as_ref())
            .and_then(|e| e.size_limit.as_ref())
            .map(|q| q.0.as_str().to_string());
        assert_eq!(
            ds_limit.as_deref(),
            Some(config.sandbox.docker_dind_storage_size.as_str())
        );
    }

    #[test]
    fn docker_storage_remains_node_backed_without_archive_management() {
        let mut config = test_config();
        assert!(!config.sandbox.node_archive_enabled);
        config.sandbox.cache_host_root = "/mnt/cache".into();
        let mut plan = test_plan();
        plan.repo_config.docker_access = Some(true);
        plan.repo_config.resources = Some(crate::repository::models::ResourceConfig {
            docker_storage: Some("50Gi".into()),
            ..Default::default()
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let docker_storage = pod
            .spec
            .as_ref()
            .unwrap()
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "docker-storage")
            .expect("docker-storage volume present");
        let host_path = docker_storage
            .host_path
            .as_ref()
            .expect("default dind storage uses hostPath");
        assert_eq!(host_path.path, "/mnt/cache/docker/task-1");
        assert_eq!(host_path.type_.as_deref(), Some("DirectoryOrCreate"));
        assert!(
            docker_storage.empty_dir.is_none(),
            "node docker storage must not use emptyDir"
        );
    }

    #[test]
    fn redact_url_userinfo_strips_credentials() {
        assert_eq!(
            redact_url_userinfo("https://user:pat@github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            redact_url_userinfo("https://oauth2:glpat-xxx@gitlab.com/group/repo.git"),
            "https://gitlab.com/group/repo.git"
        );
        // No userinfo → unchanged.
        assert_eq!(
            redact_url_userinfo("https://github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
        // A '@' in the path (not the authority) must not be treated as userinfo.
        assert_eq!(
            redact_url_userinfo("https://host/owner/repo@v1.git"),
            "https://host/owner/repo@v1.git"
        );
        // Non-HTTP(S) (scp-like SSH) forms are left untouched.
        assert_eq!(
            redact_url_userinfo("git@github.com:owner/repo.git"),
            "git@github.com:owner/repo.git"
        );
    }

    #[test]
    fn pod_spec_redacts_clone_url_and_omits_user_token() {
        let config = test_config();
        let mut plan = test_plan();
        plan.git_clone_url = "https://user:secret@github.com/test/repo.git".into();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();

        // Annotation must not carry the embedded credential.
        let anno = pod
            .metadata
            .annotations
            .as_ref()
            .unwrap()
            .get("moatless.io/repository-url")
            .unwrap();
        assert_eq!(anno, "https://github.com/test/repo.git");
        assert!(!anno.contains("secret"), "annotation leaked credentials");

        // GIT_CLONE_URL env must be credential-free too.
        let sandbox = pod
            .spec
            .as_ref()
            .unwrap()
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap();
        let env = sandbox.env.as_ref().unwrap();
        let git_url = env.iter().find(|e| e.name == "GIT_CLONE_URL").unwrap();
        assert_eq!(
            git_url.value.as_deref(),
            Some("https://github.com/test/repo.git")
        );

        // With no tokens passed, neither auth env is present.
        assert!(env.iter().all(|e| e.name != "USER_AUTH_TOKEN"));
        assert!(env.iter().all(|e| e.name != "SANDBOX_BOOTSTRAP_TOKEN"));
    }

    /// A multi-repo workspace reaches the pod as the `WORKSPACE_REPOS` list,
    /// while `GIT_CLONE_URL` still names the primary so a sandbox image predating
    /// multi-repo support keeps working.
    #[test]
    fn pod_spec_carries_every_workspace_repo() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repos.push(RepoCheckout {
            clone_url: "https://gitlab.com/acme/shared.git".into(),
            branch: Some("release".into()),
            provider: "gitlab".into(),
            mount_name: "shared".into(),
            is_primary: false,
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let sandbox = pod
            .spec
            .as_ref()
            .unwrap()
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap();
        let env = sandbox.env.as_ref().unwrap();
        let raw = env
            .iter()
            .find(|e| e.name == WORKSPACE_REPOS_ENV)
            .and_then(|e| e.value.clone())
            .expect("WORKSPACE_REPOS present");
        let parsed: Vec<RepoCheckout> = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].is_primary);
        assert_eq!(parsed[0].mount_name, "repo");
        assert_eq!(parsed[1].mount_name, "shared");
        assert_eq!(parsed[1].provider, "gitlab");
        assert_eq!(parsed[1].branch.as_deref(), Some("release"));

        let git_url = env.iter().find(|e| e.name == "GIT_CLONE_URL").unwrap();
        assert_eq!(
            git_url.value.as_deref(),
            Some("https://github.com/test/repo.git")
        );
    }

    /// Every checkout's URL is redacted, not just the primary's — a secondary
    /// repo's PAT must not land in the pod spec either.
    #[test]
    fn workspace_repos_env_redacts_every_clone_url() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repos[0].clone_url = "https://user:secret@github.com/test/repo.git".into();
        plan.repos.push(RepoCheckout {
            clone_url: "https://user:other-secret@gitlab.com/acme/shared.git".into(),
            branch: None,
            provider: "gitlab".into(),
            mount_name: "shared".into(),
            is_primary: false,
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let raw = pod
            .spec
            .as_ref()
            .unwrap()
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap()
            .env
            .as_ref()
            .unwrap()
            .iter()
            .find(|e| e.name == WORKSPACE_REPOS_ENV)
            .and_then(|e| e.value.clone())
            .expect("WORKSPACE_REPOS present");

        assert!(
            !raw.contains("secret"),
            "WORKSPACE_REPOS leaked credentials"
        );
        let parsed: Vec<RepoCheckout> = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed[0].clone_url, "https://github.com/test/repo.git");
        assert_eq!(parsed[1].clone_url, "https://gitlab.com/acme/shared.git");
    }

    /// A plan with no checkouts omits the env entirely rather than emitting an
    /// empty list.
    #[test]
    fn pod_spec_omits_workspace_repos_when_the_plan_has_none() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repos.clear();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let env = pod
            .spec
            .as_ref()
            .unwrap()
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap()
            .env
            .clone()
            .unwrap();
        assert!(env.iter().all(|e| e.name != WORKSPACE_REPOS_ENV));
    }

    #[test]
    fn pod_spec_emits_bootstrap_and_optional_legacy_token() {
        let config = test_config();
        let plan = test_plan();

        // Bootstrap only.
        let pod = build_pod(
            &config,
            "task-1",
            &plan,
            None,
            Some(SandboxAuthTokens {
                bootstrap: "boot-tok",
                legacy_user_token: None,
            }),
        )
        .unwrap();
        let env = sandbox_env(&pod);
        assert_eq!(
            env.iter()
                .find(|e| e.name == "SANDBOX_BOOTSTRAP_TOKEN")
                .and_then(|e| e.value.as_deref()),
            Some("boot-tok")
        );
        assert!(
            env.iter().all(|e| e.name != "USER_AUTH_TOKEN"),
            "legacy USER_AUTH_TOKEN must be absent when not requested"
        );

        // Bootstrap plus USER_AUTH_TOKEN.
        let pod = build_pod(
            &config,
            "task-1",
            &plan,
            None,
            Some(SandboxAuthTokens {
                bootstrap: "boot-tok",
                legacy_user_token: Some("legacy-tok"),
            }),
        )
        .unwrap();
        let env = sandbox_env(&pod);
        assert_eq!(
            env.iter()
                .find(|e| e.name == "USER_AUTH_TOKEN")
                .and_then(|e| e.value.as_deref()),
            Some("legacy-tok")
        );
    }

    fn sandbox_env(pod: &Pod) -> Vec<k8s_openapi::api::core::v1::EnvVar> {
        pod.spec
            .as_ref()
            .unwrap()
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap()
            .env
            .clone()
            .unwrap()
    }

    /// The Task's fixed Agent-harness selection reaches the sandbox container as
    /// the `AGENT_TYPE` env at construction (the fixed-harness decision), so the sandbox resolves
    /// its harness once instead of switching per Message. `auto_provision` stamps
    /// this into `plan.env_vars`; here we prove the builder delivers it verbatim.
    #[test]
    fn agent_type_env_reaches_the_sandbox_container() {
        let config = test_config();
        let mut plan = test_plan();
        plan.env_vars = Some(std::collections::HashMap::from([(
            "AGENT_TYPE".to_string(),
            "codex".to_string(),
        )]));
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert_eq!(
            sandbox_env(&pod)
                .iter()
                .find(|e| e.name == "AGENT_TYPE")
                .and_then(|e| e.value.as_deref()),
            Some("codex"),
        );
    }

    /// Per-repo `sandboxServiceAccount` overrides the global default SA, and
    /// falls back to the global default (here `None`) when unset.
    #[test]
    fn per_repo_sandbox_service_account_overrides_global() {
        // Unset on the repo → falls back to the global default (None in test_config).
        let config = test_config();
        let plan = test_plan();
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert_eq!(
            pod.spec.as_ref().unwrap().service_account_name,
            None,
            "unset per-repo SA falls back to the global default"
        );

        // Set on the repo → wins even when a global default is configured.
        let mut config = test_config();
        config.sandbox.service_account_name = Some("global-default-sa".to_string());
        let mut plan = test_plan();
        plan.repo_config.sandbox_service_account = Some("moatless-dev".to_string());
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert_eq!(
            pod.spec.as_ref().unwrap().service_account_name.as_deref(),
            Some("moatless-dev"),
            "per-repo SA overrides the global default"
        );
    }

    /// Per-repo `networkProfile` adds the `moatless.io/network-profile` pod label
    /// (which a static NetworkPolicy selects); unset/blank adds no label.
    #[test]
    fn per_repo_network_profile_sets_pod_label() {
        const KEY: &str = "moatless.io/network-profile";

        // Unset → no profile label.
        let config = test_config();
        let plan = test_plan();
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert!(
            !pod.metadata.labels.as_ref().unwrap().contains_key(KEY),
            "unset network_profile adds no label"
        );

        // Set → label carries the value.
        let mut plan = test_plan();
        plan.repo_config.network_profile = Some("sealed-egress".to_string());
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert_eq!(
            pod.metadata
                .labels
                .as_ref()
                .unwrap()
                .get(KEY)
                .map(String::as_str),
            Some("sealed-egress"),
            "network_profile sets the pod label a NetworkPolicy can select"
        );

        // Blank/whitespace → treated as unset (no label, avoids invalid label value).
        let mut plan = test_plan();
        plan.repo_config.network_profile = Some("   ".to_string());
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert!(
            !pod.metadata.labels.as_ref().unwrap().contains_key(KEY),
            "blank network_profile adds no label"
        );
    }

    /// The egress proxy env is injected only when a URL is configured AND the
    /// pod's network profile is in the opt-in list.
    #[test]
    fn egress_proxy_env_injected_only_for_opted_in_profile() {
        const URL: &str = "http://moatless-vibe-egress-proxy.moatless.svc.cluster.local:3128";

        let sandbox_proxy = |pod: &Pod, key: &str| -> Option<String> {
            pod.spec
                .as_ref()
                .unwrap()
                .containers
                .iter()
                .find(|c| c.name == "sandbox")
                .unwrap()
                .env
                .as_ref()
                .unwrap()
                .iter()
                .find(|e| e.name == key)
                .and_then(|e| e.value.clone())
        };

        // Proxy configured + profile opted in → HTTPS_PROXY + NO_PROXY set.
        let mut config = test_config();
        config.sandbox.egress_proxy_url = Some(URL.to_string());
        config.sandbox.egress_proxy_profiles = vec!["sealed-egress".to_string()];
        config.sandbox.egress_proxy_no_proxy = "localhost,.svc.cluster.local".to_string();
        let mut plan = test_plan();
        plan.repo_config.network_profile = Some("sealed-egress".to_string());
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert_eq!(sandbox_proxy(&pod, "HTTPS_PROXY").as_deref(), Some(URL));
        assert_eq!(sandbox_proxy(&pod, "http_proxy").as_deref(), Some(URL));
        assert_eq!(
            sandbox_proxy(&pod, "NO_PROXY").as_deref(),
            Some("localhost,.svc.cluster.local")
        );

        // Profile NOT in the opt-in list → no proxy env, even with a URL set.
        let mut plan = test_plan();
        plan.repo_config.network_profile = Some("some-other-profile".to_string());
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert!(sandbox_proxy(&pod, "HTTPS_PROXY").is_none());

        // No proxy URL configured (default) → no proxy env, even when opted in.
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.network_profile = Some("sealed-egress".to_string());
        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        assert!(sandbox_proxy(&pod, "HTTPS_PROXY").is_none());
    }

    /// Default (no `dockerAccess`) → hardened: no dind sidecar, no docker-storage
    /// volume, no `DOCKER_HOST` on the agent container.
    #[test]
    fn no_docker_access_means_no_dind_sidecar() {
        let config = test_config();
        let plan = test_plan(); // docker_access defaults to None

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();

        assert!(
            spec.containers.iter().all(|c| c.name != "dind"),
            "no dind sidecar without dockerAccess"
        );
        assert!(
            spec.volumes
                .as_ref()
                .unwrap()
                .iter()
                .all(|v| v.name != "docker-storage"),
            "no docker-storage volume without dockerAccess"
        );
        let sandbox = spec
            .containers
            .iter()
            .find(|c| c.name == "sandbox")
            .unwrap();
        assert!(
            sandbox
                .env
                .as_ref()
                .unwrap()
                .iter()
                .all(|e| e.name != "DOCKER_HOST"),
            "no DOCKER_HOST env without dockerAccess"
        );
    }

    /// The shared-data emptyDir is always bounded by `shared_data_size`.
    #[test]
    fn shared_data_empty_dir_is_size_capped() {
        let mut config = test_config();
        config.sandbox.shared_data_size = "512Mi".into();
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let shared = volumes
            .iter()
            .find(|v| v.name == "shared-data")
            .expect("shared-data volume present");
        assert_eq!(
            shared
                .empty_dir
                .as_ref()
                .and_then(|e| e.size_limit.as_ref())
                .map(|q| q.0.as_str()),
            Some("512Mi"),
            "shared-data emptyDir must be size-capped"
        );
    }

    /// Flag OFF (default): no pod-level securityContext, and the agent/server
    /// containers keep DAC_OVERRIDE (the root-mode behaviour).
    #[test]
    fn run_as_non_root_off_keeps_root_behaviour() {
        let config = test_config(); // run_as_non_root defaults to false
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();
        assert!(
            spec.security_context.is_none(),
            "no pod securityContext when the flag is off"
        );

        for c in &spec.containers {
            let add = c
                .security_context
                .as_ref()
                .and_then(|s| s.capabilities.as_ref())
                .and_then(|caps| caps.add.as_ref())
                .expect("app container has added capabilities");
            assert!(
                add.iter().any(|cap| cap == "DAC_OVERRIDE"),
                "root mode keeps DAC_OVERRIDE on {}",
                c.name
            );
        }
    }

    /// Flag ON: pod-level securityContext runs containers as UID/GID 1000 with
    /// fsGroup 1000 + RuntimeDefault seccomp, and the agent/server containers
    /// drop DAC_OVERRIDE (ownership is now correct) but keep NET_BIND_SERVICE.
    #[test]
    fn run_as_non_root_on_sets_pod_security_context_and_drops_dac_override() {
        let mut config = test_config();
        config.sandbox.run_as_non_root = true;
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();

        let psc = spec
            .security_context
            .as_ref()
            .expect("pod securityContext present when the flag is on");
        assert_eq!(psc.run_as_user, Some(1000));
        assert_eq!(psc.run_as_group, Some(1000));
        assert_eq!(psc.run_as_non_root, Some(true));
        assert_eq!(psc.fs_group, Some(1000));
        // OnRootMismatch avoids a full recursive chown of the workspace PVC on
        // every mount (only walks when the volume root ownership is wrong).
        assert_eq!(
            psc.fs_group_change_policy.as_deref(),
            Some("OnRootMismatch")
        );
        assert_eq!(
            psc.seccomp_profile.as_ref().map(|s| s.type_.as_str()),
            Some("RuntimeDefault")
        );

        for c in &spec.containers {
            let caps = c
                .security_context
                .as_ref()
                .and_then(|s| s.capabilities.as_ref())
                .expect("app container has capabilities");
            let add = caps.add.as_ref().expect("added capabilities");
            assert!(
                !add.iter().any(|cap| cap == "DAC_OVERRIDE"),
                "non-root mode must drop DAC_OVERRIDE on {}",
                c.name
            );
            assert!(
                add.iter().any(|cap| cap == "NET_BIND_SERVICE"),
                "non-root mode keeps NET_BIND_SERVICE on {}",
                c.name
            );
        }
    }

    /// Even with the flag ON, the privileged `dind` sidecar keeps running as root
    /// (explicit runAsUser 0 / runAsNonRoot false so the kubelet admits it under
    /// the pod-level runAsNonRoot).
    #[test]
    fn run_as_non_root_on_keeps_dind_root() {
        let mut config = test_config();
        config.sandbox.run_as_non_root = true;
        let mut plan = test_plan();
        plan.repo_config.docker_access = Some(true);

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let dind = pod
            .spec
            .as_ref()
            .unwrap()
            .containers
            .iter()
            .find(|c| c.name == "dind")
            .expect("dind sidecar present");
        let sc = dind.security_context.as_ref().unwrap();
        assert_eq!(sc.privileged, Some(true));
        assert_eq!(sc.run_as_user, Some(0));
        assert_eq!(sc.run_as_non_root, Some(false));
    }

    /// The `nvme-setup` init container keeps its privileged root context and
    /// carries modest resource requests/limits.
    #[test]
    fn nvme_setup_init_container_is_root_and_has_resources() {
        use crate::config::SandboxCacheVolume;

        let mut config = test_config();
        config.sandbox.run_as_non_root = true;
        config.sandbox.cache_volumes = vec![SandboxCacheVolume {
            name: "bun".into(),
            mount_path: "/bun-cache".into(),
            env: vec![("BUN_INSTALL_CACHE_DIR".into(), "/bun-cache".into())],
        }];
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let nvme = pod
            .spec
            .as_ref()
            .unwrap()
            .init_containers
            .as_ref()
            .unwrap()
            .iter()
            .find(|c| c.name == "nvme-setup")
            .expect("nvme-setup init container present");

        let sc = nvme.security_context.as_ref().unwrap();
        assert_eq!(sc.privileged, Some(true));
        assert_eq!(sc.run_as_user, Some(0));
        assert_eq!(sc.run_as_non_root, Some(false));

        let resources = nvme
            .resources
            .as_ref()
            .expect("nvme-setup must declare resources");
        assert!(
            resources
                .requests
                .as_ref()
                .is_some_and(|r| r.contains_key("cpu") && r.contains_key("memory"))
        );
        assert!(
            resources
                .limits
                .as_ref()
                .is_some_and(|l| l.contains_key("cpu") && l.contains_key("memory"))
        );
    }

    /// The toolchain-persistence and cargo-target flags are off by default —
    /// no cargo subPath mounts, no CARGO_TARGET_DIR — and the build-cache
    /// emptyDir is sized from config.
    #[test]
    fn toolchain_persistence_flags_default_off() {
        let config = test_config();
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let build_cache = volumes
            .iter()
            .find(|v| v.name == "build-cache")
            .expect("build-cache volume present");
        assert_eq!(
            build_cache
                .empty_dir
                .as_ref()
                .and_then(|e| e.size_limit.as_ref())
                .map(|q| q.0.as_str()),
            Some("1536Mi")
        );

        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        let mounts = container.volume_mounts.as_ref().unwrap();
        assert!(
            !mounts
                .iter()
                .any(|m| m.mount_path.starts_with("/opt/cargo")),
            "no cargo mounts without the persist flag"
        );
        let env = container.env.as_ref().unwrap();
        assert!(
            !env.iter().any(|e| e.name == "CARGO_TARGET_DIR"),
            "no CARGO_TARGET_DIR without the build-cache flag"
        );
    }

    #[test]
    fn build_cache_size_can_be_overridden_per_repo() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.resources = Some(crate::repository::models::ResourceConfig {
            build_cache: Some("40Gi".into()),
            ..Default::default()
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let build_cache = volumes
            .iter()
            .find(|v| v.name == "build-cache")
            .expect("build-cache volume present");
        assert_eq!(
            build_cache
                .empty_dir
                .as_ref()
                .and_then(|e| e.size_limit.as_ref())
                .map(|q| q.0.as_str()),
            Some("40Gi")
        );
    }

    #[test]
    fn build_cache_can_use_task_scoped_node_host_path() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.resources = Some(crate::repository::models::ResourceConfig {
            build_cache: Some("40Gi".into()),
            build_cache_mode: Some("node".into()),
            build_cache_scope: Some("task".into()),
            ..Default::default()
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();
        let build_cache = spec
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "build-cache")
            .expect("build-cache volume present");
        assert_eq!(
            build_cache
                .host_path
                .as_ref()
                .map(|host_path| host_path.path.as_str()),
            Some("/mnt/cache/build-cache/tasks/task-1")
        );
        assert!(
            build_cache.empty_dir.is_none(),
            "node build-cache must not keep the emptyDir size cap"
        );

        let setup = spec
            .init_containers
            .as_ref()
            .unwrap()
            .iter()
            .find(|container| container.name == "build-cache-setup")
            .expect("build-cache setup init container present");
        assert!(setup.volume_mounts.as_ref().is_some_and(|mounts| {
            mounts
                .iter()
                .any(|mount| mount.name == "build-cache" && mount.mount_path == "/build-cache")
        }));
    }

    #[test]
    fn build_cache_can_use_shared_node_host_path() {
        let config = test_config();
        let mut plan = test_plan();
        plan.repo_config.resources = Some(crate::repository::models::ResourceConfig {
            build_cache_mode: Some("node".into()),
            build_cache_scope: Some("node".into()),
            ..Default::default()
        });

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let spec = pod.spec.as_ref().unwrap();
        let build_cache = spec
            .volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|v| v.name == "build-cache")
            .expect("build-cache volume present");
        assert_eq!(
            build_cache
                .host_path
                .as_ref()
                .map(|host_path| host_path.path.as_str()),
            Some("/mnt/cache/build-cache/shared")
        );
        assert!(
            spec.init_containers
                .as_ref()
                .unwrap()
                .iter()
                .any(|container| container.name == "build-cache-setup")
        );
    }

    /// With the flags on: cargo registry/git ride `workspace-data` subPaths
    /// (never all of /opt/cargo — that would shadow the baked toolchain),
    /// CARGO_TARGET_DIR points at build-cache, and the build-cache size limit
    /// follows config.
    #[test]
    fn toolchain_persistence_flags_enabled() {
        let mut config = test_config();
        config.sandbox.persist_toolchain = true;
        config.sandbox.cargo_target_on_build_cache = true;
        config.sandbox.build_cache_size = "12Gi".into();
        let plan = test_plan();

        let pod = build_pod(&config, "task-1", &plan, None, None).unwrap();
        let volumes = pod.spec.as_ref().unwrap().volumes.as_ref().unwrap();
        let build_cache = volumes
            .iter()
            .find(|v| v.name == "build-cache")
            .expect("build-cache volume present");
        assert_eq!(
            build_cache
                .empty_dir
                .as_ref()
                .and_then(|e| e.size_limit.as_ref())
                .map(|q| q.0.as_str()),
            Some("12Gi")
        );

        let container = build_sandbox_container(&config, "task-1", &plan, None).unwrap();
        let mounts = container.volume_mounts.as_ref().unwrap();
        for (mount_path, sub_path) in [
            ("/opt/cargo/registry", "cargo-registry"),
            ("/opt/cargo/git", "cargo-git"),
        ] {
            let m = mounts
                .iter()
                .find(|m| m.mount_path == mount_path)
                .unwrap_or_else(|| panic!("{mount_path} mount present"));
            assert_eq!(m.name, "workspace-data");
            assert_eq!(m.sub_path.as_deref(), Some(sub_path));
        }
        assert!(
            !mounts.iter().any(|m| m.mount_path == "/opt/cargo"),
            "must never shadow the baked toolchain at /opt/cargo"
        );

        let env = container.env.as_ref().unwrap();
        let target = env
            .iter()
            .find(|e| e.name == "CARGO_TARGET_DIR")
            .expect("CARGO_TARGET_DIR set");
        assert_eq!(
            target.value.as_deref(),
            Some("/opt/moatless/build-cache/target")
        );
    }
}
