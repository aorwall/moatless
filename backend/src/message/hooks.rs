//! Hooks invoked after agent messages are persisted.
//!
//! Inspects the persisted `tool_result` blocks for known `gh ...` commands
//! and dispatches side effects:
//! - `gh pr create` → connect the channel thread for the new PR + append a
//!   `Moatless task: <url>` footer to the PR body.
//! - `gh pr comment` / `gh issue comment` → tag the persisted tool_result row
//!   with the comment id so the inbound webhook is deduped per-task.

use std::fmt::Write;

use serde_json::json;
use tracing::{debug, info, warn};

use super::models::MessageType;
use crate::auth::models::ProviderAccessToken;
use crate::channel::github_pr::api::GithubPrApi;
use crate::state::AppState;

/// Footer marker so we can detect if the URL was already appended (and skip
/// re-appending on retries). Stable across runs.
const FOOTER_MARKER: &str = "Moatless task:";

/// Identity of a PR parsed from `gh pr create` output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPr {
    owner: String,
    repo: String,
    number: u64,
}

impl ParsedPr {
    fn pr_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/pull/{}",
            self.owner, self.repo, self.number
        )
    }
}

/// Inspect the just-persisted message for known `gh ...` tool calls and run
/// the matching side effects. Errors are logged and swallowed — the event
/// handler loop is fire-and-forget.
///
/// `_message_uuid` is the System tool_result row's uuid. We intentionally do
/// not tag that row anymore — the gh side effects look up the parent Assistant
/// row's uuid via `find_tool_call` and tag it instead so the channel chip
/// renders on the visible assistant bubble. The argument is kept for
/// symmetry with sibling persistence hooks.
pub async fn on_agent_message_persisted(
    state: &AppState,
    task_id: &str,
    _message_uuid: &str,
    message_type: MessageType,
    content: &serde_json::Value,
) {
    // Tool results land on system-role messages (SDK "user" role).
    if message_type != MessageType::System {
        return;
    }

    let Some(blocks) = content.as_array() else {
        return;
    };

    for block in blocks {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        let Some(tool_use_id) = block.get("tool_use_id").and_then(|v| v.as_str()) else {
            continue;
        };

        let tool_call = match state.messages.find_tool_call(task_id, tool_use_id).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                debug!(task_id, tool_use_id, "tool_use not found for tool_result");
                continue;
            }
            Err(e) => {
                warn!(task_id, tool_use_id, error = %e, "find_tool_call failed");
                continue;
            }
        };

        let raw_cmd = tool_call
            .tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if raw_cmd.is_empty() {
            continue;
        }

        // Claude Code's `Bash` and Codex's `codex.shell` use the same
        // tool_input.command field, but Codex always wraps the command as
        // `/bin/bash -lc '<inner>'` — unwrap it so the gh subcommand parser
        // can match.
        let cmd: &str = match tool_call.tool_name.as_str() {
            "Bash" => raw_cmd,
            "codex.shell" => unwrap_shell_wrapper(raw_cmd),
            _ => continue,
        };

        let result_text = block.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let parent_uuid = tool_call.parent_message_uuid.as_str();

        if is_gh_pr_create(cmd) {
            handle_gh_pr_create(state, task_id, parent_uuid, result_text).await;
        } else if is_gh_pr_comment(cmd) || is_gh_issue_comment(cmd) {
            handle_gh_comment(state, task_id, parent_uuid, result_text).await;
        }
    }
}

// =============================================================================
// gh pr create
// =============================================================================

async fn handle_gh_pr_create(
    state: &AppState,
    task_id: &str,
    parent_uuid: &str,
    result_text: &str,
) {
    let Some(pr) = parse_pr_url(result_text) else {
        debug!(task_id, "gh pr create result had no PR URL");
        return;
    };

    let source_id = format!("{}/{}", pr.owner, pr.repo);
    let thread_properties = json!({
        "owner": pr.owner,
        "repo": pr.repo,
        "pr_number": pr.number,
    });

    match crate::channel::listen::connect_github_subscription(
        state,
        &source_id,
        task_id,
        thread_properties,
        None,
    )
    .await
    {
        Ok((rule, subscription)) => {
            info!(
                task_id,
                pr_number = pr.number,
                rule_id = %rule.id,
                subscription_id = %subscription.id,
                "Auto-connected GitHub PR subscription for PR created by agent"
            );
        }
        Err(e) => {
            warn!(task_id, pr_number = pr.number, error = %e, "Failed to auto-connect GitHub PR subscription");
        }
    }

    let external_id = crate::channel::external_id::github_pr_opened(pr.number);
    state.recently_sent.note(task_id, &external_id);
    match state
        .messages
        .set_channel_metadata(parent_uuid, &external_id, "github_pr", Some(&source_id))
        .await
    {
        Ok(true) => {
            info!(
                task_id,
                parent_uuid,
                pr_number = pr.number,
                "Tagged assistant tool_use row with channel_external_id for gh pr create"
            );
        }
        Ok(false) => {
            debug!(
                task_id,
                parent_uuid,
                pr_number = pr.number,
                "Assistant row already had channel_external_id — leaving untouched"
            );
        }
        Err(e) => {
            warn!(task_id, parent_uuid, pr_number = pr.number, error = %e, "Failed to tag assistant row with channel_external_id");
        }
    }

    try_append_task_footer(state, task_id, &pr).await;
}

async fn try_append_task_footer(state: &AppState, task_id: &str, pr: &ParsedPr) {
    let frontend_url = state.config.auth.frontend_url.trim_end_matches('/');
    if frontend_url.is_empty() {
        debug!(task_id, "frontend_url empty — skipping PR body footer");
        return;
    }
    let task_url = format!("{frontend_url}/tasks/{task_id}");

    let task = match state.tasks.find_by_id_internal(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(task_id, "Task not found when appending PR footer");
            return;
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to load task for PR footer");
            return;
        }
    };

    let Some(user_id) = task.created_by.as_deref() else {
        debug!(task_id, "Task has no created_by — skipping PR footer");
        return;
    };

    let token: ProviderAccessToken = match state
        .provider_tokens
        .find_provider_token(user_id, "github")
        .await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(
                task_id,
                user_id, "No GitHub provider token for user — skipping PR footer"
            );
            return;
        }
        Err(e) => {
            warn!(task_id, user_id, error = %e, "Failed to look up provider token for PR footer");
            return;
        }
    };

    let api = GithubPrApi::new(&token.access_token);
    let pr_info = match api.get_pr(&pr.owner, &pr.repo, pr.number).await {
        Ok(p) => p,
        Err(e) => {
            warn!(task_id, pr_url = %pr.pr_url(), error = %e, "Failed to fetch PR for footer");
            return;
        }
    };

    let existing = pr_info.body.as_deref().unwrap_or("");
    if existing.contains(FOOTER_MARKER) {
        return;
    }
    let new_body = compose_footer(existing, &task_url);

    if let Err(e) = api
        .update_pr_body(&pr.owner, &pr.repo, pr.number, &new_body)
        .await
    {
        warn!(task_id, pr_url = %pr.pr_url(), error = %e, "Failed to update PR body with footer");
        return;
    }
    info!(
        task_id,
        pr_number = pr.number,
        "Appended task URL footer to PR body"
    );
}

fn compose_footer(existing: &str, task_url: &str) -> String {
    let trimmed = existing.trim_end_matches('\n').to_string();
    if trimmed.is_empty() {
        format!("{FOOTER_MARKER} {task_url}\n")
    } else {
        let mut out = trimmed;
        let _ = write!(out, "\n\n---\n{FOOTER_MARKER} {task_url}\n");
        out
    }
}

// =============================================================================
// gh pr comment / gh issue comment
// =============================================================================

async fn handle_gh_comment(state: &AppState, task_id: &str, parent_uuid: &str, result_text: &str) {
    let Some(parsed) = parse_comment_id(result_text) else {
        debug!(task_id, "gh comment result had no comment URL");
        return;
    };

    // Wrap in the issue-comment namespace so the tagged row matches what the
    // GitHub `issue_comment` webhook will produce for the same comment.
    let external_id = crate::channel::external_id::github_issue_comment(parsed.id);
    state.recently_sent.note(task_id, &external_id);
    let source_name = format!("{}/{}", parsed.owner, parsed.repo);
    match state
        .messages
        .set_channel_metadata(parent_uuid, &external_id, "github_pr", Some(&source_name))
        .await
    {
        Ok(true) => {
            info!(
                task_id,
                parent_uuid,
                comment_id = parsed.id,
                "Tagged assistant tool_use row with channel_external_id for gh comment"
            );
        }
        Ok(false) => {
            debug!(
                task_id,
                parent_uuid,
                comment_id = parsed.id,
                "Assistant row already had channel_external_id — leaving untouched"
            );
        }
        Err(e) => {
            warn!(task_id, parent_uuid, comment_id = parsed.id, error = %e, "Failed to tag assistant row with channel_external_id");
        }
    }
}

// =============================================================================
// Command parsing
// =============================================================================

/// Detect whether a `gh ...` command line invokes `pr create`.
///
/// Tolerates leading global flags (e.g. `gh -R owner/repo pr create ...`).
fn is_gh_pr_create(cmd: &str) -> bool {
    matches_gh_subcommand(cmd, &["pr", "create"])
}

fn is_gh_pr_comment(cmd: &str) -> bool {
    matches_gh_subcommand(cmd, &["pr", "comment"])
}

fn is_gh_issue_comment(cmd: &str) -> bool {
    matches_gh_subcommand(cmd, &["issue", "comment"])
}

/// Detect whether `cmd` is a `gh` invocation whose first two non-flag
/// positionals match `expected` (e.g. `["pr", "create"]`).
///
/// Tolerates leading env-var assignments (`GH_REPO=owner/repo gh pr create …`)
/// and global flags like `-R owner/repo` / `--repo owner/repo` before, between,
/// and after the positionals.
fn matches_gh_subcommand(cmd: &str, expected: &[&str; 2]) -> bool {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let Some(gh_idx) = tokens.iter().position(|&t| t == "gh") else {
        return false;
    };
    // All preceding tokens must be env-var assignments — otherwise this is a
    // different command (e.g. `git push && gh …`) we conservatively skip.
    if !tokens[..gh_idx].iter().all(|t| is_env_assignment(t)) {
        return false;
    }

    let mut iter = tokens.iter().skip(gh_idx + 1).peekable();
    let mut matched = 0usize;
    while let Some(&tok) = iter.next() {
        if matches!(tok, "-R" | "--repo") {
            iter.next();
            continue;
        }
        if tok.starts_with('-') {
            continue;
        }
        if tok != expected[matched] {
            return false;
        }
        matched += 1;
        if matched == expected.len() {
            return true;
        }
    }
    false
}

/// Strip a `<shell> -<flags> '<inner>'` wrapper that Codex always wraps
/// shell commands in (production form: `/bin/bash -lc '<inner>'`). Falls
/// through unchanged if no recognized wrapper is present.
fn unwrap_shell_wrapper(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    const PREFIXES: &[&str] = &[
        "/bin/bash -lc ",
        "/bin/bash -c ",
        "/bin/sh -lc ",
        "/bin/sh -c ",
        "bash -lc ",
        "bash -c ",
        "sh -c ",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            if let Some(inner) = rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                return inner;
            }
            if let Some(inner) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                return inner;
            }
            return rest;
        }
    }
    cmd
}

/// `KEY=VALUE` form, where KEY is an alphanumeric/underscore identifier.
fn is_env_assignment(tok: &str) -> bool {
    let Some(eq) = tok.find('=') else {
        return false;
    };
    let key = &tok[..eq];
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// =============================================================================
// Output parsing
// =============================================================================

/// Find the LAST GitHub PR URL in `text` and decompose into owner/repo/number.
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
    let number_token = parts.next()?;
    // Strip `#issuecomment-...` or trailing path segments from the number.
    let number_str = number_token
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("");
    let number: u64 = number_str.parse().ok()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(ParsedPr {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    })
}

/// Identity of an issue/PR comment URL parsed from `gh pr/issue comment`
/// output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedComment {
    owner: String,
    repo: String,
    id: u64,
}

/// Extract owner/repo + the numeric id after `#issuecomment-` in
/// `gh pr/issue comment` output (e.g.
/// `https://github.com/foo/bar/pull/42#issuecomment-12345`).
///
/// Returns `None` when no `#issuecomment-` URL is present or the URL shape
/// doesn't have the expected `owner/repo/(pull|issues)/N#issuecomment-N`
/// segments.
fn parse_comment_id(text: &str) -> Option<ParsedComment> {
    let needle = "#issuecomment-";
    let idx = text.find(needle)?;

    // Walk backwards to the start of the URL.
    let prefix = "https://github.com/";
    let url_start = text[..idx].rfind(prefix)?;
    let url = &text[url_start..idx];

    // url is `https://github.com/<owner>/<repo>/<pull|issues>/<n>`.
    let path = url.strip_prefix(prefix)?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let kind = parts.next()?;
    if kind != "pull" && kind != "issues" {
        return None;
    }

    let rest = &text[idx + needle.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let id: u64 = rest[..end].parse().ok()?;

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some(ParsedComment {
        owner: owner.to_string(),
        repo: repo.to_string(),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_url_basic() {
        let pr = parse_pr_url("https://github.com/foo/bar/pull/42\n").unwrap();
        assert_eq!(
            pr,
            ParsedPr {
                owner: "foo".into(),
                repo: "bar".into(),
                number: 42
            }
        );
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
        assert!(parse_pr_url("Opening github.com in your browser.\n").is_none());
    }

    #[test]
    fn parse_pr_url_ignores_non_pr_urls() {
        assert!(parse_pr_url("https://github.com/foo/bar/issues/42\n").is_none());
        assert!(parse_pr_url("https://github.com/foo/bar\n").is_none());
        assert!(parse_pr_url("plain text with no urls").is_none());
    }

    #[test]
    fn is_gh_pr_create_simple() {
        assert!(is_gh_pr_create("gh pr create"));
        assert!(is_gh_pr_create("gh pr create --title foo --body bar"));
    }

    #[test]
    fn is_gh_pr_create_with_flags() {
        assert!(is_gh_pr_create("gh pr --draft create --title x"));
    }

    #[test]
    fn is_gh_pr_create_with_dash_r() {
        assert!(is_gh_pr_create("gh -R o/r pr create"));
        assert!(is_gh_pr_create("gh --repo o/r pr create"));
    }

    #[test]
    fn is_gh_pr_create_negative() {
        assert!(!is_gh_pr_create("gh pr view 42"));
        assert!(!is_gh_pr_create("gh issue create"));
        assert!(!is_gh_pr_create(""));
        assert!(!is_gh_pr_create("git pr create"));
        // Don't match across `&&` — leading `git push` isn't an env assignment.
        assert!(!is_gh_pr_create("git push && gh pr create"));
    }

    #[test]
    fn is_gh_pr_create_with_env_prefix() {
        assert!(is_gh_pr_create(
            "GH_REPO=Frank-Penny/foo gh pr create --title bar"
        ));
        assert!(is_gh_pr_create(
            "GH_REPO=Frank-Penny/foo   gh pr create --base main --head fix/foo --title \"x y\" --body-file /tmp/b.md 2>&1 | tee /tmp/out"
        ));
        assert!(is_gh_pr_create("FOO=1 BAR=2 gh pr create"));
    }

    #[test]
    fn is_gh_pr_comment_simple() {
        assert!(is_gh_pr_comment("gh pr comment 42 --body hi"));
        assert!(is_gh_pr_comment("gh -R o/r pr comment 42"));
        assert!(!is_gh_pr_comment("gh pr create"));
        assert!(!is_gh_pr_comment("gh issue comment 1"));
    }

    #[test]
    fn is_gh_issue_comment_simple() {
        assert!(is_gh_issue_comment("gh issue comment 1 --body hi"));
        assert!(!is_gh_issue_comment("gh pr comment 1"));
    }

    #[test]
    fn parse_comment_id_basic() {
        let parsed =
            parse_comment_id("https://github.com/foo/bar/pull/42#issuecomment-12345\n").unwrap();
        assert_eq!(
            parsed,
            ParsedComment {
                owner: "foo".into(),
                repo: "bar".into(),
                id: 12345,
            }
        );
    }

    #[test]
    fn parse_comment_id_for_issue_url() {
        let parsed =
            parse_comment_id("https://github.com/foo/bar/issues/7#issuecomment-99\n").unwrap();
        assert_eq!(parsed.owner, "foo");
        assert_eq!(parsed.repo, "bar");
        assert_eq!(parsed.id, 99);
    }

    #[test]
    fn parse_comment_id_none() {
        assert!(parse_comment_id("no comment url here").is_none());
        assert!(parse_comment_id("https://github.com/foo/bar/pull/42").is_none());
        // Wrong path segment.
        assert!(
            parse_comment_id("https://github.com/foo/bar/discussions/1#issuecomment-1").is_none(),
        );
    }

    #[test]
    fn footer_appends_to_existing_body() {
        let body = "## Summary\nFix bug.\n\n## Test plan\n- [x] Tests";
        let out = compose_footer(body, "https://moatless.dev/tasks/t_1");
        assert!(out.starts_with("## Summary"));
        assert!(out.contains("---\nMoatless task: https://moatless.dev/tasks/t_1\n"));
        assert!(out.contains("- [x] Tests"));
    }

    #[test]
    fn footer_handles_empty_body() {
        let out = compose_footer("", "https://moatless.dev/tasks/t_2");
        assert_eq!(out, "Moatless task: https://moatless.dev/tasks/t_2\n");
    }

    #[test]
    fn footer_trims_trailing_newlines_before_separator() {
        let out = compose_footer("body\n\n\n", "https://x/tasks/t");
        assert_eq!(out, "body\n\n---\nMoatless task: https://x/tasks/t\n");
    }

    #[test]
    fn unwrap_shell_wrapper_strips_bin_bash_lc() {
        assert_eq!(
            unwrap_shell_wrapper("/bin/bash -lc 'gh pr create --title foo'"),
            "gh pr create --title foo"
        );
    }

    #[test]
    fn unwrap_shell_wrapper_strips_double_quoted() {
        assert_eq!(
            unwrap_shell_wrapper("/bin/bash -c \"gh pr view 1\""),
            "gh pr view 1"
        );
    }

    #[test]
    fn unwrap_shell_wrapper_passthrough_when_no_wrapper() {
        assert_eq!(
            unwrap_shell_wrapper("gh pr create --title foo"),
            "gh pr create --title foo"
        );
    }

    #[test]
    fn is_gh_pr_create_codex_wrapped() {
        let raw = "/bin/bash -lc 'gh pr create --title foo --body bar'";
        assert!(is_gh_pr_create(unwrap_shell_wrapper(raw)));
    }

    #[test]
    fn is_gh_pr_comment_codex_wrapped() {
        let raw = "/bin/bash -lc 'gh pr comment 42 --body hi'";
        assert!(is_gh_pr_comment(unwrap_shell_wrapper(raw)));
    }

    #[test]
    fn is_gh_issue_comment_codex_wrapped() {
        let raw = "/bin/bash -lc 'gh issue comment 1 --body hi'";
        assert!(is_gh_issue_comment(unwrap_shell_wrapper(raw)));
    }

    #[test]
    fn is_gh_pr_create_codex_with_repo_flag() {
        let raw = "/bin/bash -lc 'gh -R o/r pr create'";
        assert!(is_gh_pr_create(unwrap_shell_wrapper(raw)));
    }
}
