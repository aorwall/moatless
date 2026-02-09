//! Kubernetes ClusterIP Service lifecycle operations.

use k8s_openapi::api::core::v1::Service;
use kube::api::{Api, PostParams};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_service_name;

/// Create a ClusterIP Service for a workspace pod.
/// Idempotent — handles AlreadyExists (409) gracefully.
pub async fn create_sandbox_service(
    k8s: &K8sClient,
    task_id: &str,
    server_ports: &BTreeMap<String, u16>,
) -> Result<String, kube::Error> {
    let services: Api<Service> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let service_name = generate_service_name(task_id);

    // Always expose sandbox port; add server ports from config
    let mut ports = vec![serde_json::json!({
        "name": "sandbox",
        "port": 8001,
        "targetPort": 8001,
        "protocol": "TCP",
    })];

    for (name, port) in server_ports {
        ports.push(serde_json::json!({
            "name": name,
            "port": port,
            "targetPort": port,
            "protocol": "TCP",
        }));
    }

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-workspace".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "sandbox-service".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

    let mut selector = BTreeMap::new();
    selector.insert("moatless.io/task-id".to_string(), task_id.to_string());

    let svc: Service = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {
            "name": service_name,
            "namespace": k8s.namespace(),
            "labels": labels,
        },
        "spec": {
            "type": "ClusterIP",
            "selector": selector,
            "ports": ports,
        }
    }))
    .map_err(kube::Error::SerdeError)?;

    match services.create(&PostParams::default(), &svc).await {
        Ok(_) => {
            info!(task_id, service_name = %service_name, "Sandbox Service created");
        }
        Err(kube::Error::Api(ref e)) if e.code == 409 => {
            debug!(task_id, service_name = %service_name, "Sandbox Service already exists, reusing");
        }
        Err(e) => return Err(e),
    }

    Ok(service_name)
}

/// Delete the Sandbox Service for a task. Handles 404 gracefully.
pub async fn delete_sandbox_service(k8s: &K8sClient, task_id: &str) -> Result<(), kube::Error> {
    let services: Api<Service> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let service_name = generate_service_name(task_id);

    match services.delete(&service_name, &Default::default()).await {
        Ok(_) => {
            info!(task_id, service_name = %service_name, "Sandbox Service deleted");
            Ok(())
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(task_id, "Sandbox Service not found, already deleted");
            Ok(())
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to delete Sandbox Service");
            Err(e)
        }
    }
}
