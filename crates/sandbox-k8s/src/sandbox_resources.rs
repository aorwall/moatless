//! The sandbox-k8s implementation of the reaper's
//! [`SandboxResources`](moatless_tasks::task::reaper::SandboxResources) port — the
//! reaper's two k8s-native cycle operations.
//!
//! The reaper (tasks band) drives these each cycle but names no Kubernetes type:
//! it holds an `Arc<dyn SandboxResources>` declared consumer-side and wired here at
//! the composition root — the `AgentErrorSink` shape. Both bodies are pure k8s
//! work — LIST managed Deployments to find the running-Sandbox set, and LIST
//! labeled Deployments/Services/PVCs/IngressRoutes then delete the ones whose
//! Task row is gone — so they live beside the k8s client rather than a band
//! above it. The one tasks-domain question the GC asks — does this task still
//! exist? — is answered through the `TaskDao` port, a legal downward
//! `sandbox-k8s -> tasks` edge (the `pod_watcher` precedent).

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Service};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, ListParams};
use tracing::{info, warn};

use super::client::K8sClient;
use moatless_tasks::task::dao::TaskDao;
use moatless_tasks::task::reaper::SandboxResources;
use moatless_backend_core::scope::ScopeContext;

/// The k8s-backed [`SandboxResources`]: owns a client to talk to the cluster and
/// a `TaskDao` to test whether a labeled resource's task still exists.
pub struct K8sSandboxResources {
    k8s: K8sClient,
    tasks: Arc<dyn TaskDao>,
}

impl K8sSandboxResources {
    pub fn new(k8s: K8sClient, tasks: Arc<dyn TaskDao>) -> Self {
        Self { k8s, tasks }
    }
}

#[async_trait]
impl SandboxResources for K8sSandboxResources {
    async fn list_running_sandbox_ids(
        &self,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        // The replica filter is k8s-spec semantics, so it lives here; the reaper
        // sees only the running task-ids it needs.
        let deployments = list_sandbox_deployments(&self.k8s).await?;
        Ok(deployments
            .into_iter()
            .filter(|d| d.desired_replicas > 0)
            .map(|d| d.task_id)
            .collect())
    }

    async fn gc_orphaned_resources(&self) {
        gc_orphaned_resources(&self.k8s, &self.tasks).await;
    }
}

/// A managed sandbox Deployment, reduced to what the reaper cycle needs.
struct DeploymentInfo {
    task_id: String,
    /// `spec.replicas` (defaults to 1 when unset). `0` means intentionally
    /// stopped; `> 0` means the sandbox is meant to be running.
    desired_replicas: i32,
}

/// List all managed sandbox Deployments (any replica count) with their task-id
/// and desired replicas via a single cluster-wide LIST.
async fn list_sandbox_deployments(k8s: &K8sClient) -> Result<Vec<DeploymentInfo>, kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("app.kubernetes.io/managed-by=moatless");

    let list = deployments.list(&lp).await?;

    let infos = list
        .items
        .into_iter()
        .filter_map(|dep| {
            let task_id = dep
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id").cloned())?;
            let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
            Some(DeploymentInfo {
                task_id,
                desired_replicas,
            })
        })
        .collect();

    Ok(infos)
}

/// Outcome of evaluating a labeled sandbox resource for orphan GC.
#[derive(Debug, PartialEq, Eq)]
enum OrphanDecision {
    /// The resource's Task row is gone, so lifecycle cannot express removal.
    Delete,
    /// An existing Task's desired-state convergence belongs to lifecycle.
    Keep,
}

/// Pure decision function: decide whether a task's leftover sandbox resources
/// are orphans safe to GC.
fn decide_orphan(task_exists: bool) -> OrphanDecision {
    if task_exists {
        OrphanDecision::Keep
    } else {
        OrphanDecision::Delete
    }
}

/// Collect the `moatless.io/task-id` label from an iterator of resource metadata
/// into `ids`.
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
/// (Deployment, Service, PVC, or IngressRoute). Best-effort: a failure listing
/// any one resource type is logged and skipped.
async fn collect_labeled_task_ids(k8s: &K8sClient) -> HashSet<String> {
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
async fn delete_orphaned_task_resources(k8s: &K8sClient, task_id: &str) {
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

/// Orphan-GC pass: LIST labeled sandbox resources, then delete only resources
/// whose Task row no longer exists. Fully best-effort.
async fn gc_orphaned_resources(k8s: &K8sClient, tasks: &Arc<dyn TaskDao>) {
    let task_ids = collect_labeled_task_ids(k8s).await;
    if task_ids.is_empty() {
        return;
    }

    // reason: orphan GC checks task existence across all owners; system ctx required.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let mut gc_count = 0u32;
    for task_id in task_ids {
        let task_exists = match tasks.find_by_id(&task_id, &sys_ctx).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                // A transient DB error must not be mistaken for a deleted task —
                // skip this task-id this cycle rather than risk deleting live
                // resources.
                warn!(task_id = %task_id, error = %e, "orphan-gc: task lookup failed, skipping");
                continue;
            }
        };

        if decide_orphan(task_exists) == OrphanDecision::Delete {
            info!(
                task_id = %task_id,
                "orphan-gc: deleting taskless sandbox resources"
            );
            delete_orphaned_task_resources(k8s, &task_id).await;
            gc_count += 1;
        }
    }

    if gc_count > 0 {
        info!(gc_count, "orphan-gc pass complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orphan_deleted_task_is_gc_immediately() {
        assert_eq!(decide_orphan(false), OrphanDecision::Delete);
    }

    #[test]
    fn orphan_open_task_is_never_gc() {
        assert_eq!(decide_orphan(true), OrphanDecision::Keep);
    }

    #[test]
    fn existing_closed_task_is_left_to_lifecycle() {
        assert_eq!(decide_orphan(true), OrphanDecision::Keep);
    }

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
