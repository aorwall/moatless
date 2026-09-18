//! Opening one port on an already-running sandbox.
//!
//! Provisioning publishes a workspace's *declared* preview-server ports as one
//! atomic pass over Service, `IngressRoutes` and the preview `NetworkPolicy`. A
//! Script's port is not declared — asked for as the script starts, against a
//! sandbox already up — so this module writes the same three objects one port
//! at a time, each write idempotent and none compensated on failure: what
//! survives is a port published but unused, never one reachable but unadmitted.

use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::{Service, ServicePort};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::core::DynamicObject;
use kube::discovery::ApiResource;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_service_name;
use super::{ingress, networkpolicy};
use moatless_core::config::Config;
use moatless_types::sandbox::SANDBOX_PREVIEW_PROXY_PORT;

/// The Service port name for a dynamically published port.
///
/// Capped at 15 characters (DNS-1123), so a Script uses a derived name —
/// `scr-65535`, 9 characters at widest — instead of its own.
pub fn script_port_name(port: u16) -> String {
    format!("scr-{port}")
}

/// The key a script's `IngressRoute` is named from.
///
/// Prefixed so a Script and a preview server sharing a name get distinct routes
/// rather than one silently reusing the other's.
fn script_route_key(name: &str) -> String {
    format!("script-{name}")
}

/// A kube API handle for Traefik `IngressRoute` CRDs. Mirrors `ingress.rs`'s
/// own private helper; both exist because the CRD has no generated type.
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
/// Routes outlive a missing Service and are the only record of a Script's port,
/// so the network heal reads this to avoid dropping one when rebuilding from
/// the declared plan. Best-effort: an unavailable/uninstalled CRD yields an
/// empty set.
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

/// Add `port` to the task's Service; returns its ownerReferences so the route
/// can be owned by the same Deployment without a second read.
///
/// Patched with a strategic merge naming only that entry — `ServiceSpec::ports`
/// keys on `port`, so a wholesale rebuilt array would let a concurrent
/// `scripts.run` drop whichever port lost the race.
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
    let ports = service
        .spec
        .as_ref()
        .and_then(|spec| spec.ports.as_ref())
        .map_or(&[][..], Vec::as_slice);
    if !needs_proxy_patch(ports, port) {
        debug!(task_id, port, "Service already carries an entry for port");
        return Ok(owners);
    }

    let patch = script_service_port_patch(port);
    services
        .patch(
            &service_name,
            &PatchParams::default(),
            &Patch::Strategic(&patch),
        )
        .await?;
    info!(task_id, port, service_name = %service_name, "Service port published");
    Ok(owners)
}

/// Whether `port` still needs the patch that routes it to the preview proxy.
///
/// Scoped to entries `script_port_name` marks as this module's own, since the
/// merge keys on `port` alone and a name-blind guard would rewrite a declared
/// server's entry too; a pre-proxy entry (`targetPort == port`) is re-patched
/// or stays unreachable.
fn needs_proxy_patch(ports: &[ServicePort], port: u16) -> bool {
    let Some(existing) = ports.iter().find(|p| p.port == i32::from(port)) else {
        return true;
    };
    if existing.name.as_deref() != Some(script_port_name(port).as_str()) {
        return false;
    }
    !existing
        .target_port
        .as_ref()
        .is_some_and(|target| *target == IntOrString::Int(i32::from(SANDBOX_PREVIEW_PROXY_PORT)))
}

/// The one-entry `spec.ports` strategic merge patch that publishes `port`.
///
/// `targetPort` is [`SANDBOX_PREVIEW_PROXY_PORT`], not `port`: the sandbox's
/// preview proxy answers on the script's behalf, so its server sees a loopback
/// `Host` and may bind `127.0.0.1`.
fn script_service_port_patch(port: u16) -> serde_json::Value {
    serde_json::json!({
        "spec": {
            "ports": [{
                "name": script_port_name(port),
                "port": i32::from(port),
                "targetPort": i32::from(SANDBOX_PREVIEW_PROXY_PORT),
                "protocol": "TCP",
            }]
        }
    })
}

/// Make `port` reachable at the task's preview hostname, returning the URL.
///
/// Three writes in order of failure cost: Service port failure fails the call;
/// `IngressRoute` failure costs only the URL; `NetworkPolicy` failure costs
/// only reachability. `Ok(None)` means no proxy domain is configured — the port
/// is still on the Service.
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

    // The proxy port, not `port`: the Service sends this route's traffic to
    // `SANDBOX_PREVIEW_PROXY_PORT` (see `script_service_port_patch`), so that is
    // the port the policy has to let Traefik reach. Idempotent, so the first
    // published port pays for every later one.
    if let Err(e) =
        networkpolicy::admit_preview_port(k8s, task_id, SANDBOX_PREVIEW_PROXY_PORT, config, owner)
            .await
    {
        warn!(task_id, port, error = %e, "Failed to admit the preview proxy port in the preview NetworkPolicy");
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
    /// of the whole list is what let a concurrent publish be dropped, so a
    /// patch that names other ports at all is the regression.
    #[test]
    fn the_service_patch_names_only_the_port_it_publishes() {
        let patch = script_service_port_patch(3000);
        let ports = patch["spec"]["ports"].as_array().expect("ports array");

        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0]["port"], 3000);
        assert_eq!(ports[0]["name"], "scr-3000");
        assert_eq!(ports[0]["protocol"], "TCP");
    }

    /// The Service port is what the route names; the target is the sandbox's
    /// preview proxy, which reads the real port back out of the `Host` —
    /// sending traffic straight at `port` forced a script's server to bind
    /// `0.0.0.0` and see an external `Host`.
    #[test]
    fn the_patch_targets_the_preview_proxy_rather_than_the_published_port() {
        for port in [3000u16, 8080, 5173] {
            let patch = script_service_port_patch(port);
            let entry = &patch["spec"]["ports"][0];
            assert_eq!(entry["port"], i32::from(port));
            assert_eq!(entry["targetPort"], i32::from(SANDBOX_PREVIEW_PROXY_PORT));
        }
    }

    fn entry(name: &str, port: u16, target: u16) -> ServicePort {
        ServicePort {
            name: Some(name.to_string()),
            port: i32::from(port),
            target_port: Some(IntOrString::Int(i32::from(target))),
            ..Default::default()
        }
    }

    #[test]
    fn a_port_the_service_does_not_carry_needs_the_patch() {
        assert!(needs_proxy_patch(&[], 3000));
        assert!(needs_proxy_patch(&[entry("scr-5173", 5173, 8003)], 3000));
    }

    /// The publish is idempotent once the entry points at the proxy.
    #[test]
    fn a_script_port_already_routed_to_the_proxy_needs_nothing() {
        let ports = [entry("scr-3000", 3000, SANDBOX_PREVIEW_PROXY_PORT)];

        assert!(!needs_proxy_patch(&ports, 3000));
    }

    /// A port published before the proxy existed carries `targetPort == port`.
    /// Leaving it keeps a `127.0.0.1`-bound server unreachable for the life of
    /// the Service.
    #[test]
    fn a_script_port_still_pointing_at_the_server_is_re_patched() {
        let ports = [entry("scr-3000", 3000, 3000)];

        assert!(needs_proxy_patch(&ports, 3000));
    }

    /// `web: 3000` declared plus a Script started on 3000 is an ordinary
    /// collision. Patching here would rename the declared server's entry and
    /// retarget it at the proxy, leaving the two writers in disagreement.
    #[test]
    fn a_declared_servers_entry_is_never_rewritten_by_a_script_naming_its_port() {
        let ports = [entry("web", 3000, 3000)];

        assert!(!needs_proxy_patch(&ports, 3000));
    }

    /// An entry with no name at all is not one this module wrote either.
    #[test]
    fn an_unnamed_entry_is_left_alone() {
        let ports = [ServicePort {
            name: None,
            port: 3000,
            target_port: Some(IntOrString::Int(3000)),
            ..Default::default()
        }];

        assert!(!needs_proxy_patch(&ports, 3000));
    }
}
