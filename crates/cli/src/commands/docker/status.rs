//! `moat docker status` — health check the running stack.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::compose::{
    detect_compose_cmd, resolve_compose_dir, run_compose, run_docker, ComposeDirSource, ENV_FILE,
};
use super::StatusArgs;
use crate::client::ClientError;
use crate::output;

#[derive(Serialize, Deserialize)]
pub(super) struct StatusReport {
    pub(super) compose_dir: String,
    pub(super) compose_dir_source: String,
    pub(super) all_healthy: bool,
    pub(super) services: Vec<ServiceStatus>,
    pub(super) endpoints: Vec<EndpointStatus>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ServiceStatus {
    pub(super) name: String,
    pub(super) state: String,
    pub(super) health: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct EndpointStatus {
    pub(super) name: String,
    pub(super) url: String,
    pub(super) ok: bool,
    pub(super) detail: String,
}

pub(super) async fn run(args: StatusArgs, json: bool) -> Result<(), ClientError> {
    let (compose_dir, source) = resolve_compose_dir(args.compose_dir.as_deref())?;

    let services = gather_services(&compose_dir).await.unwrap_or_default();
    let services_healthy = !services.is_empty()
        && services.iter().all(|s| {
            s.state == "running"
                && s.health
                    .as_deref()
                    .is_none_or(|h| h == "healthy" || h == "none" || h.is_empty())
        });

    let endpoints = gather_endpoints().await;
    let endpoints_healthy = endpoints.iter().all(|e| e.ok);

    let report = StatusReport {
        compose_dir: compose_dir.display().to_string(),
        compose_dir_source: source.as_str().to_string(),
        all_healthy: services_healthy && endpoints_healthy,
        services,
        endpoints,
    };

    if json {
        output::print_json_typed(&report);
    } else {
        print_human(&report, source);
    }

    Ok(())
}

/// Used by `install --wait` to poll for overall health without human output.
pub(super) async fn quick_all_healthy(compose_dir: &Path) -> Result<bool, ClientError> {
    let services = gather_services(compose_dir).await?;
    if services.is_empty() {
        return Ok(false);
    }
    let ok = services.iter().all(|s| {
        s.state == "running"
            && s.health
                .as_deref()
                .is_none_or(|h| h == "healthy" || h == "none" || h.is_empty())
    });
    Ok(ok)
}

async fn gather_services(compose_dir: &Path) -> Result<Vec<ServiceStatus>, ClientError> {
    // Only usable if .env exists; otherwise compose refuses to read the project.
    if !compose_dir.join(ENV_FILE).is_file() {
        return Ok(Vec::new());
    }
    let compose = detect_compose_cmd().await?;
    let out = run_compose(&compose, compose_dir, &["ps", "--format", "json"]).await?;
    if !out.success() {
        return Ok(Vec::new());
    }

    // `docker compose ps --format json` emits one JSON object per line.
    let mut services = Vec::new();
    for line in out.stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Some compose versions wrap the array; handle both.
        if let Some(arr) = v.as_array() {
            for item in arr {
                if let Some(s) = parse_service(item) {
                    services.push(s);
                }
            }
        } else if let Some(s) = parse_service(&v) {
            services.push(s);
        }
    }

    Ok(services)
}

fn parse_service(v: &serde_json::Value) -> Option<ServiceStatus> {
    let name = v
        .get("Service")
        .or_else(|| v.get("Name"))?
        .as_str()?
        .to_string();
    let state = v
        .get("State")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown")
        .to_string();
    let health = v
        .get("Health")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Some(ServiceStatus {
        name,
        state,
        health,
    })
}

async fn gather_endpoints() -> Vec<EndpointStatus> {
    let mut out = Vec::new();

    // Backend /health
    out.push(http_probe("backend", "http://localhost:8080/health").await);
    // Frontend root
    out.push(http_probe("frontend", "http://localhost:3000/").await);

    // Postgres — pg_isready inside the container (avoids needing psql on host)
    out.push(
        exec_probe(
            "postgres",
            &["exec", "moatless-postgres", "pg_isready", "-U", "moatless"],
        )
        .await,
    );

    // Redis PING
    out.push(exec_probe("redis", &["exec", "moatless-redis", "redis-cli", "PING"]).await);

    out
}

async fn http_probe(name: &str, url: &str) -> EndpointStatus {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return EndpointStatus {
                name: name.to_string(),
                url: url.to_string(),
                ok: false,
                detail: format!("client init failed: {e}"),
            };
        }
    };
    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            EndpointStatus {
                name: name.to_string(),
                url: url.to_string(),
                ok: status.is_success(),
                detail: format!("HTTP {}", status.as_u16()),
            }
        }
        Err(e) => EndpointStatus {
            name: name.to_string(),
            url: url.to_string(),
            ok: false,
            detail: format!("request failed: {e}"),
        },
    }
}

async fn exec_probe(name: &str, args: &[&str]) -> EndpointStatus {
    let out = run_docker(args).await;
    match out {
        Ok(o) if o.success() => EndpointStatus {
            name: name.to_string(),
            url: format!("docker {}", args.join(" ")),
            ok: true,
            detail: o.stdout.trim().to_string(),
        },
        Ok(o) => EndpointStatus {
            name: name.to_string(),
            url: format!("docker {}", args.join(" ")),
            ok: false,
            detail: if o.stderr.trim().is_empty() {
                format!("exit {}", o.status)
            } else {
                o.stderr.trim().to_string()
            },
        },
        Err(e) => EndpointStatus {
            name: name.to_string(),
            url: format!("docker {}", args.join(" ")),
            ok: false,
            detail: e.to_string(),
        },
    }
}

fn print_human(r: &StatusReport, source: ComposeDirSource) {
    println!("Docker status");
    println!("=============");
    println!("Work dir:    {} ({})", r.compose_dir, source.as_str());
    println!("All healthy: {}", if r.all_healthy { "yes" } else { "no" });
    println!();

    if !r.services.is_empty() {
        println!("Services:");
        let headers = ["NAME", "STATE", "HEALTH"];
        let rows: Vec<Vec<String>> = r
            .services
            .iter()
            .map(|s| {
                vec![
                    s.name.clone(),
                    s.state.clone(),
                    s.health.clone().unwrap_or_default(),
                ]
            })
            .collect();
        output::print_table(&headers, &rows);
        println!();
    }

    println!("Endpoints:");
    let headers = ["STATUS", "NAME", "DETAIL"];
    let rows: Vec<Vec<String>> = r
        .endpoints
        .iter()
        .map(|e| {
            vec![
                if e.ok {
                    "ok".to_string()
                } else {
                    "fail".to_string()
                },
                e.name.clone(),
                e.detail.clone(),
            ]
        })
        .collect();
    output::print_table(&headers, &rows);
}
