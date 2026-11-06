//! The proxied writes GitHub shows an author for. A driver signed in through
//! the GitHub App sends them as the App, credited in the text, so `gh` in a
//! sandbox posts the way the adapter's own replies do.

use axum::body::Bytes;
use axum::http::Method;
use moatless_adapter_github_pr::credit_driver;
use serde_json::Value;

/// The REST `POST`s that author, as the segments after
/// `/repos/{owner}/{name}/`, where `*` is one segment.
const REST: &[&str] = &[
    "pulls",
    "issues",
    "pulls/*/comments",
    "pulls/*/comments/*/replies",
    "pulls/*/reviews",
    "pulls/*/reviews/*/events",
    "issues/*/comments",
];

/// The GraphQL mutations that author.
pub(super) const MUTATIONS: &[&str] = &[
    "addComment",
    "addPullRequestReview",
    "addPullRequestReviewComment",
    "addPullRequestReviewThread",
    "addPullRequestReviewThreadReply",
    "createIssue",
    "createPullRequest",
    "submitPullRequestReview",
];

/// The `input` fields naming the node a GraphQL mutation acts on, in the
/// order they are tried.
const SUBJECT_FIELDS: &[&str] = &[
    "subjectId",
    "pullRequestId",
    "pullRequestReviewId",
    "pullRequestReviewThreadId",
    "repositoryId",
];

/// Whether a REST request authors.
pub(super) fn rest_authors(method: &Method, path_and_query: &str) -> bool {
    if method != Method::POST {
        return false;
    }
    let path = path_and_query.split('?').next().unwrap_or_default();
    let Some(rest) = path.strip_prefix("/repos/") else {
        return false;
    };
    let segments: Vec<&str> = rest.trim_end_matches('/').split('/').skip(2).collect();
    REST.iter().any(|pattern| {
        let pattern: Vec<&str> = pattern.split('/').collect();
        pattern.len() == segments.len()
            && pattern
                .iter()
                .zip(&segments)
                .all(|(p, s)| *p == "*" || p == s)
    })
}

/// The node id a GraphQL mutation's `input` names.
pub(super) fn graphql_subject(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let input = json.pointer("/variables/input")?;
    SUBJECT_FIELDS
        .iter()
        .find_map(|field| input.get(field)?.as_str().map(str::to_string))
}

/// `body` with `driver` credited in the text it posts: a REST body's `body`,
/// or a GraphQL mutation's `input.body`, added when absent. Every write
/// [`rest_authors`] and [`MUTATIONS`] name takes one.
pub(super) fn credit(body: &Bytes, graphql: bool, driver: &str) -> Bytes {
    let Ok(mut json) = serde_json::from_slice::<Value>(body) else {
        return body.clone();
    };
    let holder = if graphql {
        json.pointer_mut("/variables/input")
    } else {
        Some(&mut json)
    };
    let Some(Value::Object(holder)) = holder else {
        return body.clone();
    };
    let text = holder
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let credited = credit_driver(text, Some(driver));
    holder.insert("body".to_string(), Value::String(credited));
    serde_json::to_vec(&json).map_or_else(|_| body.clone(), Bytes::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_post_creating_a_pull_request_issue_comment_or_review_authors() {
        let post = |path| rest_authors(&Method::POST, path);
        assert!(post("/repos/o/r/pulls"));
        assert!(post("/repos/o/r/issues?x=1"));
        assert!(post("/repos/o/r/pulls/1/comments"));
        assert!(post("/repos/o/r/pulls/1/comments/2/replies"));
        assert!(post("/repos/o/r/pulls/1/reviews"));
        assert!(post("/repos/o/r/issues/1/comments"));
        assert!(!post("/repos/o/r/issues/1/labels"));
        assert!(!post("/repos/o/r/pulls/comments/2/reactions"));
        assert!(!post("/user/repos"));
        assert!(!rest_authors(&Method::PATCH, "/repos/o/r/pulls/1"));
    }

    #[test]
    fn the_driver_is_credited_in_the_text_each_kind_of_request_posts() {
        let rest = credit(
            &Bytes::from(r#"{"body":"hi","event":"COMMENT"}"#),
            false,
            "ada",
        );
        let rest: Value = serde_json::from_slice(&rest).unwrap();
        assert_eq!(
            rest["body"],
            "hi\n\nDriven by [@ada](https://github.com/ada)"
        );
        assert_eq!(rest["event"], "COMMENT");

        let graphql =
            Bytes::from(r#"{"query":"m","variables":{"input":{"subjectId":"PR_1","body":""}}}"#);
        let graphql: Value = serde_json::from_slice(&credit(&graphql, true, "ada")).unwrap();
        assert_eq!(
            graphql["variables"]["input"]["body"],
            "Driven by [@ada](https://github.com/ada)"
        );

        let approve = credit(&Bytes::from(r#"{"event":"APPROVE"}"#), false, "ada");
        let approve: Value = serde_json::from_slice(&approve).unwrap();
        assert_eq!(approve["body"], "Driven by [@ada](https://github.com/ada)");

        let unreadable = Bytes::from("not json");
        assert_eq!(credit(&unreadable, false, "ada"), unreadable);
    }

    #[test]
    fn a_mutation_names_its_subject_in_input() {
        let body = |input: Value| {
            serde_json::to_vec(
                &serde_json::json!({ "query": "m", "variables": { "input": input } }),
            )
            .unwrap()
        };
        assert_eq!(
            graphql_subject(&body(
                serde_json::json!({ "subjectId": "PR_1", "body": "x" })
            )),
            Some("PR_1".to_string())
        );
        assert_eq!(
            graphql_subject(&body(serde_json::json!({ "repositoryId": "R_1" }))),
            Some("R_1".to_string())
        );
        assert_eq!(
            graphql_subject(&body(serde_json::json!({ "body": "x" }))),
            None
        );
    }
}
