//! What a proxied GitHub write made, read from GitHub's answer to it, so
//! [`OwnEvents`] names the Task when the webhook for it arrives.
//!
//! [`OwnEvents`]: moatless_loops::bindings::own_events::OwnEvents

use axum::http::Method;
use moatless_adapter_github_pr::own_event::Made;
use serde_json::Value;

/// What the write at `method path_and_query` made, by GitHub's `response`.
///
/// A branch move whose answer names no commit, such as `update-branch`, makes
/// nothing this can name.
pub(super) fn from_rest(method: &Method, path_and_query: &str, response: &[u8]) -> Vec<Made> {
    let Some(rest) = repo_rest(path_and_query) else {
        return Vec::new();
    };
    let segments: Vec<&str> = rest.split('/').collect();
    let at = match (method.as_str(), segments.as_slice()) {
        ("POST", ["git", "refs"]) | ("PATCH", ["git", "refs", ..]) => Shape::Push("/object/sha"),
        ("PUT" | "DELETE", ["contents", _, ..]) => Shape::Push("/commit/sha"),
        ("POST", ["merges"]) | ("PUT", ["pulls", _, "merge"]) => Shape::Push("/sha"),
        ("POST", ["issues", _, "comments"]) => Shape::IssueComment,
        ("POST", ["pulls", _, "comments"] | ["pulls", _, "comments", _, "replies"]) => {
            Shape::ReviewComment
        }
        ("POST", ["pulls", _, "reviews"]) => Shape::Review,
        _ => return Vec::new(),
    };
    let Ok(json) = serde_json::from_slice::<Value>(response) else {
        return Vec::new();
    };
    let id = || json.get("id").and_then(Value::as_u64);
    let made = match at {
        Shape::Push(pointer) => json
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(|sha| Made::Push(sha.to_string())),
        Shape::IssueComment => id().map(Made::IssueComment),
        Shape::ReviewComment => id().map(Made::ReviewComment),
        Shape::Review => id().map(Made::Review),
    };
    made.into_iter().collect()
}

/// What a GraphQL mutation's answer names: the commit a branch moved to, and
/// a conversation comment by the `#issuecomment-` anchor of its `url`.
///
/// Only what the caller selected comes back, so a mutation that asked for
/// neither makes nothing this can name.
pub(super) fn from_graphql(response: &[u8]) -> Vec<Made> {
    let Ok(json) = serde_json::from_slice::<Value>(response) else {
        return Vec::new();
    };
    let Some(data) = json.get("data").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut made = Vec::new();
    for result in data.values() {
        for pointer in ["/commit/oid", "/mergeCommit/oid", "/ref/target/oid"] {
            if let Some(sha) = result.pointer(pointer).and_then(Value::as_str) {
                made.push(Made::Push(sha.to_string()));
            }
        }
        if let Some(id) = result
            .pointer("/commentEdge/node/url")
            .and_then(Value::as_str)
            .and_then(|url| url.rsplit_once("#issuecomment-"))
            .and_then(|(_, id)| id.parse().ok())
        {
            made.push(Made::IssueComment(id));
        }
    }
    made
}

enum Shape {
    /// The JSON pointer to the commit sha.
    Push(&'static str),
    IssueComment,
    ReviewComment,
    Review,
}

/// `path_and_query` past `/repos/{owner}/{name}/`, without its query.
fn repo_rest(path_and_query: &str) -> Option<&str> {
    let path = path_and_query.split('?').next()?.strip_prefix("/repos/")?;
    let mut parts = path.trim_end_matches('/').splitn(3, '/');
    parts.next()?;
    parts.next()?;
    parts.next().filter(|rest| !rest.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ref_moved_through_rest_names_the_commit() {
        let made = from_rest(
            &Method::PATCH,
            "/repos/acme/app/git/refs/heads/fix",
            br#"{"ref":"refs/heads/fix","object":{"sha":"abc123","type":"commit"}}"#,
        );
        assert_eq!(made, vec![Made::Push("abc123".into())]);
    }

    #[test]
    fn a_contents_write_names_its_commit_rather_than_the_blob() {
        let made = from_rest(
            &Method::PUT,
            "/repos/acme/app/contents/src/lib.rs",
            br#"{"content":{"sha":"blob"},"commit":{"sha":"c0ffee"}}"#,
        );
        assert_eq!(made, vec![Made::Push("c0ffee".into())]);
    }

    #[test]
    fn rest_comments_and_reviews_name_their_ids() {
        let body = br#"{"id":42}"#;
        assert_eq!(
            from_rest(&Method::POST, "/repos/acme/app/issues/7/comments", body),
            vec![Made::IssueComment(42)]
        );
        assert_eq!(
            from_rest(
                &Method::POST,
                "/repos/acme/app/pulls/7/comments/9/replies",
                body
            ),
            vec![Made::ReviewComment(42)]
        );
        assert_eq!(
            from_rest(&Method::POST, "/repos/acme/app/pulls/7/reviews", body),
            vec![Made::Review(42)]
        );
    }

    #[test]
    fn a_read_or_an_unknown_write_makes_nothing() {
        let body = br#"{"id":42,"sha":"abc"}"#;
        assert!(from_rest(&Method::GET, "/repos/acme/app/issues/7/comments", body).is_empty());
        assert!(from_rest(&Method::POST, "/repos/acme/app/labels", body).is_empty());
        assert!(from_rest(&Method::POST, "/gists", body).is_empty());
    }

    #[test]
    fn graphql_names_a_moved_branch_and_a_comment() {
        let made = from_graphql(
            br#"{"data":{
                "createCommitOnBranch":{"commit":{"oid":"abc123"}},
                "addComment":{"commentEdge":{"node":{"url":"https://github.com/acme/app/pull/7#issuecomment-99"}}}
            }}"#,
        );
        assert!(made.contains(&Made::Push("abc123".into())));
        assert!(made.contains(&Made::IssueComment(99)));
    }
}
