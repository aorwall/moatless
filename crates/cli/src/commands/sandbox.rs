//! Sandbox commands: status, start, stop, restart, logs, files.

use std::fmt::Write;

use clap::Subcommand;
use futures_util::StreamExt;
use moatless_api_client::types::SandboxStatusFullResponse;

use crate::client::{map_api_error, ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum SandboxAction {
    /// Show sandbox status for a task
    Status { task_id: String },
    /// Start sandbox for a task
    Start { task_id: String },
    /// Stop sandbox for a task
    Stop { task_id: String },
    /// Restart sandbox for a task
    Restart { task_id: String },
    /// Stream server logs from a sandbox
    Logs {
        task_id: String,
        /// Server name (e.g. "sandbox", "preview-server", "files-server")
        #[arg(long, short = 's', default_value = "sandbox")]
        server: String,
    },
    /// File operations in a sandbox workspace
    Files {
        task_id: String,
        #[command(subcommand)]
        action: FileAction,
    },
}

#[derive(Subcommand)]
pub enum FileAction {
    /// Show file tree with git status
    Tree {
        /// Root path relative to workspace
        #[arg(long, default_value = ".")]
        path: String,
    },
    /// Read file content
    Read {
        /// File path relative to workspace
        #[arg(long)]
        path: String,
    },
    /// List files in a directory
    List {
        /// Path relative to workspace
        #[arg(long, default_value = ".")]
        path: String,
        /// List files recursively
        #[arg(long)]
        recursive: bool,
    },
}

pub async fn run(
    client: &MoatClient,
    action: SandboxAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        SandboxAction::Status { task_id } => {
            let resp = client
                .api()
                .sandbox_status_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            let status = resp.into_inner();
            if json {
                output::print_json_typed(&status);
            } else {
                format_sandbox_status(&status);
            }
        }
        SandboxAction::Start { task_id } => {
            client
                .api()
                .provision_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Sandbox for {task_id} started.");
        }
        SandboxAction::Stop { task_id } => {
            client
                .api()
                .stop_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Sandbox for {task_id} stopped.");
        }
        SandboxAction::Restart { task_id } => {
            client
                .api()
                .restart_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Sandbox for {task_id} restarted.");
        }
        SandboxAction::Logs { task_id, server } => {
            stream_logs(client, &task_id, &server).await?;
        }
        SandboxAction::Files { task_id, action } => {
            run_file_action(client, &task_id, action, json).await?;
        }
    }
    Ok(())
}

/// Stream server logs via SSE.
async fn stream_logs(
    client: &MoatClient,
    task_id: &str,
    server: &str,
) -> Result<(), ClientError> {
    let path = format!("/api/sandbox/v1/tasks/{task_id}/servers/{server}/logs");
    let resp = client.raw_get_stream(&path).await?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    eprintln!("Streaming logs for {server} (Ctrl+C to stop)...");

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

/// Execute a file operation against the sandbox.
async fn run_file_action(
    client: &MoatClient,
    task_id: &str,
    action: FileAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        FileAction::Tree { path } => {
            let api_path = format!(
                "/api/sandbox/v1/tasks/{task_id}/files/tree?path={}",
                urlencoding::encode(&path)
            );
            let resp = client.raw_get(&api_path).await?;
            if json {
                let pretty = serde_json::to_string_pretty(&resp).unwrap_or_default();
                println!("{pretty}");
            } else {
                format_file_tree(&resp, 0);
            }
        }
        FileAction::Read { path } => {
            let api_path = format!(
                "/api/sandbox/v1/tasks/{task_id}/files/read?path={}",
                urlencoding::encode(&path)
            );
            let resp = client.raw_get(&api_path).await?;
            if json {
                let pretty = serde_json::to_string_pretty(&resp).unwrap_or_default();
                println!("{pretty}");
            } else {
                // Print file content directly.
                let content = resp
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                print!("{content}");
            }
        }
        FileAction::List { path, recursive } => {
            let mut api_path = format!(
                "/api/sandbox/v1/tasks/{task_id}/files/list?path={}",
                urlencoding::encode(&path)
            );
            if recursive {
                let _ = write!(api_path, "&recursive=true");
            }
            let resp = client.raw_get(&api_path).await?;
            if json {
                let pretty = serde_json::to_string_pretty(&resp).unwrap_or_default();
                println!("{pretty}");
            } else {
                // Print one file per line.
                if let Some(files) = resp.get("files").and_then(|v| v.as_array()) {
                    for f in files {
                        if let Some(path) = f.as_str() {
                            println!("{path}");
                        } else if let Some(path) = f.get("path").and_then(|v| v.as_str()) {
                            println!("{path}");
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Recursively format a file tree response.
fn format_file_tree(resp: &serde_json::Value, _depth: usize) {
    // Response has { "tree": [...], "gitSummary": {...} }
    let items = resp
        .get("tree")
        .and_then(|v| v.as_array())
        .or_else(|| resp.get("items").and_then(|v| v.as_array()))
        .or_else(|| resp.as_array());

    if let Some(children) = items {
        for child in children {
            print_tree_node(child, 0);
        }
    }

    fn print_tree_node(node: &serde_json::Value, depth: usize) {
        let indent = "  ".repeat(depth);
        let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let is_dir = node.get("type").and_then(|v| v.as_str()) == Some("directory");
        let is_changed = node
            .get("isChanged")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let change_tag = if is_changed { " [M]" } else { "" };

        if is_dir {
            println!("{indent}{name}/{change_tag}");
        } else {
            println!("{indent}{name}{change_tag}");
        }

        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for child in children {
                print_tree_node(child, depth + 1);
            }
        }
    }
}

fn format_sandbox_status(status: &SandboxStatusFullResponse) {
    println!("Environment: {}", status.environment_status);
    if let Some(ref err) = status.environment_error {
        println!("Error:       {err}");
    }
    println!("Agent:       {}", status.agent_state);

    if !status.servers.is_empty() {
        println!("\nServers:");
        for s in &status.servers {
            println!("  {}: {} (port {})", s.name, s.status, s.port);
            if let Some(ref err) = s.error {
                println!("    Error:  {err}");
            }
            if let Some(ref detail) = s.detail {
                println!("    Detail: {detail}");
            }
        }
    }
}
