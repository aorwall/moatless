//! What the App sends a proxied write as: which writes post text that credits
//! the driver, the repository a GraphQL write acts on, and whether GitHub
//! refused the App.

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

/// The GraphQL mutations that post text that credits the driver.
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
    "issueId",
    "labelableId",
    "assignableId",
    "repositoryId",
];

/// Whether a REST request posts text that credits the driver.
pub(super) fn rest_authors(method: &Method, path_and_query: &str) -> bool {
    method == Method::POST
        && REST
            .iter()
            .any(|pattern| repo_path_matches(path_and_query, pattern))
}

/// Whether `path_and_query` is `/repos/{owner}/{name}/` then `pattern`, where
/// `*` is one segment.
fn repo_path_matches(path_and_query: &str, pattern: &str) -> bool {
    let path = path_and_query.split('?').next().unwrap_or_default();
    let Some(rest) = path.strip_prefix("/repos/") else {
        return false;
    };
    let segments: Vec<&str> = rest.trim_end_matches('/').split('/').skip(2).collect();
    let pattern: Vec<&str> = pattern.split('/').collect();
    pattern.len() == segments.len()
        && pattern
            .iter()
            .zip(&segments)
            .all(|(p, s)| *p == "*" || p == s)
}

/// The node id a GraphQL mutation's `input` names.
pub(super) fn graphql_subject(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let input = json.pointer("/variables/input")?;
    SUBJECT_FIELDS
        .iter()
        .find_map(|field| input.get(field)?.as_str().map(str::to_string))
}

/// Whether a GraphQL answer is GitHub refusing the token and writing nothing:
/// a `FORBIDDEN` error and no mutation's result.
pub(super) fn graphql_forbidden(answer: &[u8]) -> bool {
    let Ok(json) = serde_json::from_slice::<Value>(answer) else {
        return false;
    };
    let forbidden = json["errors"]
        .as_array()
        .is_some_and(|errors| errors.iter().any(|e| e["type"] == "FORBIDDEN"));
    let wrote = json["data"]
        .as_object()
        .is_some_and(|data| data.values().any(|result| !result.is_null()));
    forbidden && !wrote
}

/// GitHub's reason in a REST or GraphQL refusal, such as "Resource not
/// accessible by integration"; empty when it gives none.
pub(super) fn refusal_message(answer: &[u8]) -> String {
    let json: Value = serde_json::from_slice(answer).unwrap_or_default();
    json["message"]
        .as_str()
        .or_else(|| json.pointer("/errors/0/message").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
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
    fn only_a_forbidden_answer_that_wrote_nothing_is_a_refusal() {
        let forbidden = r#"{"data":{"addLabelsToLabelable":null},"errors":[{"type":"FORBIDDEN","message":"Resource not accessible by integration"}]}"#;
        assert!(graphql_forbidden(forbidden.as_bytes()));
        assert!(graphql_forbidden(
            br#"{"data":null,"errors":[{"type":"FORBIDDEN"}]}"#
        ));
        assert!(!graphql_forbidden(
            br#"{"data":{"a":{"id":1},"b":null},"errors":[{"type":"FORBIDDEN"}]}"#
        ));
        assert!(!graphql_forbidden(
            br#"{"data":null,"errors":[{"type":"NOT_FOUND"}]}"#
        ));
        assert!(!graphql_forbidden(b"not json"));
        assert_eq!(
            refusal_message(forbidden.as_bytes()),
            "Resource not accessible by integration"
        );
        assert_eq!(
            refusal_message(br#"{"message":"Must have admin rights"}"#),
            "Must have admin rights"
        );
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
