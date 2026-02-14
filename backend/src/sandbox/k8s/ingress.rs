//! Traefik IngressRoute CRD lifecycle operations.
//!
//! Creates and deletes `traefik.io/v1alpha1` IngressRoute custom resources
//! for each server port exposed by a workspace pod.

use kube::api::{Api, DynamicObject, ListParams, PostParams};
use kube::discovery::ApiResource;
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_service_name;
use crate::config::Config;

const TRAEFIK_GROUP: &str = "traefik.io";
const TRAEFIK_VERSION: &str = "v1alpha1";
const TRAEFIK_KIND: &str = "IngressRoute";
const TRAEFIK_PLURAL: &str = "ingressroutes";

/// Get a kube DynamicObject API for Traefik IngressRoute CRDs.
fn ingress_api(k8s: &K8sClient) -> Api<DynamicObject> {
    let ar = ApiResource {
        group: TRAEFIK_GROUP.to_string(),
        version: TRAEFIK_VERSION.to_string(),
        api_version: format!("{TRAEFIK_GROUP}/{TRAEFIK_VERSION}"),
        kind: TRAEFIK_KIND.to_string(),
        plural: TRAEFIK_PLURAL.to_string(),
    };
    Api::namespaced_with(k8s.client.clone(), k8s.namespace(), &ar)
}

/// Create Traefik IngressRoute CRDs for each server port of a task.
///
/// Each IngressRoute maps `{task_id}--{port}.{proxy_domain}` to the
/// workspace Service. Idempotent — handles 409 (AlreadyExists) gracefully.
pub async fn create_task_ingress_routes(
    k8s: &K8sClient,
    task_id: &str,
    server_ports: &BTreeMap<String, u16>,
    config: &Config,
) -> Result<(), kube::Error> {
    let proxy_domain = match config.proxy.domain.as_deref() {
        Some(d) => d,
        None => {
            debug!(task_id, "No PROXY_DOMAIN configured, skipping IngressRoute creation");
            return Ok(());
        }
    };

    let api = ingress_api(k8s);
    let service_name = generate_service_name(task_id);

    for (server_name, port) in server_ports {
        let route_name = format!("{service_name}-{server_name}");
        // K8s name max 63 chars
        let route_name = if route_name.len() > 63 {
            route_name[..63].to_string()
        } else {
            route_name
        };

        let hostname = format!("{task_id}--{port}.{proxy_domain}");

        let mut labels = BTreeMap::new();
        labels.insert(
            "app.kubernetes.io/managed-by".to_string(),
            "moatless".to_string(),
        );
        labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

        let spec = serde_json::json!({
            "entryPoints": [config.proxy.traefik_entry_point],
            "routes": [{
                "match": format!("Host(`{}`)", hostname),
                "kind": "Rule",
                "middlewares": [{
                    "name": config.proxy.traefik_auth_middleware_name,
                    "namespace": config.proxy.traefik_auth_middleware_namespace,
                }],
                "services": [{
                    "name": service_name,
                    "port": port,
                }],
            }],
        });

        let data = serde_json::json!({
            "apiVersion": format!("{}/{}", TRAEFIK_GROUP, TRAEFIK_VERSION),
            "kind": TRAEFIK_KIND,
            "metadata": {
                "name": route_name,
                "namespace": k8s.namespace(),
                "labels": labels,
            },
            "spec": spec,
        });

        let obj: DynamicObject =
            serde_json::from_value(data).map_err(kube::Error::SerdeError)?;

        match api.create(&PostParams::default(), &obj).await {
            Ok(_) => {
                info!(task_id, route_name = %route_name, hostname = %hostname, "IngressRoute created");
            }
            Err(kube::Error::Api(ref e)) if e.code == 409 => {
                debug!(task_id, route_name = %route_name, "IngressRoute already exists, reusing");
            }
            Err(e) => {
                warn!(task_id, route_name = %route_name, error = %e, "Failed to create IngressRoute");
                return Err(e);
            }
        }
    }

    Ok(())
}

/// Delete all Traefik IngressRoute CRDs for a task.
///
/// Uses label selector `moatless.io/task-id={task_id}` to find and delete
/// all routes. Handles 404 (NotFound) gracefully.
pub async fn delete_task_ingress_routes(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), kube::Error> {
    let api = ingress_api(k8s);
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let list = match api.list(&lp).await {
        Ok(list) => list,
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(task_id, "IngressRoute CRD not found (Traefik not installed?), skipping deletion");
            return Ok(());
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to list IngressRoutes for deletion");
            return Err(e);
        }
    };

    let count = list.items.len();
    for item in list.items {
        let name = match &item.metadata.name {
            Some(n) => n.clone(),
            None => continue,
        };

        match api.delete(&name, &Default::default()).await {
            Ok(_) => {
                debug!(task_id, route_name = %name, "IngressRoute deleted");
            }
            Err(kube::Error::Api(ref e)) if e.code == 404 => {
                debug!(task_id, route_name = %name, "IngressRoute already deleted");
            }
            Err(e) => {
                warn!(task_id, route_name = %name, error = %e, "Failed to delete IngressRoute");
            }
        }
    }

    if count > 0 {
        info!(task_id, count, "IngressRoutes deleted");
    }

    Ok(())
}
