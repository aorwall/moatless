//! Opening one port on an already-running sandbox.
//!
//! A Script's port is asked for as the script starts, against a sandbox already
//! up, so this module writes the Service port, the `IngressRoute` and the
//! preview `NetworkPolicy` one port at a time, each write idempotent and none
//! compensated on failure: what survives is a port published but unused, never
//! one reachable but unadmitted.

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

/// The key a script port's `IngressRoute` is named from.
///
/// The port its `Host` is built from, deliberately rather than the Script's
/// name: the 63-char cut after the Service prefix kept 7 chars of a name, so
/// `preview-5174` and `preview-5175` collided and the second 409 went unrouted.
fn script_route_key(port: u16) -> String {
    script_port_name(port)
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
/// so the network heal rebuilds the Service from this. Best-effort: an unavailable/uninstalled CRD yields an
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
/// merge keys on `port` alone and a name-blind guard would rewrite a reserved
/// entry such as `http` too; a pre-proxy entry (`targetPort == port`) is
/// re-patched or stays unreachable.
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
/// Three writes, Service port, `IngressRoute`, `NetworkPolicy`; any failure
/// fails the call, since a URL returned without its route or policy answers
/// 404 or times out. `Ok(None)` means no proxy domain is configured — the port
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
    routes.insert(script_route_key(port), port);
    ingress::create_task_ingress_routes(k8s, task_id, &routes, config, owner)
        .await
        .inspect_err(|e| {
            warn!(task_id, script = name, port, error = %e, "Failed to create IngressRoute for script port");
        })?;

    // The proxy port, not `port`: the Service sends this route's traffic to
    // `SANDBOX_PREVIEW_PROXY_PORT` (see `script_service_port_patch`), so that is
    // the port the policy has to let Traefik reach. Idempotent, so the first
    // published port pays for every later one.
    networkpolicy::admit_preview_port(k8s, task_id, SANDBOX_PREVIEW_PROXY_PORT, config, owner)
        .await
        .inspect_err(|e| {
            warn!(task_id, port, error = %e, "Failed to admit the preview proxy port in the preview NetworkPolicy");
        })?;

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

    /// Patching a reserved entry would retarget the sandbox's own control
    /// port at the preview proxy.
    #[test]
    fn an_entry_this_module_did_not_write_is_never_rewritten() {
        let ports = [entry("http", 8001, 8001)];

        assert!(!needs_proxy_patch(&ports, 8001));
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

    /// Two Scripts whose names share their first seven characters, on a task
    /// id as long as a real one, still get a route each.
    #[test]
    fn script_routes_on_different_ports_get_different_names() {
        let service = generate_service_name("af3b4723-9378-4ecc-8c09-d9a4b9acd970");
        let names: BTreeSet<String> = [5174u16, 5175, 5176]
            .into_iter()
            .map(|port| {
                super::super::naming::generate_ingress_route_name(&service, &script_route_key(port))
            })
            .collect();

        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.iter().all(|name| name.len() <= 63), "{names:?}");
    }

    /// A Kubernetes API that serves the task's Service and refuses every
    /// `IngressRoute` write, recording each request line it answers.
    async fn api_refusing_routes(
        task_id: &str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let service = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": { "name": generate_service_name(task_id), "namespace": "test" },
            "spec": { "ports": [{
                "name": "scr-5174",
                "port": 5174,
                "targetPort": i32::from(SANDBOX_PREVIEW_PROXY_PORT),
            }] },
        })
        .to_string();
        let recorded = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 64 * 1024];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = request.lines().next().unwrap_or_default().to_string();
                recorded.lock().expect("lock").push(line.clone());
                let (status, body) = if line.starts_with("GET /api/v1/namespaces/test/services/") {
                    ("200 OK", service.clone())
                } else {
                    let refusal = serde_json::json!({
                        "kind": "Status", "apiVersion": "v1", "status": "Failure",
                        "message": "forbidden", "reason": "Forbidden", "code": 403,
                    });
                    ("403 Forbidden", refusal.to_string())
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (url, seen)
    }

    /// A route that was not written must fail the publish: returning the URL
    /// anyway is what let `scripts run` report a preview that answered 404.
    #[tokio::test]
    async fn a_failed_route_write_is_an_error_not_a_url() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let task_id = "af3b4723-9378-4ecc-8c09-d9a4b9acd970";
        let (url, seen) = api_refusing_routes(task_id).await;
        let k8s = K8sClient {
            client: kube::Client::try_from(kube::Config::new(url.parse().expect("url")))
                .expect("client"),
            namespace: std::sync::Arc::new("test".to_string()),
        };
        let mut config = Config::test_minimal();
        config.proxy.domain = Some("preview.example".to_string());

        let result = publish_task_port(&k8s, task_id, "preview-5174", 5174, &config).await;

        assert!(result.is_err(), "{result:?}");
        let seen = seen.lock().expect("lock");
        assert!(
            seen.iter().any(|line| line
                .starts_with("POST /apis/traefik.io/v1alpha1/namespaces/test/ingressroutes")),
            "{seen:?}"
        );
    }
}
