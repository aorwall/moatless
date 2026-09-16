//! The Pod object: what a plan builds, the fingerprint that decides whether a
//! rebuild is needed, and the generation stamped on it.

use super::containers::{
    build_dind_container, build_sandbox_container, build_server_containers, docker_access_enabled,
    validate_compute_mode,
};
use super::env::{chrono_now_iso, redact_url_userinfo};
use super::placement::{
    node_docker_host_path, resolve_dind_storage_backend, resolve_dind_storage_size,
    resolve_sandbox_size,
};
use super::security::pod_security_context;
use super::*;

/// Build a complete Pod spec from config + plan (no template).
///
/// Generates the sandbox container, dynamic server containers, volumes, probes,
/// and security contexts entirely in code.
pub fn build_pod(
    config: &Config,
    task_id: &str,
    plan: &ProvisionPlan,
    pvc_name: Option<&str>,
    tokens: Option<SandboxAuthTokens<'_>>,
) -> Result<Pod, String> {
    validate_compute_mode(config, plan, pvc_name)?;
    let pod_name = generate_pod_name(&plan.repo_config.name, task_id);
    let namespace = config.sandbox.namespace.clone();

    // ── Labels ──────────────────────────────────────────────────────
    let mut labels = BTreeMap::new();
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());
    labels.insert("moatless.io/sandbox-id".to_string(), task_id.to_string());
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
    // a per-task PVC when the `pvc` switch is on, a node-local emptyDir when it
    // is off. Either way it is a single
    // volume shared across the sandbox and server containers, with `/tmp` served
    // from its `tmp` subPath so `/tmp` never charges against the pod's memory
    // budget.
    let mut volumes = vec![
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
        // No PVC — either the repo turned the switch off, or the caller is
        // rebuilding around a volume that would not attach. A node-local
        // emptyDir capped at the repo's workspace size (`resources.storage`,
        // default `K8S_SANDBOX_STORAGE_SIZE`). No `medium: Memory`, so it lives
        // on node disk, not the pod memory budget.
        volumes.push(Volume {
            name: "workspace-data".to_string(),
            empty_dir: Some(EmptyDirVolumeSource {
                size_limit: Some(Quantity(resolve_sandbox_size(config, plan))),
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    // ── Sandbox container ───────────────────────────────────────────
    let sandbox_container = build_sandbox_container(config, task_id, plan, tokens, pvc_name)?;

    // ── Server containers ───────────────────────────────────────────
    let server_containers = build_server_containers(config, task_id, plan)?;

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
            StorageMedium::Node => {
                volume.host_path = Some(HostPathVolumeSource {
                    path: node_docker_host_path(config, task_id),
                    type_: Some("DirectoryOrCreate".to_string()),
                });
            }
            StorageMedium::EmptyDir => {
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

    let mut pod = Pod {
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

    let fingerprint = pod_spec_fingerprint(&pod);
    pod.metadata
        .annotations
        .get_or_insert_with(BTreeMap::new)
        .insert(POD_SPEC_HASH_ANNOTATION.to_string(), fingerprint);

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

/// Annotation carrying the [`pod_spec_fingerprint`] of the spec a pod template
/// was built from.
pub const POD_SPEC_HASH_ANNOTATION: &str = "moatless.io/pod-spec-hash";

/// Env vars minted per build, which say nothing about the config a pod runs.
const PER_BUILD_ENV: [&str; 2] = ["SANDBOX_BOOTSTRAP_TOKEN", "USER_AUTH_TOKEN"];

/// Hash the part of a pod spec that comes from the config and the plan.
///
/// [`build_pod`] stamps this under [`POD_SPEC_HASH_ANNOTATION`], and the start
/// path compares it against a stopped Deployment's value; excludes minted auth
/// tokens, `HashMap`-ordered env, and annotations (incl. build time) — the
/// token filter matches by name, so `emit_legacy_user_token_env` alone never
/// rebuilds a template.
pub fn pod_spec_fingerprint(pod: &Pod) -> String {
    let mut spec = pod.spec.clone().unwrap_or_default();
    for container in &mut spec.containers {
        canonicalize_env(container);
    }
    for container in spec.init_containers.iter_mut().flatten() {
        canonicalize_env(container);
    }

    // `serde_json::Map` is a `BTreeMap` here, so every object key is ordered.
    let canonical = serde_json::json!({
        "labels": pod.metadata.labels,
        "spec": spec,
    });
    hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
}

pub(super) fn canonicalize_env(container: &mut Container) {
    let Some(env) = container.env.as_mut() else {
        return;
    };
    env.retain(|var| !PER_BUILD_ENV.contains(&var.name.as_str()));
    env.sort_by(|left, right| left.name.cmp(&right.name));
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
