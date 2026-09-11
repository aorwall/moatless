//! Text rendering helpers for GitHub PR adapter messages.

use std::fmt::Write;

/// Inputs for [`pr_opened_text`] — grouped to keep the signature small.
pub(super) struct PrOpenedText<'a> {
    pub(super) pr_number: u64,
    pub(super) title: &'a str,
    pub(super) sender: &'a str,
    pub(super) head_ref: &'a str,
    pub(super) base_ref: &'a str,
    pub(super) html_url: &'a str,
    pub(super) owner: &'a str,
    pub(super) repo: &'a str,
}

/// Initial task message for a newly opened PR. Deliberately minimal — the PR
/// description and comments evolve after task creation, so instead of
/// inlining them (they'd go stale) the agent is pointed at the `moat gh`
/// commands that read the live state.
pub(super) fn pr_opened_text(p: &PrOpenedText<'_>) -> String {
    let mut text = format!(
        "## New Pull Request: #{}\n\n\
         **Title:** {}\n\
         **Author:** @{}\n\
         **Branch:** {} → {}\n",
        p.pr_number, p.title, p.sender, p.head_ref, p.base_ref,
    );

    if !p.html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {}", p.html_url);
    }

    let (pr_number, owner, repo) = (p.pr_number, p.owner, p.repo);
    let _ = write!(
        text,
        "\nBefore working on this PR, fetch the current state — the description \
         and comments evolve and must be read live, not from this message:\n\n\
         - Overview and description: `moat gh pr view {pr_number} --comments`\n\
         - Unified diff: `moat gh pr diff {pr_number}`\n\
         - Review/inline comments (with ids): `moat gh api repos/{owner}/{repo}/pulls/{pr_number}/comments`\n\
         - Conversation comments (with ids): `moat gh api repos/{owner}/{repo}/issues/{pr_number}/comments`\n"
    );

    text
}

/// Task message for a synchronized PR update.
pub(super) fn pr_synchronized_text(
    sender: &str,
    pr_number: u64,
    head_ref: &str,
    html_url: &str,
    before: &str,
    after: &str,
) -> String {
    let mut text = format!("**@{sender}** pushed new commits to PR #{pr_number} (`{head_ref}`)\n");

    if !html_url.is_empty() {
        let _ = writeln!(text, "{html_url}");
    }

    if !before.is_empty() && !after.is_empty() {
        let short_before = &before[..7.min(before.len())];
        let short_after = &after[..7.min(after.len())];
        let _ = writeln!(text, "\n`{short_before}` → `{short_after}`");
    }

    text
}

/// Task message for an `issue_comment` webhook. Carries the PR/issue number
/// and the numeric comment id so the agent can reply via
/// `moat gh pr reply` / the GitHub API without guessing ids.
pub(super) fn issue_comment_text(sender: &str, number: u64, comment_id: u64, body: &str) -> String {
    format!("**@{sender}** commented on #{number} (comment id: {comment_id}):\n\n{body}")
}

/// The line [`review_comment_text`] writes for a comment that opens its own
/// thread. [`super::adapter`] interpolates it into the agent's instructions, so
/// the agent is told to decide on the string a message actually carries.
pub(super) const NEW_THREAD_MARKER: &str = "> Starts a new thread — replies to nobody";

/// The line [`review_comment_text`] writes for a comment with a parent, ahead
/// of the parent's id. Interpolated into the instructions like
/// [`NEW_THREAD_MARKER`].
pub(super) const IN_REPLY_MARKER: &str = "> In reply to comment";

/// Inputs for [`review_comment_text`] — grouped to keep the signature small.
pub(super) struct ReviewCommentText<'a> {
    pub(super) sender: &'a str,
    pub(super) path: &'a str,
    pub(super) start_line: Option<u64>,
    pub(super) line: Option<u64>,
    pub(super) html_url: &'a str,
    pub(super) comment_id: u64,
    pub(super) in_reply_to_id: Option<u64>,
    pub(super) body: &'a str,
}

/// Task message for a `pull_request_review_comment` webhook (inline comment).
pub(super) fn review_comment_text(p: &ReviewCommentText<'_>) -> String {
    let mut text = format!("**@{}** commented on `{}`", p.sender, p.path);

    match (p.start_line, p.line) {
        (Some(start), Some(end)) if start != end => {
            let _ = write!(text, " (lines {start}-{end})");
        }
        (_, Some(ln)) => {
            let _ = write!(text, " (line {ln})");
        }
        _ => {}
    }
    text.push_str(":\n");

    if !p.html_url.is_empty() {
        let _ = writeln!(text, "{}", p.html_url);
    }
    let _ = writeln!(text, "Comment id: {}", p.comment_id);

    // State both cases. An absent `in_reply_to_id` is the fact that settles
    // whether the comment is addressed to the agent, and rendering only the
    // reply case leaves the agent to read meaning into a missing line.
    match p.in_reply_to_id {
        Some(reply_id) => {
            let _ = write!(text, "\n{IN_REPLY_MARKER} {reply_id}\n");
        }
        None => {
            let _ = write!(text, "\n{NEW_THREAD_MARKER}\n");
        }
    }

    let _ = write!(text, "\n{}", p.body);
    text
}

/// Task message for a submitted `pull_request_review`. The review's inline
/// comments arrive as separate review-comment events; the trailing hint tells
/// the agent how to list them together by review id.
pub(super) fn review_text(
    sender: &str,
    state_label: &str,
    review_id: u64,
    body: &str,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> String {
    // A bare verdict (e.g. an approval submitted with no summary) has an empty
    // body — render a placeholder so the message reads cleanly instead of
    // leaving a blank gap.
    let body_section = if body.is_empty() {
        "_(no review comment)_"
    } else {
        body
    };
    format!(
        "**@{sender}** — {state_label} (review id: {review_id}):\n\n{body_section}\n\n\
         Inline comments on this review: \
         `moat gh api repos/{owner}/{repo}/pulls/{pr_number}/reviews/{review_id}/comments`"
    )
}

/// Task message for a PR that GitHub reports as having merge conflicts
/// (`mergeable_state == "dirty"`). Points the agent at the live diff and
/// suggests the usual resolution path.
pub(super) fn merge_conflict_text(
    owner: &str,
    repo: &str,
    pr_number: u64,
    html_url: &str,
) -> String {
    let mut text = format!(
        "## ⚠️ Merge Conflicts on PR #{pr_number}\n\n\
         GitHub reports this PR can no longer be merged cleanly — the head \
         branch has conflicts with the base branch.\n",
    );

    if !html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {html_url}");
    }

    let _ = write!(
        text,
        "\nInspect and resolve the conflicts, then push the fix:\n\n\
         - Current diff: `moat gh pr diff {pr_number}`\n\
         - PR overview: `moat gh pr view {pr_number}`\n\
         - Conflicting files: `moat gh api repos/{owner}/{repo}/pulls/{pr_number}` (see `mergeable_state`)\n"
    );

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_opened_text_is_minimal_with_live_read_instructions() {
        let text = pr_opened_text(&PrOpenedText {
            pr_number: 42,
            title: "feat: x",
            sender: "alice",
            head_ref: "feat/x",
            base_ref: "main",
            html_url: "https://github.com/o/r/pull/42",
            owner: "o",
            repo: "r",
        });
        assert!(text.contains("## New Pull Request: #42"));
        assert!(text.contains("**Title:** feat: x"));
        assert!(text.contains("**Author:** @alice"));
        assert!(text.contains("**Branch:** feat/x → main"));
        assert!(text.contains("**Link:** https://github.com/o/r/pull/42"));
        assert!(!text.contains("### Description"));
        assert!(text.contains("`moat gh pr view 42 --comments`"));
        assert!(text.contains("`moat gh pr diff 42`"));
        assert!(text.contains("`moat gh api repos/o/r/pulls/42/comments`"));
        assert!(text.contains("`moat gh api repos/o/r/issues/42/comments`"));
    }

    #[test]
    fn pr_opened_text_omits_link_line_when_url_missing() {
        let text = pr_opened_text(&PrOpenedText {
            pr_number: 7,
            title: "t",
            sender: "bob",
            head_ref: "h",
            base_ref: "b",
            html_url: "",
            owner: "o",
            repo: "r",
        });
        assert!(!text.contains("**Link:**"));
    }

    #[test]
    fn issue_comment_text_carries_number_and_comment_id() {
        let text = issue_comment_text("alice", 42, 123_456, "looks good");
        assert_eq!(
            text,
            "**@alice** commented on #42 (comment id: 123456):\n\nlooks good"
        );
    }

    #[test]
    fn review_comment_text_carries_comment_id() {
        let text = review_comment_text(&ReviewCommentText {
            sender: "alice",
            path: "src/lib.rs",
            start_line: None,
            line: Some(10),
            html_url: "https://github.com/o/r/pull/42#discussion_r777",
            comment_id: 777,
            in_reply_to_id: None,
            body: "rename this",
        });
        assert!(text.contains("**@alice** commented on `src/lib.rs` (line 10):"));
        assert!(text.contains("https://github.com/o/r/pull/42#discussion_r777"));
        assert!(text.contains("Comment id: 777"));
        assert!(!text.contains("```diff"));
        assert!(text.ends_with("rename this"));
    }

    #[test]
    fn review_comment_text_states_whether_the_comment_replies_to_anything() {
        let mut p = ReviewCommentText {
            sender: "alice",
            path: "src/lib.rs",
            start_line: None,
            line: Some(10),
            html_url: "",
            comment_id: 777,
            in_reply_to_id: None,
            body: "rename this",
        };
        assert!(review_comment_text(&p).contains(NEW_THREAD_MARKER));

        p.in_reply_to_id = Some(555);
        let reply = review_comment_text(&p);
        assert!(reply.contains(&format!("{IN_REPLY_MARKER} 555")));
        assert!(!reply.contains(NEW_THREAD_MARKER));
    }

    #[test]
    fn review_text_carries_review_id_and_inline_comments_hint() {
        let text = review_text("alice", "Changes Requested", 555, "see notes", "o", "r", 42);
        assert!(text.starts_with("**@alice** — Changes Requested (review id: 555):\n\nsee notes"));
        assert!(text.contains("`moat gh api repos/o/r/pulls/42/reviews/555/comments`"));
    }

    #[test]
    fn review_text_renders_placeholder_for_empty_body() {
        let text = review_text("alice", "Approved", 555, "", "o", "r", 42);
        assert!(
            text.starts_with("**@alice** — Approved (review id: 555):\n\n_(no review comment)_")
        );
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn merge_conflict_text_points_at_live_diff() {
        let text = merge_conflict_text("o", "r", 42, "https://github.com/o/r/pull/42");
        assert!(text.contains("## ⚠️ Merge Conflicts on PR #42"));
        assert!(text.contains("**Link:** https://github.com/o/r/pull/42"));
        assert!(text.contains("`moat gh pr diff 42`"));
        assert!(text.contains("`moat gh api repos/o/r/pulls/42`"));
    }

    #[test]
    fn merge_conflict_text_omits_link_when_url_missing() {
        let text = merge_conflict_text("o", "r", 7, "");
        assert!(!text.contains("**Link:**"));
    }
}
