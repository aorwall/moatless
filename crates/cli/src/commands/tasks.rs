//! Task commands: list, get, create, children, start, stop, close.

use clap::Subcommand;

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
}

pub async fn run(
    client: &MoatClient,
    action: TaskAction,
    json: bool,
    config: &Config,
) -> Result<(), ClientError> {
    match action {
        TaskAction::List { status, limit } => {
            let mut path = format!("/api/v1/tasks?limit={limit}");
            if let Some(ref s) = status {
                use std::fmt::Write;
                let _ = write!(path, "&status={s}");
            }
            let resp = client.get(&path).await?;
            if json {
                output::print_json(&resp);
            } else {
                format_task_list(&resp);
            }
        }
        TaskAction::Get { task_id } => {
            let resp = client.get(&format!("/api/v1/tasks/{task_id}")).await?;
            if json {
                output::print_json(&resp);
            } else {
                format_task_detail(&resp);
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

            let mut body = serde_json::json!({
                "repositoryId": repo,
            });
            if let Some(ref n) = name {
                body["name"] = serde_json::json!(n);
            }
            if let Some(ref p) = parent_task_id {
                body["parentTaskId"] = serde_json::json!(p);
            }
            if let Some(ref m) = message {
                body["message"] = serde_json::json!(m);
            }
            if let Some(ref b) = branch {
                body["branch"] = serde_json::json!(b);
            }

            let resp = client.post("/api/v1/tasks", &body).await?;
            if json {
                output::print_json(&resp);
            } else {
                let task_id = resp
                    .get("taskId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                println!("Task created: {task_id}");
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
                .get(&format!("/api/v1/tasks?parentTaskId={parent}"))
                .await?;
            if json {
                output::print_json(&resp);
            } else {
                format_task_list(&resp);
            }
        }
        TaskAction::Start { task_id } => {
            client
                .post_empty(&format!("/api/v1/tasks/{task_id}/start"))
                .await?;
            println!("Task {task_id} started.");
        }
        TaskAction::Stop { task_id } => {
            client
                .post_empty(&format!("/api/v1/tasks/{task_id}/stop"))
                .await?;
            println!("Task {task_id} stopped.");
        }
        TaskAction::Close { task_id } => {
            client
                .post_empty(&format!("/api/v1/tasks/{task_id}/close"))
                .await?;
            println!("Task {task_id} closed.");
        }
    }
    Ok(())
}

fn format_task_list(resp: &serde_json::Value) {
    let items = resp
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|t| {
            vec![
                t.get("taskId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                t.get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                t.get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .chars()
                    .take(50)
                    .collect(),
                t.get("repositoryName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
            ]
        })
        .collect();

    output::print_table(&["TASK_ID", "STATUS", "TITLE", "REPO"], &rows);
}

fn format_task_detail(resp: &serde_json::Value) {
    let id = resp.get("taskId").and_then(|v| v.as_str()).unwrap_or("-");
    let status = resp.get("status").and_then(|v| v.as_str()).unwrap_or("-");
    let title = resp.get("title").and_then(|v| v.as_str()).unwrap_or("-");
    let repo = resp
        .get("repositoryName")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let branch = resp.get("branch").and_then(|v| v.as_str()).unwrap_or("-");
    let created_at = resp
        .get("createdAt")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let cost = resp
        .get("totalCostUsd")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let parent = resp
        .get("parentTaskId")
        .and_then(|v| v.as_str())
        .unwrap_or("none");

    println!("Task:    {id}");
    println!("Title:   {title}");
    println!("Status:  {status}");
    println!("Repo:    {repo}");
    println!("Branch:  {branch}");
    println!("Created: {created_at}");
    println!("Cost:    ${cost:.4}");
    println!("Parent:  {parent}");
}
