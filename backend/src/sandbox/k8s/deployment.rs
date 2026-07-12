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
use super::{pod_builder, pvc};
use crate::config::Config;
use crate::sandbox::error::SandboxError;
use crate::sandbox::models::ProvisionRequest;

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

/// Create a Deployment wrapping the given Pod spec.
///
/// Extracts the pod's `.spec` and wraps it as the Deployment's pod template.
/// Labels are preserved so the Service selector (`moatless.io/task-id`) still matches.
/// Returns the deployment name + UID on success.
pub async fn create_sandbox_deployment(
    k8s: &K8sClient,
    task_id: &str,
    pod: Pod,
) -> Result<DeploymentHandle, SandboxError> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());

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
            "namespace": k8s.namespace(),
            "labels": labels,
        },
        "spec": {
            "replicas": 1,
            "selector": {
                "matchLabels": {
                    "moatless.io/task-id": task_id,
                }
            },
            // Recreate (not the default RollingUpdate): the workspace PVC is a
            // ReadWriteOnce EBS volume that can only attach to one node at a
            // time. RollingUpdate would surge a new pod before the old one is
            // gone, so the replacement gets a "Multi-Attach" error and stalls
            // in Pending until AWS force-detaches (~1-6 min). Recreate tears the
            // old pod fully down (volume detaches) before scheduling the new one.
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
            // Don't keep old ReplicaSets around
            "revisionHistoryLimit": 1,
        }
    }))
    .map_err(|e| SandboxError::PodBuildError(e.to_string()))?;

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
        Err(e) => return Err(SandboxError::KubeError(e)),
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
/// We deliberately do NOT just delete the pod and let the ReplicaSet recreate
/// it. That races the ReadWriteOnce workspace PVC: the ReplicaSet schedules a
/// replacement pod immediately (possibly on another node) while the old pod is
/// still `Terminating` and holding the EBS attachment, producing a
/// "Multi-Attach" error that stalls the new pod in `Pending` for 1-6 min.
/// Scaling to 0 first lets the volume detach before the replacement is created.
pub async fn restart_deployment(k8s: &K8sClient, task_id: &str) -> Result<(), SandboxError> {
    scale_deployment(k8s, task_id, 0).await?;
    wait_for_pods_deleted(k8s, task_id).await?;
    scale_deployment(k8s, task_id, 1).await?;
    info!(task_id, "Deployment restarted (scaled 0 -> 1 after termination)");
    Ok(())
}

/// Rebuild a task's Deployment from a (possibly filtered) provision request.
///
/// Unlike [`restart_deployment`] (which restarts the *same* Deployment/spec by
/// scaling it 0 -> 1), this deletes the Deployment and recreates it from a
/// freshly built pod spec. That is what makes a removed server container
/// actually disappear from the running pod.
///
/// The existing workspace PVC is reused so no data is lost.
pub async fn redeploy_with_plan(
    k8s: &K8sClient,
    config: &Config,
    task_id: &str,
    request: &ProvisionRequest,
) -> Result<(), SandboxError> {
    let pvc_name = pvc::find_task_pvc(k8s, task_id).await;

    // Mint a fresh bootstrap token (+ legacy user token during rollout) for the
    // rebuilt pod. The long-lived token is delivered over the credential channel.
    let minted = pod_builder::mint_sandbox_tokens(config, &request.user_id)?;

    let pod = pod_builder::build_pod(
        config,
        task_id,
        &request.plan,
        pvc_name.as_deref(),
        Some(pod_builder::SandboxAuthTokens::from_minted(&minted)),
    )
    .map_err(SandboxError::PodBuildError)?;

    // Delete the old Deployment first so create rebuilds from the new spec.
    delete_deployment(k8s, task_id)
        .await
        .map_err(SandboxError::KubeError)?;

    // Wait for the old pod to fully terminate before recreating. The Recreate
    // strategy only serializes attach/detach *within* one Deployment; here we
    // delete and recreate a fresh Deployment, so nothing stops the new pod from
    // being scheduled while the old one still holds the ReadWriteOnce PVC on its
    // node -> "Multi-Attach" stall. Blocking on termination closes that window.
    wait_for_pods_deleted(k8s, task_id).await?;

    create_sandbox_deployment(k8s, task_id, pod).await?;
    info!(task_id, "Deployment redeployed with updated pod spec");
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

/// Poll until no non-terminal pods match the task's label selector (old pod
/// fully gone), or the timeout elapses.
///
/// Returns an error on timeout: proceeding while an old pod still holds the
/// ReadWriteOnce workspace PVC lets the replacement race the attachment and
/// stall in Pending with a "Multi-Attach" error for minutes — the exact failure
/// this wait exists to prevent. Callers surface it as an environment error
/// rather than silently scheduling into the stall. A transient list error is
/// still treated as best-effort (log and proceed), since it isn't the stall.
pub async fn wait_for_pods_deleted(k8s: &K8sClient, task_id: &str) -> Result<(), SandboxError> {
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
                    "Waiting for old pod(s) to terminate before redeploy"
                );
            }
            Err(e) => {
                warn!(task_id, error = %e, "Failed to list pods while waiting for termination; proceeding");
                return Ok(());
            }
        }

        if tokio::time::Instant::now() >= deadline {
            warn!(
                task_id,
                "Timed out waiting for old pod(s) to terminate before redeploy"
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
        assert_eq!(deployment_sandbox_image(&dep).as_deref(), Some("sandbox:v2"));
    }

    #[test]
    fn deployment_sandbox_image_none_when_no_sandbox_container() {
        let dep = deployment_with_containers(vec![container(
            "web-server",
            Some("server:latest"),
        )]);
        assert_eq!(deployment_sandbox_image(&dep), None);
    }

    #[test]
    fn deployment_sandbox_image_none_when_spec_absent() {
        let dep = Deployment::default();
        assert_eq!(deployment_sandbox_image(&dep), None);
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
}
