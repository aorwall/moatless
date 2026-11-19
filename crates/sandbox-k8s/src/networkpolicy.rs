//! Per-sandbox Traefik preview `NetworkPolicy` lifecycle.
//!
//! A sandbox whose Script publishes a preview port gets a `NetworkPolicy`
//! admitting the cluster's Traefik ingress pods to it — the ingress
//! counterpart to `IngressRoutes`: a route makes a port addressable, this
//! policy makes the pod reachable on it.
//!
//! Once enforced, an un-listed port times out silently (auth succeeds at the
//! edge, so it reads as an app fault, not a network one).

use k8s_openapi::api::networking::v1::NetworkPolicy;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, PostParams};
use std::collections::{BTreeMap, BTreeSet};
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::naming::generate_preview_network_policy_name;
use moatless_core::config::Config;

/// Standard Traefik chart pod label. The policy admits pods carrying it in the
/// configured ingress namespaces.
const TRAEFIK_POD_LABEL: &str = "app.kubernetes.io/name";
const TRAEFIK_POD_LABEL_VALUE: &str = "traefik";

/// A 403 on the `NetworkPolicy` API means the backend `ServiceAccount` lacks
/// the `networking.k8s.io/networkpolicies` grant (older chart release).
///
/// Creation is best-effort: this degrades preview policies to "feature off"
/// without wedging, and nothing leaks — any policy made while the grant
/// existed still carries the Deployment's ownerReference for K8s GC.
fn is_forbidden(error: &kube::Error) -> bool {
    matches!(error, kube::Error::Api(response) if response.code == 403)
}

/// Shared operator-facing hint for the 403 warnings.
const MISSING_RBAC_HINT: &str = "backend ServiceAccount lacks networking.k8s.io/networkpolicies — upgrade the chart \
     (pod-manager Role) to enable preview NetworkPolicies";

/// Pure — no cluster I/O — so the selector/port/peer shape is unit-testable.
///
/// Each Traefik namespace becomes its own `from` peer, and inside a peer the
/// `namespaceSelector` and `podSelector` must both match — "Traefik pods in
/// this namespace".
fn build_preview_network_policy(
    namespace: &str,
    task_id: &str,
    ports: &BTreeSet<u16>,
    traefik_namespaces: &[String],
    owner: Option<&OwnerReference>,
) -> Result<NetworkPolicy, kube::Error> {
    let name = generate_preview_network_policy_name(task_id);

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "moatless".to_string(),
    );
    labels.insert("moatless.io/task-id".to_string(), task_id.to_string());

    let from: Vec<serde_json::Value> = traefik_namespaces
        .iter()
        .map(|ns| {
            serde_json::json!({
                "namespaceSelector": {
                    "matchLabels": { "kubernetes.io/metadata.name": ns }
                },
                "podSelector": {
                    "matchLabels": { TRAEFIK_POD_LABEL: TRAEFIK_POD_LABEL_VALUE }
                }
            })
        })
        .collect();

    let port_rules: Vec<serde_json::Value> = ports
        .iter()
        .map(|port| serde_json::json!({ "protocol": "TCP", "port": port }))
        .collect();

    let mut metadata = serde_json::json!({
        "name": name,
        "namespace": namespace,
        "labels": labels,
    });
    if let Some(owner) = owner {
        metadata["ownerReferences"] = serde_json::json!([owner]);
    }

    serde_json::from_value(serde_json::json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "NetworkPolicy",
        "metadata": metadata,
        "spec": {
            "podSelector": {
                "matchLabels": { "moatless.io/task-id": task_id }
            },
            "policyTypes": ["Ingress"],
            "ingress": [{
                "from": from,
                "ports": port_rules,
            }],
        }
    }))
    .map_err(kube::Error::SerdeError)
}

/// How many times [`admit_preview_port`] re-reads and retries when a concurrent
/// writer changes the policy under it; more than a few concurrent `scripts.run`
/// on one task is not a real workload, so a small bound converges without
/// spinning.
const MAX_WIDEN_ATTEMPTS: usize = 5;

/// Widen the per-task preview `NetworkPolicy` to admit one more port, creating
/// it when absent.
///
/// The ingress port list has no per-port merge key, so this read-modify-writes
/// it under `resourceVersion`: a concurrent widen 409s and retries up to
/// [`MAX_WIDEN_ATTEMPTS`]. A no-op if the port is already admitted.
pub async fn admit_preview_port(
    k8s: &K8sClient,
    task_id: &str,
    port: u16,
    config: &Config,
    owner: Option<&OwnerReference>,
) -> Result<(), kube::Error> {
    let api: Api<NetworkPolicy> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = generate_preview_network_policy_name(task_id);

    for attempt in 0..MAX_WIDEN_ATTEMPTS {
        let existing = match api.get(&name).await {
            Ok(policy) => Some(policy),
            Err(kube::Error::Api(ref e)) if e.code == 404 => None,
            Err(ref e) if is_forbidden(e) => {
                warn!(
                    task_id,
                    policy_name = %name,
                    "Skipping preview NetworkPolicy widen: {MISSING_RBAC_HINT}"
                );
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        let mut ports = existing.as_ref().map_or_else(BTreeSet::new, admitted_ports);
        if !ports.insert(port) {
            debug!(task_id, port, "Preview NetworkPolicy already admits port");
            return Ok(());
        }

        let mut policy = build_preview_network_policy(
            k8s.namespace(),
            task_id,
            &ports,
            &config.proxy.traefik_ingress_namespaces,
            owner,
        )?;

        let result = match &existing {
            // A guarded replace: carrying the version we read makes a concurrent
            // widen a 409 we retry, not a lost port.
            Some(current) => {
                policy.metadata.resource_version = current.metadata.resource_version.clone();
                api.replace(&name, &PostParams::default(), &policy)
                    .await
                    .map(|_| ())
            }
            None => api
                .create(&PostParams::default(), &policy)
                .await
                .map(|_| ()),
        };

        match result {
            Ok(()) => {
                info!(task_id, policy_name = %name, port, "Preview NetworkPolicy admits port");
                return Ok(());
            }
            // Lost the race: another writer created or widened the policy between
            // our read and write. Re-read their version and add on top.
            Err(kube::Error::Api(ref e)) if e.code == 409 => {
                debug!(
                    task_id,
                    port, attempt, "Preview NetworkPolicy changed under us; retrying widen"
                );
            }
            Err(ref e) if is_forbidden(e) => {
                warn!(
                    task_id,
                    policy_name = %name,
                    "Skipping preview NetworkPolicy widen: {MISSING_RBAC_HINT}"
                );
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }

    // Contention we could not win in a bounded number of tries. Like every other
    // failure on this path it costs only reachability, so it is warned, not
    // fatal — the caller has already published the Service port and route.
    warn!(
        task_id,
        policy_name = %name,
        port,
        "Preview NetworkPolicy widen lost {MAX_WIDEN_ATTEMPTS} races; port may not be admitted"
    );
    Ok(())
}

/// The TCP ports a preview policy currently admits.
fn admitted_ports(policy: &NetworkPolicy) -> BTreeSet<u16> {
    policy.spec.as_ref().map_or_default(|spec| {
        spec.ingress
            .iter()
            .flatten()
            .filter_map(|rule| rule.ports.as_ref())
            .flatten()
            .filter_map(|p| p.port.as_ref())
            .filter_map(|port| match port {
                IntOrString::Int(value) => u16::try_from(*value).ok(),
                IntOrString::String(_) => None,
            })
            .collect()
    })
}

/// Delete the per-task preview `NetworkPolicy`. Handles 404 and a missing
/// `NetworkPolicy` grant (403) gracefully — see [`is_forbidden`].
pub async fn delete_task_preview_network_policy(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), kube::Error> {
    let api: Api<NetworkPolicy> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = generate_preview_network_policy_name(task_id);

    match api.delete(&name, &Default::default()).await {
        Ok(_) => {
            info!(task_id, policy_name = %name, "Preview NetworkPolicy deleted");
            Ok(())
        }
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(task_id, policy_name = %name, "Preview NetworkPolicy not found, already deleted");
            Ok(())
        }
        Err(ref e) if is_forbidden(e) => {
            warn!(
                task_id,
                policy_name = %name,
                "Skipping preview NetworkPolicy deletion: {MISSING_RBAC_HINT}"
            );
            Ok(())
        }
        Err(e) => {
            warn!(task_id, policy_name = %name, error = %e, "Failed to delete preview NetworkPolicy");
            Err(e)
        }
    }
}

/// Await observed absence of the task's preview `NetworkPolicy`.
///
/// Without the `NetworkPolicy` grant (403, see [`is_forbidden`]) absence is
/// unobservable, so the wait resolves immediately rather than burning its full
/// timeout and reporting the resource as lingering.
pub async fn wait_for_task_preview_network_policy_deleted(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), moatless_sandbox_runtime::error::SandboxError> {
    let api: Api<NetworkPolicy> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = generate_preview_network_policy_name(task_id);
    super::wait::wait_absent(
        task_id,
        "NetworkPolicy",
        std::time::Duration::from_mins(1),
        || async {
            match api.get_opt(&name).await {
                Ok(item) => Ok(item.is_none()),
                Err(ref error) if is_forbidden(error) => Ok(true),
                Err(error) => Err(error),
            }
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> OwnerReference {
        OwnerReference {
            api_version: "apps/v1".to_string(),
            kind: "Deployment".to_string(),
            name: "moatless-ws-task-1".to_string(),
            uid: "uid-1".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn policy_selects_task_pod_and_admits_configured_ports() {
        let ports = BTreeSet::from([5733u16, 3000u16]);
        let namespaces = vec!["moatless".to_string(), "platform-ingress".to_string()];
        let policy =
            build_preview_network_policy("moatless", "task-1", &ports, &namespaces, Some(&owner()))
                .expect("build should succeed");

        let spec = policy.spec.expect("policy has a spec");
        // Scoped to this task's pod, ingress-only.
        assert_eq!(
            spec.pod_selector
                .as_ref()
                .and_then(|s| s.match_labels.as_ref())
                .and_then(|l| l.get("moatless.io/task-id"))
                .map(String::as_str),
            Some("task-1")
        );
        assert_eq!(
            spec.policy_types.as_deref(),
            Some(&["Ingress".to_string()][..])
        );

        let ingress = spec.ingress.expect("has ingress rules");
        assert_eq!(ingress.len(), 1);
        let rule = &ingress[0];

        // One peer per Traefik namespace (they OR), each an AND of namespace + pod.
        let from = rule.from.as_ref().expect("has from peers");
        assert_eq!(from.len(), 2);
        assert_eq!(
            from[0]
                .namespace_selector
                .as_ref()
                .and_then(|s| s.match_labels.as_ref())
                .and_then(|l| l.get("kubernetes.io/metadata.name"))
                .map(String::as_str),
            Some("moatless")
        );
        assert_eq!(
            from[0]
                .pod_selector
                .as_ref()
                .and_then(|s| s.match_labels.as_ref())
                .and_then(|l| l.get(TRAEFIK_POD_LABEL))
                .map(String::as_str),
            Some(TRAEFIK_POD_LABEL_VALUE)
        );

        // Exactly the requested ports, never an empty list (which would mean "all").
        let policy_ports = rule.ports.as_ref().expect("has ports");
        let admitted: BTreeSet<i32> = policy_ports
            .iter()
            .filter_map(|p| match p.port.as_ref() {
                Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(n)) => Some(*n),
                _ => None,
            })
            .collect();
        assert_eq!(admitted, BTreeSet::from([5733, 3000]));
    }

    #[test]
    fn owner_reference_is_set_when_provided() {
        let ports = BTreeSet::from([5733u16]);
        let policy = build_preview_network_policy(
            "moatless",
            "task-1",
            &ports,
            &["moatless".to_string()],
            Some(&owner()),
        )
        .expect("build should succeed");
        let owners = policy.metadata.owner_references.expect("owner set");
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].uid, "uid-1");
        assert_eq!(
            policy.metadata.name.as_deref(),
            Some("moatless-ws-task-1-preview-ingress")
        );
    }

    #[test]
    fn admitted_ports_reads_back_every_port_the_policy_admits() {
        // The widen loop reads the current ports, adds one, and rebuilds the
        // whole policy. That only keeps concurrently-added ports if reading a
        // built policy yields exactly the ports it was built from — otherwise a
        // retry after a 409 would silently shed a port it meant to preserve.
        let ports = BTreeSet::from([8080u16, 3000u16, 5733u16]);
        let policy = build_preview_network_policy(
            "moatless",
            "task-1",
            &ports,
            &["moatless".to_string()],
            Some(&owner()),
        )
        .expect("build should succeed");
        assert_eq!(admitted_ports(&policy), ports);
    }

    fn api_error(code: u16) -> kube::Error {
        kube::Error::Api(Box::new(
            kube::core::Status::failure("networkpolicies.networking.k8s.io", "Forbidden")
                .with_code(code),
        ))
    }

    #[test]
    fn only_403_counts_as_missing_networkpolicy_grant() {
        assert!(is_forbidden(&api_error(403)));
        // Anything else stays a real error the caller must surface: a 404 has its
        // own already-deleted branch, and a 500 is genuinely worth retrying.
        assert!(!is_forbidden(&api_error(404)));
        assert!(!is_forbidden(&api_error(500)));
        assert!(!is_forbidden(&kube::Error::LinesCodecMaxLineLengthExceeded));
    }
}
