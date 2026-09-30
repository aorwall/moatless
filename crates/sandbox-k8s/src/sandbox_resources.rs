//! The reaper's two k8s-native cycle operations.
//!
//! Both are pure k8s work — LIST managed Deployments to find the
//! running-Sandbox set, and LIST labeled
//! Deployments/Services/PVCs/IngressRoutes then delete one task's resources —
//! so they live beside the k8s client. Whether a listed task is an orphan is a
//! tasks-domain question the caller answers.

use std::collections::HashSet;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Service};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, ListParams};
use tracing::warn;

use super::client::{K8sClient, K8sError};

/// Sandbox-ids whose Deployment is meant to be running.
///
/// Not the set of running Tasks: a Sandbox is named for the Task that created
/// it, but other Tasks can attach to it.
pub async fn list_running_sandbox_ids(k8s: &K8sClient) -> Result<Vec<String>, K8sError> {
    // The replica filter is k8s-spec semantics, so it lives here; the caller
    // sees only the running sandbox-ids it needs.
    let deployments = list_sandbox_deployments(k8s)
        .await
        .map_err(|e| K8sError::new("listing sandbox deployments", e))?;
    Ok(deployments
        .into_iter()
        .filter(|d| d.desired_replicas > 0)
        .map(|d| d.sandbox_id)
        .collect())
}

/// A managed sandbox Deployment, reduced to what the reaper cycle needs.
struct DeploymentInfo {
    sandbox_id: String,
    /// `spec.replicas` (defaults to 1 when unset). `0` means intentionally
    /// stopped; `> 0` means the sandbox is meant to be running.
    desired_replicas: i32,
}

/// List all managed sandbox Deployments (any replica count) with their
/// sandbox-id and desired replicas via a single cluster-wide LIST.
async fn list_sandbox_deployments(k8s: &K8sClient) -> Result<Vec<DeploymentInfo>, kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("app.kubernetes.io/managed-by=moatless");

    let list = deployments.list(&lp).await?;

    let infos = list
        .items
        .into_iter()
        .filter_map(|dep| {
            // The Deployment is labeled with its creating Task's id, which is
            // also the Sandbox's id.
            let sandbox_id = dep
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id").cloned())?;
            let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
            Some(DeploymentInfo {
                sandbox_id,
                desired_replicas,
            })
        })
        .collect();

    Ok(infos)
}

/// Collect the `moatless.io/task-id` label from an iterator of resource
/// metadata into `ids`.
fn collect_ids_from_meta<'a>(
    ids: &mut HashSet<String>,
    metas: impl Iterator<Item = &'a ObjectMeta>,
) {
    for meta in metas {
        if let Some(id) = meta
            .labels
            .as_ref()
            .and_then(|l| l.get("moatless.io/task-id"))
        {
            ids.insert(id.clone());
        }
    }
}

/// Discover every task-id that still has at least one labeled sandbox resource
/// (Deployment, Service, PVC, or `IngressRoute`). Best-effort: a failure
/// listing any one resource type is logged and skipped.
pub async fn collect_labeled_task_ids(k8s: &K8sClient) -> HashSet<String> {
    let ns = k8s.namespace();
    let lp = ListParams::default().labels("moatless.io/task-id");
    let mut ids = HashSet::new();

    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), ns);
    match deployments.list(&lp).await {
        Ok(list) => collect_ids_from_meta(&mut ids, list.items.iter().map(|d| &d.metadata)),
        Err(e) => warn!(error = %e, "orphan-gc: failed to list Deployments"),
    }

    let services: Api<Service> = Api::namespaced(k8s.client.clone(), ns);
    match services.list(&lp).await {
        Ok(list) => collect_ids_from_meta(&mut ids, list.items.iter().map(|s| &s.metadata)),
        Err(e) => warn!(error = %e, "orphan-gc: failed to list Services"),
    }

    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), ns);
    match pvcs.list(&lp).await {
        Ok(list) => collect_ids_from_meta(&mut ids, list.items.iter().map(|p| &p.metadata)),
        Err(e) => warn!(error = %e, "orphan-gc: failed to list PVCs"),
    }

    for id in super::ingress::list_ingress_route_task_ids(k8s).await {
        ids.insert(id);
    }

    ids
}

/// Best-effort delete of every labeled K8s resource for a task. Each delete is
/// idempotent (404 → Ok) and its failure is logged, never propagated.
pub async fn delete_orphaned_task_resources(k8s: &K8sClient, task_id: &str) {
    use super::{deployment, ingress, networkpolicy, pvc, service};

    if let Err(e) = deployment::delete_deployment(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete Deployment");
    }
    if let Err(e) = service::delete_sandbox_service(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete Service");
    }
    if let Err(e) = ingress::delete_task_ingress_routes(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete IngressRoutes");
    }
    if let Err(e) = networkpolicy::delete_task_preview_network_policy(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete preview NetworkPolicy");
    }
    if let Err(e) = pvc::delete_task_pvc(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete PVC");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_ids_from_meta_dedupes_and_skips_unlabeled() {
        let mut ids = HashSet::new();
        let labeled = |task_id: &str| ObjectMeta {
            labels: Some(
                [("moatless.io/task-id".to_string(), task_id.to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        let unlabeled = ObjectMeta::default();
        let metas = [labeled("a"), labeled("a"), labeled("b"), unlabeled];
        collect_ids_from_meta(&mut ids, metas.iter());
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
    }
}
