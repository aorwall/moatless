//! The gh subcommands `moat gh` models: three writes it proxies, and the reads
//! it serves from its own children.
//!
//! A successful parse here routes through `/api/v1/github/*` for task↔PR
//! binding, the `Moatless task:` footer and webhook dedup. A write whose flags
//! fail the parse is refused ([`TypedInvocation::Refused`]) rather than handed
//! to plain `gh`, which would drop all three.

use clap::Parser;
use moatless_api_schemas::github::{
    GithubCommentRequest, GithubCreatePrRequest, GithubReplyRequest,
};

use super::notified;
use super::pr_edit::{self, PrEditArgs};
use super::pr_followups;
use super::pr_view::{self, PrViewArgs};
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
    /// Read the body from a file, as real `gh` does.
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
    /// Login or `org/team` to request review from (repeatable or
    /// comma-separated, as real `gh` takes it). Requested after the PR is
    /// opened, since the create endpoint takes none.
    #[arg(long = "reviewer", short = 'r', value_delimiter = ',')]
    reviewers: Vec<String>,
    /// `owner/repo` (or a GitHub URL). Inferred from `git remote get-url
    /// origin` when omitted.
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
    /// Task to bind the PR to (defaults to `$TASK_ID`).
    #[arg(long = "task")]
    task: Option<String>,
    /// Close the bound task when the PR merges or closes.
    #[arg(long = "close-task-on-close")]
    close_task_on_close: bool,
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
    PrEdit(PrEditArgs),
    RunLog(RunLogArgs),
    Stack(StackInvocation),
    /// A proxied write moat cannot model. Exit with this message rather than
    /// passing through: plain `gh` would create an unbound PR or comment with
    /// no `Moatless task:` footer, so a CI hand-back never finds the task.
    Refused(String),
    /// `--help` on a proxied write: moat's own flags, since real `gh`'s help
    /// lists flags (`--attach`, `--fill`) that this path refuses.
    Help(String),
}

/// Classify the invocation: a parsed typed subcommand goes down the typed path,
/// an unparseable proxied write is [`TypedInvocation::Refused`], and `None`
/// means passthrough to real `gh`.
pub(super) fn parse_typed(args: &[String]) -> Option<TypedInvocation> {
    let first = args.first()?.as_str();
    let second = args.get(1)?.as_str();
    let rest = with_argv0(&args[2..]);
    match (first, second) {
        ("pr", "create") => Some(typed_or_refused(
            "pr create",
            PrCreateArgs::try_parse_from(rest),
            TypedInvocation::PrCreate,
        )),
        (noun @ ("pr" | "issue"), "comment") => Some(typed_or_refused(
            if noun == "pr" {
                "pr comment"
            } else {
                "issue comment"
            },
            CommentArgs::try_parse_from(rest),
            TypedInvocation::Comment,
        )),
        ("pr", "reply") => Some(typed_or_refused(
            "pr reply",
            ReplyArgs::try_parse_from(rest),
            TypedInvocation::PrReply,
        )),
        // Not refused: a `pr view` this path declines is a read that real
        // `gh` answers correctly, and nothing is bound or footered here.
        ("pr", "view") => pr_view::parse(rest).map(TypedInvocation::PrView),
        ("pr", "edit") => pr_edit::parse(rest).map(TypedInvocation::PrEdit),
        ("run", "view") => run_logs::parse(rest).map(TypedInvocation::RunLog),
        ("stack", _) => match strip_moat_task_flag(args) {
            Ok((args, task)) => Some(TypedInvocation::Stack(StackInvocation { args, task })),
            Err(_) => None,
        },
        _ => None,
    }
}

/// Wrap a typed parse, or refuse it with the flag that failed it.
fn typed_or_refused<T>(
    subcommand: &str,
    parsed: Result<T, clap::Error>,
    wrap: fn(T) -> TypedInvocation,
) -> TypedInvocation {
    use clap::error::ErrorKind;
    match parsed {
        Ok(v) => wrap(v),
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) =>
        {
            TypedInvocation::Help(e.render().to_string())
        }
        Err(e) => TypedInvocation::Refused(format!(
            "moat gh {subcommand}: {}. Refusing rather than running plain `gh`, \
             which would leave the PR or comment unbound to the task, with no \
             `Moatless task:` footer. Drop the flag, or see `moat gh {subcommand} --help`.",
            refusal_reason(&e)
        )),
    }
}

/// The one line of a clap error worth putting in front of an agent.
fn refusal_reason(e: &clap::Error) -> String {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    if let Some(ContextValue::String(arg)) = e.get(ContextKind::InvalidArg) {
        return match e.kind() {
            ErrorKind::UnknownArgument => format!("`{arg}` is not a flag moat understands"),
            ErrorKind::MissingRequiredArgument => format!("`{arg}` is required"),
            _ => format!("`{arg}` was rejected"),
        };
    }
    if let (ErrorKind::MissingRequiredArgument, Some(ContextValue::Strings(args))) =
        (e.kind(), e.get(ContextKind::InvalidArg))
    {
        return format!("`{}` is required", args.join("`, `"));
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
    let task_requested = task_id.is_some();
    let body = read_body(args.body, args.body_file.as_deref())?;
    let req = GithubCreatePrRequest {
        title: args.title,
        body,
        head,
        base: args.base,
        draft: if args.draft { Some(true) } else { None },
        task_id,
        close_task_on_close: args.close_task_on_close,
    };
    let resp = client
        .api()
        .github_create_pr(&target.owner, &target.repo, &req)
        .await
        .map_err(|e| target.failed("", e))?;
    if !args.labels.is_empty() {
        pr_followups::apply_labels(
            cfg.github_token_set,
            &target.owner,
            &target.repo,
            resp.number,
            &args.labels,
            &resp.html_url,
        )?;
    }
    if !args.reviewers.is_empty() {
        pr_followups::apply_reviewers(
            cfg.github_token_set,
            &target.owner,
            &target.repo,
            resp.number,
            &args.reviewers,
            &resp.html_url,
        )?;
    }
    output::emit(json, &resp, || {
        let verb = if resp.opened_existing {
            "PR already open"
        } else {
            "PR created"
        };
        println!("{verb}: {}", resp.html_url);
        if let Some(bid) = &resp.binding_id {
            println!("Bound to task via: {bid}");
        }
        if task_requested {
            for line in notified::notice(resp.binding_id.is_some(), &resp.notified_events) {
                println!("{line}");
            }
        }
    });
    Ok(())
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
    output::emit(json, &resp, || {
        println!("Comment posted: {}", resp.html_url);
        if let Some(bid) = &resp.binding_id {
            println!("Bound to task via: {bid}");
        }
    });
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
        let env_token_set = cfg.github_token_set;
        tokio::task::spawn_blocking(move || {
            reply_target::classify(
                &owner,
                &repo,
                pr_number,
                comment_id,
                github_token_for_gh(env_token_set).as_deref(),
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
    output::emit(json, &resp, || {
        println!("Reply posted: {}", resp.html_url);
        if let Some(bid) = &resp.binding_id {
            println!("Bound to task via: {bid}");
        }
    });
    Ok(())
}

/// Resolve `--body` / `--body-file` into the optional body the API takes.
///
/// clap has already rejected the case where both are given.
pub(super) fn read_body(
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
