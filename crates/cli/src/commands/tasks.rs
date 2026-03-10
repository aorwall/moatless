//! Task commands: list, get, create, children, start, stop, close, status, restart, logs, files.

use std::fmt::Write;

use clap::Subcommand;
use futures_util::StreamExt;
use moatless_api_client::types::{
    CreateTaskRequest, ListTasksResponse, SandboxStatusFullResponse, TaskResponse, TaskStatus,
};

use crate::client::{map_api_error, ClientError, MoatClient};
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum TaskAction {
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value = "50")]
        limit: i64,
    },
    /// Get task details
    Get { task_id: String },
    /// Create a new task
    Create {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        /// Do not auto-set parent_task_id from TASK_ID env
        #[arg(long)]
        no_parent: bool,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        branch: Option<String>,
    },
    /// List child tasks
    Children {
        /// Parent task ID (defaults to current TASK_ID)
        task_id: Option<String>,
    },
    /// Start a task
    Start { task_id: String },
    /// Stop a task
    Stop { task_id: String },
    /// Close a task
    Close { task_id: String },
    /// Show full task status (environment, agent, servers)
    Status { task_id: String },
    /// Restart sandbox for a task
    Restart { task_id: String },
    /// Stream server logs from a task's sandbox
    Logs {
        task_id: String,
        /// Server name (e.g. "sandbox", "preview-server", "files-server")
        #[arg(long, short = 's', default_value = "sandbox")]
        server: String,
    },
    /// File operations in a task's workspace
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
    action: TaskAction,
    json: bool,
    config: &Config,
) -> Result<(), ClientError> {
    match action {
        TaskAction::List { status, limit } => {
            let task_status = status.as_deref().and_then(parse_task_status);
            let resp = client
                .api()
                .list_tasks(None, Some(limit), None, None, None, task_status, None)
                .await
                .map_err(map_api_error)?;
            let list = resp.into_inner();
            if json {
                output::print_json_typed(&list);
            } else {
                format_task_list(&list);
            }
        }
        TaskAction::Get { task_id } => {
            let resp = client
                .api()
                .get_task(&task_id)
                .await
                .map_err(map_api_error)?;
            let task = resp.into_inner();
            if json {
                output::print_json_typed(&task);
            } else {
                format_task_detail(&task);
            }
        }
        TaskAction::Create {
            repo,
            name,
            parent,
            no_parent,
            message,
            branch,
        } => {
            let parent_task_id = if no_parent {
                None
            } else {
                parent.or_else(|| config.task_id().map(String::from))
            };

            let repo_id = resolve_repo_id(client, &repo).await?;

            let req = CreateTaskRequest {
                repository_id: repo_id,
                name,
                message,
                branch,
                parent_task_id,
                agent_mode: None,
                agent_type: None,
                contexts: Vec::new(),
                description: None,
                file_ids: Vec::new(),
                model: None,
            };
            let resp = client
                .api()
                .create_task(&req)
                .await
                .map_err(map_api_error)?;
            let created = resp.into_inner();
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Task created: {}", created.task_id);
            }
        }
        TaskAction::Children { task_id } => {
            let parent = task_id
                .or_else(|| config.task_id().map(String::from))
                .ok_or_else(|| ClientError::Api {
                    status: 400,
                    body: "No task_id provided and TASK_ID not set".to_string(),
                })?;
            let resp = client
                .api()
                .list_tasks(None, None, None, Some(&parent), None, None, None)
                .await
                .map_err(map_api_error)?;
            let list = resp.into_inner();
            if json {
                output::print_json_typed(&list);
            } else {
                format_task_list(&list);
            }
        }
        TaskAction::Start { task_id } => {
            client
                .api()
                .start_task(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Task {task_id} started.");
        }
        TaskAction::Stop { task_id } => {
            client
                .api()
                .stop_task(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Task {task_id} stopped.");
        }
        TaskAction::Close { task_id } => {
            client
                .api()
                .close_task(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Task {task_id} closed.");
        }
        TaskAction::Status { task_id } => {
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
        TaskAction::Restart { task_id } => {
            client
                .api()
                .restart_handler(&task_id)
                .await
                .map_err(map_api_error)?;
            println!("Sandbox for {task_id} restarted.");
        }
        TaskAction::Logs { task_id, server } => {
            stream_logs(client, &task_id, &server).await?;
        }
        TaskAction::Files { task_id, action } => {
            run_file_action(client, &task_id, action, json).await?;
        }
    }
    Ok(())
}

fn parse_task_status(s: &str) -> Option<TaskStatus> {
    match s {
        "open" => Some(TaskStatus::Open),
        "closed" => Some(TaskStatus::Closed),
        "error" => Some(TaskStatus::Error),
        _ => None,
    }
}

fn format_task_list(list: &ListTasksResponse) {
    let rows: Vec<Vec<String>> = list.items.iter().map(format_task_row).collect();
    output::print_table(&["TASK_ID", "STATUS", "TITLE", "REPO"], &rows);
}

fn format_task_row(t: &TaskResponse) -> Vec<String> {
    vec![
        t.task_id.clone(),
        t.status.to_string(),
        t.title.chars().take(50).collect(),
        t.repository_name.as_deref().unwrap_or("-").to_string(),
    ]
}

/// Resolve a repo argument to a repository ID.
/// If it looks like a UUID, pass through. Otherwise, search by name.
async fn resolve_repo_id(client: &MoatClient, repo_arg: &str) -> Result<String, ClientError> {
    // Simple UUID check: 36 chars with 4 hyphens.
    if repo_arg.len() == 36 && repo_arg.chars().filter(|c| *c == '-').count() == 4 {
        return Ok(repo_arg.to_string());
    }

    let resp = client
        .api()
        .list_repositories()
        .await
        .map_err(map_api_error)?;
    let repos = resp.into_inner();

    let matches: Vec<_> = repos
        .iter()
        .filter(|r| r.name.eq_ignore_ascii_case(repo_arg))
        .collect();

    match matches.len() {
        1 => Ok(matches[0].id.clone()),
        0 => Err(ClientError::Api {
            status: 404,
            body: format!(
                "No repository found with name '{repo_arg}'. Run 'moat repos list' to see available repositories."
            ),
        }),
        _ => {
            let names: Vec<_> = matches.iter().map(|r| format!("{} ({})", r.name, r.id)).collect();
            Err(ClientError::Api {
                status: 400,
                body: format!(
                    "Multiple repositories match '{repo_arg}': {}",
                    names.join(", ")
                ),
            })
        }
    }
}

fn format_task_detail(t: &TaskResponse) {
    println!("Task:    {}", t.task_id);
    println!("Title:   {}", t.title);
    println!("Status:  {}", t.status);
    println!(
        "Repo:    {}",
        t.repository_name.as_deref().unwrap_or("-")
    );
    println!("Branch:  {}", t.branch.as_deref().unwrap_or("-"));
    println!("Created: {}", t.created_at);
    println!("Cost:    ${:.4}", t.total_cost_usd);
    println!(
        "Parent:  {}",
        t.parent_task_id.as_deref().unwrap_or("none")
    );
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
                format_file_tree(&resp);
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
fn format_file_tree(resp: &serde_json::Value) {
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
