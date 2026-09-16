//! The gh subcommands `moat gh` models: three writes it proxies, and the reads
//! it serves from its own children.
//!
//! A successful parse here routes through `/api/v1/github/*` for task↔PR
//! binding, the `Moatless task:` footer and webhook dedup. An unmodeled flag
//! fails the parse and demotes the call to plain `gh`, which drops all three —
//! [`warn_on_demotion`] says so on stderr rather than letting it pass silently.

use clap::Parser;
use moatless_api_schemas::github::{
    GithubCommentRequest, GithubCreatePrRequest, GithubReplyRequest,
};

use super::pr_view::{self, PrViewArgs};
use super::process::gh_command_output;
use super::reply_target::{self, ReplyTarget};
use super::repo::{RepoTarget, current_git_branch, resolve_repo};
use super::run_logs::{self, RunLogArgs};
use super::stack::{StackInvocation, strip_moat_task_flag};
use super::token::github_token_for_gh;
use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

/// `moat gh pr create` flags (subset of gh's, plus `--task`).
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh pr create")]
pub(super) struct PrCreateArgs {
    #[arg(long, short = 't')]
    title: String,
    #[arg(long, short = 'b', conflicts_with = "body_file")]
    body: Option<String>,
    /// Read the body from a file, as real `gh` does. Without this the flag is
    /// unknown to the parser, which demotes the whole call to passthrough and
    /// silently loses the task binding and the `Moatless task:` footer.
    #[arg(long = "body-file", short = 'F')]
    body_file: Option<std::path::PathBuf>,
    /// Head branch. Defaults to the current git branch.
    #[arg(long, short = 'H')]
    head: Option<String>,
    /// Base branch. Defaults to the repository's default branch.
    #[arg(long, short = 'B')]
    base: Option<String>,
    #[arg(long, short = 'd')]
    draft: bool,
    /// Label to put on the new PR (repeatable). Applied after the PR is opened,
    /// since the create endpoint takes none.
    #[arg(long = "label", short = 'l')]
    labels: Vec<String>,
    /// `owner/repo` (or a GitHub URL). Inferred from `git remote get-url
    /// origin` when omitted.
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
    /// Task to bind the PR to (defaults to `$TASK_ID`).
    #[arg(long = "task")]
    task: Option<String>,
    /// Output raw JSON. (The global `--json` lands here when written after `gh`
    /// because of trailing-var-arg capture.)
    #[arg(long)]
    json: bool,
}

/// `moat gh pr comment` / `moat gh issue comment` flags.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh comment")]
pub(super) struct CommentArgs {
    /// PR or issue number.
    number: u64,
    #[arg(long, short = 'b', required_unless_present = "body_file")]
    body: Option<String>,
    /// Read the body from a file, as real `gh` does.
    #[arg(long = "body-file", short = 'F', conflicts_with = "body")]
    body_file: Option<std::path::PathBuf>,
    /// `owner/repo` (or a GitHub URL). Inferred from `git remote get-url
    /// origin` when omitted.
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
    /// Task to bind / dedup against (defaults to `$TASK_ID`).
    #[arg(long = "task")]
    task: Option<String>,
    /// Output raw JSON. (The global `--json` lands here when written after `gh`
    /// because of trailing-var-arg capture.)
    #[arg(long)]
    json: bool,
}

/// `moat gh pr reply` flags. Moat-only subcommand (no real-gh equivalent):
/// threaded reply to a PR review comment via the backend proxy.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh pr reply")]
pub(super) struct ReplyArgs {
    /// PR number.
    pr_number: u64,
    /// Review comment id to reply to.
    comment_id: u64,
    #[arg(long, short = 'b', required_unless_present = "body_file")]
    body: Option<String>,
    /// Read the body from a file, as real `gh` does.
    #[arg(long = "body-file", short = 'F', conflicts_with = "body")]
    body_file: Option<std::path::PathBuf>,
    /// `owner/repo` (or a GitHub URL). Inferred from `git remote get-url
    /// origin` when omitted.
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
    /// Task to bind / dedup against (defaults to `$TASK_ID`).
    #[arg(long = "task")]
    task: Option<String>,
    /// Output raw JSON. (The global `--json` lands here when written after `gh`
    /// because of trailing-var-arg capture.)
    #[arg(long)]
    json: bool,
}

/// A gh invocation we intercept: a write proxied through the backend, or a read
/// `moat` answers itself.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TypedInvocation {
    PrCreate(PrCreateArgs),
    Comment(CommentArgs),
    PrReply(ReplyArgs),
    PrView(PrViewArgs),
    RunLog(RunLogArgs),
    Stack(StackInvocation),
}

/// Classify the invocation: a successfully parsed typed subcommand goes down
/// the typed path; anything else returns `None` → passthrough, silently
/// dropping task binding, footer, and dedup.
///
/// `warn_on_demotion` reports that loss on stderr.
pub(super) fn parse_typed(args: &[String]) -> Option<TypedInvocation> {
    let first = args.first()?.as_str();
    let second = args.get(1)?.as_str();
    let rest = with_argv0(&args[2..]);
    match (first, second) {
        ("pr", "create") => warn_on_demotion("pr create", PrCreateArgs::try_parse_from(rest))
            .map(TypedInvocation::PrCreate),
        ("pr", "comment") | ("issue", "comment") => {
            warn_on_demotion("pr comment", CommentArgs::try_parse_from(rest))
                .map(TypedInvocation::Comment)
        }
        ("pr", "reply") => warn_on_demotion("pr reply", ReplyArgs::try_parse_from(rest))
            .map(TypedInvocation::PrReply),
        // No demotion warning: a `pr view` this path declines is a read that
        // real `gh` answers correctly, and nothing is bound or footered here.
        ("pr", "view") => pr_view::parse(rest).map(TypedInvocation::PrView),
        ("run", "view") => run_logs::parse(rest).map(TypedInvocation::RunLog),
        ("stack", _) => match strip_moat_task_flag(args) {
            Ok((args, task)) => Some(TypedInvocation::Stack(StackInvocation { args, task })),
            Err(_) => None,
        },
        _ => None,
    }
}

/// Unwrap a typed parse, and name what the demotion costs when it failed.
///
/// `--help`/`--version` are not demotions: clap reports them as errors, but
/// real `gh` prints better text than we would.
fn warn_on_demotion<T>(subcommand: &str, parsed: Result<T, clap::Error>) -> Option<T> {
    match parsed {
        Ok(v) => Some(v),
        Err(e) => {
            use clap::error::ErrorKind;
            if !matches!(
                e.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) {
                eprintln!(
                    "moat gh: falling back to plain `gh` for `{subcommand}` — {}. \
                     The PR or comment will NOT be bound to the task and will \
                     carry no `Moatless task:` footer.",
                    demotion_reason(&e)
                );
            }
            None
        }
    }
}

/// The one line of a clap error worth putting in front of an agent.
fn demotion_reason(e: &clap::Error) -> String {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    if let Some(ContextValue::String(arg)) = e.get(ContextKind::InvalidArg) {
        return match e.kind() {
            ErrorKind::UnknownArgument => format!("`{arg}` is not a flag moat understands"),
            ErrorKind::MissingRequiredArgument => format!("`{arg}` is required"),
            _ => format!("`{arg}` was rejected"),
        };
    }
    match e.kind() {
        ErrorKind::MissingRequiredArgument => "a required flag is missing".to_string(),
        other => format!("{other:?}"),
    }
}

/// Prepend a dummy argv[0] for clap's `try_parse_from`.
fn with_argv0(rest: &[String]) -> Vec<String> {
    std::iter::once("moat-gh".to_string())
        .chain(rest.iter().cloned())
        .collect()
}

pub(super) async fn run_pr_create(
    client: &MoatClient,
    cfg: &Config,
    args: PrCreateArgs,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = json || args.json;
    let target = resolve_repo(client, args.repo.as_deref()).await?;
    let head = match args.head {
        Some(h) => h,
        None => current_git_branch()?,
    };
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let body = read_body(args.body, args.body_file.as_deref())?;
    let req = GithubCreatePrRequest {
        title: args.title,
        body,
        head,
        base: args.base,
        draft: if args.draft { Some(true) } else { None },
        task_id,
    };
    let resp = client
        .api()
        .github_create_pr(&target.owner, &target.repo, &req)
        .await
        .map_err(|e| target.failed("", e))?;
    if !args.labels.is_empty() {
        apply_labels(
            &target.owner,
            &target.repo,
            resp.number,
            &args.labels,
            &resp.html_url,
        )?;
    }
    if json {
        output::print_json_typed(&resp);
    } else {
        let verb = if resp.opened_existing {
            "PR already open"
        } else {
            "PR created"
        };
        println!("{verb}: {}", resp.html_url);
        if let Some(bid) = resp.binding_id {
            println!("Bound to task via: {bid}");
        }
    }
    Ok(())
}

/// Put `labels` on the pull request just opened, or fail naming the ones that
/// did not land.
///
/// The create endpoint carries no labels, so they go on in a second call; the
/// response lists what the issue now has, and a label the repository does not
/// define is a 404 there rather than a silent drop.
fn apply_labels(
    owner: &str,
    repo: &str,
    number: u64,
    labels: &[String],
    html_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = vec![
        "api".to_string(),
        format!("repos/{owner}/{repo}/issues/{number}/labels"),
        "--method".to_string(),
        "POST".to_string(),
    ];
    for label in labels {
        args.push("-f".to_string());
        args.push(format!("labels[]={label}"));
    }
    let token = github_token_for_gh();
    match gh_command_output(&args, token.as_deref(), None) {
        Ok(body) => match labels_missing_from(&body, labels) {
            missing if missing.is_empty() => Ok(()),
            missing => Err(label_failure(
                html_url,
                number,
                &missing,
                "the labels GitHub answered with do not include it",
            )
            .into()),
        },
        Err(e) => Err(label_failure(html_url, number, labels, &e.to_string()).into()),
    }
}

/// The requested labels the issue does not carry, by name and case-insensitively
/// as GitHub matches them. A body that does not parse yields every label asked
/// for: an unreadable answer confirms nothing.
fn labels_missing_from(body: &[u8], requested: &[String]) -> Vec<String> {
    let applied: Vec<String> = match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
            .map(str::to_ascii_lowercase)
            .collect(),
        _ => Vec::new(),
    };
    requested
        .iter()
        .filter(|label| !applied.contains(&label.to_ascii_lowercase()))
        .cloned()
        .collect()
}

fn label_failure(html_url: &str, number: u64, labels: &[String], cause: &str) -> String {
    let named = labels
        .iter()
        .map(|label| format!("`{label}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "PR opened and bound at {html_url}, but {named} did not land: {cause}. \
         Put it on with `moat gh pr edit {number} --add-label <name>`; a label the \
         repository does not define has to be created first with \
         `moat gh label create <name>`."
    )
}

pub(super) async fn run_comment(
    client: &MoatClient,
    cfg: &Config,
    args: CommentArgs,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = json || args.json;
    let target = resolve_repo(client, args.repo.as_deref()).await?;
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let body = required_body(args.body, args.body_file.as_deref())?;
    post_issue_comment(client, &target, args.number, body, task_id, json).await
}

async fn post_issue_comment(
    client: &MoatClient,
    target: &RepoTarget,
    number: u64,
    body: String,
    task_id: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let req = GithubCommentRequest { body, task_id };
    let resp = client
        .api()
        .github_post_issue_comment(&target.owner, &target.repo, number, &req)
        .await
        .map_err(|e| target.failed(&format!("#{number}"), e))?;
    if json {
        output::print_json_typed(&resp);
    } else {
        println!("Comment posted: {}", resp.html_url);
        if let Some(bid) = resp.binding_id {
            println!("Bound to task via: {bid}");
        }
    }
    Ok(())
}

pub(super) async fn run_pr_reply(
    client: &MoatClient,
    cfg: &Config,
    args: ReplyArgs,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = json || args.json;
    let target = resolve_repo(client, args.repo.as_deref()).await?;
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let body = required_body(args.body, args.body_file.as_deref())?;

    // Classify before posting rather than after a failure: a fallback that ran
    // on any error would post twice whenever the reply itself had succeeded.
    // Its `git credential` and two `gh` reads block, so they run on a blocking
    // thread instead of on the worker this future is polled by.
    let classified = {
        let (owner, repo) = (target.owner.clone(), target.repo.clone());
        let (pr_number, comment_id) = (args.pr_number, args.comment_id);
        tokio::task::spawn_blocking(move || {
            reply_target::classify(
                &owner,
                &repo,
                pr_number,
                comment_id,
                github_token_for_gh().as_deref(),
            )
        })
        .await?
    };
    match classified {
        ReplyTarget::ConversationComment => {
            eprintln!(
                "moat gh: comment {} is a top-level conversation comment, which has no thread \
                 to reply in — posting a new comment on #{} instead (`moat gh pr comment`).",
                args.comment_id, args.pr_number
            );
            return post_issue_comment(client, &target, args.pr_number, body, task_id, json).await;
        }
        ReplyTarget::Elsewhere(number) => {
            return Err(format!(
                "comment {} belongs to {}#{number}, not #{} — reply on the pull request the \
                 comment is on, or pass `-R owner/repo`",
                args.comment_id,
                target.slug(),
                args.pr_number
            )
            .into());
        }
        ReplyTarget::Ambiguous => {
            return Err(format!(
                "comment {id} names both a review comment and a conversation comment on \
                 {}#{pr} — the two id spaces number independently and collided here. \
                 Post the conversation reply with `moat gh pr comment {pr}`, or reply in \
                 the review thread with `moat gh api \
                 repos/{}/{}/pulls/{pr}/comments/{id}/replies -f body=…`.",
                target.slug(),
                target.owner,
                target.repo,
                id = args.comment_id,
                pr = args.pr_number,
            )
            .into());
        }
        ReplyTarget::Unreadable => {
            return Err(format!(
                "could not read what comment {id} on {}#{pr} names (the failure is above), and \
                 the id alone does not say: a review comment and a conversation comment can \
                 carry it at once. Retry, or address the surface directly — \
                 `moat gh pr comment {pr}` for a conversation comment, or `moat gh api \
                 repos/{}/{}/pulls/{pr}/comments/{id}/replies -f body=…` for a review thread.",
                target.slug(),
                target.owner,
                target.repo,
                id = args.comment_id,
                pr = args.pr_number,
            )
            .into());
        }
        ReplyTarget::ReviewComment | ReplyTarget::Unknown => {}
    }

    let req = GithubReplyRequest { body, task_id };
    let resp = client
        .api()
        .github_reply_review_comment(
            &target.owner,
            &target.repo,
            args.pr_number,
            args.comment_id,
            &req,
        )
        .await
        .map_err(|e| target.failed(&format!("#{}", args.pr_number), e))?;
    if json {
        output::print_json_typed(&resp);
    } else {
        println!("Reply posted: {}", resp.html_url);
        if let Some(bid) = resp.binding_id {
            println!("Bound to task via: {bid}");
        }
    }
    Ok(())
}

/// Resolve `--body` / `--body-file` into the optional body the API takes.
///
/// clap has already rejected the case where both are given.
fn read_body(
    body: Option<String>,
    body_file: Option<&std::path::Path>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match (body, body_file) {
        (Some(b), _) => Ok(Some(b)),
        (None, Some(path)) => {
            Ok(Some(std::fs::read_to_string(path).map_err(|e| {
                format!("failed to read --body-file {}: {e}", path.display())
            })?))
        }
        (None, None) => Ok(None),
    }
}

/// Same, where the API requires a body. clap guarantees one of the two is
/// present, so `None` here is unreachable rather than a user error.
fn required_body(
    body: Option<String>,
    body_file: Option<&std::path::Path>,
) -> Result<String, Box<dyn std::error::Error>> {
    read_body(body, body_file)?.ok_or_else(|| "no --body or --body-file given".into())
}

#[cfg(test)]
mod tests;
