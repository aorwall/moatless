//! Preflight: inspect the host for a Docker Compose install.

use serde::{Deserialize, Serialize};

use super::PreflightArgs;
use super::docker_compose::{
    COMPOSE_FILE, ComposeDirSource, ENV_FILE, cache_compose_dir, detect_compose_cmd, port_in_use,
    require_binary, resolve_compose_dir,
};
use crate::client::ClientError;
use crate::output;

#[derive(Serialize, Deserialize)]
pub(super) struct PreflightReport {
    pub(super) compose_command: Option<String>,
    pub(super) compose_dir: String,
    pub(super) compose_dir_source: String,
    pub(super) compose_file_present: bool,
    pub(super) env_file_present: bool,
    pub(super) arch: String,
    pub(super) checks: Vec<Check>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Check {
    pub(super) name: String,
    pub(super) ok: bool,
    pub(super) detail: String,
    pub(super) hint: Option<String>,
}

const REQUIRED_PORTS: &[(u16, &str)] = &[
    (3000, "frontend (Nginx)"),
    (8080, "backend (Axum)"),
    (5432, "postgres"),
    (9999, "dozzle"),
];

pub(super) async fn run(args: PreflightArgs, json: bool) -> Result<(), ClientError> {
    let report = gather(args).await?;
    output::emit(json, &report, || print_human(&report));
    Ok(())
}

async fn gather(args: PreflightArgs) -> Result<PreflightReport, ClientError> {
    let mut checks = Vec::new();

    let docker_ok = require_binary("docker");
    checks.push(check_result(
        "docker installed",
        &docker_ok,
        Some("Install Docker Engine 24+: https://docs.docker.com/engine/install/"),
    ));
    docker_ok?;

    let compose = detect_compose_cmd().await;
    let compose_command = match &compose {
        Ok(c) => Some(c.label().to_string()),
        Err(_) => None,
    };
    checks.push(Check {
        name: "docker compose available".to_string(),
        ok: compose.is_ok(),
        detail: compose_command
            .clone()
            .unwrap_or_else(|| "not found".to_string()),
        hint: if compose.is_err() {
            Some(
                "Install the Docker Compose v2 plugin: https://docs.docker.com/compose/install/"
                    .to_string(),
            )
        } else {
            None
        },
    });

    let (compose_dir, source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    let compose_file_present = compose_dir.join(COMPOSE_FILE).is_file();
    let env_file_present = compose_dir.join(ENV_FILE).is_file();

    checks.push(Check {
        name: "compose work dir".to_string(),
        ok: true,
        detail: format!("{} ({})", compose_dir.display(), source.as_str()),
        hint: None,
    });

    checks.push(Check {
        name: format!("{COMPOSE_FILE} present"),
        ok: compose_file_present,
        detail: if compose_file_present {
            "found".to_string()
        } else {
            "missing".to_string()
        },
        hint: if compose_file_present {
            None
        } else {
            Some(missing_compose_hint(source))
        },
    });

    checks.push(Check {
        name: format!("{ENV_FILE} present"),
        ok: env_file_present,
        detail: if env_file_present {
            "found".to_string()
        } else {
            "missing".to_string()
        },
        hint: if env_file_present {
            None
        } else {
            Some(format!("Run `moat docker init` to create {ENV_FILE}."))
        },
    });

    for (port, service) in REQUIRED_PORTS {
        let busy = port_in_use(*port).await;
        checks.push(Check {
            name: format!("port :{port} free"),
            ok: !busy,
            detail: if busy {
                format!("in use (needed by {service})")
            } else {
                format!("free (for {service})")
            },
            hint: if busy {
                Some(format!(
                    "Stop the process on :{port}, or override via {ENV_FILE} before install."
                ))
            } else {
                None
            },
        });
    }

    // 5. arch (warn on Apple Silicon — sandbox image often needs a local build)
    let arch = std::env::consts::ARCH.to_string();
    if arch == "aarch64" && std::env::consts::OS == "macos" {
        checks.push(Check {
            name: "arch".to_string(),
            ok: true,
            detail: format!("{arch} (Apple Silicon)"),
            hint: Some(
                "If sandbox image startup fails, build native arm64 images: `docker/build-sandbox.sh --local --only base`"
                    .to_string(),
            ),
        });
    } else {
        checks.push(Check {
            name: "arch".to_string(),
            ok: true,
            detail: arch.clone(),
            hint: None,
        });
    }

    Ok(PreflightReport {
        compose_command,
        compose_dir: compose_dir.display().to_string(),
        compose_dir_source: source.as_str().to_string(),
        compose_file_present,
        env_file_present,
        arch,
        checks,
    })
}

fn missing_compose_hint(source: ComposeDirSource) -> String {
    match source {
        ComposeDirSource::Flag => {
            "Pass `--compose-dir` to a directory that contains docker-compose.yml".to_string()
        }
        ComposeDirSource::Repo => format!(
            "Expected {COMPOSE_FILE} at {}/docker-compose/{COMPOSE_FILE}",
            std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default()
        ),
        ComposeDirSource::Cache => {
            let cache_display = cache_compose_dir().map_or_else(
                |_| "~/.moatless/compose".to_string(),
                |p| p.display().to_string(),
            );
            format!("Populate {cache_display} via `moat docker init`")
        }
    }
}

fn check_result(name: &str, result: &Result<(), ClientError>, hint: Option<&str>) -> Check {
    match result {
        Ok(()) => Check {
            name: name.to_string(),
            ok: true,
            detail: "ok".to_string(),
            hint: None,
        },
        Err(e) => Check {
            name: name.to_string(),
            ok: false,
            detail: e.to_string(),
            hint: hint.map(ToString::to_string),
        },
    }
}

fn print_human(r: &PreflightReport) {
    println!("Docker preflight");
    println!("================");
    println!(
        "Compose cmd:     {}",
        r.compose_command.as_deref().unwrap_or("<none>")
    );
    println!(
        "Compose work dir: {} ({})",
        r.compose_dir, r.compose_dir_source
    );
    println!("Arch:            {}", r.arch);
    println!();

    let headers = ["STATUS", "CHECK", "DETAIL"];
    let rows: Vec<Vec<String>> = r
        .checks
        .iter()
        .map(|c| {
            vec![
                if c.ok {
                    "ok".to_string()
                } else {
                    "fail".to_string()
                },
                c.name.clone(),
                c.detail.clone(),
            ]
        })
        .collect();
    output::print_table(&headers, &rows);

    let hints: Vec<&Check> = r.checks.iter().filter(|c| c.hint.is_some()).collect();
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
