//! Per-sandbox Traefik preview NetworkPolicy lifecycle.
//!
//! Each running sandbox that exposes preview servers gets a NetworkPolicy
//! admitting the cluster's Traefik ingress pods to exactly that sandbox's
//! declared preview ports. It is the ingress counterpart to the IngressRoutes:
//! a route makes a port addressable, this policy makes the pod reachable on it.
//!
//! Deriving the allowed ports from the same server map the routes use is the
//! point — a static, hand-maintained port allowlist drifts out of sync with the
//! ports sandboxes actually serve, and once ingress NetworkPolicies are enforced
//! every un-listed preview port silently times out (auth succeeds at the edge
//! first, so it reads as an application fault rather than a network one).

use k8s_openapi::api::networking::v1::NetworkPolicy;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
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

/// A 403 on the NetworkPolicy API means the backend ServiceAccount holds no
/// `networking.k8s.io/networkpolicies` grant — the chart's pod-manager Role
/// carries it, so a cluster running an older release lands here.
///
/// Preview policies are optional ingress hardening, so that cluster degrades to
/// "feature off" rather than wedging: creation is already best-effort, and a
/// hard-failing delete makes every remove and redeploy retry a permission error
/// no retry can fix. Nothing leaks either — a ServiceAccount that cannot delete
/// policies could not create them, and policies created while the grant existed
/// carry the task Deployment's ownerReference, so K8s GC still collects them.
fn is_forbidden(error: &kube::Error) -> bool {
    matches!(error, kube::Error::Api(response) if response.code == 403)
}

/// Shared operator-facing hint for the 403 warnings.
const MISSING_RBAC_HINT: &str =
    "backend ServiceAccount lacks networking.k8s.io/networkpolicies — upgrade the chart \
     (pod-manager Role) to enable preview NetworkPolicies";

/// Build the per-task preview NetworkPolicy object. Pure — no cluster I/O — so
/// the selector/port/peer shape is unit-testable.
///
/// Each Traefik namespace becomes its own `from` peer so the peers OR: traffic
/// from a Traefik pod in *any* listed namespace is admitted. The
/// namespaceSelector and podSelector share one peer element, which ANDs them —
/// the intent being "Traefik pods that live in this namespace".
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

/// Create the per-task preview NetworkPolicy admitting Traefik to `ports`.
/// Idempotent — handles AlreadyExists (409) gracefully.
///
/// `owner`, when present, is set as the policy's `ownerReference` (the task's
/// Deployment) so K8s garbage collection cascades its deletion. Called with an
/// empty `ports` set it is a no-op: an ingress rule with an empty ports list
/// matches *all* ports in the NetworkPolicy API, which would admit Traefik to
/// the reserved infrastructure ports — the opposite of the intent.
pub async fn create_task_preview_network_policy(
    k8s: &K8sClient,
    task_id: &str,
    ports: &BTreeSet<u16>,
    config: &Config,
    owner: Option<&OwnerReference>,
) -> Result<(), kube::Error> {
    if ports.is_empty() {
        debug!(task_id, "No preview ports; skipping preview NetworkPolicy");
        return Ok(());
    }

    let api: Api<NetworkPolicy> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let name = generate_preview_network_policy_name(task_id);
    let policy = build_preview_network_policy(
        k8s.namespace(),
        task_id,
        ports,
        &config.proxy.traefik_ingress_namespaces,
        owner,
    )?;

    match api.create(&PostParams::default(), &policy).await {
        Ok(_) => {
            info!(task_id, policy_name = %name, ?ports, "Preview NetworkPolicy created");
        }
        Err(kube::Error::Api(ref e)) if e.code == 409 => {
            debug!(task_id, policy_name = %name, "Preview NetworkPolicy already exists, reusing");
        }
        Err(ref e) if is_forbidden(e) => {
            warn!(
                task_id,
                policy_name = %name,
                "Skipping preview NetworkPolicy: {MISSING_RBAC_HINT}"
            );
        }
        Err(e) => {
            warn!(task_id, policy_name = %name, error = %e, "Failed to create preview NetworkPolicy");
            return Err(e);
        }
    }

    Ok(())
}

/// Delete the per-task preview NetworkPolicy. Handles 404 and a missing
/// NetworkPolicy grant (403) gracefully — see [`is_forbidden`].
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

/// Await observed absence of the task's preview NetworkPolicy. Without the
/// NetworkPolicy grant (403) absence is unobservable and there is nothing to
/// observe — see [`is_forbidden`] — so the wait resolves immediately rather than
/// burning its full timeout and reporting the resource as lingering.
pub async fn wait_for_task_preview_network_policy_deleted(
    k8s: &K8sClient,
    task_id: &str,
) -> Result<(), moatless_sandbox_control::error::SandboxError> {
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
