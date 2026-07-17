/// `github:ic:<id>` - issue comment, also used for PR conversation comments.
pub fn issue_comment(id: u64) -> String {
    format!("github:ic:{id}")
}

/// `github:rc:<id>` - pull request review comment.
pub fn review_comment(id: u64) -> String {
    format!("github:rc:{id}")
}

/// `github:rv:<id>` - pull request review submission.
pub fn review(id: u64) -> String {
    format!("github:rv:{id}")
}

/// `github:pr:<action>:<pr_number>[:<sha>]` - PR-level event.
pub fn pr_event(action: &str, pr_number: u64, sha: Option<&str>) -> String {
    match sha {
        Some(s) if !s.is_empty() => format!("github:pr:{action}:{pr_number}:{s}"),
        _ => format!("github:pr:{action}:{pr_number}"),
    }
}

/// `github:pr:opened:<pr_number>`.
pub fn pr_opened(pr_number: u64) -> String {
    pr_event("opened", pr_number, None)
}

/// `github:pr:synchronize:<pr_number>:<sha>`.
pub fn pr_synchronize(pr_number: u64, sha: &str) -> String {
    pr_event("synchronize", pr_number, Some(sha))
}

/// `github:cr:<id>` - check_run completion.
pub fn check_run(id: u64) -> String {
    format!("github:cr:{id}")
}

/// `github:cs:<id>` - check_suite completion.
pub fn check_suite(id: u64) -> String {
    format!("github:cs:{id}")
}

/// `github:wfrun:<id>` - post-merge `workflow_run` completion on the base
/// branch. Keyed by the run id so each workflow run notifies at most once.
pub fn workflow_run(id: u64) -> String {
    format!("github:wfrun:{id}")
}

/// `github:conflict:<pr_number>:<sha>` - merge-conflict notification.
///
/// Keyed by the PR head sha so a persisting conflict is announced once per
/// pushed commit: re-checking the same head is deduped, but a new push (new
/// sha) that is still conflicted re-notifies.
pub fn merge_conflict(pr_number: u64, sha: &str) -> String {
    if sha.is_empty() {
        format!("github:conflict:{pr_number}")
    } else {
        format!("github:conflict:{pr_number}:{sha}")
    }
}

/// Kind + numeric id of a GitHub comment reaction target.
pub enum CommentTarget {
    Issue(u64),
    Review(u64),
}

/// Parse an external id into a comment kind + id usable by the reactions API.
/// Only `github:ic:<id>` and `github:rc:<id>` are valid reaction targets.
pub fn comment_target_from_id(external_id: &str) -> Option<CommentTarget> {
    if let Some(rest) = external_id.strip_prefix("github:ic:") {
        return rest.parse::<u64>().ok().map(CommentTarget::Issue);
    }
    if let Some(rest) = external_id.strip_prefix("github:rc:") {
        return rest.parse::<u64>().ok().map(CommentTarget::Review);
    }
    None
}

pub fn aliases(external_id: &str) -> Vec<String> {
    let tail = external_id.rsplit(':').next().unwrap_or("");
    if tail.is_empty() || !tail.chars().all(|c| c.is_ascii_digit()) {
        return vec![external_id.to_string()];
    }

    if external_id.starts_with("github:ic:") || external_id.starts_with("github:rc:") {
        return vec![external_id.to_string(), tail.to_string()];
    }

    vec![external_id.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_formats() {
        assert_eq!(issue_comment(12345), "github:ic:12345");
        assert_eq!(review_comment(99), "github:rc:99");
        assert_eq!(review(7), "github:rv:7");
    }

    #[test]
    fn pr_event_formats() {
        assert_eq!(pr_event("opened", 42, None), "github:pr:opened:42");
        assert_eq!(
            pr_event("synchronize", 42, Some("abcdef0")),
            "github:pr:synchronize:42:abcdef0"
        );
        assert_eq!(pr_opened(42), "github:pr:opened:42");
        assert_eq!(
            pr_synchronize(42, "7fc06122f3aa1b4ed6cdee04b40dabf01dadbeef"),
            "github:pr:synchronize:42:7fc06122f3aa1b4ed6cdee04b40dabf01dadbeef"
        );
    }

    #[test]
    fn check_formats() {
        assert_eq!(check_run(1), "github:cr:1");
        assert_eq!(check_suite(2), "github:cs:2");
    }

    #[test]
    fn merge_conflict_formats() {
        assert_eq!(merge_conflict(42, "abcdef0"), "github:conflict:42:abcdef0");
        assert_eq!(merge_conflict(42, ""), "github:conflict:42");
    }

    #[test]
    fn aliases_include_legacy_comment_ids() {
        assert_eq!(
            aliases("github:ic:42"),
            vec!["github:ic:42".to_string(), "42".to_string()]
        );
        assert_eq!(
            aliases("github:rc:99"),
            vec!["github:rc:99".to_string(), "99".to_string()]
        );
    }

    #[test]
    fn comment_target_parses_issue_and_review_ids() {
        assert!(matches!(
            comment_target_from_id("github:ic:42"),
            Some(CommentTarget::Issue(42))
        ));
        assert!(matches!(
            comment_target_from_id("github:rc:99"),
            Some(CommentTarget::Review(99))
        ));
        assert!(comment_target_from_id("github:pr:opened:7").is_none());
        assert!(comment_target_from_id("github:ic:abc").is_none());
        assert!(comment_target_from_id("nonsense").is_none());
    }

    #[test]
    fn aliases_skip_new_only_namespaces() {
        assert_eq!(
            aliases("github:pr:opened:7"),
            vec!["github:pr:opened:7".to_string()]
        );
        assert_eq!(aliases("github:rv:5"), vec!["github:rv:5".to_string()]);
        assert_eq!(aliases("github:cr:8"), vec!["github:cr:8".to_string()]);
        assert_eq!(aliases("github:cs:9"), vec!["github:cs:9".to_string()]);
    }
}
