//! `moat feedback` — record qualitative feedback about a task, and read back
//! what has been recorded across tasks.

use clap::Subcommand;
use moatless_api_schemas::feedback::{
    CreateFeedbackRequest, FeedbackCategory, FeedbackSource, FeedbackStatus, FeedbackSurface,
    ListAllFeedbackQuery, UpdateFeedbackStatusRequest,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

/// Longest comment rendered in the table before `--full` is needed.
const COMMENT_WIDTH: usize = 90;

#[derive(Subcommand)]
pub enum FeedbackAction {
    /// List feedback across tasks, newest first
    List {
        /// Workspace id of the task the feedback is about
        #[arg(long)]
        workspace: Option<String>,
        /// Repository id of the task the feedback is about
        #[arg(long)]
        repo: Option<String>,
        /// Single task id
        #[arg(long)]
        task: Option<String>,
        /// Who submitted it: `user` or `agent`
        #[arg(long)]
        source: Option<String>,
        /// Where it was recorded: `task`, `message`, `tool_call`, or
        /// `preview_failure`
        #[arg(long)]
        surface: Option<String>,
        /// What it is about: `environment`, `codebase`, `instruction`, or
        /// `other`
        #[arg(long)]
        category: Option<String>,
        /// Triage state: `open`, `fixed`, or `not_relevant`
        #[arg(long)]
        status: Option<String>,
        /// User id of the submitter
        #[arg(long = "created-by")]
        created_by: Option<String>,
        /// Keep feedback from this instant onward: RFC 3339 or `YYYY-MM-DD`
        #[arg(long)]
        since: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i64,
        #[arg(long, default_value_t = 0)]
        offset: i64,
        /// Print every comment in full instead of one truncated row each
        #[arg(long)]
        full: bool,
    },
    /// Set the triage state of one feedback item
    Status {
        /// Feedback id, as printed by `feedback list`
        feedback_id: String,
        /// `open`, `fixed`, or `not_relevant`
        #[arg(long)]
        status: String,
        /// Why. Replaces the note the previous status carried
        #[arg(long)]
        note: Option<String>,
    },
}

pub struct FeedbackArgs {
    pub task_id: String,
    pub comment: String,
    pub category: Option<String>,
    pub message_uuid: Option<String>,
    pub tool_use_id: Option<String>,
}

pub async fn run(client: &MoatClient, args: FeedbackArgs, json: bool) -> Result<(), ClientError> {
    let surface = if args.tool_use_id.is_some() {
        FeedbackSurface::ToolCall
    } else if args.message_uuid.is_some() {
        FeedbackSurface::Message
    } else {
        FeedbackSurface::Task
    };

    let req = CreateFeedbackRequest {
        task_id: args.task_id,
        message_uuid: args.message_uuid,
        tool_use_id: args.tool_use_id,
        surface,
        category: args.category.as_deref().map(parse_category).transpose()?,
        comment: args.comment,
        metadata: None,
    };

    let resp = client.api().submit_feedback(&req).await?;

    if json {
        output::print_json_typed(&resp);
    } else {
        println!("Recorded feedback {}", resp.feedback_id);
    }
    Ok(())
}

pub async fn set_status(
    client: &MoatClient,
    feedback_id: String,
    status: String,
    note: Option<String>,
    json: bool,
) -> Result<(), ClientError> {
    let req = UpdateFeedbackStatusRequest {
        status: parse_status(&status)?,
        note,
    };

    let resp = client.api().set_feedback_status(&feedback_id, &req).await?;

    if json {
        output::print_json_typed(&resp);
    } else {
        println!("Marked {} {}", resp.feedback_id, resp.status);
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "one parameter per CLI filter flag"
)]
pub async fn list(
    client: &MoatClient,
    workspace: Option<String>,
    repo: Option<String>,
    task: Option<String>,
    source: Option<String>,
    surface: Option<String>,
    category: Option<String>,
    status: Option<String>,
    created_by: Option<String>,
    since: Option<String>,
    limit: i64,
    offset: i64,
    full: bool,
    json: bool,
) -> Result<(), ClientError> {
    let query = ListAllFeedbackQuery {
        workspace_id: workspace,
        repository_id: repo,
        task_id: task,
        source: source.as_deref().map(parse_source).transpose()?,
        surface: surface.as_deref().map(parse_surface).transpose()?,
        category: category.as_deref().map(parse_category).transpose()?,
        status: status.as_deref().map(parse_status).transpose()?,
        created_by,
        since,
        limit: Some(limit),
        offset: Some(offset),
    };

    let resp = client.api().list_all_feedback(&query).await?;

    if json {
        output::print_json_typed(&resp);
        return Ok(());
    }

    if resp.items.is_empty() {
        println!("No feedback matches.");
        return Ok(());
    }

    if full {
        for item in &resp.items {
            let f = &item.feedback;
            println!("{}", "-".repeat(100));
            println!(
                "{}  {}/{}  [{}]  {}  {}  {}",
                f.created_at,
                f.source,
                f.surface,
                f.category.map_or("-", |c| c.as_str()),
                f.status,
                item.repository_name
                    .as_deref()
                    .unwrap_or(&item.repository_id),
                item.task_title
            );
            println!("task {}  feedback {}", f.task_id, f.feedback_id);
            if let Some(note) = &f.status_note {
                println!("status note: {note}");
            }
            println!("{}", f.comment);
        }
    } else {
        let rows: Vec<Vec<String>> = resp
            .items
            .iter()
            .map(|item| {
                let f = &item.feedback;
                vec![
                    f.created_at.clone(),
                    f.source.to_string(),
                    f.surface.to_string(),
                    f.category.map_or("-", |c| c.as_str()).to_string(),
                    f.status.to_string(),
                    item.repository_name
                        .clone()
                        .unwrap_or_else(|| item.repository_id.clone()),
                    truncate(&item.task_title, 30),
                    truncate(&f.comment.replace('\n', " "), COMMENT_WIDTH),
                ]
            })
            .collect();
        output::print_table(
            &[
                "CREATED", "SOURCE", "SURFACE", "CATEGORY", "STATUS", "REPO", "TASK", "COMMENT",
            ],
            &rows,
        );
    }

    println!(
        "\n{} of {} (offset {})",
        resp.items.len(),
        resp.total,
        resp.offset
    );
    Ok(())
}

fn parse_source(raw: &str) -> Result<FeedbackSource, ClientError> {
    FeedbackSource::try_from(raw)
        .map_err(|_| ClientError::Other("source must be `user` or `agent`".to_string()))
}

fn parse_surface(raw: &str) -> Result<FeedbackSurface, ClientError> {
    FeedbackSurface::try_from(raw).map_err(|_| {
        ClientError::Other(
            "surface must be `task`, `message`, `tool_call`, or `preview_failure`".to_string(),
        )
    })
}

fn parse_category(raw: &str) -> Result<FeedbackCategory, ClientError> {
    FeedbackCategory::try_from(raw).map_err(|_| {
        ClientError::Other(
            "category must be `environment`, `codebase`, `instruction`, or `other`".to_string(),
        )
    })
}

fn parse_status(raw: &str) -> Result<FeedbackStatus, ClientError> {
    FeedbackStatus::try_from(raw).map_err(|_| {
        ClientError::Other("status must be `open`, `fixed`, or `not_relevant`".to_string())
    })
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let head: String = value.chars().take(width.saturating_sub(1)).collect();
    format!("{head}…")
}

#[derive(clap::Args)]
pub struct FeedbackInvocation {
    #[command(subcommand)]
    pub action: Option<FeedbackAction>,
    /// The feedback message (what went wrong / what could be better).
    #[arg(long, short = 'c')]
    pub comment: Option<String>,
    /// Task the feedback is about. Defaults to `$TASK_ID`.
    #[arg(long)]
    pub task: Option<String>,
    /// What the feedback is about: `environment` (the sandbox), `codebase`
    /// (this repository's code or tooling), `instruction` (`AGENTS.md`,
    /// `CLAUDE.md`, or a skill), or `other`.
    #[arg(long)]
    pub category: Option<String>,
    /// Optional: scope feedback to a specific assistant message
    /// (`conversation_messages.uuid`). Left unset, it best-effort attaches to the
    /// task's most recent activity instead of the whole task.
    #[arg(long)]
    pub message: Option<String>,
    /// Optional: scope feedback to a specific tool call within the referenced
    /// message. Requires `--message`.
    #[arg(long = "tool-use-id")]
    pub tool_use_id: Option<String>,
}

pub async fn dispatch(
    moat: &MoatClient,
    cfg: &crate::config::Config,
    args: FeedbackInvocation,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let FeedbackInvocation {
        action,
        comment,
        task,
        category,
        message,
        tool_use_id,
    } = args;
    match action {
        Some(FeedbackAction::List {
            workspace,
            repo,
            task,
            source,
            surface,
            category,
            status,
            created_by,
            since,
            limit,
            offset,
            full,
        }) => {
            list(
                moat, workspace, repo, task, source, surface, category, status, created_by, since,
                limit, offset, full, json,
            )
            .await?;
        }
        Some(FeedbackAction::Status {
            feedback_id,
            status,
            note,
        }) => {
            set_status(moat, feedback_id, status, note, json).await?;
        }
        None => {
            let comment = comment
                .ok_or("--comment is required to record feedback (or use `feedback list`)")?;
            if tool_use_id.is_some() && message.is_none() {
                return Err("--tool-use-id requires --message".into());
            }
            let task_id = match task {
                Some(id) => id,
                None => cfg.require_task_id()?,
            };
            run(
                moat,
                FeedbackArgs {
                    task_id,
                    comment,
                    category,
                    message_uuid: message,
                    tool_use_id,
                },
                json,
            )
            .await?;
        }
    }
    Ok(())
}
