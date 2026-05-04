//! Typed builders for `task_messages.channel_external_id`.
//!
//! Each provider+event-kind combination has its own string-prefixed namespace
//! so that distinct event types can never dedup against each other (e.g. an
//! issue-comment id `12345` would otherwise collide with a review-comment id
//! `12345` and silently suppress one).
//!
//! [`legacy_and_new`] returns the new prefixed form together with any legacy
//! bare form so the dedup query continues to recognize rows written before
//! the prefix scheme existed.

/// `github:ic:<id>` — issue comment (also used for PR conversation comments).
pub fn github_issue_comment(id: u64) -> String {
    format!("github:ic:{id}")
}

/// `github:rc:<id>` — pull request review comment (inline, threaded).
pub fn github_review_comment(id: u64) -> String {
    format!("github:rc:{id}")
}

/// `github:rv:<id>` — pull request review (top-level review submit).
pub fn github_review(id: u64) -> String {
    format!("github:rv:{id}")
}

/// `github:pr:<action>:<pr_number>[:<sha>]` — PR-level event.
///
/// `action` is the webhook action (`opened`, `reopened`, `synchronize`,
/// `closed`). `sha` disambiguates `synchronize` events for the same PR —
/// without it, successive pushes would dedup against each other.
pub fn github_pr_event(action: &str, pr_number: u64, sha: Option<&str>) -> String {
    match sha {
        Some(s) if !s.is_empty() => format!("github:pr:{action}:{pr_number}:{s}"),
        _ => format!("github:pr:{action}:{pr_number}"),
    }
}

/// `github:pr:opened:<pr_number>` — convenience for the `gh pr create` post-message
/// hook so the assistant tool_use row that created the PR is tagged with the
/// same id the inbound `pull_request` `opened` webhook would produce.
pub fn github_pr_opened(pr_number: u64) -> String {
    github_pr_event("opened", pr_number, None)
}

/// `github:cr:<id>` — `check_run` completion.
pub fn github_check_run(id: u64) -> String {
    format!("github:cr:{id}")
}

/// `github:cs:<id>` — `check_suite` completion.
pub fn github_check_suite(id: u64) -> String {
    format!("github:cs:{id}")
}

/// `slack:ts:<ts>` — Slack message ts.
pub fn slack_ts(ts: &str) -> String {
    format!("slack:ts:{ts}")
}

/// `tg:m:<id>` — Telegram message id.
pub fn telegram_msg(id: i64) -> String {
    format!("tg:m:{id}")
}

/// Expand an external id into the set of forms the dedup check should match.
///
/// For new prefixed ids that wrap a bare numeric tail (e.g. `github:ic:12345`),
/// returns `[new, legacy_bare_id]` so we keep matching old rows written before
/// the prefix scheme. For non-numeric tails or already-bare ids, returns just
/// the input.
pub fn legacy_and_new(ext_id: &str) -> Vec<String> {
    if !ext_id.contains(':') {
        return vec![ext_id.to_string()];
    }

    // Pull the bare numeric tail when one exists. Skip prefixes that don't have
    // a corresponding pre-prefix legacy form (`github:pr:*` is new in this
    // change set — there were no legacy rows for it).
    let tail = ext_id.rsplit(':').next().unwrap_or("");
    if tail.is_empty() || !tail.chars().all(|c| c.is_ascii_digit()) {
        return vec![ext_id.to_string()];
    }

    // Only the comment-style namespaces had legacy bare-int rows. The PR/check
    // event prefixes are introduced together with their tagging code, so there
    // are no pre-existing bare rows for them.
    if ext_id.starts_with("github:ic:")
        || ext_id.starts_with("github:rc:")
        || ext_id.starts_with("tg:m:")
    {
        return vec![ext_id.to_string(), tail.to_string()];
    }

    vec![ext_id.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_comment_format() {
        assert_eq!(github_issue_comment(12345), "github:ic:12345");
    }

    #[test]
    fn review_comment_format() {
        assert_eq!(github_review_comment(99), "github:rc:99");
    }

    #[test]
    fn review_format() {
        assert_eq!(github_review(7), "github:rv:7");
    }

    #[test]
    fn pr_event_with_sha() {
        assert_eq!(
            github_pr_event("synchronize", 42, Some("abcdef0")),
            "github:pr:synchronize:42:abcdef0"
        );
    }

    #[test]
    fn pr_event_without_sha() {
        assert_eq!(github_pr_event("opened", 42, None), "github:pr:opened:42");
        // Empty sha is treated as absent.
        assert_eq!(
            github_pr_event("opened", 42, Some("")),
            "github:pr:opened:42"
        );
    }

    #[test]
    fn pr_opened_format() {
        assert_eq!(github_pr_opened(42), "github:pr:opened:42");
    }

    #[test]
    fn check_run_check_suite_format() {
        assert_eq!(github_check_run(1), "github:cr:1");
        assert_eq!(github_check_suite(2), "github:cs:2");
    }

    #[test]
    fn slack_telegram_format() {
        assert_eq!(slack_ts("1700000000.000100"), "slack:ts:1700000000.000100");
        assert_eq!(telegram_msg(789), "tg:m:789");
    }

    #[test]
    fn legacy_and_new_for_issue_comment() {
        assert_eq!(
            legacy_and_new("github:ic:42"),
            vec!["github:ic:42".to_string(), "42".to_string()]
        );
    }

    #[test]
    fn legacy_and_new_for_review_comment() {
        assert_eq!(
            legacy_and_new("github:rc:99"),
            vec!["github:rc:99".to_string(), "99".to_string()]
        );
    }

    #[test]
    fn legacy_and_new_for_telegram() {
        assert_eq!(
            legacy_and_new("tg:m:789"),
            vec!["tg:m:789".to_string(), "789".to_string()]
        );
    }

    #[test]
    fn legacy_and_new_for_new_only_namespaces() {
        // PR events are new — no legacy bare form.
        assert_eq!(
            legacy_and_new("github:pr:opened:7"),
            vec!["github:pr:opened:7".to_string()]
        );
        assert_eq!(
            legacy_and_new("github:pr:synchronize:7:abc1234"),
            vec!["github:pr:synchronize:7:abc1234".to_string()]
        );
        assert_eq!(
            legacy_and_new("github:rv:5"),
            vec!["github:rv:5".to_string()]
        );
        assert_eq!(
            legacy_and_new("github:cr:8"),
            vec!["github:cr:8".to_string()]
        );
        assert_eq!(
            legacy_and_new("github:cs:9"),
            vec!["github:cs:9".to_string()]
        );
    }

    #[test]
    fn legacy_and_new_for_bare_input() {
        // Already-bare ids pass through unchanged.
        assert_eq!(legacy_and_new("12345"), vec!["12345".to_string()]);
    }

    #[test]
    fn legacy_and_new_for_non_numeric_tail() {
        // Slack ts has a dot, not a bare int — no legacy bare form to add.
        assert_eq!(
            legacy_and_new("slack:ts:1700000000.000100"),
            vec!["slack:ts:1700000000.000100".to_string()]
        );
    }
}
