//! Kubernetes `ClusterIP` Service lifecycle operations.

use k8s_openapi::api::core::v1::Service;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{Api, ListParams, PostParams};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_service_name;
use moatless_types::sandbox::SANDBOX_PREVIEW_PROXY_PORT;

/// Check whether the task's sandbox Service exists. Returns `Ok(false)` on 404.
pub async fn sandbox_service_exists(k8s: &K8sClient, task_id: &str) -> Result<bool, kube::Error> {
    let services: Api<Service> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    match services.get(&generate_service_name(task_id)).await {
        Ok(_) => Ok(true),
        Err(kube::Error::Api(ref e)) if e.code == 404 => Ok(false),
        Err(e) => Err(e),
    }
}

/// Every task id that currently has a sandbox Service, in one namespace LIST.
///
/// The fleet observer pairs this with its Deployment LIST, which is why it is a
/// LIST and not a GET per task: the observer's budget is one call per resource
/// kind per tick, not one per sandbox.
pub async fn list_sandbox_service_task_ids(
    k8s: &K8sClient,
) -> Result<HashSet<String>, kube::Error> {
    let services: Api<Service> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("moatless.io/task-id");
    Ok(services
        .list(&lp)
        .await?
        .items
        .into_iter()
        .filter_map(|svc| svc.metadata.labels?.get("moatless.io/task-id").cloned())
        .collect())
}

/// The Service's whole `spec.ports` list: the sandbox's own HTTP and gRPC
/// ports, then one entry per declared server, then one per script port.
///
/// A declared server is forwarded to the port its sidecar binds. A script port
/// is forwarded to [`SANDBOX_PREVIEW_PROXY_PORT`], matching what
/// `edge::script_service_port_patch` writes when the port is first published —
/// the two are the only writers of a script port's `targetPort`, and a rebuild
/// here that used `port` would silently undo the proxy.
fn service_port_entries(
    server_ports: &BTreeMap<String, u16>,
    script_ports: &BTreeSet<u16>,
    grpc_port: u16,
) -> Vec<serde_json::Value> {
    let mut ports = vec![
        serde_json::json!({
            "name": "http",
            "port": 8001,
            "targetPort": 8001,
            "protocol": "TCP",
        }),
        serde_json::json!({
            "name": "grpc",
            "port": grpc_port,
            "targetPort": grpc_port,
            "protocol": "TCP",
        }),
    ];

    for (name, port) in server_ports {
        ports.push(serde_json::json!({
            "name": name,
            "port": port,
            "targetPort": port,
            "protocol": "TCP",
        }));
    }

    for port in script_ports {
        ports.push(serde_json::json!({
            "name": super::edge::script_port_name(*port),
            "port": port,
            "targetPort": SANDBOX_PREVIEW_PROXY_PORT,
            "protocol": "TCP",
        }));
    }

    ports
}

/// Create a `ClusterIP` Service for a workspace pod.
/// Idempotent — handles `AlreadyExists` (409) gracefully.
///
/// `server_ports` are the declared preview servers, each forwarded to the port
/// its sidecar binds. `script_ports` are ports a Script published against the
/// live sandbox; they are forwarded to [`SANDBOX_PREVIEW_PROXY_PORT`] instead,
/// matching what `edge::script_service_port_patch` writes when the port is first
/// published. They are passed apart rather than derived from the entry name so
/// the target is stated by the caller that knows which kind it holds.
///
/// `owner`, when present, is set as the Service's `ownerReference` (the task's
/// Deployment) so K8s garbage collection cascades the Service's deletion when
/// the Deployment is deleted.
pub async fn create_sandbox_service(
    k8s: &K8sClient,
    task_id: &str,
    server_ports: &BTreeMap<String, u16>,
    script_ports: &BTreeSet<u16>,
    grpc_port: u16,
    owner: Option<&OwnerReference>,
) -> Result<String, kube::Error> {
    let services: Api<Service> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let service_name = generate_service_name(task_id);

    let ports = service_port_entries(server_ports, script_ports, grpc_port);

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "moatless-sandbox".to_string(),
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
    labels.insert("moatless.io/sandbox-id".to_string(), task_id.to_string());

    let mut selector = BTreeMap::new();
    selector.insert("moatless.io/task-id".to_string(), task_id.to_string());

    let mut metadata = serde_json::json!({
        "name": service_name,
        "namespace": k8s.namespace(),
        "labels": labels,
    });
    if let Some(owner) = owner {
        metadata["ownerReferences"] = serde_json::json!([owner]);
    }

    let svc: Service = serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": metadata,
        "spec": {
            "type": "ClusterIP",
            "selector": selector,
            "ports": ports,
            "publishNotReadyAddresses": true,
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

/// Await observed absence of the task's Sandbox Service.
pub async fn wait_for_sandbox_service_deleted(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), moatless_sandbox_runtime::error::SandboxError> {
    let services: Api<Service> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let service_name = generate_service_name(task_id);
    super::wait::wait_absent(
        task_id,
        "Service",
        std::time::Duration::from_mins(1),
        || async {
            services
                .get_opt(&service_name)
                .await
                .map(|item| item.is_none())
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(
        servers: &[(&str, u16)],
        scripts: &[u16],
    ) -> Vec<serde_json::Value> {
        let servers = servers
            .iter()
            .map(|(name, port)| ((*name).to_string(), *port))
            .collect();
        let scripts = scripts.iter().copied().collect();
        service_port_entries(&servers, &scripts, 50052)
    }

    fn find<'a>(ports: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
        ports
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("no port named {name}"))
    }

    /// The heal recreates a Service from the ports it read back out of the
    /// surviving `IngressRoutes`. Rebuilding a script port with
    /// `targetPort == port` would restore the pre-proxy wiring, and a
    /// `127.0.0.1`-bound server is then unreachable rather than merely awkward.
    #[test]
    fn a_script_port_is_rebuilt_pointing_at_the_preview_proxy() {
        let ports = entries(&[], &[3100, 5173]);

        for port in [3100u16, 5173] {
            let entry = find(&ports, &super::super::edge::script_port_name(port));
            assert_eq!(entry["port"], port);
            assert_eq!(entry["targetPort"], SANDBOX_PREVIEW_PROXY_PORT);
        }
    }

    /// A declared server runs in its own sidecar and is reached directly.
    #[test]
    fn a_declared_server_still_targets_its_own_port() {
        let ports = entries(&[("web", 3000)], &[]);

        let entry = find(&ports, "web");
        assert_eq!(entry["port"], 3000);
        assert_eq!(entry["targetPort"], 3000);
    }

    #[test]
    fn the_sandbox_control_ports_are_always_present() {
        let ports = entries(&[], &[]);

        assert_eq!(find(&ports, "http")["targetPort"], 8001);
        assert_eq!(find(&ports, "grpc")["targetPort"], 50052);
    }

    /// Both kinds coexist on one Service after a heal.
    #[test]
    fn declared_and_script_ports_keep_their_own_targets_side_by_side() {
        let ports = entries(&[("web", 3000)], &[3100]);

        assert_eq!(find(&ports, "web")["targetPort"], 3000);
        assert_eq!(find(&ports, "scr-3100")["targetPort"], SANDBOX_PREVIEW_PROXY_PORT);
        assert_eq!(ports.len(), 4);
    }
}
