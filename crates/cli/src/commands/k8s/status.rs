//! `moat k8s status` — report health of the current installation.

use serde::{Deserialize, Serialize};

use super::helpers::{helm, kubectl_json};
use super::StatusArgs;
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
    namespace: String,
    release: String,
    release_revision: Option<String>,
    deployments: Vec<DeploymentStatus>,
    ingresses: Vec<IngressStatus>,
    pending_pods: Vec<String>,
    all_healthy: bool,
}

pub(super) async fn run(args: StatusArgs, json_out: bool) -> Result<(), ClientError> {
    let revision = helm_release_revision(&args.release, &args.namespace)
        .await
        .ok()
        .flatten();

    let deployments = gather_deployments(&args.namespace, &args.release).await?;
    let ingresses = gather_ingresses(&args.namespace).await?;
    let pending_pods = gather_pending_pods(&args.namespace).await?;

    let all_healthy = deployments.iter().all(|d| d.healthy) && pending_pods.is_empty();

    let report = Report {
        namespace: args.namespace.clone(),
        release: args.release.clone(),
        release_revision: revision,
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
        return Err(ClientError::Other(
            "Installation is not fully healthy. See report above; run `kubectl -n {ns} describe ...` or `moat k8s status --json` for details.".to_string(),
        ));
    }
    Ok(())
}

async fn helm_release_revision(
    release: &str,
    namespace: &str,
) -> Result<Option<String>, ClientError> {
    let out = helm(&["status", release, "-n", namespace, "-o", "json"]).await?;
    if !out.success() {
        return Ok(None);
    }
    let v: serde_json::Value = match serde_json::from_str(&out.stdout) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    Ok(v.get("version")
        .and_then(serde_json::Value::as_i64)
        .map(|i| i.to_string()))
}

async fn gather_deployments(
    namespace: &str,
    release: &str,
) -> Result<Vec<DeploymentStatus>, ClientError> {
    let selector = format!("app.kubernetes.io/instance={release}");
    let Some(v) = kubectl_json(&[
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

async fn gather_ingresses(namespace: &str) -> Result<Vec<IngressStatus>, ClientError> {
    let Some(v) = kubectl_json(&["-n", namespace, "get", "ingress", "-o", "json"]).await? else {
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
                .map(|rules| {
                    rules
                        .iter()
                        .filter_map(|r| r.get("host").and_then(|h| h.as_str()).map(String::from))
                        .collect()
                })
                .unwrap_or_default();
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

async fn gather_pending_pods(namespace: &str) -> Result<Vec<String>, ClientError> {
    let Some(v) = kubectl_json(&["-n", namespace, "get", "pods", "-o", "json"]).await? else {
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
        "Release:   {} (revision {})",
        r.release,
        r.release_revision.as_deref().unwrap_or("?")
    );
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
