//! Text rendering helpers for GitHub PR adapter messages.

use std::fmt::Write;

use super::api::CheckAnnotation;

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

/// Initial task message for a newly opened PR.
///
/// Deliberately minimal: the PR description and comments evolve after task
/// creation, so instead of inlining them (they'd go stale) the agent is pointed
/// at the `moat gh` commands that read live state.
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

/// Task message for an `issue_comment` webhook. Carries the PR/issue number and
/// the numeric comment id so the agent can reply via `moat gh pr reply` / the
/// GitHub API without guessing ids.
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

/// A check run's `output.summary`, which an app may fill with an entire test
/// log.
const CHECK_RUN_SUMMARY_MAX_LEN: usize = 2000;

/// Annotations rendered into a check-run notice.
const MAX_RENDERED_ANNOTATIONS: usize = 20;

/// Per-annotation budget, so one runaway message cannot crowd out the
/// annotations under it.
const MAX_ANNOTATION_MESSAGE_LEN: usize = 400;

/// Inputs for [`check_run_failed_text`] — grouped to keep the signature small.
pub(super) struct CheckRunFailedText<'a> {
    pub(super) name: &'a str,
    pub(super) conclusion: &'a str,
    pub(super) app_name: &'a str,
    pub(super) head_sha: &'a str,
    pub(super) html_url: &'a str,
    pub(super) output_title: &'a str,
    pub(super) output_summary: &'a str,
}

/// Task message for a check run that finished on a notifiable failure.
///
/// Ends on a newline so [`check_run_annotations_text`] can be appended to it
/// without running its heading into whatever the summary left behind.
pub(super) fn check_run_failed_text(p: &CheckRunFailedText<'_>) -> String {
    let short_sha = truncate_on_char_boundary(p.head_sha, 7);
    let mut text = format!(
        "## CI Check Failed: {}\n\n\
         **Conclusion:** {}\n\
         **CI:** {}\n\
         **Commit:** `{short_sha}`\n",
        p.name, p.conclusion, p.app_name,
    );

    if !p.html_url.is_empty() {
        let _ = writeln!(text, "**Link:** {}", p.html_url);
    }

    push_check_output(&mut text, p.output_title, p.output_summary);
    text
}

/// Append a check run's `### Output` block, unless it has neither a title nor
/// a summary.
fn push_check_output(text: &mut String, title: &str, summary: &str) {
    if title.is_empty() && summary.is_empty() {
        return;
    }
    text.push_str("\n### Output\n");
    if !title.is_empty() {
        let _ = writeln!(text, "**{title}**");
    }
    if !summary.is_empty() {
        let shown = truncate_on_char_boundary(summary, CHECK_RUN_SUMMARY_MAX_LEN);
        let ellipsis = if shown.len() < summary.len() {
            "..."
        } else {
            ""
        };
        let _ = writeln!(text, "{shown}{ellipsis}");
    }
}

/// Inputs for [`check_run_trigger_text`].
pub(super) struct CheckRunTriggerText<'a> {
    pub(super) name: &'a str,
    pub(super) conclusion: &'a str,
    pub(super) output_title: &'a str,
    pub(super) output_summary: &'a str,
}

/// The block naming the check run that opened a Task, appended to
/// [`pr_opened_text`]. Ends on a newline.
pub(super) fn check_run_trigger_text(p: &CheckRunTriggerText<'_>) -> String {
    let mut text = format!(
        "\n## Triggered by check: {}\n\n**Conclusion:** {}\n",
        p.name, p.conclusion,
    );
    push_check_output(&mut text, p.output_title, p.output_summary);
    text
}

/// The failing files and lines of a check run, as a markdown list.
///
/// Sorted by level so a truncated list keeps the failures: a lint job reports
/// its warnings through the same endpoint as the handful of errors that
/// actually failed it.
pub(super) fn check_run_annotations_text(annotations: &[CheckAnnotation]) -> String {
    if annotations.is_empty() {
        return String::new();
    }

    let mut ordered: Vec<&CheckAnnotation> = annotations.iter().collect();
    ordered.sort_by_key(|annotation| severity_rank(annotation.annotation_level.as_deref()));

    let mut text = String::from("\n### Annotations\n");
    for annotation in ordered.iter().take(MAX_RENDERED_ANNOTATIONS) {
        let _ = writeln!(text, "- {}", annotation_line(annotation));
    }
    let hidden = ordered.len().saturating_sub(MAX_RENDERED_ANNOTATIONS);
    if hidden > 0 {
        let _ = writeln!(text, "- _…and {hidden} more_");
    }
    text
}

/// `failure` before `warning` before everything else, for [`Vec::sort_by_key`],
/// whose stability leaves GitHub's own order intact within a level.
///
/// A missing level ranks as a failure, matching what [`annotation_line`] calls
/// it: ranking it below `warning` would make the cap drop the one thing the
/// list would have rendered as a failure.
fn severity_rank(level: Option<&str>) -> u8 {
    match level {
        Some("failure") | None => 0,
        Some("warning") => 1,
        _ => 2,
    }
}

fn annotation_line(annotation: &CheckAnnotation) -> String {
    let location = match (annotation.start_line, annotation.end_line) {
        (Some(start), Some(end)) if end != start => {
            format!("{}:{start}-{end}", annotation.path)
        }
        (Some(start), _) => format!("{}:{start}", annotation.path),
        (None, _) => annotation.path.clone(),
    };
    let level = annotation.annotation_level.as_deref().unwrap_or("failure");

    let full = annotation.message.as_deref().unwrap_or("").trim();
    let clipped = truncate_on_char_boundary(full, MAX_ANNOTATION_MESSAGE_LEN);
    let ellipsis = if clipped.len() < full.len() {
        "…"
    } else {
        ""
    };
    // Continuation lines are indented into the bullet, so a compiler message
    // that spans lines stays one list item.
    let message = format!("{clipped}{ellipsis}").replace('\n', "\n  ");

    if location.is_empty() {
        format!("{level}: {message}")
    } else {
        format!("`{location}` — {level}: {message}")
    }
}

/// Truncate to at most `max` bytes without splitting a UTF-8 character.
///
/// Every byte budget in this crate counts text anyone with write access can
/// author, and slicing a `str` mid-character panics.
pub(super) fn truncate_on_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// `body` with a line crediting `driver_login`, the person an App posted it
/// for; unchanged when there is none or the line is already there.
///
/// The login is a profile link rather than a bare `@login`: GitHub mentions
/// nobody inside a link, so the driver is not notified of every comment.
pub fn credit_driver(body: &str, driver_login: Option<&str>) -> String {
    let Some(login) = driver_login else {
        return body.to_string();
    };
    let credit = format!("Driven by [@{login}](https://github.com/{login})");
    if body.contains(&credit) {
        body.to_string()
    } else if body.trim().is_empty() {
        credit
    } else {
        format!("{}\n\n{credit}", body.trim_end())
    }
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

    fn annotation(path: &str, level: &str, message: &str) -> CheckAnnotation {
        CheckAnnotation {
            path: path.to_string(),
            start_line: Some(10),
            end_line: Some(10),
            annotation_level: Some(level.to_string()),
            message: Some(message.to_string()),
        }
    }

    #[test]
    fn check_run_failed_text_ends_on_a_newline_so_annotations_append_cleanly() {
        let text = check_run_failed_text(&CheckRunFailedText {
            name: "Rust architecture",
            conclusion: "failure",
            app_name: "GitHub Actions",
            head_sha: "abcdef0123456789",
            html_url: "https://github.com/o/r/runs/31",
            output_title: "",
            output_summary: "",
        });
        assert!(text.contains("## CI Check Failed: Rust architecture"));
        assert!(text.contains("**Commit:** `abcdef0`"));
        assert!(!text.contains("### Output"));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn check_run_failed_text_truncates_a_long_summary_on_a_char_boundary() {
        let summary = "é".repeat(CHECK_RUN_SUMMARY_MAX_LEN);
        let text = check_run_failed_text(&CheckRunFailedText {
            name: "CI",
            conclusion: "failure",
            app_name: "GitHub Actions",
            head_sha: "abc",
            html_url: "",
            output_title: "",
            output_summary: &summary,
        });
        assert!(text.contains("..."));
    }

    #[test]
    fn check_run_annotations_text_renders_failures_before_warnings() {
        let text = check_run_annotations_text(&[
            annotation("b.rs", "warning", "second"),
            annotation("a.rs", "failure", "first"),
        ]);
        let first = text.find("a.rs").expect("failure rendered");
        let second = text.find("b.rs").expect("warning rendered");
        assert!(first < second);
        assert!(text.starts_with("\n### Annotations\n"));
        assert!(text.contains("- `a.rs:10` — failure: first"));
    }

    #[test]
    fn check_run_annotations_text_caps_the_list_and_counts_the_rest() {
        let many: Vec<CheckAnnotation> = (0..MAX_RENDERED_ANNOTATIONS + 3)
            .map(|i| annotation(&format!("f{i}.rs"), "failure", "boom"))
            .collect();
        let text = check_run_annotations_text(&many);
        assert_eq!(text.matches("- `f").count(), MAX_RENDERED_ANNOTATIONS);
        assert!(text.contains("- _…and 3 more_"));
    }

    #[test]
    fn check_run_annotations_text_keeps_a_multi_line_message_in_one_bullet() {
        let text = check_run_annotations_text(&[annotation("a.rs", "failure", "one\ntwo")]);
        assert!(text.contains("- `a.rs:10` — failure: one\n  two"));
    }

    #[test]
    fn check_run_annotations_text_renders_a_line_range_and_a_pathless_finding() {
        let mut ranged = annotation("a.rs", "failure", "boom");
        ranged.end_line = Some(14);
        let text = check_run_annotations_text(&[ranged]);
        assert!(text.contains("- `a.rs:10-14` — failure: boom"));

        let bare = CheckAnnotation {
            path: String::new(),
            start_line: None,
            end_line: None,
            annotation_level: None,
            message: Some("step failed".to_string()),
        };
        let text = check_run_annotations_text(&[bare]);
        assert!(text.contains("- failure: step failed"));
    }

    #[test]
    fn check_run_annotations_text_truncates_a_long_message_on_a_char_boundary() {
        let text = check_run_annotations_text(&[annotation(
            "a.rs",
            "failure",
            &"é".repeat(MAX_ANNOTATION_MESSAGE_LEN),
        )]);
        assert!(text.contains('…'));
    }

    #[test]
    fn check_run_annotations_text_is_empty_when_there_are_none() {
        assert!(check_run_annotations_text(&[]).is_empty());
    }

    /// The rank and the rendered label must agree on a missing level, or the
    /// cap drops what the list would have called a failure.
    #[test]
    fn check_run_annotations_text_ranks_a_level_less_finding_as_a_failure() {
        let mut unknown = annotation("a.rs", "failure", "no level");
        unknown.annotation_level = None;
        let text = check_run_annotations_text(&[
            annotation("b.rs", "warning", "warn"),
            unknown,
            annotation("c.rs", "failure", "fail"),
        ]);
        let warning = text.find("b.rs").expect("warning rendered");
        assert!(text.find("a.rs").expect("unknown rendered") < warning);
        assert!(text.find("c.rs").expect("failure rendered") < warning);
        assert!(text.contains("- `a.rs:10` — failure: no level"));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate_on_char_boundary("hello", 10), "hello");
        assert_eq!(truncate_on_char_boundary("hello", 3), "hel");
        // 'é' is 2 bytes — truncating mid-char backs off to the boundary.
        assert_eq!(truncate_on_char_boundary("é", 1), "");
        assert_eq!(truncate_on_char_boundary("aé", 2), "a");
    }

    #[test]
    fn a_driver_is_credited_once_without_a_mention() {
        let credited = credit_driver("Fixes the bug.\n", Some("ada"));
        assert_eq!(
            credited,
            "Fixes the bug.\n\nDriven by [@ada](https://github.com/ada)"
        );
        assert_eq!(credit_driver(&credited, Some("ada")), credited);
        assert_eq!(
            credit_driver("", Some("ada")),
            "Driven by [@ada](https://github.com/ada)"
        );
        assert_eq!(credit_driver("As myself.", None), "As myself.");
    }
}
