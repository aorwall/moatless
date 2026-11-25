//! Traefik `IngressRoute` CRD lifecycle operations.
//!
//! Creates and deletes `traefik.io/v1alpha1` `IngressRoute` custom resources
//! for each preview port a Script publishes.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{Api, DynamicObject, ListParams, PostParams};
use kube::discovery::ApiResource;
use std::collections::BTreeMap;
use tracing::{debug, error, info, warn};

use super::client::K8sClient;
use super::naming::{generate_ingress_route_name, generate_service_name};
use moatless_core::config::Config;

const TRAEFIK_GROUP: &str = "traefik.io";
const TRAEFIK_VERSION: &str = "v1alpha1";
const TRAEFIK_KIND: &str = "IngressRoute";
const TRAEFIK_PLURAL: &str = "ingressroutes";

/// Get a kube `DynamicObject` API for Traefik `IngressRoute` CRDs.
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

/// Maps `{task_id}--{port}.{proxy_domain}` to the workspace Service for each
/// `route key → port` entry; idempotent — handles 409 (`AlreadyExists`) gracefully.
///
/// `owner`, when present, is set as each route's `ownerReference` (the task's
/// Deployment) so K8s garbage collection cascades their deletion.
pub async fn create_task_ingress_routes(
    k8s: &K8sClient,
    task_id: &str,
    routes: &BTreeMap<String, u16>,
    config: &Config,
    owner: Option<&OwnerReference>,
) -> Result<(), kube::Error> {
    let proxy_domain = if let Some(d) = config.proxy.domain.as_deref() {
        d
    } else {
        debug!(
            task_id,
            "No PROXY_DOMAIN configured, skipping IngressRoute creation"
        );
        return Ok(());
    };

    let api = ingress_api(k8s);
    let service_name = generate_service_name(task_id);

    for (route_key, port) in routes {
        // The route key carries a user-supplied Script name, unsanitized, so it
        // may contain multi-byte chars — build the route name via the shared
        // helper, which sanitizes, truncates char-safely to 63, and trims any
        // trailing '-'.
        let route_name = generate_ingress_route_name(&service_name, route_key);

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

        let mut metadata = serde_json::json!({
            "name": route_name,
            "namespace": k8s.namespace(),
            "labels": labels,
        });
        if let Some(owner) = owner {
            metadata["ownerReferences"] = serde_json::json!([owner]);
        }

        let data = serde_json::json!({
            "apiVersion": format!("{}/{}", TRAEFIK_GROUP, TRAEFIK_VERSION),
            "kind": TRAEFIK_KIND,
            "metadata": metadata,
            "spec": spec,
        });

        let obj: DynamicObject = serde_json::from_value(data).map_err(kube::Error::SerdeError)?;

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

/// Log an error at startup when `PROXY_DOMAIN` is set on a cluster without the
/// Traefik `IngressRoute` CRD: every Script preview would otherwise fail
/// silently, one `warn` per publish.
pub async fn warn_if_previews_unroutable(k8s: &K8sClient, config: &Config) {
    if config.proxy.domain.is_none() {
        return;
    }
    match ingress_api(k8s).list(&ListParams::default().limit(1)).await {
        Ok(_) => {}
        Err(kube::Error::Api(ref e)) if e.code == 404 => error!(
            "PROXY_DOMAIN is set but the cluster has no Traefik IngressRoute CRD, so Script \
             previews get no URL. Install Traefik, or unset PROXY_DOMAIN \
             (chart value `proxy.previews.enabled: false`)"
        ),
        Err(e) => warn!(error = %e, "Could not check for the Traefik IngressRoute CRD"),
    }
}

/// Collect the `moatless.io/task-id` label of every managed `IngressRoute`.
///
/// Used by the reaper's orphan-GC pass; returns an empty vec (not an error)
/// when the Traefik CRD isn't installed (404) or the list fails — orphan GC is
/// best-effort.
pub async fn list_ingress_route_task_ids(k8s: &K8sClient) -> Vec<String> {
    let api = ingress_api(k8s);
    let lp = ListParams::default().labels("moatless.io/task-id");

    let list = match api.list(&lp).await {
        Ok(list) => list,
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(
                "IngressRoute CRD not found (Traefik not installed?), skipping orphan discovery"
            );
            return Vec::new();
        }
        Err(e) => {
            warn!(error = %e, "Failed to list IngressRoutes for orphan discovery");
            return Vec::new();
        }
    };

    list.items
        .into_iter()
        .filter_map(|item| {
            item.metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id").cloned())
        })
        .collect()
}

/// Delete all Traefik `IngressRoute` CRDs for a task.
///
/// Uses label selector `moatless.io/task-id={task_id}` to find and delete all
/// routes. Handles 404 (`NotFound`) gracefully.
pub async fn delete_task_ingress_routes(k8s: &K8sClient, task_id: &str) -> Result<(), kube::Error> {
    let api = ingress_api(k8s);
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let list = match api.list(&lp).await {
        Ok(list) => list,
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(
                task_id,
                "IngressRoute CRD not found (Traefik not installed?), skipping deletion"
            );
            return Ok(());
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to list IngressRoutes for deletion");
            return Err(e);
        }
    };

    let count = list.items.len();
    let mut first_error = None;
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
                first_error.get_or_insert(e);
            }
        }
    }

    if count > 0 {
        info!(task_id, count, "IngressRoutes deleted");
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Await observed absence of every task-owned `IngressRoute`.
pub async fn wait_for_task_ingress_routes_deleted(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), moatless_sandbox_runtime::error::SandboxError> {
    let api = ingress_api(k8s);
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));
    super::wait::wait_absent(
        task_id,
        "IngressRoute",
        std::time::Duration::from_mins(1),
        || async {
            match api.list(&lp).await {
                Ok(list) => Ok(list.items.is_empty()),
                Err(kube::Error::Api(ref error)) if error.code == 404 => Ok(true),
                Err(error) => Err(error),
            }
        },
    )
    .await
}
