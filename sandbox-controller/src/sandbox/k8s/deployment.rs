//! Kubernetes Deployment lifecycle operations for workspace sandboxes.
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
use crate::sandbox::error::SandboxError;

/// Create a Deployment wrapping the given Pod spec.
///
/// Extracts the pod's `.spec` and wraps it as the Deployment's pod template.
/// Labels are preserved so the Service selector (`moatless.io/task-id`) still matches.
/// Returns the deployment name on success.
pub async fn create_workspace_deployment(
    k8s: &K8sClient,
    task_id: &str,
    pod: Pod,
) -> Result<String, SandboxError> {
    let deployments: Api<Deployment> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());

    let deployment_name = generate_deployment_name(task_id);
    let pod_labels = pod.metadata.labels.clone().unwrap_or_default();
    let pod_annotations = pod.metadata.annotations.clone().unwrap_or_default();
    let pod_spec = pod.spec.ok_or_else(|| {
        SandboxError::PodBuildError("Pod has no spec".to_string())
    })?;

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
        "moatless-workspace".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "worker".to_string(),
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

    let pod_spec_value = serde_json::to_value(&pod_spec)
        .map_err(|e| SandboxError::PodBuildError(e.to_string()))?;

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

    match deployments.create(&PostParams::default(), &deployment).await {
        Ok(_) => {
            info!(task_id, deployment_name = %deployment_name, "Deployment created");
        }
        Err(kube::Error::Api(ref e)) if e.code == 409 => {
            debug!(task_id, deployment_name = %deployment_name, "Deployment already exists, reusing");
        }
        Err(e) => return Err(SandboxError::KubeError(e)),
    }

    Ok(deployment_name)
}

/// Scale a Deployment to the specified number of replicas.
///
/// Uses the PATCH scale subresource for efficiency.
pub async fn scale_deployment(
    k8s: &K8sClient,
    task_id: &str,
    replicas: u32,
) -> Result<(), kube::Error> {
    let deployments: Api<Deployment> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());
    let deployment_name = generate_deployment_name(task_id);

    let patch = serde_json::json!({
        "spec": {
            "replicas": replicas,
        }
    });

    deployments
        .patch(
            &deployment_name,
            &PatchParams::apply("sandbox-controller"),
            &Patch::Merge(&patch),
        )
        .await?;

    info!(task_id, deployment_name = %deployment_name, replicas, "Deployment scaled");
    Ok(())
}

/// Delete a Deployment for a task. Handles 404 gracefully.
pub async fn delete_deployment(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), kube::Error> {
    let deployments: Api<Deployment> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());
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

/// Restart a Deployment by deleting all its pods.
///
/// The Deployment controller will automatically recreate them.
pub async fn restart_deployment(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), kube::Error> {
    let pods_api: Api<Pod> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let list = pods_api.list(&lp).await?;
    for pod in list.items {
        if let Some(ref name) = pod.metadata.name {
            pods_api.delete(name, &Default::default()).await?;
            info!(task_id, pod_name = %name, "Pod deleted for restart (Deployment will recreate)");
        }
    }

    Ok(())
}

/// Find a Deployment for a task by label selector.
pub async fn find_deployment(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<Option<Deployment>, kube::Error> {
    let deployments: Api<Deployment> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());
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
    let deployments: Api<Deployment> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());

    match deployments.get(&deployment_name).await {
        Ok(dep) => {
            let desired = dep
                .spec
                .as_ref()
                .and_then(|s| s.replicas)
                .unwrap_or(1);
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

/// Generate a deterministic Deployment name from a task ID.
///
/// Uses the same base as the service name for consistency.
fn generate_deployment_name(task_id: &str) -> String {
    
    // Service name is already "moatless-ws-{task_id}", reuse it for the deployment
    generate_service_name(task_id)
}
