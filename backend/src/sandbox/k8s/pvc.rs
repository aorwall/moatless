//! PVC (PersistentVolumeClaim) lifecycle operations.

use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::api::{Api, ListParams, PostParams};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_pvc_name;
use crate::config::Config;

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

/// Create a PVC for workspace persistence
pub async fn create_task_pvc(
    k8s: &K8sClient,
    config: &Config,
    task_id: &str,
) -> Result<String, kube::Error> {
    if !config.sandbox.storage_enabled {
        debug!(task_id, "Sandbox storage disabled, skipping PVC creation");
        return Ok(String::new());
    }

    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let pvc_name = generate_pvc_name(task_id);

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-workspace".to_string(),
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
                    "storage": config.sandbox.storage_size,
                }
            },
            "storageClassName": config.sandbox.storage_class,
        }
    }))
    .map_err(kube::Error::SerdeError)?;

    pvcs.create(&PostParams::default(), &pvc).await?;
    info!(task_id, pvc_name = %pvc_name, "Task PVC created");

    Ok(pvc_name)
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
