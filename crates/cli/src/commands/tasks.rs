//! Task commands: list, get, create, children, start, stop, close, status, restart, logs, files.

use std::collections::BTreeMap;
use std::fmt::Write;

use clap::Subcommand;
use futures_util::StreamExt;
use moatless_api_schemas::enums::TaskStatus;
use moatless_api_schemas::sandbox::{
    SandboxStatusFullResponse, ServerConfigResponse, ServerOverride, ServerOverridesResponse,
    ServerStatusResponse,
    UpdateServerConfigRequest,
};
use moatless_api_schemas::shared::AgentMode;
use moatless_api_schemas::task::{
    CreateTaskRequest, ListTasksQuery, ListTasksResponse, TaskResponse, UpdateTaskMetadataRequest,
    UpdateTaskTtlRequest,
};

use crate::client::{ClientError, MoatClient};
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
    /// Create a new task (starts the sandbox and, if --message is provided, the agent)
    Create {
        /// Repository ID or name. Optional when --workspace is given (the
        /// workspace's primary repository is used)
        #[arg(long, required_unless_present = "workspace")]
        repo: Option<String>,
        /// Workspace ID (ws_...) or name to run the task in; wins over --repo
        #[arg(long)]
        workspace: Option<String>,
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
        /// Agent mode: "edit" or "plan"
        #[arg(long)]
        mode: Option<String>,
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
    /// Update task name and/or description
    Update {
        task_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Update a task's TTLs (idle sandbox stop and/or inactivity auto-close)
    Ttl {
        task_id: String,
        /// Idle TTL in minutes (0 = never stop the sandbox)
        #[arg(long)]
        ttl_minutes: Option<i32>,
        /// Inactivity TTL in days (0 = never auto-close the task)
        #[arg(long)]
        inactive_ttl_days: Option<i32>,
    },
    /// Show full task status (environment, agent, servers)
    Status { task_id: String },
    /// Restart sandbox for a task
    Restart { task_id: String },
    /// Redeploy sandbox (recreate with fresh repo config, preserves workspace data)
    Redeploy { task_id: String },
    /// Stream server logs from a task's sandbox
    Logs {
        task_id: String,
        /// Server name (e.g. "sandbox", "preview", "files")
        #[arg(long, short = 's', default_value = "sandbox")]
        server: String,
        /// Fetch logs from previous (crashed/restarted) container instance
        #[arg(long)]
        previous: bool,
    },
    /// Diagnose sandbox issues (status + container diagnostics + recent errors)
    Diagnose {
        task_id: String,
        /// Number of error events to show
        #[arg(long, default_value = "10")]
        limit: i64,
    },
    /// Fetch preview server page content
    Preview {
        task_id: String,
        /// Server name (e.g. "preview", "files")
        #[arg(long, short = 's', default_value = "preview")]
        server: String,
        /// URL path to fetch (default: "/")
        #[arg(long, default_value = "/")]
        path: String,
    },
    /// File operations in a task's workspace
    Files {
        task_id: String,
        #[command(subcommand)]
        action: FileAction,
    },
    /// Inspect and live-update a task's preview servers
    Servers {
        task_id: String,
        #[command(subcommand)]
        action: ServerAction,
    },
}

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
            let query = ListTasksQuery {
                limit: Some(limit),
                offset: None,
                status: task_status,
                repository_id: None,
                created_by: None,
                tag: None,
                parent_task_id: None,
                followed_by: None,
            };
            let list = client.api().list_tasks(&query).await?;
            if json {
                output::print_json_typed(&list);
            } else {
                format_task_list(&list);
            }
        }
        TaskAction::Get { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let task = client.api().get_task(&task_id).await?;
            if json {
                output::print_json_typed(&task);
            } else {
                format_task_detail(&task);
            }
        }
        TaskAction::Create {
            repo,
            workspace,
            name,
            parent,
            no_parent,
            message,
            branch,
            mode,
        } => {
            let parent_task_id = if no_parent {
                None
            } else {
                parent.or_else(|| config.task_id().map(String::from))
            };

            let repository_id = match &repo {
                Some(r) => Some(resolve_repo_id(client, r).await?),
                None => None,
            };
            let workspace_id = match &workspace {
                Some(w) => Some(resolve_workspace_id(client, w).await?),
                None => None,
            };

            let has_message = message.is_some();
            let req = CreateTaskRequest {
                repository_id,
                workspace_id,
                name,
                message,
                branch,
                parent_task_id,
                agent_mode: mode.map(AgentMode),
                agent_type: None,
                contexts: Vec::new(),
                description: None,
                file_ids: Vec::new(),
                model: None,
                headless: None,
            };
            let created = client.api().create_task(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else if has_message {
                println!(
                    "Task {} created and started (status: {}). Sandbox provisioning; agent will begin once the sandbox is ready.",
                    created.task_id, created.status
                );
            } else {
                println!(
                    "Task {} created (status: {}). Sandbox provisioning; send a message with `moat send --task <id>` to start the agent.",
                    created.task_id, created.status
                );
            }
        }
        TaskAction::Children { task_id } => {
            let parent = match task_id {
                Some(task_ref) => resolve_task_id(client, &task_ref).await?,
                None => config
                    .task_id()
                    .map(String::from)
                    .ok_or_else(|| ClientError::Api {
                        status: 400,
                        body: "No task_id provided and TASK_ID not set".to_string(),
                    })?,
            };
            let query = ListTasksQuery {
                limit: None,
                offset: None,
                status: None,
                repository_id: None,
                created_by: None,
                tag: None,
                parent_task_id: Some(parent),
                followed_by: None,
            };
            let list = client.api().list_tasks(&query).await?;
            if json {
                output::print_json_typed(&list);
            } else {
                format_task_list(&list);
            }
        }
        TaskAction::Start { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            client.api().start_task(&task_id).await?;
            println!("Task {task_id} started.");
        }
        TaskAction::Stop { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            client.api().stop_task(&task_id).await?;
            println!("Task {task_id} stopped.");
        }
        TaskAction::Close { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            client.api().close_task(&task_id).await?;
            println!("Task {task_id} closed.");
        }
        TaskAction::Update {
            task_id,
            name,
            description,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            if name.is_none() && description.is_none() {
                return Err(ClientError::Api {
                    status: 400,
                    body: "Provide --name and/or --description".to_string(),
                });
            }
            let req = UpdateTaskMetadataRequest { name, description };
            let updated = client.api().update_task_metadata(&task_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Task {} updated.", updated.task_id);
            }
        }
        TaskAction::Ttl {
            task_id,
            ttl_minutes,
            inactive_ttl_days,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            if ttl_minutes.is_none() && inactive_ttl_days.is_none() {
                return Err(ClientError::Api {
                    status: 400,
                    body: "Provide --ttl-minutes and/or --inactive-ttl-days".to_string(),
                });
            }
            let req = UpdateTaskTtlRequest {
                ttl_minutes,
                inactive_ttl_days,
            };
            let updated = client.api().update_ttl(&task_id, &req).await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!(
                    "Task {} TTL updated (idle={}m, inactive={}d).",
                    updated.task_id, updated.ttl_minutes, updated.inactive_ttl_days
                );
            }
        }
        TaskAction::Status { task_id } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let status = client.api().sandbox_status(&task_id).await?;
            if json {
                output::print_json_typed(&status);
            } else {
                format_sandbox_status(&status);
            }
        }
        TaskAction::Restart { task_id } => {
            if config.is_sandbox() {
                return Err(ClientError::Api {
                    status: 400,
                    body: "Cannot restart the entire sandbox from within it. Use `moat tasks logs $TASK_ID --server <name>` to diagnose server issues.".to_string(),
                });
            }
            let task_id = resolve_task_id(client, &task_id).await?;
            client.api().restart_sandbox(&task_id).await?;
            println!("Sandbox for {task_id} restarted.");
        }
        TaskAction::Redeploy { task_id } => {
            if config.is_sandbox() {
                return Err(ClientError::Api {
                    status: 400,
                    body: "Cannot redeploy the sandbox from within it.".to_string(),
                });
            }
            let task_id = resolve_task_id(client, &task_id).await?;
            let resp = client.api().redeploy_sandbox(&task_id).await?;
            println!(
                "Sandbox for {task_id} redeployed (status: {:?}).",
                resp.status
            );
        }
        TaskAction::Logs {
            task_id,
            server,
            previous,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            stream_logs(client, &task_id, &server, previous).await?;
        }
        TaskAction::Diagnose { task_id, limit } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            run_diagnose(client, &task_id, limit, json).await?;
        }
        TaskAction::Preview {
            task_id,
            server,
            path,
        } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            let status = client.api().sandbox_status(&task_id).await?;
            let entry = status
                .servers
                .iter()
                .find(|s| s.name == server)
                .ok_or_else(|| ClientError::Api {
                    status: 404,
                    body: format!(
                        "No server '{}' found. Available: {}",
                        server,
                        status
                            .servers
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })?;
            let base_url = entry.url.as_deref().ok_or_else(|| ClientError::Api {
                status: 404,
                body: format!(
                    "Server '{}' has no URL (status: {:?})",
                    server, entry.status
                ),
            })?;
            let url = format!("{}{}", base_url.trim_end_matches('/'), path);
            let body = client.raw_get_url(&url).await?;
            print!("{body}");
        }
        TaskAction::Files { task_id, action } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            run_file_action(client, &task_id, action, json).await?;
        }
        // Deliberately allowed from inside the sandbox, unlike `restart`/`redeploy`:
        // these only restart a preview sidecar, never the agent container.
        TaskAction::Servers { task_id, action } => {
            let task_id = resolve_task_id(client, &task_id).await?;
            run_server_action(client, &task_id, action, json).await?;
        }
    }
    Ok(())
}

async fn run_server_action(
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
            no_restart,
        } => {
            if start_command.is_empty() && env.is_empty() && unset.is_empty() {
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
                port: None,
                docker_image: None,
            };
            let resp = client.api().update_server_config(task_id, &server, &req).await?;
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
        return;
    }
    for server in &status.servers {
        println!("{} ({}) port {}", server.name, server.status, server.port);
        if let Some(error) = &server.error {
            println!("  error: {error}");
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
    output::print_table(&["TASK_ID", "SHORT", "STATUS", "TITLE", "REPO"], &rows);
}

fn format_task_row(t: &TaskResponse) -> Vec<String> {
    vec![
        t.task_id.clone(),
        t.short_id.as_deref().unwrap_or("-").to_string(),
        t.status.to_string(),
        t.title.chars().take(50).collect(),
        t.repository_name.as_deref().unwrap_or("-").to_string(),
    ]
}

/// True if `s` looks like a full task_id (UUID: 36 chars with 4 hyphens).
fn looks_like_task_id(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4
}

/// Resolve a task reference to a full task_id.
///
/// Accepts either a full task_id (UUID — passed through) or the 8-char shortId
/// shown in the UI, which is looked up among the most recent tasks. Short ids
/// are unique, so the first match wins.
async fn resolve_task_id(client: &MoatClient, task_ref: &str) -> Result<String, ClientError> {
    if looks_like_task_id(task_ref) {
        return Ok(task_ref.to_string());
    }
    let query = ListTasksQuery {
        limit: Some(200),
        offset: None,
        status: None,
        repository_id: None,
        created_by: None,
        tag: None,
        parent_task_id: None,
        followed_by: None,
    };
    let list = client.api().list_tasks(&query).await?;
    match list
        .items
        .iter()
        .find(|t| t.short_id.as_deref() == Some(task_ref))
    {
        Some(t) => Ok(t.task_id.clone()),
        None => Err(ClientError::Api {
            status: 404,
            body: format!(
                "No task found for '{task_ref}'. Pass a full task_id (UUID), or run 'moat tasks list' — short ids resolve only among the 200 most recent tasks."
            ),
        }),
    }
}

/// Resolve a repo argument to a repository ID.
/// If it looks like a UUID, pass through. Otherwise, search by name.
async fn resolve_repo_id(client: &MoatClient, repo_arg: &str) -> Result<String, ClientError> {
    // Simple UUID check: 36 chars with 4 hyphens.
    if repo_arg.len() == 36 && repo_arg.chars().filter(|c| *c == '-').count() == 4 {
        return Ok(repo_arg.to_string());
    }

    let repos = client.api().list_repositories().await?;

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

/// Resolve a workspace argument to a workspace ID.
/// `ws_`-prefixed IDs pass through. Otherwise, search by name.
async fn resolve_workspace_id(client: &MoatClient, ws_arg: &str) -> Result<String, ClientError> {
    if ws_arg.starts_with("ws_") {
        return Ok(ws_arg.to_string());
    }

    let workspaces = client.api().list_workspaces().await?;

    let matches: Vec<_> = workspaces
        .iter()
        .filter(|w| w.name.eq_ignore_ascii_case(ws_arg))
        .collect();

    match matches.len() {
        1 => Ok(matches[0].id.clone()),
        0 => Err(ClientError::Api {
            status: 404,
            body: format!(
                "No workspace found with name '{ws_arg}'. Run 'moat workspaces list' to see available workspaces."
            ),
        }),
        _ => {
            let names: Vec<_> = matches.iter().map(|w| format!("{} ({})", w.name, w.id)).collect();
            Err(ClientError::Api {
                status: 400,
                body: format!("Multiple workspaces match '{ws_arg}': {}", names.join(", ")),
            })
        }
    }
}

fn format_task_detail(t: &TaskResponse) {
    println!("Task:    {}", t.task_id);
    println!("ShortId: {}", t.short_id.as_deref().unwrap_or("-"));
    println!("Title:   {}", t.title);
    println!("Status:  {}", t.status);
    println!("Repo:    {}", t.repository_name.as_deref().unwrap_or("-"));
    println!("Branch:  {}", t.branch.as_deref().unwrap_or("-"));
    println!("Created: {}", t.created_at);
    println!("Cost:    ${:.4}", t.total_cost_usd);
    println!("Parent:  {}", t.parent_task_id.as_deref().unwrap_or("none"));
    if !t.relations.is_empty() {
        println!("Relations:");
        for r in &t.relations {
            let via = r
                .context
                .get("via")
                .and_then(|v| v.as_str())
                .map(|s| format!(" (via {s})"))
                .unwrap_or_default();
            println!("  {} → {}{}", r.relation_type.as_str(), r.to_task_id, via);
        }
    }
}

fn format_sandbox_status(status: &SandboxStatusFullResponse) {
    println!("Sandbox:    {}", status.sandbox_status);
    if let Some(ref err) = status.sandbox_error {
        println!("Error:       {err}");
    }
    let agent_status = status
        .agent_status
        .map_or_else(|| "n/a".to_string(), |value| value.to_string());
    println!("Agent:       {agent_status}");

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

    format_container_diagnostics_json(None);
}

fn format_container_diagnostics_json(diagnostics: Option<&Vec<serde_json::Value>>) {
    let items = match diagnostics {
        Some(items) if !items.is_empty() => items,
        _ => return,
    };

    println!("\nContainer Diagnostics:");
    for c in items {
        let name = c
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        let state = c
            .get("state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        let restarts = c
            .get("restartCount")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        println!("  {name}:");
        println!("    State:          {state}");
        println!("    Restart Count:  {restarts}");

        if let Some(code) = c.get("exitCode").and_then(serde_json::Value::as_i64) {
            println!("    Exit Code:      {code}");
        }
        if let Some(reason) = c
            .get("terminatedReason")
            .and_then(serde_json::Value::as_str)
        {
            println!("    Terminated:     {reason}");
        }
        if let Some(code) = c.get("lastExitCode").and_then(serde_json::Value::as_i64) {
            println!("    Last Exit Code: {code}");
        }
        if let Some(reason) = c
            .get("lastTerminatedReason")
            .and_then(serde_json::Value::as_str)
        {
            println!("    Last Terminated: {reason}");
        }
        if let Some(reason) = c.get("waitingReason").and_then(serde_json::Value::as_str) {
            println!("    Waiting:        {reason}");
        }
        // Resource usage (from K8s Metrics API)
        let has_cpu = c.get("cpuMillicores").and_then(serde_json::Value::as_u64);
        let has_mem = c.get("memoryMb").and_then(serde_json::Value::as_u64);
        if has_cpu.is_some() || has_mem.is_some() {
            if let Some(cpu) = has_cpu {
                println!("    CPU:            {cpu}m");
            }
            if let Some(mem) = has_mem {
                println!("    Memory:         {mem}Mi");
            }
        }
    }
}

/// Diagnose sandbox issues: aggregates status + container diagnostics + recent errors.
async fn run_diagnose(
    client: &MoatClient,
    task_id: &str,
    limit: i64,
    json: bool,
) -> Result<(), ClientError> {
    // Fetch status (includes container diagnostics from Phase 1)
    let status_path = format!("/api/sandbox/v1/tasks/{task_id}/status");
    let status = client.raw_get(&status_path).await?;

    // Fetch recent error events
    let events_path =
        format!("/api/v1/tasks/{task_id}/events/history?limit={limit}&types=sandbox,server");
    let events_resp = client.raw_get(&events_path).await;

    if json {
        let mut report = serde_json::json!({
            "taskId": task_id,
            "status": status,
        });
        if let Ok(ref events) = events_resp {
            report["events"] = events.clone();
        }
        let pretty = serde_json::to_string_pretty(&report).unwrap_or_default();
        println!("{pretty}");
        return Ok(());
    }

    // Format human-readable report
    println!("=== Sandbox Diagnostics: {task_id} ===\n");

    // Status section
    println!("Status:");
    let sandbox_status = status
        .get("sandboxStatus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    println!("  Sandbox:      {sandbox_status}");

    let agent_status = status
        .get("agentStatus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("n/a");
    println!("  Agent:        {agent_status}");

    if let Some(err) = status
        .get("sandboxError")
        .and_then(serde_json::Value::as_str)
    {
        println!("  Error:        {err}");
    }

    // Container diagnostics section
    let diagnostics = status
        .get("containerDiagnostics")
        .and_then(serde_json::Value::as_array);
    format_container_diagnostics_json(diagnostics);

    // Runtime events section (from K8s/Docker events)
    if let Some(rt_events) = status
        .get("runtimeEvents")
        .and_then(serde_json::Value::as_array)
    {
        if !rt_events.is_empty() {
            println!("\nRuntime Events:");
            for e in rt_events {
                let reason = e
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let message = e
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let severity = e
                    .get("severity")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("normal");
                let count = e
                    .get("count")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(1);
                let ts = e
                    .get("lastTimestamp")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let ts = if ts.len() > 19 { &ts[..19] } else { ts };
                let sev_tag = if severity == "warning" { " [WARN]" } else { "" };
                println!("  {ts}  {reason}{sev_tag} (x{count})  {message}");
            }
        }
    }

    // Servers section
    if let Some(servers) = status.get("servers").and_then(serde_json::Value::as_array) {
        if !servers.is_empty() {
            println!("\nServers:");
            for s in servers {
                let name = s
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let srv_status = s
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let port = s
                    .get("port")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                println!("  {name}: {srv_status} (port {port})");
                if let Some(err) = s.get("error").and_then(serde_json::Value::as_str) {
                    println!("    Error: {err}");
                }
            }
        }
    }

    // Recent events section
    if let Ok(events) = events_resp {
        let items = events
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Filter to error events only
        let errors: Vec<_> = items
            .iter()
            .filter(|e| {
                e.get("isError")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            })
            .collect();

        if errors.is_empty() {
            println!("\nNo recent error events.");
        } else {
            println!("\nRecent Error Events:");
            for e in &errors {
                let ts = e
                    .get("timestamp")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let ts = if ts.len() > 19 { &ts[..19] } else { ts };
                let event_type = e
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let sub_type = e
                    .get("subType")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-");
                let payload = e
                    .get("payload")
                    .and_then(|v| {
                        v.get("message")
                            .or_else(|| v.get("error"))
                            .or_else(|| v.get("status"))
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or("");
                println!("  {ts}  {event_type:<8} {sub_type:<10} {payload}");
            }
        }
    }

    Ok(())
}

/// Stream server logs via SSE.
async fn stream_logs(
    client: &MoatClient,
    task_id: &str,
    server: &str,
    previous: bool,
) -> Result<(), ClientError> {
    let mut path = format!("/api/sandbox/v1/tasks/{task_id}/servers/{server}/logs");
    if previous {
        path.push_str("?previous=true");
    }
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
                let content = resp.get("content").and_then(|v| v.as_str()).unwrap_or("");
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

#[cfg(test)]
mod tests {
    use super::looks_like_task_id;

    #[test]
    fn recognizes_full_uuid_task_id() {
        assert!(looks_like_task_id("1f0d0ebb-0a8e-49c2-94ff-77e8ea50b8bb"));
        assert!(looks_like_task_id("873cc644-04af-415d-9e15-fe167f4d15dd"));
    }

    #[test]
    fn rejects_short_id_and_junk() {
        // 8-char shortIds shown in the UI must NOT be treated as full ids.
        assert!(!looks_like_task_id("K9PCBWwH"));
        assert!(!looks_like_task_id("Gs0d_Tcb"));
        assert!(!looks_like_task_id(""));
        // Right length, wrong hyphen count.
        assert!(!looks_like_task_id(
            "1f0d0ebb00a8e049c2094ff077e8ea50b8bbxx"
        ));
    }
}
