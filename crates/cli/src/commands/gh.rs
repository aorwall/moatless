//! `moat gh` — GitHub proxy for sandbox agents.
//!
//! Typed subcommands (`pr create`/`comment`/`reply`, `stack`) route through
//! `/api/v1/github/*` for task↔PR binding, the `Moatless task:` footer, and
//! webhook dedup. `gh api` passes through, except a POST creating a
//! review/issue comment records its id via `/api/v1/github/external-events`. An
//! unmodeled flag silently demotes a typed call to passthrough (warned via
//! `warn_on_demotion`); a typed-path failure always errors, never falls back.

use clap::Parser;
use moatless_api_schemas::github::{
    GithubBindPrRequest, GithubCommentRequest, GithubCreatePrRequest, GithubExternalEventRequest,
    GithubExternalEventSubject, GithubReplyRequest,
};
use moatless_workspace_types::workspace::proxy_clone_repository_id;
use std::borrow::Cow;
use std::collections::HashSet;
use std::io::Write;

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

/// `moat gh pr create` flags (subset of gh's, plus `--task`).
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh pr create")]
struct PrCreateArgs {
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
struct CommentArgs {
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
struct ReplyArgs {
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

/// `moat gh stack ...` invocation. The payload is intentionally raw so the
/// official gh-stack extension remains the command grammar owner.
#[derive(Debug, PartialEq, Eq)]
struct StackInvocation {
    args: Vec<String>,
    task: Option<String>,
}

/// A gh invocation we intercept and proxy through the backend.
#[derive(Debug, PartialEq, Eq)]
enum TypedInvocation {
    PrCreate(PrCreateArgs),
    Comment(CommentArgs),
    PrReply(ReplyArgs),
    Stack(StackInvocation),
}

pub async fn run(
    client: &MoatClient,
    cfg: &Config,
    args: Vec<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match parse_typed(&args) {
        Some(TypedInvocation::PrCreate(a)) => {
            let json = json || a.json;
            run_pr_create(client, cfg, a, json).await
        }
        Some(TypedInvocation::Comment(a)) => {
            let json = json || a.json;
            run_comment(client, cfg, a, json).await
        }
        Some(TypedInvocation::PrReply(a)) => {
            let json = json || a.json;
            run_pr_reply(client, cfg, a, json).await
        }
        Some(TypedInvocation::Stack(a)) => run_stack(client, cfg, a, json).await,
        None if args.first().is_some_and(|a| a == "api") => {
            let gh_repo = origin_gh_repo(client).await;
            run_api(client, cfg, &args, gh_repo.as_deref()).await
        }
        None => {
            let gh_repo = origin_gh_repo(client).await;
            passthrough(&args, gh_repo.as_deref())
        }
    }
}

/// A subprocess status returned to the executable entry point.
#[derive(Debug, thiserror::Error)]
#[error("gh exited with status {0}")]
pub struct CommandExit(pub i32);

fn command_status(code: i32) -> Result<(), Box<dyn std::error::Error>> {
    if code == 0 {
        Ok(())
    } else {
        Err(Box::new(CommandExit(code)))
    }
}

/// Classify the invocation: a successfully parsed typed subcommand goes down
/// the typed path; anything else returns `None` → passthrough, silently
/// dropping task binding, footer, and dedup.
///
/// `warn_on_demotion` reports that loss on stderr.
fn parse_typed(args: &[String]) -> Option<TypedInvocation> {
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

async fn run_pr_create(
    client: &MoatClient,
    cfg: &Config,
    args: PrCreateArgs,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (owner, repo) = resolve_repo(client, args.repo.as_deref()).await?;
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
    let resp = client.api().github_create_pr(&owner, &repo, &req).await?;
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

async fn run_comment(
    client: &MoatClient,
    cfg: &Config,
    args: CommentArgs,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (owner, repo) = resolve_repo(client, args.repo.as_deref()).await?;
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let body = required_body(args.body, args.body_file.as_deref())?;
    let req = GithubCommentRequest { body, task_id };
    let resp = client
        .api()
        .github_post_issue_comment(&owner, &repo, args.number, &req)
        .await?;
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

async fn run_pr_reply(
    client: &MoatClient,
    cfg: &Config,
    args: ReplyArgs,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (owner, repo) = resolve_repo(client, args.repo.as_deref()).await?;
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let body = required_body(args.body, args.body_file.as_deref())?;
    let req = GithubReplyRequest { body, task_id };
    let resp = client
        .api()
        .github_reply_review_comment(&owner, &repo, args.pr_number, args.comment_id, &req)
        .await?;
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

async fn run_stack(
    client: &MoatClient,
    cfg: &Config,
    invocation: StackInvocation,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let token = github_token_for_gh();
    let gh_repo = origin_gh_repo(client).await;
    let status = gh_command(&invocation.args, token.as_deref(), gh_repo.as_deref())
        .status()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    if !status.success() {
        return command_status(status.code().unwrap_or(1));
    }

    let Some(subcommand) = invocation.args.get(1).map(String::as_str) else {
        return Ok(());
    };
    if !matches!(subcommand, "link" | "submit") {
        return Ok(());
    }

    let Some(task_id) = non_empty_task_id(
        invocation
            .task
            .or_else(|| cfg.task_id().map(str::to_string)),
    ) else {
        return Ok(());
    };

    let prs = stack_prs_to_bind(
        client,
        &invocation.args,
        token.as_deref(),
        gh_repo.as_deref(),
    )
    .await?;
    if prs.is_empty() {
        return Ok(());
    }

    let mut bound = Vec::new();
    for pr in prs {
        let req = GithubBindPrRequest {
            task_id: task_id.clone(),
        };
        let resp = client
            .api()
            .github_bind_pr(&pr.owner, &pr.repo, pr.number, &req)
            .await?;
        bound.push(resp);
    }

    if json {
        output::print_json_typed(&bound);
    } else {
        for pr in bound {
            println!("Stack PR linked to task: {}", pr.html_url);
            if let Some(bid) = pr.binding_id {
                println!("Bound to task via: {bid}");
            }
        }
    }

    Ok(())
}

/// Exec the local `gh` binary with the original args, inheriting stdio, and
/// propagate its exit code.
fn passthrough(args: &[String], gh_repo: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    // Thread the git-credential token exactly as the typed and stack paths do,
    // so a passthrough (`pr view`, `api`, `run watch`, …) authenticates the
    // same way. See [`resolve_gh_token`] for which source wins.
    let token = github_token_for_gh();
    let status = gh_command(args, token.as_deref(), gh_repo)
        .status()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    command_status(status.code().unwrap_or(1))
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

fn gh_command(
    args: &[String],
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> std::process::Command {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(args);
    if let Some(token) = token {
        cmd.env("GH_TOKEN", token);
    }
    // An explicit `-R` on the command line still wins: gh reads the flag first.
    if let Some(gh_repo) = gh_repo {
        cmd.env("GH_REPO", gh_repo);
    }
    cmd
}

/// The broker socket `docker/moatless-git-cred.sh` pulls from. Must match the
/// path that script bakes in, and the one `moatless-sandbox` binds.
const BROKER_SOCKET: &str = "/run/moatless/credentials.sock";

pub(super) fn github_token_for_gh() -> Option<String> {
    resolve_gh_token(
        std::path::Path::new(BROKER_SOCKET).exists(),
        env_github_token_is_set(),
        || git_credential_password("github.com").ok(),
    )
}

fn env_github_token_is_set() -> bool {
    std::env::var("GH_TOKEN").is_ok_and(|v| !v.trim().is_empty())
        || std::env::var("GITHUB_TOKEN").is_ok_and(|v| !v.trim().is_empty())
}

/// Pick the token to hand `gh`, or `None` to let `gh` resolve its own.
///
/// With the broker present the credential helper wins over `GH_TOKEN`, which is
/// a provision-time snapshot that 401s once its installation token expires
/// while the helper mints a fresh one per call. Off the broker an exported
/// `GH_TOKEN` keeps precedence.
fn resolve_gh_token(
    broker_present: bool,
    env_token_set: bool,
    credential: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if env_token_set && !broker_present {
        return None;
    }
    credential()
}

fn git_credential_password(host: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut child = std::process::Command::new("git")
        .args(["credential", "fill"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to run git credential fill: {e}"))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or("failed to open git credential fill stdin")?;
        write!(stdin, "protocol=https\nhost={host}\n\n")?;
    }

    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err("git credential fill did not return a GitHub token".into());
    }

    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("password="))
        .map(str::trim)
        .filter(|password| !password.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "git credential fill did not return a password".into())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PullRequestRef {
    owner: String,
    repo: String,
    number: u64,
}

async fn stack_prs_to_bind(
    client: &MoatClient,
    args: &[String],
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Vec<PullRequestRef>, Box<dyn std::error::Error>> {
    let subcommand = args.get(1).map(String::as_str);
    let (owner, repo) = resolve_repo(client, gh_repo).await?;
    let mut refs = Vec::new();

    match subcommand {
        Some("link") => {
            for operand in stack_link_operands(&args[2..]) {
                if let Some(pr) = parse_pull_request_url(&operand) {
                    refs.push(pr);
                    continue;
                }
                if let Some(pr) = resolve_pr_target(&owner, &repo, &operand, token)? {
                    refs.push(pr);
                }
            }
        }
        Some("submit") => {
            for branch in stack_view_branch_names(token, gh_repo)? {
                if let Some(pr) = resolve_pr_target(&owner, &repo, &branch, token)? {
                    refs.push(pr);
                }
            }
        }
        _ => {}
    }

    let mut seen = HashSet::new();
    refs.retain(|pr| seen.insert(pr.clone()));
    Ok(refs)
}

fn stack_link_operands(args: &[String]) -> Vec<String> {
    let mut operands = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--" => {
                operands.extend(iter.cloned());
                break;
            }
            "--base" | "--remote" => {
                let _ = iter.next();
            }
            "--open" => {}
            value if value.starts_with("--base=") || value.starts_with("--remote=") => {}
            value if value.starts_with('-') => {}
            value => operands.push(value.to_string()),
        }
    }
    operands
}

fn stack_view_branch_names(
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let args = vec![
        "stack".to_string(),
        "view".to_string(),
        "--json".to_string(),
    ];
    let out = gh_command_output(&args, token, gh_repo)?;
    let value: serde_json::Value = serde_json::from_slice(&out)?;
    let branches = value
        .get("branches")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|branch| branch.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect();
    Ok(branches)
}

fn resolve_pr_target(
    owner: &str,
    repo: &str,
    target: &str,
    token: Option<&str>,
) -> Result<Option<PullRequestRef>, Box<dyn std::error::Error>> {
    let repo_flag = format!("{owner}/{repo}");
    let args = vec![
        "pr".to_string(),
        "view".to_string(),
        target.to_string(),
        "--repo".to_string(),
        repo_flag,
        "--json".to_string(),
        "number,url".to_string(),
    ];
    let out = match gh_command_output(&args, token, None) {
        Ok(out) => out,
        Err(_) => return Ok(None),
    };
    let value: serde_json::Value = serde_json::from_slice(&out)?;
    if let Some(url) = value.get("url").and_then(serde_json::Value::as_str) {
        if let Some(pr) = parse_pull_request_url(url) {
            return Ok(Some(pr));
        }
    }
    let Some(number) = value.get("number").and_then(serde_json::Value::as_u64) else {
        return Ok(None);
    };
    Ok(Some(PullRequestRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    }))
}

pub(super) fn gh_command_output(
    args: &[String],
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let output = gh_command(args, token, gh_repo)
        .output()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(format!(
        "`gh {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
    .into())
}

fn parse_pull_request_url(url: &str) -> Option<PullRequestRef> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if parts.next()? != "pull" {
        return None;
    }
    let number = parts.next()?.parse().ok()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(PullRequestRef {
        owner,
        repo,
        number,
    })
}

fn strip_moat_task_flag(args: &[String]) -> Result<(Vec<String>, Option<String>), String> {
    let mut stripped = Vec::new();
    let mut task = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--task" {
            let Some(value) = iter.next() else {
                return Err("--task requires a value".to_string());
            };
            task = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--task=") {
            if value.is_empty() {
                return Err("--task requires a value".to_string());
            }
            task = Some(value.to_string());
        } else {
            stripped.push(arg.to_string());
        }
    }
    Ok((stripped, task))
}

fn non_empty_task_id(task_id: Option<String>) -> Option<String> {
    task_id.filter(|value| !value.trim().is_empty())
}

// ---------------------------------------------------------------------------
// owner/repo + branch inference
// ---------------------------------------------------------------------------

/// Resolve `(owner, repo)` from an explicit `-R` value or the cwd git repo.
async fn resolve_repo(
    client: &MoatClient,
    flag: Option<&str>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Some(value) = flag {
        return parse_repo_arg(value)
            .ok_or_else(|| format!("invalid repository `{value}` — expected owner/repo").into());
    }
    Ok(origin_repo(client).await?)
}

/// Why `origin` named no GitHub repository.
///
/// The two callers differ on what a failure means: [`resolve_repo`] reports
/// either, while a passthrough falls back to `gh`'s own resolution and
/// announces only the variant that is not simply a checkout `moat gh` does not
/// serve.
#[derive(Debug, thiserror::Error)]
enum OriginError {
    /// `origin` is absent, or names a repository this command has no path to.
    #[error("{0}")]
    Unaddressable(String),
    /// A proxied origin whose repository row could not be read.
    #[error("{0}")]
    Unreadable(String),
}

/// What an `origin` URL names, before anything is read back from the backend.
enum OriginUrl<'a> {
    /// A GitHub remote, in any of the URL forms.
    Github(String, String),
    /// A `moat repos clone` remote, which names a repository id and no owner.
    Proxied(&'a str),
    /// Neither, so `moat gh` cannot address it.
    Foreign,
}

/// The `user:password@` span in an HTTP(S) URL, as `(start, end)` byte offsets
/// around the credential itself.
///
/// Deliberately HTTP(S) only: `ssh://git@github.com/o/r` carries a username and
/// no secret, which [`parse_github_remote`] needs in place.
fn userinfo_span(url: &str) -> Option<(usize, usize)> {
    let authority = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .map(|rest| url.len() - rest.len())?;
    let end = url[authority..]
        .find('/')
        .map_or(url.len(), |i| authority + i);
    let at = url[authority..end].rfind('@')? + authority;
    Some((authority, at + 1))
}

/// `url` with any credential dropped, for matching.
fn strip_userinfo(url: &str) -> Cow<'_, str> {
    match userinfo_span(url) {
        Some((start, end)) => Cow::Owned(format!("{}{}", &url[..start], &url[end..])),
        None => Cow::Borrowed(url),
    }
}

/// `url` with any credential masked, for printing.
///
/// Masked rather than dropped so the reader can tell the remote carries one —
/// which is often itself the reason a command behaved unexpectedly.
fn redact_userinfo(url: &str) -> Cow<'_, str> {
    match userinfo_span(url) {
        Some((start, end)) => Cow::Owned(format!("{}***@{}", &url[..start], &url[end..])),
        None => Cow::Borrowed(url),
    }
}

fn classify_origin<'a>(base_url: &str, url: &'a str) -> OriginUrl<'a> {
    if let Some((owner, name)) = parse_github_remote(url) {
        return OriginUrl::Github(owner, name);
    }
    match proxy_clone_repository_id(base_url, url).or_else(|| any_host_proxy_repository_id(url)) {
        Some(id) => OriginUrl::Proxied(id),
        None => OriginUrl::Foreign,
    }
}

/// `(owner, repo)` for the cwd's `origin` remote — the one repository every
/// `moat gh` call addresses, read or write.
///
/// Deliberately `origin` and not `gh`'s own pick, which prefers a remote named
/// `upstream`: in a fork clone holding both, a read would answer about the
/// upstream PR sharing the number while a write went to the fork.
async fn origin_repo(client: &MoatClient) -> Result<(String, String), OriginError> {
    let url = git_stdout(&["remote", "get-url", "origin"]).map_err(|e| {
        OriginError::Unaddressable(format!(
            "could not infer the GitHub repository ({e}) — pass `-R owner/repo`"
        ))
    })?;
    let bare = strip_userinfo(url.trim());
    match classify_origin(client.base_url(), &bare) {
        OriginUrl::Github(owner, name) => Ok((owner, name)),
        OriginUrl::Proxied(id) => proxied_repo(client, id).await,
        OriginUrl::Foreign => Err(OriginError::Unaddressable(format!(
            "origin remote `{}` is not a GitHub repository — pass `-R owner/repo`",
            redact_userinfo(url.trim())
        ))),
    }
}

/// The repository id in a `/git/{id}` remote served by any host.
///
/// The sandbox writes this remote from `BACKEND_API_URL` while the CLI matches
/// it against whatever the login profile resolved to, so a deployment naming
/// one backend two ways would otherwise reject its own proxied remote. Only the
/// id comes from the URL; the row is still read from our own backend.
fn any_host_proxy_repository_id(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let (_, path) = rest.split_once("/git/")?;
    let id = path.trim_end_matches('/');
    let id = id.strip_suffix(".git").unwrap_or(id);
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// The `(owner, repo)` the repository row behind a proxied origin names.
async fn proxied_repo(
    client: &MoatClient,
    repository_id: &str,
) -> Result<(String, String), OriginError> {
    let row = client
        .api()
        .get_repository(repository_id)
        .await
        .map_err(|e| {
            OriginError::Unreadable(format!(
                "could not read the proxied origin's repository ({e})"
            ))
        })?;
    let remote = row.remote.ok_or_else(|| {
        OriginError::Unaddressable(format!("repository {repository_id} stores no remote URL"))
    })?;
    let host = remote.host_repo.ok_or_else(|| {
        OriginError::Unaddressable(format!(
            "repository {repository_id} has no resolved git host to address"
        ))
    })?;
    if host.provider != moatless_workspace_types::repository::RepositoryProvider::Github {
        return Err(OriginError::Unaddressable(format!(
            "repository {repository_id} is hosted on {}, and `moat gh` serves GitHub only — \
             there is no pull-request path for it from a sandbox",
            host.provider
        )));
    }
    Ok((host.owner, host.name))
}

/// The `owner/repo` to hand a spawned `gh` as `GH_REPO`, so a passthrough reads
/// the repository [`origin_repo`] writes to.
async fn origin_gh_repo(client: &MoatClient) -> Option<String> {
    if std::env::var_os("GH_REPO").is_some() {
        return None;
    }
    match origin_repo(client).await {
        Ok((owner, name)) => Some(format!("{owner}/{name}")),
        // A checkout `moat gh` does not serve, a Forgejo one above all, hits
        // this on every invocation, so it stays quiet and leaves `gh` to itself.
        Err(OriginError::Unaddressable(_)) => None,
        Err(e @ OriginError::Unreadable(_)) => {
            eprintln!("moat: {e}");
            None
        }
    }
}

/// Parse a `-R` value: `owner/repo` shorthand or any GitHub remote URL form.
fn parse_repo_arg(value: &str) -> Option<(String, String)> {
    if let Some(parsed) = parse_github_remote(value) {
        return Some(parsed);
    }
    let mut parts = value.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((owner, repo))
}

/// Decompose a GitHub remote URL into `(owner, repo)`. Supports HTTPS, SSH
/// shorthand (`git@github.com:owner/repo`), and the explicit `ssh://` form.
fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

fn current_git_branch() -> Result<String, Box<dyn std::error::Error>> {
    let branch = git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"])
        .map_err(|e| format!("could not infer the head branch ({e}) — pass `--head <branch>`"))?;
    let branch = branch.trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Err("detached HEAD — pass `--head <branch>`".into());
    }
    Ok(branch)
}

fn git_stdout(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ===========================================================================
// `moat gh api` — passthrough that records what it created
// ===========================================================================

/// What a raw `gh api` POST creates, and where.
///
/// Carries no footer and bypasses the typed proxy, so nothing else here
/// recognises it as the task's own work.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApiCreation {
    owner: String,
    repo: String,
    /// The pull request or issue the endpoint path names.
    number: u64,
    kind: ApiCreated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiCreated {
    /// `POST repos/{owner}/{repo}/pulls/{n}/comments` — a new inline review
    /// comment, the shape the reviewer loop's prompt asks agents to use.
    ReviewComment,
    /// `POST repos/{owner}/{repo}/pulls/{n}/reviews` — a review submission.
    Review,
    /// `POST repos/{owner}/{repo}/issues/{n}/comments` — a conversation comment
    /// on a PR or an issue.
    IssueComment,
}

impl ApiCreated {
    /// The fragment GitHub puts before the object's id in its `html_url`.
    fn html_anchor(self) -> &'static str {
        match self {
            Self::ReviewComment => "#discussion_r",
            Self::Review => "#pullrequestreview-",
            Self::IssueComment => "#issuecomment-",
        }
    }

    fn event_subject(self, id: u64) -> GithubExternalEventSubject {
        match self {
            Self::ReviewComment => GithubExternalEventSubject::ReviewComment(id),
            Self::Review => GithubExternalEventSubject::Review(id),
            Self::IssueComment => GithubExternalEventSubject::IssueComment(id),
        }
    }
}

/// Run `gh api …`, and record what a recognised POST created.
///
/// An unrecognised call passes through with inherited stdio at no extra cost; a
/// recognised one captures stdout to read the created id before writing it
/// through unchanged.
async fn run_api(
    client: &MoatClient,
    cfg: &Config,
    args: &[String],
    gh_repo: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(created) = parse_api_creation(&args[1..]) else {
        return passthrough(args, gh_repo);
    };
    let Some(task_id) = cfg.task_id().map(str::to_string) else {
        return passthrough(args, gh_repo);
    };

    let token = github_token_for_gh();
    let output = gh_command(args, token.as_deref(), gh_repo)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;

    // Write the response through before anything else: the agent's next step
    // reads it, and a recording failure must not swallow it.
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stdout().flush()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        match parse_created_id(&text, created.kind) {
            Some(id) => {
                record_event(client, &task_id, &created, created.kind.event_subject(id)).await;

                // The review id recognises none of the comments the same call
                // created; see `review_comment_ids`.
                if created.kind == ApiCreated::Review {
                    for comment_id in review_comment_ids(&created, id, token.as_deref()) {
                        record_event(
                            client,
                            &task_id,
                            &created,
                            GithubExternalEventSubject::ReviewComment(comment_id),
                        )
                        .await;
                    }
                }
            }
            None => eprintln!("moat: gh api response named no id — nothing recorded"),
        }
    }

    command_status(output.status.code().unwrap_or(1))
}

/// Record one created object against the task, and report a failure to stderr.
async fn record_event(
    client: &MoatClient,
    task_id: &str,
    created: &ApiCreation,
    subject: GithubExternalEventSubject,
) {
    let req = GithubExternalEventRequest {
        task_id: task_id.to_string(),
        subject,
        source_name: Some(format!("{}/{}", created.owner, created.repo)),
    };
    if let Err(e) = client.api().github_record_external_event(&req).await {
        // Nothing is suppressed as a result: the event is delivered and the
        // task reads back its own comment, which costs one wasted turn.
        eprintln!("moat: could not record what this call created: {e}");
    }
}

/// The ids of the inline comments a submitted review created.
///
/// Each arrives as its own webhook under its own id, unrecognised by the review
/// id, so the ids are read back from the endpoint that lists them; a failure
/// yields none since the review is already recorded.
fn review_comment_ids(created: &ApiCreation, review_id: u64, token: Option<&str>) -> Vec<u64> {
    // `--slurp` is what makes the body parseable: `--paginate` alone prints one
    // JSON array per page, and two pages concatenated parse as nothing.
    let args = vec![
        "api".to_string(),
        review_comments_endpoint(created, review_id),
        "--paginate".to_string(),
        "--slurp".to_string(),
    ];
    match gh_command_output(&args, token, None) {
        Ok(body) => ids_in_comment_list(&body),
        Err(e) => {
            eprintln!("moat: could not list the review's inline comments: {e}");
            Vec::new()
        }
    }
}

fn review_comments_endpoint(created: &ApiCreation, review_id: u64) -> String {
    format!(
        "repos/{}/{}/pulls/{}/reviews/{review_id}/comments",
        created.owner, created.repo, created.number
    )
}

/// The `id` of every comment in a slurped page list — an array of pages, each
/// itself an array of comments. Anything else yields none.
fn ids_in_comment_list(body: &[u8]) -> Vec<u64> {
    let Ok(serde_json::Value::Array(pages)) = serde_json::from_slice::<serde_json::Value>(body)
    else {
        return Vec::new();
    };
    pages
        .iter()
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(|item| item.get("id").and_then(serde_json::Value::as_u64))
        .collect()
}

/// Detect a `gh api` argument list that creates a review comment, a review, or
/// an issue comment, and name what it creates.
///
/// Recognised by endpoint shape wherever it sits among the flags; the call must
/// be a POST, since a GET of the same endpoint must never record somebody
/// else's comment.
fn parse_api_creation(args: &[String]) -> Option<ApiCreation> {
    if !api_posts(args) {
        return None;
    }
    args.iter().find_map(|arg| parse_api_endpoint(arg))
}

/// Whether a `gh api` argument list issues a POST.
///
/// `gh api` sends GET unless the call carries a body (`-f`/`-F`/`--input`); an
/// explicit `--method`/`-X` overrides either way.
fn api_posts(args: &[String]) -> bool {
    let mut explicit: Option<String> = None;
    let mut has_body = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let arg = arg.as_str();
        if arg == "--method" || arg == "-X" {
            explicit = iter.next().map(|v| v.to_ascii_uppercase());
        } else if let Some(value) = arg
            .strip_prefix("--method=")
            .or_else(|| arg.strip_prefix("-X"))
            .filter(|v| !v.is_empty())
        {
            explicit = Some(value.to_ascii_uppercase());
        } else if matches!(arg, "-f" | "-F" | "--field" | "--raw-field" | "--input") {
            has_body = true;
            iter.next();
        } else if (arg.starts_with("-f") || arg.starts_with("-F"))
            || arg.starts_with("--field=")
            || arg.starts_with("--raw-field=")
            || arg.starts_with("--input=")
        {
            has_body = true;
        }
    }

    explicit.map_or(has_body, |method| method == "POST")
}

/// Decompose a `gh api` endpoint argument into the repository it targets and
/// the kind of event a POST to it creates.
///
/// Accepts bare, rooted, and absolute (`https://api.github.com/...`) spellings,
/// ignoring any query string; a deeper path like `.../comments/{id}/replies`
/// returns `None`.
fn parse_api_endpoint(arg: &str) -> Option<ApiCreation> {
    let path = arg
        .strip_prefix("https://api.github.com/")
        .or_else(|| arg.strip_prefix("http://api.github.com/"))
        .unwrap_or(arg);
    let path = path.split(['?', '#']).next()?;
    let path = path.trim_matches('/');

    let mut parts = path.split('/');
    if parts.next()? != "repos" {
        return None;
    }
    let owner = parts.next()?;
    let repo = parts.next()?;
    let subject = parts.next()?;
    let number = parts.next()?;
    let leaf = parts.next()?;
    if parts.next().is_some() || owner.is_empty() || repo.is_empty() {
        return None;
    }
    let number = number.parse::<u64>().ok()?;

    let kind = match (subject, leaf) {
        ("pulls", "comments") => ApiCreated::ReviewComment,
        ("pulls", "reviews") => ApiCreated::Review,
        ("issues", "comments") => ApiCreated::IssueComment,
        _ => return None,
    };

    Some(ApiCreation {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        kind,
    })
}

/// The id GitHub gave the object the call created.
///
/// Read from the response's `id` field, or failing that the anchor in its
/// `html_url` — both read the id back out of the response, since one
/// reconstructed from the request would risk silently recording somebody else's
/// event.
fn parse_created_id(text: &str, kind: ApiCreated) -> Option<u64> {
    if let Ok(serde_json::Value::Object(body)) =
        serde_json::from_str::<serde_json::Value>(text.trim())
    {
        if let Some(id) = body.get("id").and_then(serde_json::Value::as_u64) {
            return Some(id);
        }
    }
    last_id_after(text, kind.html_anchor())
}

/// The digits following the last occurrence of `anchor` in `text`.
fn last_id_after(text: &str, anchor: &str) -> Option<u64> {
    let idx = text.rfind(anchor)?;
    let rest = &text[idx + anchor.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests;
