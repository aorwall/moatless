//! `moat gh` — GitHub proxy for sandbox agents.
//!
//! Intercepts the mutations that need backend side effects (task↔PR
//! binding, `Moatless task:` footer, webhook echo dedup) and routes them
//! through the typed `/api/v1/github/*` endpoints:
//! - `moat gh pr create …`
//! - `moat gh pr comment <number> …`
//! - `moat gh issue comment <number> …`
//! - `moat gh pr reply <number> <comment_id> …` (moat-only; real `gh` has no
//!   such subcommand, so a parse failure falls through and errors there)
//!
//! Everything else — and any of these with flags the typed parser doesn't
//! understand (`--fill`, `--body-file`, …) — passes through verbatim to the
//! local `gh` binary so an agent is never stuck. Typed-path API failures are
//! reported as errors WITHOUT falling back to raw `gh`, to avoid
//! double-posting.

use clap::Parser;
use moatless_api_schemas::github::{
    GithubCommentRequest, GithubCreatePrRequest, GithubReplyRequest,
};

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;

/// `moat gh pr create` flags (subset of gh's, plus `--task`).
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh pr create")]
struct PrCreateArgs {
    #[arg(long, short = 't')]
    title: String,
    #[arg(long, short = 'b')]
    body: Option<String>,
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
    /// Task to bind the PR to (defaults to $TASK_ID).
    #[arg(long = "task")]
    task: Option<String>,
    /// Output raw JSON. (The global `--json` lands here when written after
    /// `gh` because of trailing-var-arg capture.)
    #[arg(long)]
    json: bool,
}

/// `moat gh pr comment` / `moat gh issue comment` flags.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "moat gh comment")]
struct CommentArgs {
    /// PR or issue number.
    number: u64,
    #[arg(long, short = 'b')]
    body: String,
    /// `owner/repo` (or a GitHub URL). Inferred from `git remote get-url
    /// origin` when omitted.
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
    /// Task to bind / dedup against (defaults to $TASK_ID).
    #[arg(long = "task")]
    task: Option<String>,
    /// Output raw JSON. (The global `--json` lands here when written after
    /// `gh` because of trailing-var-arg capture.)
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
    #[arg(long, short = 'b')]
    body: String,
    /// `owner/repo` (or a GitHub URL). Inferred from `git remote get-url
    /// origin` when omitted.
    #[arg(long = "repo", short = 'R')]
    repo: Option<String>,
    /// Task to bind / dedup against (defaults to $TASK_ID).
    #[arg(long = "task")]
    task: Option<String>,
    /// Output raw JSON. (The global `--json` lands here when written after
    /// `gh` because of trailing-var-arg capture.)
    #[arg(long)]
    json: bool,
}

/// A gh invocation we intercept and proxy through the backend.
#[derive(Debug, PartialEq, Eq)]
enum TypedInvocation {
    PrCreate(PrCreateArgs),
    Comment(CommentArgs),
    PrReply(ReplyArgs),
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
        None => passthrough(&args),
    }
}

/// Classify the invocation: a successfully parsed `pr create` / `pr comment`
/// / `issue comment` is handled by the typed path; anything else (including
/// those three with unrecognized flags) returns `None` → passthrough.
fn parse_typed(args: &[String]) -> Option<TypedInvocation> {
    let first = args.first()?.as_str();
    let second = args.get(1)?.as_str();
    let rest = with_argv0(&args[2..]);
    match (first, second) {
        ("pr", "create") => PrCreateArgs::try_parse_from(rest)
            .ok()
            .map(TypedInvocation::PrCreate),
        ("pr", "comment") | ("issue", "comment") => CommentArgs::try_parse_from(rest)
            .ok()
            .map(TypedInvocation::Comment),
        ("pr", "reply") => ReplyArgs::try_parse_from(rest)
            .ok()
            .map(TypedInvocation::PrReply),
        _ => None,
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
    let (owner, repo) = resolve_repo(args.repo.as_deref())?;
    let head = match args.head {
        Some(h) => h,
        None => current_git_branch()?,
    };
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let req = GithubCreatePrRequest {
        title: args.title,
        body: args.body,
        head,
        base: args.base,
        draft: if args.draft { Some(true) } else { None },
        task_id,
    };
    let resp = client.api().github_create_pr(&owner, &repo, &req).await?;
    if json {
        output::print_json_typed(&resp);
    } else {
        println!("PR created: {}", resp.html_url);
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
    let (owner, repo) = resolve_repo(args.repo.as_deref())?;
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let req = GithubCommentRequest {
        body: args.body,
        task_id,
    };
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
    let (owner, repo) = resolve_repo(args.repo.as_deref())?;
    let task_id = args.task.or_else(|| cfg.task_id().map(str::to_string));
    let req = GithubReplyRequest {
        body: args.body,
        task_id,
    };
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

/// Exec the local `gh` binary with the original args, inheriting stdio, and
/// propagate its exit code.
fn passthrough(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let status = std::process::Command::new("gh")
        .args(args)
        .status()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    std::process::exit(status.code().unwrap_or(1));
}

// ---------------------------------------------------------------------------
// owner/repo + branch inference
// ---------------------------------------------------------------------------

/// Resolve `(owner, repo)` from an explicit `-R` value or the cwd git repo.
fn resolve_repo(flag: Option<&str>) -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Some(value) = flag {
        return parse_repo_arg(value)
            .ok_or_else(|| format!("invalid repository `{value}` — expected owner/repo").into());
    }
    let url = git_stdout(&["remote", "get-url", "origin"]).map_err(|e| {
        format!("could not infer the GitHub repository ({e}) — pass `-R owner/repo`")
    })?;
    parse_github_remote(url.trim()).ok_or_else(|| {
        format!(
            "origin remote `{}` is not a GitHub repository — pass `-R owner/repo`",
            url.trim()
        )
        .into()
    })
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
/// Mirrors `parse_github_remote` in the backend's `message/hooks.rs`.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_pr_create_with_typed_flags() {
        let parsed = parse_typed(&argv(&[
            "pr", "create", "--title", "fix: x", "--body", "b", "--head", "feat/x", "--base",
            "main", "--draft", "-R", "o/r", "--task", "t_1",
        ]));
        let Some(TypedInvocation::PrCreate(a)) = parsed else {
            panic!("expected typed pr create, got {parsed:?}");
        };
        assert_eq!(a.title, "fix: x");
        assert_eq!(a.body.as_deref(), Some("b"));
        assert_eq!(a.head.as_deref(), Some("feat/x"));
        assert_eq!(a.base.as_deref(), Some("main"));
        assert!(a.draft);
        assert_eq!(a.repo.as_deref(), Some("o/r"));
        assert_eq!(a.task.as_deref(), Some("t_1"));
    }

    #[test]
    fn parses_pr_create_short_flags() {
        let parsed = parse_typed(&argv(&["pr", "create", "-t", "title", "-b", "body"]));
        assert!(matches!(parsed, Some(TypedInvocation::PrCreate(_))));
    }

    #[test]
    fn pr_create_with_unknown_flag_falls_through() {
        // `--fill` / `--body-file` are not typed — must go to the real gh.
        assert_eq!(parse_typed(&argv(&["pr", "create", "--fill"])), None);
        assert_eq!(
            parse_typed(&argv(&[
                "pr",
                "create",
                "-t",
                "t",
                "--body-file",
                "/tmp/b.md"
            ])),
            None
        );
        // Missing required --title also falls through.
        assert_eq!(parse_typed(&argv(&["pr", "create"])), None);
    }

    #[test]
    fn parses_pr_and_issue_comment() {
        let parsed = parse_typed(&argv(&["pr", "comment", "42", "--body", "hi"]));
        let Some(TypedInvocation::Comment(a)) = parsed else {
            panic!("expected typed comment, got {parsed:?}");
        };
        assert_eq!(a.number, 42);
        assert_eq!(a.body, "hi");

        assert!(matches!(
            parse_typed(&argv(&["issue", "comment", "7", "-b", "hi", "-R", "o/r"])),
            Some(TypedInvocation::Comment(_))
        ));
    }

    #[test]
    fn comment_with_non_numeric_target_falls_through() {
        // gh accepts URLs/branches as the comment target — we only type numbers.
        assert_eq!(
            parse_typed(&argv(&[
                "pr",
                "comment",
                "https://github.com/o/r/pull/42",
                "--body",
                "hi"
            ])),
            None
        );
        // No --body → editor-based flow → passthrough.
        assert_eq!(parse_typed(&argv(&["pr", "comment", "42"])), None);
    }

    #[test]
    fn parses_pr_reply() {
        let parsed = parse_typed(&argv(&[
            "pr", "reply", "42", "777", "--body", "fixed", "-R", "o/r", "--task", "t_1",
        ]));
        let Some(TypedInvocation::PrReply(a)) = parsed else {
            panic!("expected typed pr reply, got {parsed:?}");
        };
        assert_eq!(a.pr_number, 42);
        assert_eq!(a.comment_id, 777);
        assert_eq!(a.body, "fixed");
        assert_eq!(a.repo.as_deref(), Some("o/r"));
        assert_eq!(a.task.as_deref(), Some("t_1"));
    }

    #[test]
    fn pr_reply_missing_body_falls_through() {
        // No --body → fall through to the real gh (which errors: `pr reply`
        // is moat-only). Better than a half-typed editor flow.
        assert_eq!(parse_typed(&argv(&["pr", "reply", "42", "777"])), None);
    }

    #[test]
    fn pr_reply_non_numeric_positionals_fall_through() {
        assert_eq!(
            parse_typed(&argv(&["pr", "reply", "abc", "777", "--body", "x"])),
            None
        );
        assert_eq!(
            parse_typed(&argv(&["pr", "reply", "42", "not-an-id", "--body", "x"])),
            None
        );
        // Missing comment_id entirely also falls through.
        assert_eq!(
            parse_typed(&argv(&["pr", "reply", "42", "--body", "x"])),
            None
        );
    }

    #[test]
    fn other_subcommands_fall_through() {
        assert_eq!(parse_typed(&argv(&["pr", "view", "42"])), None);
        assert_eq!(parse_typed(&argv(&["api", "/user"])), None);
        assert_eq!(parse_typed(&argv(&["run", "watch", "1"])), None);
        assert_eq!(parse_typed(&argv(&["issue", "create", "-t", "x"])), None);
        assert_eq!(parse_typed(&argv(&[])), None);
        assert_eq!(parse_typed(&argv(&["pr"])), None);
    }

    #[test]
    fn parse_repo_arg_accepts_slug_and_urls() {
        assert_eq!(parse_repo_arg("o/r"), Some(("o".into(), "r".into())));
        assert_eq!(
            parse_repo_arg("https://github.com/o/r.git"),
            Some(("o".into(), "r".into()))
        );
        assert_eq!(
            parse_repo_arg("git@github.com:o/r.git"),
            Some(("o".into(), "r".into()))
        );
        assert_eq!(parse_repo_arg("just-a-name"), None);
        assert_eq!(parse_repo_arg("a/b/c"), None);
        assert_eq!(parse_repo_arg("/r"), None);
    }

    #[test]
    fn parse_github_remote_forms() {
        assert_eq!(
            parse_github_remote("https://github.com/soaplabs/soap-backend.git"),
            Some(("soaplabs".into(), "soap-backend".into()))
        );
        assert_eq!(
            parse_github_remote("ssh://git@github.com/o/r.git"),
            Some(("o".into(), "r".into()))
        );
        assert_eq!(parse_github_remote("https://gitlab.com/o/r.git"), None);
    }
}
