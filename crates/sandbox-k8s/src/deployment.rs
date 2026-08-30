//! Kubernetes Deployment lifecycle operations for sandboxes.
//!
//! Wraps pod specs (built by `pod_builder`) in Deployments for automatic
//! restart, stop (scale to 0), and start (scale to 1) without needing
//! the full ProvisionPlan.

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, Patch, PatchParams, PostParams};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_service_name;
use super::pod_builder;
use moatless_core::config::Config;
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::runtime::{
    generation_from_label, RuntimeOperationContext, OPERATION_GENERATION_LABEL,
};
use moatless_sandbox_runtime::types::OperationGeneration;
use moatless_sandbox_runtime::models::ProvisionRequest;

/// Name + UID of a created (or reused) Deployment.
///
/// The `uid` is used to build `ownerReferences` on the Service and IngressRoutes
/// so K8s garbage collection cascades their deletion when the Deployment is
/// deleted. It is `None` only when a reused (409) Deployment couldn't be
/// re-fetched — callers then simply skip owner-referencing (best effort).
pub struct DeploymentHandle {
    pub name: String,
    pub uid: Option<String>,
}

/// Build the Deployment object wrapping the given Pod spec.
///
/// Extracts the pod's `.spec` and wraps it as the Deployment's pod template.
/// Labels are preserved so the Service selector (`moatless.io/task-id`) still matches.
fn build_deployment_object(
    namespace: &str,
    task_id: &str,
    pod: Pod,
) -> Result<Deployment, SandboxError> {
    let deployment_name = generate_deployment_name(task_id);
    let pod_labels = pod.metadata.labels.clone().unwrap_or_default();
    let pod_annotations = pod.metadata.annotations.clone().unwrap_or_default();
    let pod_spec = pod
        .spec
        .ok_or_else(|| SandboxError::PodBuildError("Pod has no spec".to_string()))?;

    // Deployments only support restartPolicy "Always"
    if let Some(ref policy) = pod_spec.restart_policy {
        if policy != "Always" {
            return Err(SandboxError::PodBuildError(format!(
                "Pod template has restartPolicy \"{policy}\", but Deployments only support \"Always\""
            )));
        }
    }

    let mut labels = BTreeMap::new();
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
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

    // Merge pod labels into deployment labels
    for (k, v) in &pod_labels {
        labels.entry(k.clone()).or_insert_with(|| v.clone());
    }

    let pod_spec_value =
        serde_json::to_value(&pod_spec).map_err(|e| SandboxError::PodBuildError(e.to_string()))?;

    let deployment: Deployment = serde_json::from_value(serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": deployment_name,
            "namespace": namespace,
            "labels": labels,
        },
        "spec": {
            "replicas": 1,
            "selector": {
                "matchLabels": {
                    "moatless.io/task-id": task_id,
                }
            },
            // Recreate: the workspace PVC is a ReadWriteOnce EBS volume that can
            // only attach to one node at a time. The current pod must terminate
            // and detach the volume before a replacement is scheduled.
            "strategy": {
                "type": "Recreate",
            },
            "template": {
                "metadata": {
                    "labels": pod_labels,
                    "annotations": pod_annotations,
                },
                "spec": pod_spec_value,
            },
            // Keep only the current ReplicaSet.
            "revisionHistoryLimit": 1,
        }
    }))
    .map_err(|e| SandboxError::PodBuildError(e.to_string()))?;

    Ok(deployment)
}

/// Create a Deployment wrapping the given Pod spec.
///
/// Returns the deployment name + UID on success. An already-existing
/// Deployment (409) is reused.
pub async fn create_sandbox_deployment(
    k8s: &K8sClient,
    task_id: &str,
    pod: Pod,
) -> Result<DeploymentHandle, SandboxError> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let deployment_name = generate_deployment_name(task_id);
    let deployment = build_deployment_object(k8s.namespace(), task_id, pod)?;

    let uid = match deployments
        .create(&PostParams::default(), &deployment)
        .await
    {
        Ok(created) => {
            info!(task_id, deployment_name = %deployment_name, "Deployment created");
            created.metadata.uid
        }
        Err(kube::Error::Api(ref e)) if e.code == 409 => {
            debug!(task_id, deployment_name = %deployment_name, "Deployment already exists, reusing");
            // Re-fetch so callers can still owner-reference dependents. A failed
            // GET is non-fatal: owner-referencing is best-effort.
            match deployments.get(&deployment_name).await {
                Ok(existing) => existing.metadata.uid,
                Err(e) => {
                    warn!(task_id, error = %e, "Failed to fetch existing Deployment UID");
                    None
                }
            }
        }
        Err(e) => return Err(SandboxError::KubeError(Box::new(e))),
    };

    Ok(DeploymentHandle {
        name: deployment_name,
        uid,
    })
}

/// Scale a Deployment to the specified number of replicas.
///
/// Uses the PATCH scale subresource for efficiency.
pub async fn scale_deployment(
    k8s: &K8sClient,
    task_id: &str,
    replicas: u32,
) -> Result<(), kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let deployment_name = generate_deployment_name(task_id);

    let patch = serde_json::json!({
        "spec": {
            "replicas": replicas,
        }
    });

    deployments
        .patch(
            &deployment_name,
            &PatchParams::apply("moatless-vibe-backend"),
            &Patch::Merge(&patch),
        )
        .await?;

    info!(task_id, deployment_name = %deployment_name, replicas, "Deployment scaled");
    Ok(())
}

fn operation_generation_patch(ctx: RuntimeOperationContext) -> serde_json::Value {
    serde_json::json!({
        "spec": {
            "template": {
                "metadata": {
                    "labels": {
                        OPERATION_GENERATION_LABEL: ctx.generation_label(),
                    }
                }
            }
        }
    })
}

/// Stamp the replacement generation onto the Deployment's pod template before
/// scaling up. The merge patch leaves the `Recreate` strategy and existing pod
/// labels untouched.
pub async fn stamp_operation_generation(
    k8s: &K8sClient,
    task_id: &str,
    ctx: RuntimeOperationContext,
) -> Result<(), kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let deployment_name = generate_deployment_name(task_id);
    deployments
        .patch(
            &deployment_name,
            &PatchParams::apply("moatless-vibe-backend"),
            &Patch::Merge(&operation_generation_patch(ctx)),
        )
        .await?;
    Ok(())
}

/// Place a stopped/restarting node-backed sandbox on the node that owns its
/// retained hostPath data, or clear any prior placement when `node_name` is
/// `None`. The merge patch preserves the rest of the pod template.
///
/// Clearing is load-bearing: a Deployment keeps the placement an earlier start
/// wrote, so once its recorded nodes are gone the sandbox stays bound to a
/// destroyed node and never schedules again.
pub async fn set_node_placement(
    k8s: &K8sClient,
    task_id: &str,
    node_name: Option<&str>,
) -> Result<(), kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let deployment_name = generate_deployment_name(task_id);
    // `null` clears `nodeName` and whichever affinity mode is unused, so a merge
    // patch can neither stack two placements nor leave a stale one behind.
    let node_affinity = node_name
        .map(pod_builder::retained_node_affinity)
        .and_then(|affinity| affinity.node_affinity)
        .unwrap_or_default();
    let patch = serde_json::json!({
        "spec": {
            "template": {
                "spec": {
                    "nodeName": null,
                    "affinity": {
                        "nodeAffinity": {
                            "requiredDuringSchedulingIgnoredDuringExecution":
                                node_affinity.required_during_scheduling_ignored_during_execution,
                            "preferredDuringSchedulingIgnoredDuringExecution":
                                node_affinity.preferred_during_scheduling_ignored_during_execution,
                        }
                    }
                }
            }
        }
    });
    deployments
        .patch(
            &deployment_name,
            &PatchParams::apply("moatless-vibe-backend"),
            &Patch::Merge(&patch),
        )
        .await?;
    Ok(())
}

/// Delete a Deployment for a task. Handles 404 gracefully.
pub async fn delete_deployment(k8s: &K8sClient, task_id: &str) -> Result<(), kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let deployment_name = generate_deployment_name(task_id);

    match deployments
        .delete(&deployment_name, &Default::default())
        .await
    {
        Ok(_) => {
            info!(task_id, deployment_name = %deployment_name, "Deployment deleted");
            Ok(())
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(task_id, "Deployment not found, already deleted");
            Ok(())
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to delete Deployment");
            Err(e)
        }
    }
}

/// Restart a Deployment gracefully: scale to 0, wait for the pod to fully
/// terminate, then scale back to 1.
///
/// Scaling to 0 first lets the ReadWriteOnce workspace PVC detach before the
/// replacement pod is created.
pub async fn restart_deployment(
    k8s: &K8sClient,
    task_id: &str,
    ctx: RuntimeOperationContext,
) -> Result<(), SandboxError> {
    scale_deployment(k8s, task_id, 0)
        .await
        .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
    wait_for_pods_deleted(k8s, task_id).await?;
    stamp_operation_generation(k8s, task_id, ctx)
        .await
        .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
    scale_deployment(k8s, task_id, 1)
        .await
        .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
    info!(
        task_id,
        "Deployment restarted (scaled 0 -> 1 after termination)"
    );
    Ok(())
}

/// Rebuild a task's Deployment from a (possibly filtered) provision request.
///
/// Replaces the Deployment's spec with a freshly built pod template. Removed
/// server containers disappear from the running pod through this path.
///
/// The replace is done in place (GET + PUT). The Service and IngressRoutes carry
/// an `ownerReference` to the Deployment, so the Deployment UID must be preserved
/// to keep the sandbox reachable over gRPC and preview routes. The `Recreate`
/// strategy still terminates the current pod and detaches the ReadWriteOnce PVC
/// before scheduling the replacement.
///
/// `pvc_name` is resolved by the caller, which is the only party that knows
/// whether this rebuild wants the task's PVC: an ordinary redeploy passes the
/// existing claim so no data is lost, while the attach-failure fallback passes
/// `None` to come back up on an emptyDir restored from S3.
pub async fn redeploy_with_plan(
    k8s: &K8sClient,
    config: &Config,
    task_id: &str,
    request: &ProvisionRequest,
    ctx: RuntimeOperationContext,
    retained_node: Option<&str>,
    pvc_name: Option<&str>,
) -> Result<(), SandboxError> {
    // Mint fresh auth tokens for the rebuilt pod. The long-lived token is
    // delivered over the credential channel.
    let minted = pod_builder::mint_sandbox_tokens(config, task_id, &request.user_id)?;

    let mut pod = pod_builder::build_pod(
        config,
        task_id,
        &request.plan,
        pvc_name,
        Some(pod_builder::SandboxAuthTokens::from_minted(&minted)),
    )
    .map_err(SandboxError::PodBuildError)?;
    if let Some(node_name) = retained_node {
        pod_builder::pin_pod_to_node(&mut pod, node_name)?;
    }
    pod_builder::stamp_operation_generation(&mut pod, ctx);

    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let deployment_name = generate_deployment_name(task_id);
    let mut deployment = build_deployment_object(k8s.namespace(), task_id, pod)?;

    match deployments.get(&deployment_name).await {
        Ok(existing) => {
            // Replace requires the current resourceVersion for optimistic concurrency.
            deployment.metadata.resource_version = existing.metadata.resource_version;
            deployments
                .replace(&deployment_name, &PostParams::default(), &deployment)
                .await
                .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
            info!(task_id, deployment_name = %deployment_name, "Deployment replaced in place with updated pod spec");
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            // No existing Deployment (cleaned up out of band) — create fresh.
            deployments
                .create(&PostParams::default(), &deployment)
                .await
                .map_err(|e| SandboxError::KubeError(Box::new(e)))?;
            info!(task_id, deployment_name = %deployment_name, "Deployment created (was missing during redeploy)");
        }
        Err(e) => return Err(SandboxError::KubeError(Box::new(e))),
    }
    Ok(())
}

/// True when a pod can still hold volume attachments and so must be waited on
/// before recreating its Deployment. Pods in a terminal phase
/// (`Succeeded`/`Failed`) have no running containers and hold no attachments —
/// notably, K8s never garbage-collects `Failed` pods (e.g. evicted for
/// exceeding an emptyDir sizeLimit), so treating them as blocking would stall
/// every subsequent start until the timeout.
fn pod_blocks_redeploy(pod: &Pod) -> bool {
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("");
    !matches!(phase, "Succeeded" | "Failed")
}

/// Poll until no non-terminal pods match the task's label selector, or the
/// timeout elapses.
///
/// Returns an error on timeout: proceeding while a terminating pod still holds the
/// ReadWriteOnce workspace PVC lets the replacement race the attachment and
/// stall in Pending with a "Multi-Attach" error for minutes — the exact failure
/// this wait exists to prevent. Callers surface it as an environment error
/// rather than silently scheduling into the stall. A transient list error is
/// still treated as best-effort (log and proceed), since it isn't the stall.
pub async fn wait_for_pods_deleted(k8s: &K8sClient, task_id: &str) -> Result<(), SandboxError> {
    wait_for_pods_deleted_inner(k8s, task_id, false).await
}

/// Strict removal variant: observation failures are retried until the same
/// bounded deadline instead of being treated as evidence of absence.
pub async fn wait_for_all_pods_deleted(k8s: &K8sClient, task_id: &str) -> Result<(), SandboxError> {
    wait_for_pods_deleted_inner(k8s, task_id, true).await
}

async fn wait_for_pods_deleted_inner(
    k8s: &K8sClient,
    task_id: &str,
    strict_observation: bool,
) -> Result<(), SandboxError> {
    use std::time::Duration;

    const POLL_INTERVAL: Duration = Duration::from_secs(2);
    // Pods have a 30s termination grace period; allow headroom for detach.
    const TIMEOUT: Duration = Duration::from_secs(90);

    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        match pods_api.list(&lp).await {
            Ok(list) => {
                let blocking = list.items.iter().filter(|p| pod_blocks_redeploy(p)).count();
                if blocking == 0 {
                    debug!(task_id, "All pods terminated, safe to recreate Deployment");
                    return Ok(());
                }
                debug!(
                    task_id,
                    remaining = blocking,
                    "Waiting for current pod(s) to terminate before redeploy"
                );
            }
            Err(e) => {
                if strict_observation {
                    warn!(task_id, error = %e, "Failed to observe pod deletion; retrying");
                } else {
                    warn!(task_id, error = %e, "Failed to list pods while waiting for termination; proceeding");
                    return Ok(());
                }
            }
        }

        if tokio::time::Instant::now() >= deadline {
            warn!(
                task_id,
                "Timed out waiting for current pod(s) to terminate before redeploy"
            );
            return Err(SandboxError::NotReady(format!(
                "Timed out after {}s waiting for the previous sandbox pod to terminate; \
                 its workspace volume is still attached. Retry shortly.",
                TIMEOUT.as_secs()
            )));
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Find a Deployment for a task by label selector.
pub async fn find_deployment(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<Option<Deployment>, kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let list = deployments.list(&lp).await?;
    Ok(list.items.into_iter().next())
}

/// Await observed absence of every Deployment for a task.
pub async fn wait_for_deployment_deleted(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), SandboxError> {
    super::wait::wait_absent(
        task_id,
        "Deployment",
        std::time::Duration::from_mins(1),
        || async {
            find_deployment(k8s, task_id)
                .await
                .map(|item| item.is_none())
        },
    )
    .await
}

/// Coarse state for one labeled sandbox Deployment.
pub struct SandboxDeploymentState {
    pub task_id: String,
    pub desired_replicas: i32,
    pub ready_replicas: i32,
    pub operation_generation: Option<OperationGeneration>,
}

/// Coarse desired/ready replica state for every labeled sandbox Deployment in
/// one LIST — the reconciler's per-tick fleet observation.
pub async fn list_sandbox_deployment_states(
    k8s: &K8sClient,
) -> Result<Vec<SandboxDeploymentState>, kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("moatless.io/task-id");
    let list = deployments.list(&lp).await?;

    Ok(list
        .items
        .into_iter()
        .filter_map(|dep| {
            let task_id = dep
                .metadata
                .labels
                .as_ref()?
                .get("moatless.io/task-id")?
                .clone();
            let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
            let ready = dep
                .status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0);
            let generation = dep
                .spec
                .as_ref()
                .and_then(|spec| spec.template.metadata.as_ref())
                .and_then(|metadata| metadata.labels.as_ref())
                .and_then(|labels| generation_from_label(labels.get(OPERATION_GENERATION_LABEL)));
            Some(SandboxDeploymentState {
                task_id,
                desired_replicas: desired,
                ready_replicas: ready,
                operation_generation: generation,
            })
        })
        .collect())
}

/// Get the current replica counts (desired, ready) for a task's Deployment.
pub async fn get_deployment_replicas(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(i32, i32), kube::Error> {
    let deployment_name = generate_deployment_name(task_id);
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());

    match deployments.get(&deployment_name).await {
        Ok(dep) => {
            let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
            let ready = dep
                .status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0);
            Ok((desired, ready))
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => Ok((0, 0)),
        Err(e) => Err(e),
    }
}

/// Extract the image of the `sandbox` container from a Deployment's pod template.
///
/// Returns `None` if the template, container list, or the `sandbox` container's
/// image are absent. Used by the start path to detect whether a stopped
/// Deployment is pinned to a stale image tag.
pub fn deployment_sandbox_image(dep: &Deployment) -> Option<String> {
    dep.spec
        .as_ref()?
        .template
        .spec
        .as_ref()?
        .containers
        .iter()
        .find(|c| c.name == "sandbox")
        .and_then(|c| c.image.clone())
}

/// Generate a deterministic Deployment name from a task ID.
///
/// Uses the same base as the service name for consistency.
fn generate_deployment_name(task_id: &str) -> String {
    // Service name is already "moatless-ws-{task_id}", reuse it for the deployment
    generate_service_name(task_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::apps::v1::DeploymentSpec;
    use k8s_openapi::api::core::v1::{Container, PodSpec, PodTemplateSpec};

    fn deployment_with_containers(containers: Vec<Container>) -> Deployment {
        Deployment {
            spec: Some(DeploymentSpec {
                template: PodTemplateSpec {
                    spec: Some(PodSpec {
                        containers,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn container(name: &str, image: Option<&str>) -> Container {
        Container {
            name: name.to_string(),
            image: image.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn deployment_sandbox_image_extracts_sandbox_container() {
        let dep = deployment_with_containers(vec![
            container("web-server", Some("server:latest")),
            container("sandbox", Some("sandbox:v2")),
        ]);
        assert_eq!(
            deployment_sandbox_image(&dep).as_deref(),
            Some("sandbox:v2")
        );
    }

    #[test]
    fn deployment_sandbox_image_none_when_no_sandbox_container() {
        let dep = deployment_with_containers(vec![container("web-server", Some("server:latest"))]);
        assert_eq!(deployment_sandbox_image(&dep), None);
    }

    #[test]
    fn deployment_sandbox_image_none_when_spec_absent() {
        let dep = Deployment::default();
        assert_eq!(deployment_sandbox_image(&dep), None);
    }

    #[test]
    fn restart_patch_labels_replacement_without_changing_strategy() {
        let patch =
            operation_generation_patch(RuntimeOperationContext::new(OperationGeneration(23)));
        assert_eq!(
            patch["spec"]["template"]["metadata"]["labels"][OPERATION_GENERATION_LABEL],
            "23"
        );
        assert!(
            patch["spec"].get("strategy").is_none(),
            "the merge patch preserves the Deployment's Recreate strategy"
        );
    }

    fn pod_in_phase(phase: Option<&str>) -> Pod {
        Pod {
            status: phase.map(|p| k8s_openapi::api::core::v1::PodStatus {
                phase: Some(p.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Terminal pods (evicted corpses, completed pods) hold no volume
    /// attachments and must not stall the pre-scale-up wait — otherwise a
    /// never-GC'd evicted pod blocks every start of its task.
    #[test]
    fn terminal_pods_do_not_block_redeploy() {
        assert!(!pod_blocks_redeploy(&pod_in_phase(Some("Failed"))));
        assert!(!pod_blocks_redeploy(&pod_in_phase(Some("Succeeded"))));
    }

    #[test]
    fn live_pods_block_redeploy() {
        assert!(pod_blocks_redeploy(&pod_in_phase(Some("Running"))));
        assert!(pod_blocks_redeploy(&pod_in_phase(Some("Pending"))));
        // Unknown/missing phase: err on the side of waiting.
        assert!(pod_blocks_redeploy(&pod_in_phase(None)));
    }

    fn buildable_pod() -> Pod {
        Pod {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                labels: Some(BTreeMap::from([(
                    "moatless.io/task-id".to_string(),
                    "task-1".to_string(),
                )])),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![container("sandbox", Some("sandbox:v1"))],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Kubernetes accepts an in-place Deployment replace only when the name and
    /// immutable `spec.selector` match the existing object. Both must therefore be
    /// deterministic functions of the task_id, independent of the wrapped pod spec.
    #[test]
    fn build_deployment_object_is_stable_across_pod_specs() {
        let dep =
            build_deployment_object("ns", "task-1", buildable_pod()).expect("build should succeed");

        assert_eq!(dep.metadata.name.as_deref(), Some("moatless-ws-task-1"));
        let spec = dep.spec.as_ref().expect("deployment has a spec");
        assert_eq!(
            spec.selector
                .match_labels
                .as_ref()
                .and_then(|l| l.get("moatless.io/task-id"))
                .map(String::as_str),
            Some("task-1")
        );
        // Recreate lets the controller terminate the current pod and detach the
        // RWO PVC before scheduling the replacement.
        assert_eq!(
            spec.strategy.as_ref().and_then(|s| s.type_.as_deref()),
            Some("Recreate")
        );
    }

    #[test]
    fn build_deployment_object_rejects_non_always_restart_policy() {
        let mut pod = buildable_pod();
        if let Some(spec) = pod.spec.as_mut() {
            spec.restart_policy = Some("Never".to_string());
        }
        assert!(build_deployment_object("ns", "task-1", pod).is_err());
    }
}
