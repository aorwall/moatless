//! Task commands: list, get, create, children, start, stop, close.

use clap::Subcommand;
use moatless_api_client::types::{CreateTaskRequest, ListTasksResponse, TaskResponse, TaskStatus};

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

            let req = CreateTaskRequest {
                repository_id: repo,
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
