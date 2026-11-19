//! `moat k8s status` — report health of the current installation.

use serde::{Deserialize, Serialize};

use super::StatusArgs;
use super::cluster::KubeTarget;
use crate::client::ClientError;
use crate::output;

#[derive(Serialize, Deserialize)]
struct DeploymentStatus {
    name: String,
    ready: i64,
    desired: i64,
    up_to_date: i64,
    available: i64,
    healthy: bool,
}

#[derive(Serialize, Deserialize)]
struct IngressStatus {
    name: String,
    hosts: Vec<String>,
    address: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Report {
    context: Option<String>,
    namespace: String,
    release: String,
    release_revision: Option<String>,
    release_status: Option<String>,
    deployments: Vec<DeploymentStatus>,
    ingresses: Vec<IngressStatus>,
    pending_pods: Vec<String>,
    all_healthy: bool,
}

pub(super) async fn run(args: StatusArgs, json_out: bool) -> Result<(), ClientError> {
    let target = KubeTarget::new(args.context.clone());
    let release = helm_release(&target, &args.release, &args.namespace).await?;

    let deployments = gather_deployments(&target, &args.namespace, &args.release).await?;
    let ingresses = gather_ingresses(&target, &args.namespace, &args.release).await?;
    let pending_pods = gather_pending_pods(&target, &args.namespace, &args.release).await?;

    let all_healthy = installation_is_healthy(&release, &deployments, &pending_pods);

    let report = Report {
        context: target.context().map(ToString::to_string),
        namespace: args.namespace.clone(),
        release: args.release.clone(),
        release_revision: release.revision,
        release_status: release.status,
        deployments,
        ingresses,
        pending_pods,
        all_healthy,
    };

    if json_out {
        output::print_json_typed(&report);
    } else {
        print_human(&report);
    }

    if !report.all_healthy {
        return Err(ClientError::Other(format!(
            "Installation is not fully healthy. See report above; inspect namespace '{}' in context '{}' for details.",
            report.namespace,
            report.context.as_deref().unwrap_or("<current>"),
        )));
    }
    Ok(())
}

#[derive(Default)]
struct HelmRelease {
    revision: Option<String>,
    status: Option<String>,
}

fn installation_is_healthy(
    release: &HelmRelease,
    deployments: &[DeploymentStatus],
    pending_pods: &[String],
) -> bool {
    release.status.as_deref() == Some("deployed")
        && !deployments.is_empty()
        && deployments.iter().all(|deployment| deployment.healthy)
        && pending_pods.is_empty()
}

async fn helm_release(
    target: &KubeTarget,
    release: &str,
    namespace: &str,
) -> Result<HelmRelease, ClientError> {
    let out = target
        .helm(&["status", release, "-n", namespace, "-o", "json"])
        .await?;
    if !out.success() {
        return Ok(HelmRelease::default());
    }
    let v: serde_json::Value = match serde_json::from_str(&out.stdout) {
        Ok(v) => v,
        Err(_) => return Ok(HelmRelease::default()),
    };
    Ok(HelmRelease {
        revision: v
            .get("version")
            .and_then(serde_json::Value::as_i64)
            .map(|value| value.to_string()),
        status: v
            .pointer("/info/status")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
    })
}

async fn gather_deployments(
    target: &KubeTarget,
    namespace: &str,
    release: &str,
) -> Result<Vec<DeploymentStatus>, ClientError> {
    let selector = format!("app.kubernetes.io/instance={release}");
    let Some(v) = target
        .kubectl_json(&[
            "-n",
            namespace,
            "get",
            "deployment",
            "-l",
            &selector,
            "-o",
            "json",
        ])
        .await?
    else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        for item in items {
            let name = item
                .pointer("/metadata/name")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let desired = item
                .pointer("/spec/replicas")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let ready = item
                .pointer("/status/readyReplicas")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let up_to_date = item
                .pointer("/status/updatedReplicas")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let available = item
                .pointer("/status/availableReplicas")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let healthy = ready == desired && desired > 0;
            out.push(DeploymentStatus {
                name,
                ready,
                desired,
                up_to_date,
                available,
                healthy,
            });
        }
    }
    Ok(out)
}

async fn gather_ingresses(
    target: &KubeTarget,
    namespace: &str,
    release: &str,
) -> Result<Vec<IngressStatus>, ClientError> {
    let selector = format!("app.kubernetes.io/instance={release}");
    let Some(v) = target
        .kubectl_json(&[
            "-n", namespace, "get", "ingress", "-l", &selector, "-o", "json",
        ])
        .await?
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        for item in items {
            let name = item
                .pointer("/metadata/name")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let hosts: Vec<String> = item
                .pointer("/spec/rules")
                .and_then(|r| r.as_array())
                .map_or_default(|rules| {
                    rules
                        .iter()
                        .filter_map(|r| r.get("host").and_then(|h| h.as_str()).map(String::from))
                        .collect()
                });
            let address = item
                .pointer("/status/loadBalancer/ingress/0/hostname")
                .or_else(|| item.pointer("/status/loadBalancer/ingress/0/ip"))
                .and_then(|x| x.as_str())
                .map(String::from);
            out.push(IngressStatus {
                name,
                hosts,
                address,
            });
        }
    }
    Ok(out)
}

async fn gather_pending_pods(
    target: &KubeTarget,
    namespace: &str,
    release: &str,
) -> Result<Vec<String>, ClientError> {
    let selector = format!("app.kubernetes.io/instance={release}");
    let Some(v) = target
        .kubectl_json(&[
            "-n", namespace, "get", "pods", "-l", &selector, "-o", "json",
        ])
        .await?
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        for item in items {
            let phase = item
                .pointer("/status/phase")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            if phase != "Running" && phase != "Succeeded" {
                let name = item
                    .pointer("/metadata/name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(format!("{name} ({phase})"));
            }
        }
    }
    Ok(out)
}

fn print_human(r: &Report) {
    println!(
        "Release:   {} (revision {}, status {})",
        r.release,
        r.release_revision.as_deref().unwrap_or("?"),
        r.release_status.as_deref().unwrap_or("missing")
    );
    println!("Context:   {}", r.context.as_deref().unwrap_or("<current>"));
    println!("Namespace: {}", r.namespace);
    println!();

    if r.deployments.is_empty() {
        println!("No deployments found for release {}.", r.release);
    } else {
        let headers = ["STATUS", "DEPLOYMENT", "READY", "UP-TO-DATE", "AVAILABLE"];
        let rows: Vec<Vec<String>> = r
            .deployments
            .iter()
            .map(|d| {
                vec![
                    if d.healthy {
                        "ok".to_string()
                    } else {
                        "fail".to_string()
                    },
                    d.name.clone(),
                    format!("{}/{}", d.ready, d.desired),
                    d.up_to_date.to_string(),
                    d.available.to_string(),
                ]
            })
            .collect();
        output::print_table(&headers, &rows);
    }

    if !r.ingresses.is_empty() {
        println!();
        let headers = ["INGRESS", "HOSTS", "ADDRESS"];
        let rows: Vec<Vec<String>> = r
            .ingresses
            .iter()
            .map(|i| {
                vec![
                    i.name.clone(),
                    i.hosts.join(", "),
                    i.address.clone().unwrap_or_else(|| "<pending>".into()),
                ]
            })
            .collect();
        output::print_table(&headers, &rows);
    }

    if !r.pending_pods.is_empty() {
        println!();
        println!("Non-Running pods:");
        for p in &r.pending_pods {
            println!("  - {p}");
        }
    }

    println!();
    if r.all_healthy {
        println!("All checks green.");
    } else {
        println!("Some checks failed. Run with --json for the full report.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployment(healthy: bool) -> DeploymentStatus {
        DeploymentStatus {
            name: "backend".to_string(),
            ready: i64::from(healthy),
            desired: 1,
            up_to_date: i64::from(healthy),
            available: i64::from(healthy),
            healthy,
        }
    }

    #[test]
    fn healthy_requires_a_deployed_release_and_expected_workloads() {
        let deployed = HelmRelease {
            revision: Some("1".to_string()),
            status: Some("deployed".to_string()),
        };

        assert!(installation_is_healthy(&deployed, &[deployment(true)], &[]));
        assert!(!installation_is_healthy(&deployed, &[], &[]));
        assert!(!installation_is_healthy(
            &HelmRelease {
                revision: Some("1".to_string()),
                status: Some("pending-install".to_string()),
            },
            &[deployment(true)],
            &[],
        ));
        assert!(!installation_is_healthy(
            &HelmRelease::default(),
            &[deployment(true)],
            &[],
        ));
        assert!(!installation_is_healthy(
            &deployed,
            &[deployment(false)],
            &[],
        ));
        assert!(!installation_is_healthy(
            &deployed,
            &[deployment(true)],
            &["pod (Pending)".to_string()],
        ));
    }
}
