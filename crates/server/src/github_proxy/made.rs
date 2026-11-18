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
pub(super) fn from_rest(
    method: &Method,
    path_and_query: &str,
    request: &[u8],
    response: &[u8],
) -> Vec<Made> {
    let Some(rest) = repo_rest(path_and_query) else {
        return Vec::new();
    };
    let segments: Vec<&str> = rest.split('/').collect();
    let at = match (method.as_str(), segments.as_slice()) {
        ("POST", ["git", "refs"]) | ("PATCH", ["git", "refs", ..]) => Shape::Push("/object/sha"),
        ("PUT" | "DELETE", ["contents", _, ..]) => Shape::Push("/commit/sha"),
        ("POST", ["merges"]) | ("PUT", ["pulls", _, "merge"]) => Shape::Push("/sha"),
        ("POST", ["issues", _, "comments"]) => Shape::IssueComment,
        ("PATCH", ["issues", "comments", _]) => Shape::IssueCommentEdit,
        ("POST", ["pulls", _, "comments"] | ["pulls", _, "comments", _, "replies"]) => {
            Shape::ReviewComment
        }
        ("POST", ["pulls", _, "reviews"]) => Shape::Review,
        ("PATCH", ["pulls", _]) => Shape::PullRequest,
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
        Shape::IssueCommentEdit => json
            .get("body")
            .and_then(Value::as_str)
            .map(Made::issue_comment_edit),
        Shape::ReviewComment => id().map(Made::ReviewComment),
        Shape::Review => id().map(Made::Review),
        Shape::PullRequest => return rest_pull_request_edits(request, &json),
    };
    made.into_iter().collect()
}

/// What a GraphQL mutation's answer names: a moved branch's commit, and a
/// conversation comment by its `#issuecomment-` anchor; only what the caller
/// selected comes back. A pull request edit is read off `request` instead:
/// `gh pr edit` selects only the PR's id.
pub(super) fn from_graphql(request: &[u8], response: &[u8]) -> Vec<Made> {
    let Ok(json) = serde_json::from_slice::<Value>(response) else {
        return Vec::new();
    };
    let Some(data) = json.get("data").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut made = Vec::new();
    if data.get("updatePullRequest").is_some_and(|r| !r.is_null()) {
        made.extend(graphql_pull_request_edits(request));
    }
    if data.get("updateIssueComment").is_some_and(|r| !r.is_null()) {
        made.extend(graphql_comment_edit(request));
    }
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

/// The `updatePullRequest` input among the request's variables: the one
/// naming a `pullRequestId`. An input written inline in the query is missed.
fn graphql_pull_request_edits(request: &[u8]) -> Vec<Made> {
    let Ok(json) = serde_json::from_slice::<Value>(request) else {
        return Vec::new();
    };
    let Some(input) = json
        .get("variables")
        .and_then(Value::as_object)
        .and_then(|vars| vars.values().find(|v| v.get("pullRequestId").is_some()))
    else {
        return Vec::new();
    };
    let field = |name: &str| input.get(name).and_then(Value::as_str);
    pull_request_edits(field("title"), field("body"), field("baseRefName"))
}

/// The `updateIssueComment` input among the request's variables: the one
/// naming both an `id` and a `body`. An input written inline is missed.
fn graphql_comment_edit(request: &[u8]) -> Option<Made> {
    let json = serde_json::from_slice::<Value>(request).ok()?;
    json.get("variables")?
        .as_object()?
        .values()
        .filter(|input| input.get("id").is_some())
        .find_map(|input| input.get("body").and_then(Value::as_str))
        .map(Made::issue_comment_edit)
}

/// The fields a REST `PATCH` set, valued as GitHub's answer holds them, so a
/// field the request left alone makes no claim a later edit could match.
fn rest_pull_request_edits(request: &[u8], answer: &Value) -> Vec<Made> {
    let Ok(set) = serde_json::from_slice::<Value>(request) else {
        return Vec::new();
    };
    let held = |field: &str, pointer: &str| {
        set.get(field)
            .and_then(|_| answer.pointer(pointer))
            .and_then(Value::as_str)
    };
    pull_request_edits(
        held("title", "/title"),
        held("body", "/body"),
        held("base", "/base/ref"),
    )
}

fn pull_request_edits(title: Option<&str>, body: Option<&str>, base: Option<&str>) -> Vec<Made> {
    [("title", title), ("body", body), ("base", base)]
        .into_iter()
        .filter_map(|(field, value)| value.map(|v| Made::pull_request_edit(field, v)))
        .collect()
}

enum Shape {
    /// The JSON pointer to the commit sha.
    Push(&'static str),
    IssueComment,
    IssueCommentEdit,
    ReviewComment,
    Review,
    PullRequest,
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
            b"{}",
            br#"{"ref":"refs/heads/fix","object":{"sha":"abc123","type":"commit"}}"#,
        );
        assert_eq!(made, vec![Made::Push("abc123".into())]);
    }

    #[test]
    fn a_contents_write_names_its_commit_rather_than_the_blob() {
        let made = from_rest(
            &Method::PUT,
            "/repos/acme/app/contents/src/lib.rs",
            b"{}",
            br#"{"content":{"sha":"blob"},"commit":{"sha":"c0ffee"}}"#,
        );
        assert_eq!(made, vec![Made::Push("c0ffee".into())]);
    }

    #[test]
    fn rest_comments_and_reviews_name_their_ids() {
        let body = br#"{"id":42}"#;
        assert_eq!(
            from_rest(
                &Method::POST,
                "/repos/acme/app/issues/7/comments",
                b"{}",
                body
            ),
            vec![Made::IssueComment(42)]
        );
        assert_eq!(
            from_rest(
                &Method::POST,
                "/repos/acme/app/pulls/7/comments/9/replies",
                b"{}",
                body
            ),
            vec![Made::ReviewComment(42)]
        );
        assert_eq!(
            from_rest(
                &Method::POST,
                "/repos/acme/app/pulls/7/reviews",
                b"{}",
                body
            ),
            vec![Made::Review(42)]
        );
    }

    #[test]
    fn a_read_or_an_unknown_write_makes_nothing() {
        let body = br#"{"id":42,"sha":"abc"}"#;
        assert!(
            from_rest(
                &Method::GET,
                "/repos/acme/app/issues/7/comments",
                b"{}",
                body
            )
            .is_empty()
        );
        assert!(from_rest(&Method::POST, "/repos/acme/app/labels", b"{}", body).is_empty());
        assert!(from_rest(&Method::POST, "/gists", b"{}", body).is_empty());
    }

    #[test]
    fn graphql_names_a_moved_branch_and_a_comment() {
        let made = from_graphql(
            b"{}",
            br#"{"data":{
                "createCommitOnBranch":{"commit":{"oid":"abc123"}},
                "addComment":{"commentEdge":{"node":{"url":"https://github.com/acme/app/pull/7#issuecomment-99"}}}
            }}"#,
        );
        assert!(made.contains(&Made::Push("abc123".into())));
        assert!(made.contains(&Made::IssueComment(99)));
    }

    #[test]
    fn a_rest_pull_request_edit_names_only_the_fields_it_set_as_github_holds_them() {
        let made = from_rest(
            &Method::PATCH,
            "/repos/acme/app/pulls/7",
            br#"{"title":"feat: y ","body":"Plan"}"#,
            br#"{"number":7,"title":"feat: y","body":"Plan","base":{"ref":"main"}}"#,
        );
        assert_eq!(
            made,
            vec![
                Made::pull_request_edit("title", "feat: y"),
                Made::pull_request_edit("body", "Plan"),
            ]
        );
    }

    #[test]
    fn a_graphql_pull_request_edit_names_the_fields_it_set() {
        let request = br#"{"query":"mutation PullRequestUpdate($input: UpdatePullRequestInput!) { updatePullRequest(input: $input) { pullRequest { id } } }",
            "variables":{"input":{"pullRequestId":"PR_1","body":"Plan"}}}"#;
        let made = from_graphql(
            request,
            br#"{"data":{"updatePullRequest":{"pullRequest":{"id":"PR_1"}}}}"#,
        );
        assert_eq!(made, vec![Made::pull_request_edit("body", "Plan")]);
        assert!(from_graphql(request, br#"{"data":{"updatePullRequest":null}}"#).is_empty());
    }

    #[test]
    fn a_comment_edit_names_the_body_github_holds() {
        let made = from_rest(
            &Method::PATCH,
            "/repos/acme/app/issues/comments/42",
            br#"{"body":"Plan: 1 to add\r\n"}"#,
            br#"{"id":42,"body":"Plan: 1 to add\n"}"#,
        );
        assert_eq!(made, vec![Made::issue_comment_edit("Plan: 1 to add\n")]);

        let request = br#"{"query":"mutation($input:UpdateIssueCommentInput!){updateIssueComment(input:$input){issueComment{id}}}","variables":{"input":{"id":"IC_kw","body":"Plan: done"}}}"#;
        assert_eq!(
            from_graphql(
                request,
                br#"{"data":{"updateIssueComment":{"issueComment":{"id":"IC_kw"}}}}"#
            ),
            vec![Made::issue_comment_edit("Plan: done")]
        );
    }
}
