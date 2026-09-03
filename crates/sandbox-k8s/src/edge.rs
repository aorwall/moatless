//! Opening one port on an already-running sandbox.
//!
//! Provisioning publishes the ports a workspace *declares* — its preview
//! servers — as one atomic pass over Service, `IngressRoutes` and the preview
//! `NetworkPolicy`. A Script's port is not declared: it is asked for as the
//! script starts, against a sandbox that is already up. This module is that
//! second entry point, and it writes the same three objects one port at a time.
//!
//! Each write is individually idempotent, and none is compensated on failure:
//! an orphaned Service port or route reaches nothing and is collected with the
//! Deployment. What survives a failure is a port that is published but unused,
//! never a port that is reachable but unadmitted.

use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::Service;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::core::DynamicObject;
use kube::discovery::ApiResource;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_service_name;
use super::{ingress, networkpolicy};
use moatless_core::config::Config;

/// The Service port name for a dynamically published port.
///
/// A Service port name is a DNS-1123 label capped at **15 characters**, and the
/// declared-server path gets away with using the server name verbatim only
/// because those names are short and hand-written. A Script name is neither, so
/// its port carries a derived name instead — `scr-65535` is 9 characters at the
/// widest, and the port is already unique within the Service.
pub fn script_port_name(port: u16) -> String {
    format!("scr-{port}")
}

/// The key a script's `IngressRoute` is named from.
///
/// Prefixed so a Script and a preview server that happen to share a name get
/// distinct routes rather than one silently reusing the other's.
fn script_route_key(name: &str) -> String {
    format!("script-{name}")
}

/// A kube API handle for Traefik `IngressRoute` CRDs. Mirrors `ingress.rs`'s own
/// private helper; both exist because the CRD has no generated type.
fn ingress_api(k8s: &K8sClient) -> Api<DynamicObject> {
    let ar = ApiResource {
        group: "traefik.io".to_string(),
        version: "v1alpha1".to_string(),
        api_version: "traefik.io/v1alpha1".to_string(),
        kind: "IngressRoute".to_string(),
        plural: "ingressroutes".to_string(),
    };
    Api::namespaced_with(k8s.client.clone(), k8s.namespace(), &ar)
}

/// Every port the task's `IngressRoutes` currently point at.
///
/// The routes are the record of what has been published: they are separate
/// objects that outlive a missing Service, and nothing else persists a Script's
/// port. The network heal reads this so rebuilding the Service from the
/// declared plan does not silently drop a port a Script opened.
///
/// Best-effort, like the rest of the orphan-discovery reads — an unavailable or
/// uninstalled CRD yields an empty set rather than an error.
pub async fn published_task_ports(k8s: &K8sClient, task_id: &str) -> BTreeSet<u16> {
    let api = ingress_api(k8s);
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let list = match api.list(&lp).await {
        Ok(list) => list,
        Err(kube::Error::Api(ref e)) if e.code == 404 => return BTreeSet::new(),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to list IngressRoutes for published ports");
            return BTreeSet::new();
        }
    };

    let mut ports = BTreeSet::new();
    for item in list.items {
        let Some(routes) = item.data["spec"]["routes"].as_array() else {
            continue;
        };
        for route in routes {
            let Some(services) = route["services"].as_array() else {
                continue;
            };
            for service in services {
                if let Some(port) = service["port"].as_u64().and_then(|p| u16::try_from(p).ok()) {
                    ports.insert(port);
                }
            }
        }
    }
    ports
}

/// Add `port` to the task's Service, and return the Service's ownerReferences
/// so the route can be owned by the same Deployment without a second read.
///
/// The port is added with a **strategic** merge patch carrying only that one
/// entry. `ServiceSpec::ports` declares `port` as its patch merge key, so
/// Kubernetes merges the entry into the live list server-side rather than
/// replacing it. That is what makes two concurrent `scripts.run` calls safe:
/// building the whole array from a read and sending it wholesale would let the
/// later write drop whichever port won the earlier race, leaving that script's
/// `IngressRoute` pointing at a Service port that no longer exists.
async fn patch_service_port(
    k8s: &K8sClient,
    task_id: &str,
    port: u16,
) -> Result<Vec<OwnerReference>, kube::Error> {
    let services: Api<Service> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let service_name = generate_service_name(task_id);
    let service = services.get(&service_name).await?;

    let owners = service
        .metadata
        .owner_references
        .clone()
        .unwrap_or_default();
    let already_exposed = service
        .spec
        .as_ref()
        .and_then(|spec| spec.ports.as_ref())
        .is_some_and(|ports| ports.iter().any(|p| p.port == i32::from(port)));
    if already_exposed {
        debug!(task_id, port, "Service already exposes port");
        return Ok(owners);
    }

    let patch = script_service_port_patch(port);
    services
        .patch(&service_name, &PatchParams::default(), &Patch::Strategic(&patch))
        .await?;
    info!(task_id, port, service_name = %service_name, "Service port published");
    Ok(owners)
}

/// The one-entry `spec.ports` strategic merge patch that publishes `port`.
fn script_service_port_patch(port: u16) -> serde_json::Value {
    serde_json::json!({
        "spec": {
            "ports": [{
                "name": script_port_name(port),
                "port": i32::from(port),
                "targetPort": i32::from(port),
                "protocol": "TCP",
            }]
        }
    })
}

/// Make `port` reachable at the task's preview hostname, returning the URL.
///
/// Three writes, in the order that keeps the failure modes benign:
///
/// 1. The Service port. Fails the call — without it the later writes describe a
///    route to nothing, and the caller has not started anything yet.
/// 2. The `IngressRoute`. A failure here costs only the URL.
/// 3. The preview `NetworkPolicy` allowance. A failure here — typically a missing
///    RBAC grant — costs only reachability, the same degradation declared
///    preview servers already have.
///
/// Returns `Ok(None)` when the deployment has no proxy domain: the port is
/// still on the Service, there is simply no external name for it.
pub async fn publish_task_port(
    k8s: &K8sClient,
    task_id: &str,
    name: &str,
    port: u16,
    config: &Config,
) -> Result<Option<String>, kube::Error> {
    let owners = patch_service_port(k8s, task_id, port).await?;
    let owner = owners.first();

    let Some(domain) = config.proxy.domain.as_deref() else {
        debug!(
            task_id,
            port, "No PROXY_DOMAIN configured; port has no external URL"
        );
        return Ok(None);
    };

    let mut routes = std::collections::BTreeMap::new();
    routes.insert(script_route_key(name), port);
    if let Err(e) = ingress::create_task_ingress_routes(k8s, task_id, &routes, config, owner).await
    {
        warn!(task_id, port, error = %e, "Failed to create IngressRoute for script port");
        return Ok(None);
    }

    if let Err(e) = networkpolicy::admit_preview_port(k8s, task_id, port, config, owner).await {
        warn!(task_id, port, error = %e, "Failed to admit script port in the preview NetworkPolicy");
    }

    Ok(Some(format!(
        "{}://{}--{}.{}",
        config.proxy.protocol, task_id, port, domain
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Service port name is a DNS-1123 label capped at 15 characters — the
    /// reason a script port does not reuse the script's own name.
    #[test]
    fn script_port_name_fits_the_service_port_cap() {
        for port in [1u16, 3000, 8080, u16::MAX] {
            let name = script_port_name(port);
            assert!(name.len() <= 15, "{name} is longer than 15 chars");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{name} is not a DNS-1123 label"
            );
            assert!(!name.ends_with('-'));
        }
    }

    #[test]
    fn script_routes_do_not_collide_with_a_same_named_server() {
        assert_eq!(script_route_key("dev"), "script-dev");
        assert_ne!(script_route_key("dev"), "dev");
    }

    /// The patch carries only the port being published. Sending a rebuilt copy
    /// of the whole list is what let a concurrent publish be dropped, so a patch
    /// that names other ports at all is the regression.
    #[test]
    fn the_service_patch_names_only_the_port_it_publishes() {
        let patch = script_service_port_patch(3000);
        let ports = patch["spec"]["ports"].as_array().expect("ports array");

        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0]["port"], 3000);
        assert_eq!(ports[0]["targetPort"], 3000);
        assert_eq!(ports[0]["name"], "scr-3000");
        assert_eq!(ports[0]["protocol"], "TCP");
    }

}
