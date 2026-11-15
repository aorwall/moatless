//! `moat linear` — Linear issue/comment commands via the workspace OAuth token
//! held by the backend.
//!
//! Wraps the typed `/api/v1/linear/*` endpoints and adds a `graphql`
//! passthrough for anything not yet wrapped.

use std::collections::HashMap;

use clap::Subcommand;
use moatless_api_schemas::linear::{
    LinearCommentCreateRequest, LinearGraphqlRequest, LinearIssueCreateRequest,
    LinearIssueListParams, LinearIssueRelateRequest, LinearIssueUpdateRequest,
    LinearLabelMutationRequest, LinearLabelOp, LinearRelationType,
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
        /// Team key (SP), name, or UUID. A state, label or project name is only
        /// unique within a team; without this you get whichever team matches
        /// first
        #[arg(long)]
        team: Option<String>,
        /// Optional Linear connection ID (required only when more than one is
        /// accessible)
        #[arg(long)]
        connection: Option<String>,
    },
    /// Raw GraphQL passthrough — read from positional arg or stdin
    Graphql {
        /// GraphQL query/mutation. Reads from stdin if omitted.
        query: Option<String>,
        /// Variable in the form KEY=VALUE (repeatable). Values are parsed as
        /// JSON when valid, otherwise treated as strings.
        #[arg(long = "var")]
        vars: Vec<String>,
        /// Optional Linear connection ID
        #[arg(long)]
        connection: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum IssueAction {
    /// Fetch one issue by identifier (SP-123) or UUID, with its comment thread
    Get {
        id: String,
        #[arg(long)]
        connection: Option<String>,
    },
    /// Create an issue. Linear shows you as its creator, via the app.
    Create {
        #[arg(long)]
        title: String,
        /// Issue description. `--body` is accepted for it, matching
        /// `moat gh pr create`.
        #[arg(long, alias = "body")]
        description: Option<String>,
        /// Read the description from a file.
        #[arg(long = "description-file", alias = "body-file", short = 'F')]
        description_file: Option<std::path::PathBuf>,
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
        /// Moatless login, email, display name, `me`, or UUID. Defaults to
        /// `--reporter`, else you when you are not an app user; an app is never
        /// the default.
        #[arg(long)]
        assignee: Option<String>,
        /// Moatless login, email, display name, `me`, or UUID. Use this for an
        /// app agent: Linear routes an app out of `assignee` into `delegate`.
        #[arg(long)]
        delegate: Option<String>,
        /// Moatless login, email, display name, `me`, or UUID. Added as a
        /// subscriber and mentioned in the description (records who requested
        /// the issue).
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
        /// Replacement description. An empty value is refused — use
        /// `--clear-description` to blank one.
        #[arg(long, alias = "body")]
        description: Option<String>,
        /// Read the replacement description from a file.
        #[arg(long = "description-file", alias = "body-file", short = 'F')]
        description_file: Option<std::path::PathBuf>,
        /// Erase the issue's description.
        #[arg(long, conflicts_with_all = ["description", "description_file"])]
        clear_description: bool,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        project: Option<String>,
        /// Moatless login, email, display name, `me`, or UUID
        #[arg(long)]
        assignee: Option<String>,
        /// App agent to delegate to. Pass `null` (literal) to clear.
        #[arg(long)]
        delegate: Option<String>,
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
        #[arg(value_name = "BODY")]
        body: Option<String>,
        /// The same body as the positional argument, spelled the way
        /// `moat gh issue comment` spells it.
        #[arg(long = "body", short = 'b')]
        body_flag: Option<String>,
        /// Read the body from a file.
        #[arg(long = "body-file", short = 'F')]
        body_file: Option<std::path::PathBuf>,
        /// Task to associate with (defaults to `$TASK_ID`). Replies under the
        /// comment route back to that task automatically.
        #[arg(long = "task")]
        task: Option<String>,
        /// Listen to every comment on the issue, not just replies under this
        /// one (for a ticket you intend to keep following in full).
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
    /// Relate two issues (`related`, `duplicate`, `blocks`, `similar`)
    #[command(after_help = "Examples:\n  \
        moat linear issue relate SP-1 SP-2 --type blocks\n  \
        moat linear issue relate SP-1 blocks SP-2")]
    Relate {
        /// Issue identifier (SP-123) or UUID
        id: String,
        /// The other issue (identifier or UUID), optionally preceded by the
        /// relation type in place of `--type`
        #[arg(value_name = "[TYPE] RELATED_ISSUE", num_args = 1..=2, required = true)]
        rest: Vec<String>,
        /// `blocks` reads left-to-right: `ID` blocks `RELATED_ISSUE`. Defaults
        /// to `related`
        #[arg(long = "type", value_enum)]
        relation: Option<RelationTypeArg>,
        #[arg(long)]
        connection: Option<String>,
    },
    /// List issues matching a filter — newest activity first
    List {
        /// Free text matched against title, description and comments
        #[arg(long = "query")]
        query: Option<String>,
        /// Team key (`SP`) or UUID
        #[arg(long)]
        team: Option<String>,
        /// Project name or UUID
        #[arg(long)]
        project: Option<String>,
        /// Workflow state name (`In Review`) or UUID
        #[arg(long)]
        state: Option<String>,
        /// Label name or UUID
        #[arg(long)]
        label: Option<String>,
        /// Moatless login, email, display name, `me`, or UUID
        #[arg(long)]
        assignee: Option<String>,
        /// Moatless login, email, display name, `me`, or UUID. An issue
        /// delegated to an app agent has no assignee, so `--assignee` alone
        /// misses it.
        #[arg(long)]
        delegate: Option<String>,
        /// Drop issues whose state type is `completed` or `canceled`
        #[arg(long = "open")]
        open_only: bool,
        /// Defaults to 50, capped at 250
        #[arg(long)]
        limit: Option<i64>,
        #[arg(long)]
        connection: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum RelationTypeArg {
    Related,
    Duplicate,
    Blocks,
    Similar,
}

impl From<RelationTypeArg> for LinearRelationType {
    fn from(value: RelationTypeArg) -> Self {
        match value {
            RelationTypeArg::Related => Self::Related,
            RelationTypeArg::Duplicate => Self::Duplicate,
            RelationTypeArg::Blocks => Self::Blocks,
            RelationTypeArg::Similar => Self::Similar,
        }
    }
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
            team,
            connection,
        } => {
            run_lookup(
                client,
                &kind,
                &name,
                team.as_deref(),
                connection.as_deref(),
                json,
            )
            .await
        }
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
            output::emit(json, &resp, || print_issue(&resp.data));
            Ok(())
        }
        IssueAction::Create {
            title,
            description,
            description_file,
            team,
            state,
            project,
            labels,
            assignee,
            delegate,
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
            let description = one_text("description", description, description_file.as_deref())?;
            let req = LinearIssueCreateRequest {
                title,
                description,
                team,
                state,
                project,
                labels,
                assignee,
                delegate,
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
            output::emit(json, &resp, || {
                println!("Created {}", issue_ref(&resp.data));
                if let Some(bid) = &resp.binding_id {
                    println!("Bound to task via: {bid}");
                }
            });
            fail_on_warnings(&resp.warnings)
        }
        IssueAction::Update {
            id,
            title,
            description,
            description_file,
            clear_description,
            state,
            project,
            assignee,
            delegate,
            priority,
            estimate,
            due_date,
            cycle,
            parent,
            connection,
        } => {
            let description = one_text("description", description, description_file.as_deref())?;
            let req = LinearIssueUpdateRequest {
                task_id: cfg.task_id().map(str::to_string),
                title,
                description,
                clear_description: clear_description.then_some(true),
                state,
                project,
                assignee,
                delegate,
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
            output::emit(json, &resp, || {
                println!(
                    "Updated {} ({}) {}",
                    str_at(&resp.data, "/identifier"),
                    updated_fields(&req).join(", "),
                    str_at(&resp.data, "/url")
                );
            });
            fail_on_warnings(&resp.warnings)
        }
        IssueAction::Move {
            id,
            state,
            connection,
        } => {
            // Only the confirmation line reads the previous state, and a failed
            // read must not stop a move that would land.
            let before = if json {
                None
            } else {
                client
                    .api()
                    .linear_get_issue(&id, connection.as_deref())
                    .await
                    .ok()
            };
            let req = LinearIssueUpdateRequest {
                task_id: cfg.task_id().map(str::to_string),
                state: Some(state),
                ..Default::default()
            };
            let resp = client
                .api()
                .linear_update_issue(&id, &req, connection.as_deref())
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                println!(
                    "{}: {} -> {}",
                    str_at(&resp.data, "/identifier"),
                    before
                        .as_ref()
                        .map_or("?", |b| str_at(&b.data, "/state/name")),
                    str_at(&resp.data, "/state/name"),
                );
            }
            Ok(())
        }
        IssueAction::Comment {
            id,
            body,
            body_flag,
            body_file,
            task,
            bind,
            connection,
        } => {
            let body = comment_body(body, body_flag, body_file.as_deref())?;
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
            output::emit(json, &resp, || {
                let url = resp.data.get("url").and_then(Value::as_str).unwrap_or("");
                println!("Comment posted: {url}");
                if let Some(bid) = &resp.binding_id {
                    println!("Bound to task via: {bid}");
                }
            });
            Ok(())
        }
        IssueAction::Label { action } => run_label(client, action, json).await,
        IssueAction::Relate {
            id,
            rest,
            relation,
            connection,
        } => {
            let (relation, related_issue) = relation_and_issue(relation, rest)?;
            let req = LinearIssueRelateRequest {
                related_issue,
                relation: relation.into(),
            };
            let resp = client
                .api()
                .linear_relate_issue(&id, &req, connection.as_deref())
                .await?;
            if json {
                output::print_json_typed(&resp);
            } else {
                let kind = resp.data.get("type").and_then(Value::as_str).unwrap_or("?");
                let left = resp
                    .data
                    .pointer("/issue/identifier")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let right = resp
                    .data
                    .pointer("/relatedIssue/identifier")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                println!("{left} {kind} {right}");
            }
            Ok(())
        }
        IssueAction::List {
            query,
            team,
            project,
            state,
            label,
            assignee,
            delegate,
            open_only,
            limit,
            connection,
        } => {
            let params = LinearIssueListParams {
                query,
                team,
                project,
                state,
                label,
                assignee,
                delegate,
                open_only: open_only.then_some(true),
                limit,
                connection_id: connection,
            };
            let resp = client.api().linear_list_issues(&params).await?;
            if json {
                output::print_json_typed(&resp);
            } else if resp.data.is_empty() {
                println!("(no match)");
            } else {
                for issue in &resp.data {
                    print_issue_row(issue);
                }
            }
            Ok(())
        }
    }
}

/// Every spelling `issue comment` accepts for its body, for the error a caller
/// reads when they passed none of them or more than one.
const BODY_FORMS: &str = "Pass the comment body once: as the positional argument, \
                          `--body <text>`, or `--body-file <path>`";

/// One text value from the inline flag or the file flag, or `None` when the
/// caller passed neither.
///
/// An empty file is refused: `--description-file` over a path that no longer
/// holds what it held is the same accident as `--description "$(cat gone)"`,
/// and Linear reads an empty description as "erase it".
fn one_text(
    field: &str,
    inline: Option<String>,
    file: Option<&std::path::Path>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match (inline, file) {
        (Some(_), Some(_)) => Err(format!(
            "Pass the {field} once: `--{field} <text>` or `--{field}-file <path>`"
        )
        .into()),
        (Some(text), None) => Ok(Some(text)),
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read --{field}-file {}: {e}", path.display()))?;
            if text.trim().is_empty() {
                return Err(format!("--{field}-file {} is empty", path.display()).into());
            }
            Ok(Some(text))
        }
        (None, None) => Ok(None),
    }
}

/// The comment body, from the positional argument, `--body`, `--body-file`, or
/// stdin.
///
/// The positional came first and keeps working; `--body`/`--body-file` are what
/// `moat gh issue comment` next door takes, which is what a caller reaches for.
fn comment_body(
    positional: Option<String>,
    flag: Option<String>,
    file: Option<&std::path::Path>,
) -> Result<String, Box<dyn std::error::Error>> {
    if [positional.is_some(), flag.is_some(), file.is_some()]
        .into_iter()
        .filter(|given| *given)
        .count()
        > 1
    {
        return Err(BODY_FORMS.into());
    }
    let body = match one_text("body", positional.or(flag), file)? {
        Some(text) => text,
        None => read_text_or_stdin(None, BODY_FORMS)?,
    };
    if body.trim().is_empty() {
        return Err(format!("The comment body is empty. {BODY_FORMS}").into());
    }
    Ok(body)
}

/// `SP-123 <url>`: how a write names the issue it changed.
fn issue_ref(issue: &Value) -> String {
    format!("{} {}", str_at(issue, "/identifier"), str_at(issue, "/url"))
        .trim_end()
        .to_string()
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("?")
}

/// The fields an update request writes, named as the flags that set them.
fn updated_fields(req: &LinearIssueUpdateRequest) -> Vec<&'static str> {
    [
        ("title", req.title.is_some()),
        (
            "description",
            req.description.is_some() || req.clear_description.is_some(),
        ),
        ("state", req.state.is_some()),
        ("project", req.project.is_some()),
        ("assignee", req.assignee.is_some()),
        ("delegate", req.delegate.is_some()),
        ("priority", req.priority.is_some()),
        ("estimate", req.estimate.is_some()),
        ("due", req.due_date.is_some()),
        ("cycle", req.cycle.is_some()),
        ("parent", req.parent.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, set)| set.then_some(name))
    .collect()
}

/// An error for a write that landed but lost content, so the command exits
/// non-zero after printing its confirmation.
fn fail_on_warnings(warnings: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if warnings.is_empty() {
        Ok(())
    } else {
        Err(warnings.join("\n").into())
    }
}

/// The relation type and the other issue from `relate`'s trailing arguments:
/// `RELATED_ISSUE` alone, or `TYPE RELATED_ISSUE`.
fn relation_and_issue(
    flag: Option<RelationTypeArg>,
    mut rest: Vec<String>,
) -> Result<(RelationTypeArg, String), Box<dyn std::error::Error>> {
    let related = rest.pop().ok_or("Pass the related issue")?;
    match (rest.pop(), flag) {
        (None, flag) => Ok((flag.unwrap_or(RelationTypeArg::Related), related)),
        (Some(_), Some(_)) => {
            Err("Pass the relation type once: positionally or as `--type`".into())
        }
        (Some(kind), None) => {
            let kind =
                <RelationTypeArg as clap::ValueEnum>::from_str(&kind, true).map_err(|_| {
                    format!(
                        "`{kind}` is not a relation type: use related, duplicate, blocks or similar"
                    )
                })?;
            Ok((kind, related))
        }
    }
}

/// One line per issue, for the listing. `print_issue` stays the detail view.
fn print_issue_row(issue: &Value) {
    let identifier = issue
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let state = issue
        .pointer("/state/name")
        .and_then(Value::as_str)
        .unwrap_or("?");
    // An issue delegated to an app agent has no assignee. Fall back to the
    // delegate so the owner column is never a bare `-` on claimed work.
    let owner = match issue
        .pointer("/assignee/displayName")
        .and_then(Value::as_str)
    {
        Some(a) => a.to_string(),
        None => match issue
            .pointer("/delegate/displayName")
            .and_then(Value::as_str)
        {
            Some(d) => format!("{d} (delegate)"),
            None => "-".to_string(),
        },
    };
    let title = issue.get("title").and_then(Value::as_str).unwrap_or("");
    println!("{identifier}\t{state}\t{owner}\t{title}");
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
            &LinearLabelMutationRequest {
                label: label.clone(),
                op,
            },
            connection.as_deref(),
        )
        .await?;
    output::emit(json, &resp, || {
        let done = match op {
            LinearLabelOp::Add => "added label",
            LinearLabelOp::Remove => "removed label",
        };
        println!("{id}: {done} {label}");
    });
    Ok(())
}

async fn run_lookup(
    client: &MoatClient,
    kind: &str,
    name: &str,
    team: Option<&str>,
    connection: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let resp = client
        .api()
        .linear_lookup(kind, name, team, connection)
        .await?;
    output::emit(json, &resp, || {
        match (resp.id.as_deref(), resp.name.as_deref()) {
            (Some(id), Some(name)) => println!("{id}\t{name}"),
            _ => println!("(no match)"),
        }
    });
    Ok(())
}

async fn run_graphql(
    client: &MoatClient,
    query: Option<String>,
    vars: &[String],
    connection: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let query = read_text_or_stdin(query, "Pass the query as the positional argument")?;
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

/// Pretty-print an issue payload's key fields, for `get`.
fn print_issue(issue: &Value) {
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
    // Printed whenever set: an issue delegated to an app agent keeps
    // `assignee` empty, so without this line a successful delegation reads as
    // "unassigned" and a caller retries or re-claims it.
    let delegate = issue
        .pointer("/delegate/displayName")
        .or_else(|| issue.pointer("/delegate/name"))
        .and_then(Value::as_str)
        .unwrap_or("");
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
    let description = issue
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_end();

    println!("{identifier}  {title}");
    println!("  state:    {state}");
    println!("  assignee: {assignee}");
    if !delegate.is_empty() {
        println!("  delegate: {delegate}");
    }
    if !project.is_empty() {
        println!("  project:  {project}");
    }
    if !labels.is_empty() {
        println!("  labels:   {}", labels.join(", "));
    }
    if !url.is_empty() {
        println!("  url:      {url}");
    }
    if !description.is_empty() {
        println!();
        println!("{description}");
    }
    print_comments(issue);
}

/// The issue's comments, oldest first, or empty for a payload carrying none.
///
/// Linear hands back the newest first, which is the wrong end to start reading
/// a discussion from.
fn comments_oldest_first(issue: &Value) -> Vec<&Value> {
    issue
        .pointer("/comments/nodes")
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().rev().collect())
        .unwrap_or_default()
}

/// A comment's author: a person's display name, or the app that posted it.
///
/// A comment from an OAuth app has `user: null` and names itself in `botActor`.
fn comment_author(comment: &Value) -> &str {
    comment
        .pointer("/user/displayName")
        .or_else(|| comment.pointer("/user/name"))
        .or_else(|| comment.pointer("/botActor/name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

/// What the header says about a thread longer than the selection read.
///
/// Without it the count reads as the thread's length, and a reader catching up
/// on a 60-comment issue takes comment 11 for the opening one.
fn truncation_note(issue: &Value) -> &'static str {
    match issue
        .pointer("/comments/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
    {
        Some(true) => ", most recent shown",
        _ => "",
    }
}

/// The issue's comment thread, for the payloads that carry one.
fn print_comments(issue: &Value) {
    let comments = comments_oldest_first(issue);
    if comments.is_empty() {
        return;
    }
    println!();
    println!(
        "--- {} comment(s){} ---",
        comments.len(),
        truncation_note(issue)
    );
    for comment in comments {
        let author = comment_author(comment);
        let at = comment
            .get("createdAt")
            .and_then(Value::as_str)
            .unwrap_or("");
        let body = comment
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end();
        println!();
        println!("{author}  {at}");
        println!("{body}");
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    /// A parser around [`IssueAction`] alone, so a case names the subcommand
    /// and its flags and nothing else.
    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        action: IssueAction,
    }

    fn parse(args: &[&str]) -> IssueAction {
        TestCli::try_parse_from(std::iter::once("moat-linear").chain(args.iter().copied()))
            .expect("the arguments parse")
            .action
    }

    /// A path under the temp directory nothing else writes, named for the case.
    fn temp_file(case: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("moat-linear-{case}-{}", std::process::id()));
        std::fs::write(&path, contents).expect("the temp directory is writable");
        path
    }

    #[test]
    fn a_comment_takes_its_body_as_the_positional_argument() {
        let body = comment_body(Some("the note".into()), None, None).expect("one form is enough");
        assert_eq!(body, "the note");
    }

    /// The spelling `moat gh issue comment` takes, which is what a caller who
    /// used that command next reaches for.
    #[test]
    fn a_comment_takes_its_body_from_the_flag() {
        let body = comment_body(None, Some("the note".into()), None).expect("one form is enough");
        assert_eq!(body, "the note");
    }

    #[test]
    fn a_comment_takes_its_body_from_a_file() {
        let path = temp_file("body", "the note");
        let body = comment_body(None, None, Some(&path)).expect("the file is readable");
        assert_eq!(body, "the note");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn two_body_forms_at_once_name_all_three() {
        let err = comment_body(Some("one".into()), Some("two".into()), None)
            .expect_err("two forms is one too many");
        let message = err.to_string();
        assert!(message.contains("positional"), "{message}");
        assert!(message.contains("--body <text>"), "{message}");
        assert!(message.contains("--body-file <path>"), "{message}");
    }

    #[test]
    fn an_empty_comment_body_is_refused() {
        let err = comment_body(Some("   ".into()), None, None)
            .expect_err("an empty comment is not a comment");
        assert!(err.to_string().contains("empty"), "{err}");
    }

    /// `--description-file` over a recycled path reads as nothing, which on an
    /// update would erase the description it was meant to replace.
    #[test]
    fn an_empty_description_file_is_refused() {
        let path = temp_file("empty", "\n");
        let err = one_text("description", None, Some(&path)).expect_err("an empty file is refused");
        assert!(err.to_string().contains("is empty"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_description_file_names_the_path() {
        let path = std::env::temp_dir().join("moat-linear-does-not-exist");
        let err = one_text("description", None, Some(&path)).expect_err("the file is not there");
        assert!(err.to_string().contains("--description-file"), "{err}");
    }

    #[test]
    fn a_comment_accepts_both_spellings_of_its_body() {
        assert!(matches!(
            parse(&["comment", "SP-1", "the note"]),
            IssueAction::Comment { body: Some(b), .. } if b == "the note"
        ));
        assert!(matches!(
            parse(&["comment", "SP-1", "--body", "the note"]),
            IssueAction::Comment { body_flag: Some(b), .. } if b == "the note"
        ));
    }

    #[test]
    fn create_and_update_accept_body_for_description() {
        assert!(matches!(
            parse(&["create", "--title", "t", "--body", "d"]),
            IssueAction::Create { description: Some(d), .. } if d == "d"
        ));
        assert!(matches!(
            parse(&["update", "SP-1", "--body", "d"]),
            IssueAction::Update { description: Some(d), .. } if d == "d"
        ));
    }

    #[test]
    fn a_comment_thread_reads_oldest_first() {
        let issue = serde_json::json!({
            "comments": { "nodes": [
                { "id": "newest", "createdAt": "2026-09-18T17:00:00.000Z" },
                { "id": "oldest", "createdAt": "2026-09-18T16:00:00.000Z" },
            ] }
        });
        let ids: Vec<&str> = comments_oldest_first(&issue)
            .iter()
            .filter_map(|c| c.get("id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, ["oldest", "newest"]);
    }

    /// A write's issue payload carries no `comments` key at all.
    #[test]
    fn an_issue_without_comments_yields_none() {
        assert!(comments_oldest_first(&serde_json::json!({ "identifier": "SP-1" })).is_empty());
    }

    #[test]
    fn a_comment_from_an_app_is_named_by_its_bot_actor() {
        let comment = serde_json::json!({ "user": null, "botActor": { "name": "Moatless" } });
        assert_eq!(comment_author(&comment), "Moatless");

        let person = serde_json::json!({ "user": { "name": "ada", "displayName": "Ada" } });
        assert_eq!(comment_author(&person), "Ada");
        assert_eq!(comment_author(&serde_json::json!({})), "unknown");
    }

    /// The count is what the payload carries, so a longer thread has to say so
    /// rather than let 50 read as the whole discussion.
    #[test]
    fn a_thread_longer_than_the_selection_says_so() {
        let more = serde_json::json!({ "comments": { "pageInfo": { "hasNextPage": true } } });
        assert_eq!(truncation_note(&more), ", most recent shown");

        let all = serde_json::json!({ "comments": { "pageInfo": { "hasNextPage": false } } });
        assert_eq!(truncation_note(&all), "");
        assert_eq!(truncation_note(&serde_json::json!({})), "");
    }

    fn relate(args: &[&str]) -> Result<(RelationTypeArg, String), Box<dyn std::error::Error>> {
        match parse(&[&["relate", "SP-1"], args].concat()) {
            IssueAction::Relate { rest, relation, .. } => relation_and_issue(relation, rest),
            _ => unreachable!("the subcommand is relate"),
        }
    }

    #[test]
    fn a_relation_takes_its_type_as_a_flag_or_positionally() {
        let (kind, other) = relate(&["SP-2", "--type", "blocks"]).expect("the flag form");
        assert!(matches!(kind, RelationTypeArg::Blocks) && other == "SP-2");
        let (kind, other) = relate(&["blocks", "SP-2"]).expect("the positional form");
        assert!(matches!(kind, RelationTypeArg::Blocks) && other == "SP-2");
        let (kind, _) = relate(&["SP-2"]).expect("the type is optional");
        assert!(matches!(kind, RelationTypeArg::Related));
    }

    #[test]
    fn a_relation_type_given_twice_or_unknown_is_refused() {
        assert!(relate(&["blocks", "SP-2", "--type", "related"]).is_err());
        let err = relate(&["parent", "SP-2"]).expect_err("not a relation type");
        assert!(err.to_string().contains("related, duplicate"), "{err}");
    }

    #[test]
    fn an_update_names_the_fields_it_wrote() {
        let req = LinearIssueUpdateRequest {
            description: Some("d".into()),
            state: Some("Done".into()),
            ..Default::default()
        };
        assert_eq!(updated_fields(&req), ["description", "state"]);
    }

    #[test]
    fn clearing_a_description_rules_out_writing_one() {
        let parsed = TestCli::try_parse_from([
            "moat-linear",
            "update",
            "SP-1",
            "--clear-description",
            "--description",
            "d",
        ]);
        assert!(parsed.is_err(), "the two ask for different things");
    }
}
