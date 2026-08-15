//! PVC (PersistentVolumeClaim) lifecycle operations.

use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::api::{Api, ListParams, PostParams};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_pvc_name;
use moatless_core::config::Config;
use moatless_sandbox_control::error::SandboxError;

/// Find existing PVC for a task by label selector
pub async fn find_task_pvc(k8s: &K8sClient, task_id: &str) -> Option<String> {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    match pvcs.list(&lp).await {
        Ok(list) => {
            if let Some(pvc) = list.items.first() {
                let name = pvc.metadata.name.clone().unwrap_or_default();
                debug!(task_id, pvc_name = %name, "Found existing PVC");
                Some(name)
            } else {
                None
            }
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to find task PVC");
            None
        }
    }
}

/// Resolve the PVC storage size: a per-repo override wins over the global default.
fn resolve_storage_size<'a>(override_size: Option<&'a str>, default: &'a str) -> &'a str {
    override_size
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
}

/// Create a PVC for workspace persistence.
///
/// `storage_size_override` is an optional per-repo size (e.g. from
/// `RepoConfig.resources.storage`); when `None` the global
/// `config.sandbox.storage_size` default applies.
pub async fn create_task_pvc(
    k8s: &K8sClient,
    config: &Config,
    task_id: &str,
    storage_size_override: Option<&str>,
) -> Result<String, kube::Error> {
    if !config.sandbox.storage_enabled {
        debug!(task_id, "Sandbox storage disabled, skipping PVC creation");
        return Ok(String::new());
    }

    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let pvc_name = generate_pvc_name(task_id);
    let storage_size = resolve_storage_size(storage_size_override, &config.sandbox.storage_size);

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-sandbox".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "storage".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

    let pvc: PersistentVolumeClaim = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": pvc_name,
            "namespace": k8s.namespace(),
            "labels": labels,
        },
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "resources": {
                "requests": {
                    "storage": storage_size,
                }
            },
            "storageClassName": config.sandbox.storage_class,
        }
    }))
    .map_err(kube::Error::SerdeError)?;

    match pvcs.create(&PostParams::default(), &pvc).await {
        Ok(_) => {
            info!(task_id, pvc_name = %pvc_name, storage_size, "Task PVC created");
            Ok(pvc_name)
        }
        // A concurrent provision (or a leftover from a prior run) already created
        // the PVC. Reuse the existing one instead of failing — degrading to
        // ephemeral storage here would silently drop workspace persistence.
        Err(kube::Error::Api(ref e)) if e.code == 409 => {
            if let Some(existing) = find_task_pvc(k8s, task_id).await {
                info!(task_id, pvc_name = %existing, "PVC already exists (409), reusing existing");
                Ok(existing)
            } else {
                info!(task_id, pvc_name = %pvc_name, "PVC already exists (409), reusing by name");
                Ok(pvc_name)
            }
        }
        Err(e) => Err(e),
    }
}

/// Whether the task's PVC has reached the `Bound` phase.
///
/// The attach-failure fallback uses this to decide the orphaned claim's fate: a
/// `Bound` PVC (the ordinary Multi-Attach case — the volume exists, it is just
/// still attached to a terminating pod's node) is left in place so its data
/// survives for the next clean start, while an unbound (`Pending`) claim has no
/// backing volume and is a safe orphan to delete. On a read error the claim is
/// treated as bound so uncertainty never destroys data.
pub async fn task_pvc_is_bound(k8s: &K8sClient, task_id: &str) -> bool {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    match pvcs.list(&lp).await {
        Ok(list) => list.items.iter().any(|pvc| {
            pvc.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Bound")
        }),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to read task PVC phase; treating as bound");
            true
        }
    }
}

/// Delete all PVCs for a task
pub async fn delete_task_pvc(k8s: &K8sClient, task_id: &str) -> Result<(), kube::Error> {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let list = pvcs.list(&lp).await?;
    for pvc in list.items {
        if let Some(name) = &pvc.metadata.name {
            pvcs.delete(name, &Default::default()).await?;
            info!(task_id, pvc_name = %name, "Task PVC deleted");
        }
    }

    Ok(())
}

/// Await observed absence of every PVC for a task.
///
/// Strict removal requires the PVC be *gone*, not merely accepted for deletion:
/// a delete request returns immediately but reclaim can lag, and a lingering PVC
/// bills indefinitely. A transient list error is retried within the window; a
/// timeout surfaces so the caller withholds `Removed` and retries.
pub async fn wait_for_pvc_deleted(k8s: &K8sClient, task_id: &str) -> Result<(), SandboxError> {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));
    super::wait::wait_absent(
        task_id,
        "PVC",
        std::time::Duration::from_mins(1),
        || async { pvcs.list(&lp).await.map(|list| list.items.is_empty()) },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::resolve_storage_size;

    #[test]
    fn override_wins_over_default() {
        assert_eq!(resolve_storage_size(Some("8Gi"), "3Gi"), "8Gi");
    }

    #[test]
    fn none_falls_back_to_default() {
        assert_eq!(resolve_storage_size(None, "3Gi"), "3Gi");
    }

    #[test]
    fn blank_override_falls_back_to_default() {
        assert_eq!(resolve_storage_size(Some("   "), "3Gi"), "3Gi");
        assert_eq!(resolve_storage_size(Some(""), "3Gi"), "3Gi");
    }
}
