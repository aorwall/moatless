//! `moat linear` — Linear issue/comment commands via the workspace OAuth
//! token held by the backend. Wraps the typed `/api/v1/linear/*` endpoints
//! and adds a `graphql` passthrough for anything not yet wrapped.

use std::collections::HashMap;

use clap::Subcommand;
use moatless_api_schemas::linear::{
    LinearCommentCreateRequest, LinearGraphqlRequest, LinearIssueCreateRequest,
    LinearIssueUpdateRequest, LinearLabelMutationRequest, LinearLabelOp,
};
use serde_json::Value;

use crate::client::MoatClient;
use crate::commands::input::read_text_or_stdin;
use crate::config::Config;
use crate::output;

#[derive(Subcommand)]
pub enum LinearAction {
    /// Issue management
    Issue {
        #[command(subcommand)]
        action: Box<IssueAction>,
    },
    /// Resolve a Linear entity name to its UUID
    Lookup {
        /// One of: state, label, project, user, team
        kind: String,
        /// Name to resolve (case-insensitive)
        name: String,
        /// Optional Linear connection ID (required only when more than one is accessible)
        #[arg(long)]
        connection: Option<String>,
    },
    /// Raw GraphQL passthrough — read from positional arg or stdin
    Graphql {
        /// GraphQL query/mutation. Reads from stdin if omitted.
        query: Option<String>,
        /// Variable in the form KEY=VALUE (repeatable). Values are parsed
        /// as JSON when valid, otherwise treated as strings.
        #[arg(long = "var")]
        vars: Vec<String>,
        /// Optional Linear connection ID
        #[arg(long)]
        connection: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum IssueAction {
    /// Fetch one issue by identifier (SP-123) or UUID
    Get {
        id: String,
        #[arg(long)]
        connection: Option<String>,
    },
    /// Create an issue
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: Option<String>,
        /// Team key (`SP`) or UUID. Defaults to the only accessible team.
        #[arg(long)]
        team: Option<String>,
        /// Workflow state name (`Ready`, `In Review`) or UUID
        #[arg(long)]
        state: Option<String>,
        /// Project name or UUID
        #[arg(long)]
        project: Option<String>,
        /// Label name or UUID (repeatable)
        #[arg(long = "label")]
        labels: Vec<String>,
        /// Assignee email, display name, `me`, or UUID
        #[arg(long)]
        assignee: Option<String>,
        /// Reporter email, display name, `me`, or UUID. Added as a subscriber
        /// and mentioned in the description (records who requested the issue).
        #[arg(long)]
        reporter: Option<String>,
        /// 0 No-priority, 1 Urgent, 2 High, 3 Normal, 4 Low
        #[arg(long)]
        priority: Option<i32>,
        /// T-shirt estimate (1=XS, 2=S, 3=M, 5=L, 8=XL)
        #[arg(long)]
        estimate: Option<f64>,
        /// Due date YYYY-MM-DD
        #[arg(long = "due")]
        due_date: Option<String>,
        /// Parent issue SP-N or UUID
        #[arg(long)]
        parent: Option<String>,
        /// Cycle number or `active`
        #[arg(long)]
        cycle: Option<String>,
        /// Optional template UUID
        #[arg(long)]
        template_id: Option<String>,
        /// Task to associate the new issue with (defaults to `$TASK_ID`).
        /// Records a traceability attachment; does NOT make the issue a reply
        /// target unless `--bind` is also passed.
        #[arg(long = "task")]
        task: Option<String>,
        /// Make the created issue a reply target (creates a task binding so
        /// future comments route back to the task). Off by default — a filed
        /// ticket is a work artifact, not a conversation channel.
        #[arg(long)]
        bind: bool,
        #[arg(long)]
        connection: Option<String>,
    },
    /// Update an issue (any subset of fields)
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        priority: Option<i32>,
        #[arg(long)]
        estimate: Option<f64>,
        #[arg(long = "due")]
        due_date: Option<String>,
        #[arg(long)]
        cycle: Option<String>,
        /// Pass `null` (literal) to clear
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        connection: Option<String>,
    },
    /// Shorthand for `update --state <state-name>`
    Move {
        id: String,
        /// Workflow state name or UUID
        state: String,
        #[arg(long)]
        connection: Option<String>,
    },
    /// Post a comment on an issue
    Comment {
        id: String,
        /// Comment body. Reads from stdin if omitted.
        body: Option<String>,
        /// Task to associate with (defaults to `$TASK_ID`). A comment on an
        /// already-bound issue refreshes its binding automatically.
        #[arg(long = "task")]
        task: Option<String>,
        /// Force a reply binding even when the issue isn't already bound (for
        /// a proactive comment on an issue you intend to keep following).
        #[arg(long)]
        bind: bool,
        #[arg(long)]
        connection: Option<String>,
    },
    /// Non-destructive label add / remove
    Label {
        #[command(subcommand)]
        action: LabelAction,
    },
}

#[derive(Subcommand)]
pub enum LabelAction {
    /// Add a label to an issue
    Add {
        id: String,
        /// Label name or UUID
        label: String,
        #[arg(long)]
        connection: Option<String>,
    },
    /// Remove a label from an issue
    Remove {
        id: String,
        label: String,
        #[arg(long)]
        connection: Option<String>,
    },
}

pub async fn run(
    client: &MoatClient,
    cfg: &Config,
    action: LinearAction,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        LinearAction::Issue { action } => run_issue(client, cfg, *action, json).await,
        LinearAction::Lookup {
            kind,
            name,
            connection,
        } => run_lookup(client, &kind, &name, connection.as_deref(), json).await,
        LinearAction::Graphql {
            query,
            vars,
            connection,
        } => run_graphql(client, query, &vars, connection.as_deref(), json).await,
    }
}

async fn run_issue(
    client: &MoatClient,
    cfg: &Config,
    action: IssueAction,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        IssueAction::Get { id, connection } => {
            let resp = client
                .api()
                .linear_get_issue(&id, connection.as_deref())
                .await?;
            print_issue(&resp.data, json);
            Ok(())
        }
        IssueAction::Create {
            title,
            description,
            team,
            state,
            project,
            labels,
            assignee,
            reporter,
            priority,
            estimate,
            due_date,
            parent,
            cycle,
            template_id,
            task,
            bind,
            connection,
        } => {
            let task_id = task.or_else(|| cfg.task_id().map(str::to_string));
            let req = LinearIssueCreateRequest {
                title,
                description,
                team,
                state,
                project,
                labels,
                assignee,
                reporter,
                priority,
                estimate,
                due_date,
                parent,
                cycle,
                template_id,
                task_id,
                bind: bind.then_some(true),
            };
            let resp = client
                .api()
                .linear_create_issue(&req, connection.as_deref())
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                print_issue(&resp.data, false);
                if let Some(bid) = resp.binding_id {
                    println!("\nBound to task via: {bid}");
                }
            }
            Ok(())
        }
        IssueAction::Update {
            id,
            title,
            description,
            state,
            project,
            assignee,
            priority,
            estimate,
            due_date,
            cycle,
            parent,
            connection,
        } => {
            let req = LinearIssueUpdateRequest {
                title,
                description,
                state,
                project,
                assignee,
                priority,
                estimate,
                due_date,
                cycle,
                parent,
            };
            let resp = client
                .api()
                .linear_update_issue(&id, &req, connection.as_deref())
                .await?;
            print_issue(&resp.data, json);
            Ok(())
        }
        IssueAction::Move {
            id,
            state,
            connection,
        } => {
            let req = LinearIssueUpdateRequest {
                state: Some(state),
                ..Default::default()
            };
            let resp = client
                .api()
                .linear_update_issue(&id, &req, connection.as_deref())
                .await?;
            print_issue(&resp.data, json);
            Ok(())
        }
        IssueAction::Comment {
            id,
            body,
            task,
            bind,
            connection,
        } => {
            let body = read_text_or_stdin(body)?;
            let task_id = task.or_else(|| cfg.task_id().map(str::to_string));
            let req = LinearCommentCreateRequest {
                body,
                task_id,
                bind: bind.then_some(true),
            };
            let resp = client
                .api()
                .linear_create_comment(&id, &req, connection.as_deref())
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                let url = resp.data.get("url").and_then(Value::as_str).unwrap_or("");
                println!("Comment posted: {url}");
                if let Some(bid) = resp.binding_id {
                    println!("Bound to task via: {bid}");
                }
            }
            Ok(())
        }
        IssueAction::Label { action } => run_label(client, action, json).await,
    }
}

async fn run_label(
    client: &MoatClient,
    action: LabelAction,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (id, label, op, connection) = match action {
        LabelAction::Add {
            id,
            label,
            connection,
        } => (id, label, LinearLabelOp::Add, connection),
        LabelAction::Remove {
            id,
            label,
            connection,
        } => (id, label, LinearLabelOp::Remove, connection),
    };
    let resp = client
        .api()
        .linear_mutate_label(
            &id,
            &LinearLabelMutationRequest { label, op },
            connection.as_deref(),
        )
        .await?;
    if json {
        output::print_json_typed(&resp);
    } else {
        println!("ok");
    }
    Ok(())
}

async fn run_lookup(
    client: &MoatClient,
    kind: &str,
    name: &str,
    connection: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let resp = client.api().linear_lookup(kind, name, connection).await?;
    if json {
        output::print_json_typed(&resp);
    } else {
        match (resp.id.as_deref(), resp.name.as_deref()) {
            (Some(id), Some(name)) => println!("{id}\t{name}"),
            _ => println!("(no match)"),
        }
    }
    Ok(())
}

async fn run_graphql(
    client: &MoatClient,
    query: Option<String>,
    vars: &[String],
    connection: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let query = read_text_or_stdin(query)?;
    let variables = if vars.is_empty() {
        None
    } else {
        let mut map: HashMap<String, Value> = HashMap::new();
        for raw in vars {
            let (k, v) = raw
                .split_once('=')
                .ok_or_else(|| format!("--var must be KEY=VALUE, got `{raw}`"))?;
            let parsed =
                serde_json::from_str::<Value>(v).unwrap_or_else(|_| Value::String(v.to_string()));
            map.insert(k.to_string(), parsed);
        }
        Some(serde_json::to_value(map)?)
    };
    let req = LinearGraphqlRequest { query, variables };
    let resp = client.api().linear_graphql(&req, connection).await?;
    let _ = json;
    // GraphQL responses are arbitrary — always print JSON.
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

/// Pretty-print an issue payload. JSON mode prints the raw value;
/// human mode prints the key fields.
fn print_issue(issue: &Value, json: bool) {
    if json {
        output::print_json_typed(&issue);
        return;
    }
    let identifier = issue
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let title = issue.get("title").and_then(Value::as_str).unwrap_or("");
    let state = issue
        .pointer("/state/name")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let assignee = issue
        .pointer("/assignee/displayName")
        .or_else(|| issue.pointer("/assignee/name"))
        .and_then(Value::as_str)
        .unwrap_or("unassigned");
    let project = issue
        .pointer("/project/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let labels: Vec<&str> = issue
        .pointer("/labels/nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|n| n.get("name").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    let url = issue.get("url").and_then(Value::as_str).unwrap_or("");

    println!("{identifier}  {title}");
    println!("  state:    {state}");
    println!("  assignee: {assignee}");
    if !project.is_empty() {
        println!("  project:  {project}");
    }
    if !labels.is_empty() {
        println!("  labels:   {}", labels.join(", "));
    }
    if !url.is_empty() {
        println!("  url:      {url}");
    }
}
