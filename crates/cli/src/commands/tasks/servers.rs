//! A task's preview servers: their status, their live config override, and
//! their logs.

use crate::client::{ClientError, MoatClient};
use crate::output;
use clap::Subcommand;
use futures_util::StreamExt;
use moatless_api_schemas::sandbox::{
    ServerConfigResponse, ServerOverride, ServerOverridesResponse, ServerStatusResponse,
    UpdateServerConfigRequest,
};
use std::collections::BTreeMap;

#[derive(Subcommand)]
pub enum ServerAction {
    /// List servers with their status and any live config override
    List,
    /// Set a server's start command and/or environment, then restart it
    Set {
        /// Server name (e.g. "dev", "preview")
        server: String,
        /// Start command; repeat to run several, joined with `&&`
        #[arg(long)]
        start_command: Vec<String>,
        /// Environment variable as KEY=VALUE; repeatable
        #[arg(long = "env", short = 'e')]
        env: Vec<String>,
        /// Environment variable to remove; repeatable
        #[arg(long = "unset")]
        unset: Vec<String>,
        /// Port the server listens on. Cannot be changed live — it is wired
        /// into the Service and `IngressRoute` at provision time, so the server
        /// rejects this with guidance to redeploy instead.
        #[arg(long)]
        port: Option<u16>,
        /// Container image for the server. Cannot be changed live — the image
        /// is the container's identity, so the server rejects this with
        /// guidance to redeploy instead.
        #[arg(long)]
        docker_image: Option<String>,
        /// Apply without restarting (takes effect on the next restart)
        #[arg(long)]
        no_restart: bool,
    },
    /// Restart a server in place, keeping its current override
    Restart {
        /// Server name (e.g. "dev", "preview")
        server: String,
    },
    /// Drop a server's override and restart it on the workspace config
    Reset {
        /// Server name (e.g. "dev", "preview")
        server: String,
    },
}

pub(super) async fn run_server_action(
    client: &MoatClient,
    task_id: &str,
    action: ServerAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ServerAction::List => {
            let status = client.api().server_status(task_id).await?;
            // Status is config-first and works on a stopped sandbox; overrides
            // live only on the workspace volume and so need a running one.
            // Report them as unknown rather than as "none" when unreachable —
            // "none" would be a lie the user would act on.
            let overrides = client.api().list_server_config(task_id).await.ok();
            if json {
                output::print_json_typed(&serde_json::json!({
                    "servers": status.servers,
                    "overrides": overrides.as_ref().map(|o| &o.overrides),
                }));
            } else {
                format_server_list(&status, overrides.as_ref());
            }
        }
        ServerAction::Set {
            server,
            start_command,
            env,
            unset,
            port,
            docker_image,
            no_restart,
        } => {
            if start_command.is_empty()
                && env.is_empty()
                && unset.is_empty()
                && port.is_none()
                && docker_image.is_none()
            {
                return Err(ClientError::Api {
                    status: 400,
                    body: "Nothing to set. Pass --start-command, --env KEY=VALUE, or --unset KEY."
                        .to_string(),
                });
            }
            let mut parsed_env = BTreeMap::new();
            for entry in &env {
                let (key, value) = entry.split_once('=').ok_or_else(|| ClientError::Api {
                    status: 400,
                    body: format!("Invalid --env '{entry}'. Expected KEY=VALUE."),
                })?;
                parsed_env.insert(key.to_string(), value.to_string());
            }
            let req = UpdateServerConfigRequest {
                start_commands: (!start_command.is_empty()).then_some(start_command),
                env: (!parsed_env.is_empty()).then_some(parsed_env),
                unset_env: (!unset.is_empty()).then_some(unset),
                restart: !no_restart,
                port,
                docker_image,
            };
            let resp = client
                .api()
                .update_server_config(task_id, &server, &req)
                .await?;
            print_server_config(&resp, json);
        }
        ServerAction::Restart { server } => {
            let resp = client.api().restart_server(task_id, &server).await?;
            print_server_config(&resp, json);
        }
        ServerAction::Reset { server } => {
            let resp = client.api().reset_server_config(task_id, &server).await?;
            print_server_config(&resp, json);
        }
    }
    Ok(())
}

fn print_server_config(resp: &ServerConfigResponse, json: bool) {
    if json {
        output::print_json_typed(resp);
        return;
    }
    println!("Server: {}", resp.server_name);
    print_override(&resp.config_override, "  ");
    println!("  generation: {}", resp.generation);
    println!(
        "  {}",
        if resp.restarted {
            "restarted"
        } else {
            "not restarted (applies on next restart)"
        }
    );
}

fn print_override(ov: &ServerOverride, indent: &str) {
    match &ov.start_command {
        Some(cmd) => println!("{indent}start command: {cmd}"),
        None => println!("{indent}start command: (workspace default)"),
    }
    for (key, value) in &ov.env {
        println!("{indent}env {key}={value}");
    }
    for key in &ov.unset_env {
        println!("{indent}unset {key}");
    }
}

fn format_server_list(status: &ServerStatusResponse, overrides: Option<&ServerOverridesResponse>) {
    if status.servers.is_empty() {
        println!("No servers configured.");
        // A script that declares a port publishes a route and serves on it, so
        // a live preview with nothing listed here is the ordinary case rather
        // than a contradiction. Scripts are not servers: nothing supervises
        // them, and their output goes to the terminal they run in.
        println!(
            "A port opened by `moat tasks scripts run` is not listed here — see `moat tasks scripts list`."
        );
        return;
    }
    for server in &status.servers {
        println!("{} ({}) port {}", server.name, server.status, server.port);
        if let Some(error) = &server.error {
            println!("  error: {error}");
        }
        if let Some(detail) = &server.detail {
            println!("  detail: {detail}");
        }
        match overrides {
            None => println!("  override: unknown (sandbox not reachable)"),
            Some(list) => match list.overrides.iter().find(|o| o.server_name == server.name) {
                Some(entry) => {
                    print_override(&entry.config_override, "  override ");
                    println!("  override generation: {}", entry.generation);
                }
                None => println!("  override: none"),
            },
        }
    }
}

/// Stream server logs via SSE.
///
/// With `follow` the stream stays open until Ctrl+C; without it the backend
/// ends the response after the buffered lines, so a caller need not wrap this
/// in `timeout`.
pub(super) async fn stream_logs(
    client: &MoatClient,
    task_id: &str,
    server: &str,
    previous: bool,
    follow: bool,
    tail: i64,
) -> Result<(), ClientError> {
    let path = format!(
        "/api/sandbox/v1/tasks/{task_id}/servers/{server}/logs\
         ?previous={previous}&follow={follow}&tail={tail}"
    );
    let resp = client.raw_get_stream(&path).await?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    if follow {
        eprintln!("Streaming logs for {server} (Ctrl+C to stop)...");
    }

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        // SSE format: lines starting with "data:" followed by content.
                        while let Some(pos) = buf.find('\n') {
                            let line: String = buf.drain(..=pos).collect();
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            if let Some(data) = line.strip_prefix("data:") {
                                println!("{}", data.trim_start());
                            } else if !line.starts_with(':') {
                                // Print non-comment, non-data lines as-is.
                                println!("{line}");
                            }
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("Stream error: {e}");
                        break;
                    }
                    None => break,
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nStopped.");
                break;
            }
        }
    }
    Ok(())
}
