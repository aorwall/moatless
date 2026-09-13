//! The external event ids GitHub events are recorded under.
//!
//! GitHub numbers comment/review/check/run ids globally, but pull requests only
//! per repository, so a PR-level id must carry the whole subject
//! (`owner/repo#number`, via [`GithubPrSubject::key`]) rather than the bare
//! number — an ambiguous id would collide across repositories in
//! `task_messages` and `task_outbound_events`.
//!
//! Rows written under the old `github:pr:<action>:<number>` shape were
//! rewritten in place by migration `0148`; the two shapes are never aliased
//! together, since that would hand the ambiguous string back to readers
//! matching on id alone.

use super::subject::GithubPrSubject;

/// `github:ic:<id>` - issue comment, also used for PR conversation comments.
pub(super) fn issue_comment(id: u64) -> String {
    format!("github:ic:{id}")
}

/// `github:rc:<id>` - pull request review comment.
pub(super) fn review_comment(id: u64) -> String {
    format!("github:rc:{id}")
}

/// `github:rv:<id>` - pull request review submission.
pub(super) fn review(id: u64) -> String {
    format!("github:rv:{id}")
}

/// `github:pr:<action>:<owner>/<repo>#<number>[:<sha>]` - PR-level event.
pub(super) fn pr_event(action: &str, subject: &GithubPrSubject, sha: Option<&str>) -> String {
    let key = subject.key();
    match sha {
        Some(s) if !s.is_empty() => format!("github:pr:{action}:{key}:{s}"),
        _ => format!("github:pr:{action}:{key}"),
    }
}

/// `github:pr:opened:<owner>/<repo>#<number>`.
pub(super) fn pr_opened(subject: &GithubPrSubject) -> String {
    pr_event("opened", subject, None)
}

/// `github:pr:synchronize:<owner>/<repo>#<number>:<sha>`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "stable GitHub PR external-id codec retained")
)]
pub(super) fn pr_synchronize(subject: &GithubPrSubject, sha: &str) -> String {
    pr_event("synchronize", subject, Some(sha))
}

/// The prefix [`aliases`] recognises a `synchronize` id by.
const SYNCHRONIZE_PREFIX: &str = "github:pr:synchronize:";

/// `github:push:<owner>/<repo>:<sha>` - a sha the Task pushed.
///
/// Carries no PR number, since the sandbox files the claim before the PR exists
/// on the first push; [`aliases`] reattaches it to the `synchronize` event
/// instead.
pub(super) fn push(owner: &str, repo: &str, sha: &str) -> String {
    push_on(&format!("{owner}/{repo}"), sha)
}

fn push_on(repo_full_name: &str, sha: &str) -> String {
    format!("github:push:{repo_full_name}:{sha}")
}

/// `github:pr:merged:<owner>/<repo>#<number>` - the merged-PR notification.
///
/// Distinct from `github:pr:closed:...` (the auto-close notice's namespace):
/// sharing an id would let whichever ran first suppress the other's receipt.
pub(super) fn pr_merged(subject: &GithubPrSubject) -> String {
    pr_event("merged", subject, None)
}

/// `github:cr:<id>` - `check_run` completion.
pub(super) fn check_run(id: u64) -> String {
    format!("github:cr:{id}")
}

/// `github:cs:<id>` - `check_suite` completion.
pub(super) fn check_suite(id: u64) -> String {
    format!("github:cs:{id}")
}

/// `github:wfrun:<id>` - post-merge `workflow_run` completion on the base
/// branch. Keyed by the run id so each workflow run notifies at most once.
pub(super) fn workflow_run(id: u64) -> String {
    format!("github:wfrun:{id}")
}

/// `github:conflict:<owner>/<repo>#<number>:<sha>` - merge-conflict
/// notification.
///
/// Keyed by the PR head sha: re-checking the same head is deduped, but a new
/// push that is still conflicted re-notifies.
pub(super) fn merge_conflict(subject: &GithubPrSubject, sha: &str) -> String {
    let key = subject.key();
    if sha.is_empty() {
        format!("github:conflict:{key}")
    } else {
        format!("github:conflict:{key}:{sha}")
    }
}

/// Kind + numeric id of a GitHub comment reaction target.
pub(super) enum CommentTarget {
    Issue(u64),
    Review(u64),
}

/// Parse an external id into a comment kind + id usable by the reactions API.
/// Only `github:ic:<id>` and `github:rc:<id>` are valid reaction targets.
pub(super) fn comment_target_from_id(external_id: &str) -> Option<CommentTarget> {
    if let Some(rest) = external_id.strip_prefix("github:ic:") {
        return rest.parse::<u64>().ok().map(CommentTarget::Issue);
    }
    if let Some(rest) = external_id.strip_prefix("github:rc:") {
        return rest.parse::<u64>().ok().map(CommentTarget::Review);
    }
    None
}

/// The ids a stored row may carry for the same event.
///
/// A `synchronize` id aliases onto the [`push`] claim for the same sha (naming
/// the Task that pushed); a PR-level id never aliases its old ambiguous
/// number-only shape — migration `0148` rewrote those rows instead.
pub(super) fn aliases(external_id: &str) -> Vec<String> {
    // Ahead of the digit test below: a sha is hex, so a `synchronize` id would
    // fall out of it with no alias.
    if let Some(rest) = external_id.strip_prefix(SYNCHRONIZE_PREFIX) {
        let mut out = vec![external_id.to_string()];
        if let Some((subject_key, sha)) = rest.rsplit_once(':') {
            if let Some((repo_full_name, _number)) = subject_key.rsplit_once('#') {
                if !sha.is_empty() && !repo_full_name.is_empty() {
                    out.push(push_on(repo_full_name, sha));
                }
            }
        }
        return out;
    }

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

    fn subject(repo: &str, number: u64) -> GithubPrSubject {
        GithubPrSubject::new("acme", repo, number)
    }

    #[test]
    fn pr_event_formats() {
        let pr = subject("widgets", 42);
        assert_eq!(
            pr_event("opened", &pr, None),
            "github:pr:opened:acme/widgets#42"
        );
        assert_eq!(
            pr_event("synchronize", &pr, Some("abcdef0")),
            "github:pr:synchronize:acme/widgets#42:abcdef0"
        );
        assert_eq!(pr_opened(&pr), "github:pr:opened:acme/widgets#42");
        assert_eq!(pr_merged(&pr), "github:pr:merged:acme/widgets#42");
        assert_ne!(pr_merged(&pr), pr_event("closed", &pr, None));
        assert_eq!(
            pr_synchronize(&pr, "7fc06122f3aa1b4ed6cdee04b40dabf01dadbeef"),
            "github:pr:synchronize:acme/widgets#42:7fc06122f3aa1b4ed6cdee04b40dabf01dadbeef"
        );
    }

    /// The collision the format exists to remove: one number, two repositories,
    /// and the ids must not be equal.
    #[test]
    fn the_same_number_in_two_repositories_gives_two_ids() {
        for id in [
            pr_opened(&subject("widgets", 607)),
            pr_merged(&subject("widgets", 607)),
            pr_event("closed", &subject("widgets", 607), None),
            merge_conflict(&subject("widgets", 607), "abcdef0"),
        ] {
            let other = id.replace("acme/widgets", "acme/gadgets");
            assert_ne!(id, other);
        }

        assert_ne!(
            pr_opened(&subject("widgets", 607)),
            pr_opened(&subject("gadgets", 607)),
        );
    }

    #[test]
    fn check_formats() {
        assert_eq!(check_run(1), "github:cr:1");
        assert_eq!(check_suite(2), "github:cs:2");
    }

    #[test]
    fn merge_conflict_formats() {
        let pr = subject("widgets", 42);
        assert_eq!(
            merge_conflict(&pr, "abcdef0"),
            "github:conflict:acme/widgets#42:abcdef0"
        );
        assert_eq!(merge_conflict(&pr, ""), "github:conflict:acme/widgets#42");
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

    /// A PR-level id is never aliased onto the number-only string, in either
    /// shape — the current one because it has no number-only tail, the legacy
    /// one because handing that string back is the collision itself.
    #[test]
    fn aliases_skip_new_only_namespaces() {
        assert_eq!(
            aliases(&pr_opened(&subject("widgets", 7))),
            vec!["github:pr:opened:acme/widgets#7".to_string()]
        );
        assert_eq!(
            aliases("github:pr:opened:7"),
            vec!["github:pr:opened:7".to_string()]
        );
        assert_eq!(aliases("github:rv:5"), vec!["github:rv:5".to_string()]);
        assert_eq!(aliases("github:cr:8"), vec!["github:cr:8".to_string()]);
        assert_eq!(aliases("github:cs:9"), vec!["github:cs:9".to_string()]);
    }

    #[test]
    fn push_format() {
        assert_eq!(
            push("acme", "widgets", "be1d882"),
            "github:push:acme/widgets:be1d882"
        );
    }

    /// The claim the sandbox files carries no PR number, so the `synchronize`
    /// id has to drop its own to reach the claim.
    #[test]
    fn a_synchronize_id_aliases_onto_the_push_claim_for_its_sha() {
        assert_eq!(
            aliases(&pr_synchronize(&subject("widgets", 42), "be1d882")),
            vec![
                "github:pr:synchronize:acme/widgets#42:be1d882".to_string(),
                "github:push:acme/widgets:be1d882".to_string(),
            ]
        );
    }

    /// The alias names the repository the event happened in, so a second
    /// repository's push of the same sha is a different claim.
    #[test]
    fn the_push_alias_keeps_the_repository() {
        let acme = aliases(&pr_synchronize(&subject("widgets", 42), "deadbee"));
        let other = aliases("github:pr:synchronize:other/widgets#42:deadbee");
        assert_ne!(acme[1], other[1]);
    }

    /// GitHub sends `before`/`after` empty on a force-push to an unknown state;
    /// `pr_event` then writes no sha, and there is no claim to reach.
    #[test]
    fn a_synchronize_id_without_a_sha_has_no_push_alias() {
        assert_eq!(
            aliases("github:pr:synchronize:acme/widgets#42"),
            vec!["github:pr:synchronize:acme/widgets#42".to_string()]
        );
    }
}
