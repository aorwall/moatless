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
use crate::channel::provider::ChannelType;
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

        if tool_call.tool_name != "Bash" {
            continue;
        }

        let cmd = tool_call
            .tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if cmd.is_empty() {
            continue;
        }

        let result_text = block
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if is_gh_pr_create(cmd) {
            handle_gh_pr_create(state, task_id, result_text).await;
        } else if is_gh_pr_comment(cmd) || is_gh_issue_comment(cmd) {
            handle_gh_comment(state, task_id, _message_uuid, result_text).await;
        }
    }
}

// =============================================================================
// gh pr create
// =============================================================================

async fn handle_gh_pr_create(state: &AppState, task_id: &str, result_text: &str) {
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

    match crate::channel::listen::connect(
        state,
        ChannelType::GithubPr,
        &source_id,
        task_id,
        &pr.number.to_string(),
        thread_properties,
        None,
    )
    .await
    {
        Ok((config_id, thread)) => {
            info!(
                task_id,
                pr_number = pr.number,
                config_id = %config_id,
                thread_id = thread.id,
                "Auto-connected channel thread for PR created by agent"
            );
        }
        Err(e) => {
            warn!(task_id, pr_number = pr.number, error = %e, "Failed to auto-connect channel thread for PR");
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
            warn!(task_id, user_id, "No GitHub provider token for user — skipping PR footer");
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
    info!(task_id, pr_number = pr.number, "Appended task URL footer to PR body");
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

async fn handle_gh_comment(
    state: &AppState,
    task_id: &str,
    message_uuid: &str,
    result_text: &str,
) {
    let Some(comment_id) = parse_comment_id(result_text) else {
        debug!(task_id, "gh comment result had no comment URL");
        return;
    };

    let external_id = comment_id.to_string();
    match state
        .messages
        .set_channel_external_id(message_uuid, &external_id)
        .await
    {
        Ok(true) => {
            info!(
                task_id,
                message_uuid,
                comment_id,
                "Tagged tool_result with channel_external_id for gh comment"
            );
        }
        Ok(false) => {
            debug!(
                task_id,
                message_uuid,
                comment_id,
                "tool_result row already had channel_external_id — leaving untouched"
            );
        }
        Err(e) => {
            warn!(task_id, message_uuid, comment_id, error = %e, "Failed to tag tool_result with channel_external_id");
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

/// Extract the numeric id after `#issuecomment-` in `gh pr/issue comment`
/// output (e.g. `https://github.com/foo/bar/pull/42#issuecomment-12345`).
fn parse_comment_id(text: &str) -> Option<u64> {
    let needle = "#issuecomment-";
    let idx = text.find(needle)?;
    let rest = &text[idx + needle.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
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
        assert!(is_gh_pr_create(
            "gh pr create --title foo --body bar"
        ));
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
        assert!(is_gh_pr_create("GH_REPO=Frank-Penny/foo gh pr create --title bar"));
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
        assert_eq!(
            parse_comment_id("https://github.com/foo/bar/pull/42#issuecomment-12345\n"),
            Some(12345)
        );
    }

    #[test]
    fn parse_comment_id_none() {
        assert_eq!(parse_comment_id("no comment url here"), None);
        assert_eq!(parse_comment_id("https://github.com/foo/bar/pull/42"), None);
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
}
