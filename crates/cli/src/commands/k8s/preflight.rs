//! Preflight: inspect the current cluster and report readiness.
//!
//! All checks are read-only. Output is a structured report that the skill can
//! parse (with `--json`) to decide the next step.

use serde::{Deserialize, Serialize};

use super::cluster::{DEFAULT_NAMESPACE, KubeTarget, require_binary};
use super::{ComputeMode, PreflightArgs, Provider, StorageMode};
use crate::client::ClientError;
use crate::output;

#[derive(Serialize, Deserialize)]
pub(super) struct PreflightReport {
    pub(super) ready: bool,
    pub(super) context: Option<String>,
    pub(super) cluster_type: Provider,
    pub(super) namespace: String,
    pub(super) namespace_exists: bool,
    pub(super) ingress_controllers: Vec<String>,
    pub(super) storage_classes: Vec<String>,
    pub(super) default_storage_class: Option<String>,
    pub(super) crds: CrdStatus,
    pub(super) checks: Vec<Check>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct CrdStatus {
    pub(super) traefik_ingressroute: bool,
    pub(super) cert_manager: bool,
    pub(super) prometheus_operator: bool,
    pub(super) cilium: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Check {
    pub(super) name: String,
    pub(super) ok: bool,
    /// A failed blocking check makes the aggregate report not ready.
    pub(super) blocking: bool,
    pub(super) detail: String,
    /// If `ok` is false, a one-line remediation hint the skill can surface.
    pub(super) hint: Option<String>,
}

pub(super) async fn run(args: PreflightArgs, json: bool) -> Result<(), ClientError> {
    let namespace = args
        .namespace
        .clone()
        .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());
    let report = gather(&namespace, &args).await?;

    output::emit(json, &report, || print_human(&report));

    Ok(())
}

async fn gather(namespace: &str, args: &PreflightArgs) -> Result<PreflightReport, ClientError> {
    let mut checks = Vec::new();

    let kubectl_ok = require_binary("kubectl");
    checks.push(check_from_result(
        "kubectl installed",
        kubectl_ok
            .as_ref()
            .map(|()| String::new())
            .map_err(ToString::to_string),
        Some("Install kubectl: https://kubernetes.io/docs/tasks/tools/"),
    ));
    kubectl_ok?;

    let helm_ok = require_binary("helm");
    checks.push(check_from_result(
        "helm installed",
        helm_ok
            .as_ref()
            .map(|()| String::new())
            .map_err(ToString::to_string),
        Some("Install helm: https://helm.sh/docs/intro/install/"),
    ));
    helm_ok?;

    let current_context = super::cluster::current_context().await.ok().flatten();
    let context = args.context.clone().or(current_context);
    let target = KubeTarget::new(args.context.clone());
    checks.push(Check {
        name: "target kubectl context".to_string(),
        ok: context.is_some(),
        blocking: true,
        detail: context.clone().unwrap_or_else(|| "<none>".to_string()),
        hint: if context.is_none() {
            Some("Pass `--context <name>` or select a current kubectl context.".to_string())
        } else {
            None
        },
    });

    let nodes = target.kubectl_json(&["get", "nodes", "-o", "json"]).await?;
    let cluster_type = detect_provider(&nodes);
    checks.push(Check {
        name: "cluster reachable".to_string(),
        ok: nodes.is_some(),
        blocking: true,
        detail: match &nodes {
            Some(v) => format!(
                "{} node(s), detected {}",
                v.get("items")
                    .and_then(|i| i.as_array())
                    .map_or(0, std::vec::Vec::len),
                cluster_type.as_str()
            ),
            None => "could not list nodes".to_string(),
        },
        hint: if nodes.is_none() {
            Some("Check that `kubectl cluster-info` succeeds.".to_string())
        } else {
            None
        },
    });

    let ns = target
        .kubectl_json(&["get", "namespace", namespace, "-o", "json"])
        .await?;
    let namespace_exists = ns.is_some();
    checks.push(Check {
        name: format!("namespace '{namespace}' exists"),
        ok: namespace_exists,
        blocking: true,
        detail: if namespace_exists {
            format!("{namespace} ready")
        } else {
            format!("{namespace} not found")
        },
        hint: if namespace_exists {
            None
        } else {
            Some(format!(
                "Create it: `kubectl create namespace {namespace}` (or Helm will create it with --create-namespace)"
            ))
        },
    });

    let ingress_controllers = detect_ingress_controllers(&target).await?;
    checks.push(Check {
        name: "ingress controller present".to_string(),
        ok: !ingress_controllers.is_empty(),
        blocking: true,
        detail: if ingress_controllers.is_empty() {
            "none detected".to_string()
        } else {
            ingress_controllers.join(", ")
        },
        hint: if ingress_controllers.is_empty() {
            Some(match cluster_type {
                Provider::Aks => "Enable AKS web app routing or install nginx: https://learn.microsoft.com/azure/aks/app-routing".to_string(),
                Provider::Eks => "Install the AWS Load Balancer Controller: https://docs.aws.amazon.com/eks/latest/userguide/alb-ingress.html".to_string(),
                Provider::Generic => "Install an ingress controller (nginx, Traefik, or cloud-provider LB).".to_string(),
            })
        } else {
            None
        },
    });

    let (storage_classes, default_storage_class) = detect_storage_classes(&target).await?;
    let storage_required = !matches!(args.storage_mode, StorageMode::S3 | StorageMode::Ephemeral);
    checks.push(Check {
        name: "default StorageClass present".to_string(),
        ok: !storage_required || default_storage_class.is_some(),
        blocking: storage_required,
        detail: if storage_required {
            default_storage_class
                .clone()
                .unwrap_or_else(|| "none".to_string())
        } else {
            format!("not required for {} workspace storage", args.storage_mode.as_str())
        },
        hint: if storage_required && default_storage_class.is_none() {
            Some("Mark a StorageClass default or set `kubernetes.sandboxStorage.storageClass` in values.local.yaml.".to_string())
        } else {
            None
        },
    });

    let crds = detect_crds(&target).await?;
    checks.push(Check {
        name: "Traefik IngressRoute CRD".to_string(),
        ok: crds.traefik_ingressroute,
        blocking: true,
        detail: if crds.traefik_ingressroute {
            "installed".to_string()
        } else {
            "missing".to_string()
        },
        hint: if crds.traefik_ingressroute {
            None
        } else {
            Some("Install Traefik for per-sandbox preview routing: `helm repo add traefik https://traefik.github.io/charts && helm install traefik traefik/traefik`".to_string())
        },
    });

    checks.push(Check {
        name: "cert-manager CRDs (optional)".to_string(),
        ok: crds.cert_manager,
        blocking: false,
        detail: if crds.cert_manager { "installed".to_string() } else { "missing".to_string() },
        hint: if crds.cert_manager {
            None
        } else {
            Some("Skip if TLS terminates at your load balancer, else install cert-manager: https://cert-manager.io/docs/installation/".to_string())
        },
    });

    if matches!(args.compute_mode, ComputeMode::Fargate) {
        checks.push(Check {
            name: "sandbox NetworkPolicy enforcement".to_string(),
            ok: false,
            blocking: false,
            detail: "EKS VPC CNI does not enforce Kubernetes NetworkPolicy on Fargate pods"
                .to_string(),
            hint: Some(
                "Use this preset only for trusted repositories; use managed nodes for untrusted code isolation."
                    .to_string(),
            ),
        });
    }

    let ready = report_ready(&checks);

    Ok(PreflightReport {
        ready,
        context,
        cluster_type,
        namespace: namespace.to_string(),
        namespace_exists,
        ingress_controllers,
        storage_classes,
        default_storage_class,
        crds,
        checks,
    })
}

fn report_ready(checks: &[Check]) -> bool {
    checks.iter().all(|check| check.ok || !check.blocking)
}

fn check_from_result(name: &str, result: Result<String, String>, hint: Option<&str>) -> Check {
    match result {
        Ok(detail) => Check {
            name: name.to_string(),
            ok: true,
            blocking: true,
            detail: if detail.is_empty() {
                "ok".to_string()
            } else {
                detail
            },
            hint: None,
        },
        Err(err) => Check {
            name: name.to_string(),
            ok: false,
            blocking: true,
            detail: err,
            hint: hint.map(ToString::to_string),
        },
    }
}

fn detect_provider(nodes: &Option<serde_json::Value>) -> Provider {
    let Some(v) = nodes else {
        return Provider::Generic;
    };
    let Some(items) = v.get("items").and_then(|i| i.as_array()) else {
        return Provider::Generic;
    };
    for node in items {
        let provider_id = node
            .pointer("/spec/providerID")
            .and_then(|p| p.as_str())
            .unwrap_or("");
        if provider_id.starts_with("azure://") {
            return Provider::Aks;
        }
        if provider_id.starts_with("aws://") {
            return Provider::Eks;
        }
        let labels = node.pointer("/metadata/labels");
        if let Some(labels) = labels.and_then(|l| l.as_object()) {
            if labels.contains_key("kubernetes.azure.com/cluster") {
                return Provider::Aks;
            }
            if labels.contains_key("eks.amazonaws.com/nodegroup") {
                return Provider::Eks;
            }
        }
    }
    Provider::Generic
}

async fn detect_ingress_controllers(target: &KubeTarget) -> Result<Vec<String>, ClientError> {
    let mut found = Vec::new();
    let classes = target
        .kubectl_json(&["get", "ingressclass", "-o", "json"])
        .await?;
    if let Some(v) = classes {
        if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
            for item in items {
                if let Some(name) = item.pointer("/metadata/name").and_then(|n| n.as_str()) {
                    found.push(name.to_string());
                }
            }
        }
    }
    Ok(found)
}

async fn detect_storage_classes(
    target: &KubeTarget,
) -> Result<(Vec<String>, Option<String>), ClientError> {
    let mut all = Vec::new();
    let mut default = None;
    let classes = target
        .kubectl_json(&["get", "storageclass", "-o", "json"])
        .await?;
    if let Some(v) = classes {
        if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
            for item in items {
                let Some(name) = item.pointer("/metadata/name").and_then(|n| n.as_str()) else {
                    continue;
                };
                all.push(name.to_string());
                let annotations = item
                    .pointer("/metadata/annotations")
                    .and_then(|a| a.as_object());
                if let Some(annotations) = annotations {
                    let is_default = annotations
                        .get("storageclass.kubernetes.io/is-default-class")
                        .or_else(|| {
                            annotations.get("storageclass.beta.kubernetes.io/is-default-class")
                        })
                        .and_then(|v| v.as_str())
                        == Some("true");
                    if is_default && default.is_none() {
                        default = Some(name.to_string());
                    }
                }
            }
        }
    }
    Ok((all, default))
}

async fn detect_crds(target: &KubeTarget) -> Result<CrdStatus, ClientError> {
    async fn has(target: &KubeTarget, crd: &str) -> bool {
        match target.kubectl(&["get", "crd", crd, "--no-headers"]).await {
            Ok(o) => o.success(),
            Err(_) => false,
        }
    }

    Ok(CrdStatus {
        traefik_ingressroute: has(target, "ingressroutes.traefik.io").await,
        cert_manager: has(target, "clusterissuers.cert-manager.io").await,
        prometheus_operator: has(target, "podmonitors.monitoring.coreos.com").await,
        cilium: has(target, "ciliumnetworkpolicies.cilium.io").await,
    })
}

fn print_human(r: &PreflightReport) {
    println!("Preflight report");
    println!("================");
    println!(
        "Context:         {}",
        r.context.as_deref().unwrap_or("<none>")
    );
    println!("Cluster type:    {}", r.cluster_type.as_str());
    println!("Ready:           {}", r.ready);
    println!(
        "Target namespace: {} ({})",
        r.namespace,
        if r.namespace_exists {
            "exists"
        } else {
            "missing"
        }
    );
    println!();

    let headers = ["STATUS", "CHECK", "DETAIL"];
    let rows: Vec<Vec<String>> = r
        .checks
        .iter()
        .map(|c| {
            vec![
                if c.ok {
                    "ok".to_string()
                } else if !c.blocking {
                    "warn".to_string()
                } else {
                    "fail".to_string()
                },
                c.name.clone(),
                c.detail.clone(),
            ]
        })
        .collect();
    output::print_table(&headers, &rows);

    let hints: Vec<&Check> = r
        .checks
        .iter()
        .filter(|c| !c.ok && c.hint.is_some())
        .collect();
    if !hints.is_empty() {
        println!();
        println!("Hints:");
        for c in hints {
            if let Some(hint) = &c.hint {
                println!("  - {}: {hint}", c.name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(ok: bool, blocking: bool) -> Check {
        Check {
            name: "test".into(),
            ok,
            blocking,
            detail: String::new(),
            hint: None,
        }
    }

    #[test]
    fn warnings_do_not_block_readiness() {
        assert!(report_ready(&[check(true, true), check(false, false)]));
    }

    #[test]
    fn failed_requirements_block_readiness() {
        assert!(!report_ready(&[check(true, true), check(false, true)]));
    }
}
