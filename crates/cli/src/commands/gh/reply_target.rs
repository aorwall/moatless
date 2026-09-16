//! What a `moat gh pr reply` comment id actually names.
//!
//! GitHub keeps review comments and conversation comments in two id spaces,
//! and a delivered PR message labels both `comment id: <n>`. Only the first can
//! be replied to in a thread. Both spaces are read before the call is routed:
//! they number independently, so one integer can name a comment in each, and
//! the one on the pull request the caller named is the one they meant.

use serde::Deserialize;

use super::process::gh_command_output;

/// What the id names, relative to the pull request the caller passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplyTarget {
    /// An inline review comment on this pull request: it has a thread.
    ReviewComment,
    /// A conversation comment on this pull request: no thread to reply in, so
    /// the reply becomes a top-level comment.
    ConversationComment,
    /// A comment of either kind on another pull request or issue in the same
    /// repository.
    Elsewhere(u64),
    /// A comment of each kind on this pull request, both carrying this id.
    /// Nothing distinguishes them, so the caller picks.
    Ambiguous,
    /// Both spaces answered, and neither holds the id — a deleted comment. The
    /// reply is attempted anyway and fails on its own terms.
    Unknown,
    /// A space could not be read, so what the id names is undecided.
    Unreadable,
}

/// What one id space said about the comment id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    /// The comment is there, on this pull request or issue.
    On(u64),
    /// A 404: this space holds no comment with that id.
    Missing,
    /// The lookup never answered, so this space said nothing either way.
    Failed,
}

#[derive(Deserialize)]
struct ReviewCommentRef {
    /// `…/repos/{owner}/{repo}/pulls/{number}`.
    #[serde(default)]
    pull_request_url: String,
}

#[derive(Deserialize)]
struct IssueCommentRef {
    /// `…/repos/{owner}/{repo}/issues/{number}`.
    #[serde(default)]
    issue_url: String,
}

pub(super) fn classify(
    owner: &str,
    repo: &str,
    pr_number: u64,
    comment_id: u64,
    token: Option<&str>,
) -> ReplyTarget {
    let base = format!("repos/{owner}/{repo}");
    let review = lookup(
        read::<ReviewCommentRef>(&format!("{base}/pulls/comments/{comment_id}"), token)
            .map(|c| c.map(|c| c.pull_request_url)),
    );
    let issue = lookup(
        read::<IssueCommentRef>(&format!("{base}/issues/comments/{comment_id}"), token)
            .map(|c| c.map(|c| c.issue_url)),
    );
    resolve(review, issue, pr_number)
}

/// Which comment the caller meant, given what each id space said.
///
/// A comment on the pull request they named wins over one elsewhere, whichever
/// space it came from: an id that reaches this code was read off a message
/// about that pull request. Only a space that answered decides anything. That a
/// review comment carries the id proves one exists, not that the delivered id
/// came from that space, so a lookup that failed leaves the reply unroutable
/// rather than aiming it at the thread of a comment that merely collides.
fn resolve(review: Lookup, issue: Lookup, pr_number: u64) -> ReplyTarget {
    use Lookup::{Failed, Missing, On};
    match (review, issue) {
        (On(a), On(b)) if a == pr_number && b == pr_number => ReplyTarget::Ambiguous,
        (Failed, _) | (_, Failed) => ReplyTarget::Unreadable,
        (On(a), _) if a == pr_number => ReplyTarget::ReviewComment,
        (_, On(b)) if b == pr_number => ReplyTarget::ConversationComment,
        (On(number), _) | (_, On(number)) => ReplyTarget::Elsewhere(number),
        (Missing, Missing) => ReplyTarget::Unknown,
    }
}

/// What one read says about the space it addressed, announcing a failure rather
/// than passing it off as an absent comment.
fn lookup(read: Result<Option<String>, String>) -> Lookup {
    let parent = read.and_then(|url| match url {
        Some(url) => trailing_number(&url).map(Some).ok_or(format!(
            "a comment lookup answered with parent `{url}`, which names no pull request"
        )),
        None => Ok(None),
    });
    match parent {
        Ok(Some(number)) => Lookup::On(number),
        Ok(None) => Lookup::Missing,
        Err(e) => {
            eprintln!("moat gh: {e} — replying without classifying the comment id");
            Lookup::Failed
        }
    }
}

/// One GET for a comment's parent URL: `Ok(None)` is a 404, and an `Err` is a
/// lookup that never answered.
fn read<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    token: Option<&str>,
) -> Result<Option<T>, String> {
    let argv = vec!["api".to_string(), endpoint.to_string()];
    match gh_command_output(&argv, token, None) {
        Ok(body) => serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| format!("`gh api {endpoint}` returned no readable comment: {e}")),
        Err(e) if is_not_found(&e.to_string()) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Whether a failed `gh api` call is GitHub answering 404.
///
/// `gh` exits 1 for every HTTP error alike and prints the status into its own
/// message, so the status text is what separates an absent comment from an
/// expired token, a rate limit or an outage.
fn is_not_found(message: &str) -> bool {
    message.contains("(HTTP 404)")
}

/// The number ending a GitHub API URL, such as the `42` in `…/pulls/42`.
fn trailing_number(url: &str) -> Option<u64> {
    url.rsplit('/').next()?.parse().ok()
}

#[cfg(test)]
mod tests;
