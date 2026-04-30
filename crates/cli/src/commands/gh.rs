//! `gh` proxy command.
//!
//! Passes args straight through to the real `gh` CLI. When running inside a
//! task sandbox (`TASK_ID` set) AND the args invoke `pr create`, parses the
//! resulting PR URL and calls `connect_channel` so reviewer comments on the
//! new PR flow back into the task's channel feed.
//!
//! Standalone invocations (no `TASK_ID`) are pure passthrough — no API
//! traffic, no token refresh.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use moatless_api_schemas::channel::ConnectChannelRequest;
use moatless_api_schemas::enums::ChannelType;
use serde_json::json;
use tokio::process::Command;

use crate::client::{ClientError, MoatClient};
use crate::config::Config;
use crate::output;

// reason: token freshness is handled upstream — sandbox/src/init/{credentials,refresher}.rs
// rewrite ~/.config/gh/hosts.yml on every agent turn and every 15 min, and `gh auth
// git-credential` reads it on each invocation. No per-call refresh needed here.

/// Directory where the agent's `gh` shim is installed. `real_gh_path()`
/// filters this out of `which gh` candidates so the proxy never re-enters
/// itself when invoked from the agent's PATH.
const SHIM_DIR: &str = "/opt/moatless/shims";

/// Hard fallback when neither `MOAT_GH_REAL` nor `which` find a non-shim `gh`.
const DEFAULT_GH_PATH: &str = "/usr/bin/gh";

pub async fn run(
    client: &MoatClient,
    cfg: &Config,
    args: Vec<String>,
    json: bool,
) -> Result<(), ClientError> {
    let real_gh = real_gh_path();
    let task_id = cfg.task_id().map(String::from);
    let trigger_connect = task_id.is_some() && is_pr_create(&args);

    if trigger_connect {
        run_with_connect(client, &real_gh, &args, task_id.as_deref().unwrap_or(""), json).await
    } else {
        run_passthrough(&real_gh, &args).await
    }
}

/// Pure passthrough: inherit stdio, propagate exit code via `std::process::exit`.
async fn run_passthrough(real_gh: &Path, args: &[String]) -> Result<(), ClientError> {
    let status = Command::new(real_gh)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|e| spawn_err(real_gh, e))?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// `gh pr create` flow: capture stdout to find the PR URL, mirror it to our
/// own stdout, then auto-connect the channel thread on success.
async fn run_with_connect(
    client: &MoatClient,
    real_gh: &Path,
    args: &[String],
    task_id: &str,
    json: bool,
) -> Result<(), ClientError> {
    let output = Command::new(real_gh)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .await
        .map_err(|e| spawn_err(real_gh, e))?;

    // Mirror gh's stdout to our own so the agent still sees the PR URL.
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&output.stdout);
    let _ = out.flush();
    drop(out);

    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    if let Some(pr) = parse_pr_url(&stdout_str) {
        try_connect_channel(client, task_id, &pr, json).await;
    }
    Ok(())
}

fn spawn_err(real_gh: &Path, e: std::io::Error) -> ClientError {
    if e.kind() == std::io::ErrorKind::NotFound {
        ClientError::Other(format!(
            "gh CLI not found at {} — install gh or set MOAT_GH_REAL",
            real_gh.display()
        ))
    } else {
        ClientError::Other(format!("Failed to spawn gh ({}): {e}", real_gh.display()))
    }
}

/// Identity of a PR parsed from `gh pr create` output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPr {
    owner: String,
    repo: String,
    number: u64,
}

/// Detect whether `args` invoke `pr create`.
///
/// Tolerates leading global flags (e.g. `gh -R owner/repo pr create ...`).
fn is_pr_create(args: &[String]) -> bool {
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        // gh global flags that take a value — skip the value too.
        if matches!(arg.as_str(), "-R" | "--repo") {
            iter.next();
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        if arg != "pr" {
            return false;
        }
        // Found `pr` — next non-flag positional must be `create`.
        for next in iter.by_ref() {
            if next.starts_with('-') {
                continue;
            }
            return next == "create";
        }
        return false;
    }
    false
}

/// Find the LAST GitHub PR URL in `text` and decompose into owner/repo/number.
///
/// Returns `None` when the text contains no PR URL (e.g. `gh pr create --web`
/// only prints "Opening github.com in your browser.").
fn parse_pr_url(text: &str) -> Option<ParsedPr> {
    let needle = "https://github.com/";
    let mut last: Option<ParsedPr> = None;
    let mut start = 0;
    while let Some(idx) = text[start..].find(needle) {
        let abs = start + idx;
        let rest = &text[abs..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let url = &rest[..end];
        if let Some(pr) = decompose_pr_url(url) {
            last = Some(pr);
        }
        start = abs + needle.len();
    }
    last
}

fn decompose_pr_url(url: &str) -> Option<ParsedPr> {
    let path = url.strip_prefix("https://github.com/")?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next() != Some("pull") {
        return None;
    }
    let number: u64 = parts.next()?.parse().ok()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(ParsedPr {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    })
}

async fn try_connect_channel(client: &MoatClient, task_id: &str, pr: &ParsedPr, json_output: bool) {
    let req = ConnectChannelRequest {
        task_id: task_id.to_string(),
        provider: ChannelType::GithubPr,
        source_id: Some(format!("{}/{}", pr.owner, pr.repo)),
        external_thread_id: pr.number.to_string(),
        thread_properties: Some(json!({
            "owner": pr.owner,
            "repo": pr.repo,
            "pr_number": pr.number,
        })),
        filter_user_id: None,
    };

    match client.api().connect_channel(&req).await {
        Ok(resp) => {
            if json_output {
                output::print_json_typed(&resp);
            } else {
                eprintln!(
                    "[moat gh] Connected PR #{} ({}/{}) → channel thread (config={}, thread={})",
                    pr.number, pr.owner, pr.repo, resp.config_id, resp.thread_id
                );
            }
        }
        Err(e) => {
            eprintln!(
                "[moat gh] PR #{} created but channel auto-connect failed: {e}",
                pr.number
            );
        }
    }
}

/// Resolve the path to the real `gh` binary, filtering out the shim directory.
///
/// Lookup order:
/// 1. `MOAT_GH_REAL` env var (set by sandbox init)
/// 2. `which gh` candidates outside `SHIM_DIR`
/// 3. `DEFAULT_GH_PATH`
fn real_gh_path() -> PathBuf {
    if let Ok(p) = std::env::var("MOAT_GH_REAL") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(paths) = which::which_all_global("gh") {
        for path in paths {
            if !is_under_shim_dir(&path) {
                return path;
            }
        }
    }
    PathBuf::from(DEFAULT_GH_PATH)
}

fn is_under_shim_dir(path: &Path) -> bool {
    path.starts_with(SHIM_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn pr_create_simple() {
        assert!(is_pr_create(&[s("pr"), s("create")]));
    }

    #[test]
    fn pr_create_with_args() {
        assert!(is_pr_create(&[
            s("pr"),
            s("create"),
            s("--title"),
            s("foo"),
            s("--body"),
            s("bar"),
        ]));
    }

    #[test]
    fn pr_create_with_repo_flag_before() {
        assert!(is_pr_create(&[s("--repo"), s("o/r"), s("pr"), s("create")]));
        assert!(is_pr_create(&[s("-R"), s("o/r"), s("pr"), s("create")]));
    }

    #[test]
    fn pr_create_with_flag_between_pr_and_create() {
        assert!(is_pr_create(&[
            s("pr"),
            s("--draft"),
            s("create"),
            s("--title"),
            s("x"),
        ]));
    }

    #[test]
    fn pr_view_is_not_create() {
        assert!(!is_pr_create(&[s("pr"), s("view"), s("42")]));
    }

    #[test]
    fn issue_create_is_not_pr_create() {
        assert!(!is_pr_create(&[s("issue"), s("create")]));
    }

    #[test]
    fn empty_args() {
        assert!(!is_pr_create(&[]));
    }

    #[test]
    fn parse_pr_url_basic() {
        let pr = parse_pr_url("https://github.com/foo/bar/pull/42\n").unwrap();
        assert_eq!(
            pr,
            ParsedPr {
                owner: s("foo"),
                repo: s("bar"),
                number: 42
            }
        );
    }

    #[test]
    fn parse_pr_url_with_preceding_lines() {
        let input =
            "Creating pull request for branch...\nhttps://github.com/octocat/hello-world/pull/123\n";
        let pr = parse_pr_url(input).unwrap();
        assert_eq!(pr.number, 123);
        assert_eq!(pr.owner, "octocat");
        assert_eq!(pr.repo, "hello-world");
    }

    #[test]
    fn parse_pr_url_picks_last_match() {
        let input = "https://github.com/a/b/pull/1\nhttps://github.com/c/d/pull/2";
        let pr = parse_pr_url(input).unwrap();
        assert_eq!(pr.owner, "c");
        assert_eq!(pr.number, 2);
    }

    #[test]
    fn parse_pr_url_none_for_browser_message() {
        // gh pr create --web prints "Opening github.com in your browser..."
        assert!(parse_pr_url("Opening github.com in your browser.\n").is_none());
    }

    #[test]
    fn parse_pr_url_ignores_non_pr_urls() {
        assert!(parse_pr_url("https://github.com/foo/bar/issues/42\n").is_none());
        assert!(parse_pr_url("https://github.com/foo/bar\n").is_none());
        assert!(parse_pr_url("plain text with no urls").is_none());
    }

    #[test]
    fn parse_pr_url_handles_quotes() {
        let pr = parse_pr_url("\"https://github.com/foo/bar/pull/7\"").unwrap();
        assert_eq!(pr.number, 7);
    }

    #[test]
    fn shim_filter() {
        assert!(is_under_shim_dir(Path::new("/opt/moatless/shims/gh")));
        assert!(is_under_shim_dir(Path::new("/opt/moatless/shims/sub/gh")));
        assert!(!is_under_shim_dir(Path::new("/usr/bin/gh")));
        assert!(!is_under_shim_dir(Path::new("/usr/local/bin/gh")));
    }
}
