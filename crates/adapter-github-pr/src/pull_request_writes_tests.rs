use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{Method, StatusCode};

use super::*;
use crate::api::http_client;
use crate::test_support::{StubResponse, start_github_api_stub};

fn position(line: u64, side: DiffSide) -> ReviewPosition {
    ReviewPosition { line, side }
}

#[test]
fn a_review_sends_a_range_only_when_it_spans_lines() {
    let comments = [
        ReviewCommentDraft {
            path: "src/a.rs".into(),
            position: position(9, DiffSide::Right),
            start: Some(position(7, DiffSide::Right)),
            body: "range".into(),
        },
        ReviewCommentDraft {
            path: "src/b.rs".into(),
            position: position(3, DiffSide::Left),
            start: Some(position(3, DiffSide::Left)),
            body: "one line".into(),
        },
    ];
    let body = review_submission(ReviewVerdict::RequestChanges, "fix", &comments);
    assert_eq!(
        body,
        serde_json::json!({
            "event": "REQUEST_CHANGES",
            "body": "fix",
            "comments": [
                {"path": "src/a.rs", "line": 9, "side": "RIGHT", "body": "range",
                 "start_line": 7, "start_side": "RIGHT"},
                {"path": "src/b.rs", "line": 3, "side": "LEFT", "body": "one line"},
            ],
        })
    );
}

#[test]
fn reactions_are_named_as_github_spells_them() {
    assert_eq!(reaction_content("thumbs-up").unwrap(), "THUMBS_UP");
    assert_eq!(reaction_content("eyes").unwrap(), "EYES");
    assert!(reaction_content("party").is_err());
}

#[test]
fn a_label_is_one_path_segment() {
    assert_eq!(path_segment("good first/issue"), "good%20first%2Fissue");
}

/// A GraphQL stub answering each request with the next of `answers`.
async fn graphql_stub(answers: Vec<serde_json::Value>) -> crate::test_support::GithubApiStub {
    let next = Arc::new(AtomicUsize::new(0));
    let answers = Arc::new(answers);
    start_github_api_stub(move |method, path, uri| {
        if method != Method::POST || path != "/graphql" {
            return StubResponse::not_found(uri);
        }
        let index = next.fetch_add(1, Ordering::SeqCst);
        match answers.get(index) {
            Some(answer) => StubResponse::json(StatusCode::OK, answer.clone()),
            None => StubResponse::not_found(uri),
        }
    })
    .await
}

fn api(stub: &crate::test_support::GithubApiStub) -> GithubPrApi {
    GithubPrApi::new(
        http_client(moatless_core::config::HttpConfig::default().github),
        "ghp_viewer",
        stub.base_url().to_string(),
    )
}

#[tokio::test]
async fn a_merge_names_the_pull_request_by_its_node_id() {
    let stub = graphql_stub(vec![
        serde_json::json!({"data": {"repository": {"pullRequest": {"id": "PR_1", "headRef": {"id": "REF_1"}}}}}),
        serde_json::json!({"data": {"mergePullRequest": {"pullRequest": {"id": "PR_1"}}}}),
    ])
    .await;
    run_action(
        &api(&stub),
        "acme",
        "widgets",
        7,
        &PullRequestAction::Merge(MergeMethod::Squash),
    )
    .await
    .expect("the merge succeeds");
    let requests = stub.recorded_requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer ghp_viewer")
    );
    let sent: serde_json::Value = serde_json::from_str(&requests[1].body).unwrap();
    assert_eq!(
        sent["variables"],
        serde_json::json!({"id": "PR_1", "method": "SQUASH"})
    );
}

#[tokio::test]
async fn a_refused_mutation_is_an_error() {
    let stub = graphql_stub(vec![
        serde_json::json!({"data": {"repository": {"pullRequest": {"id": "PR_1", "headRef": null}}}}),
        serde_json::json!({
            "data": {"closePullRequest": null},
            "errors": [{"type": "FORBIDDEN", "message": "no"}],
        }),
    ])
    .await;
    let result = run_action(&api(&stub), "acme", "widgets", 7, &PullRequestAction::Close).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_deleted_branch_is_not_deleted_again() {
    let stub = graphql_stub(vec![serde_json::json!(
        {"data": {"repository": {"pullRequest": {"id": "PR_1", "headRef": null}}}}
    )])
    .await;
    let result = run_action(
        &api(&stub),
        "acme",
        "widgets",
        7,
        &PullRequestAction::DeleteBranch,
    )
    .await;
    assert!(result.is_err());
    assert_eq!(stub.recorded_requests().await.len(), 1);
}

#[tokio::test]
async fn removing_a_label_already_gone_succeeds() {
    let stub = start_github_api_stub(|method, _path, uri| {
        if method == Method::DELETE {
            StubResponse::not_found(uri)
        } else {
            StubResponse::empty(StatusCode::FORBIDDEN)
        }
    })
    .await;
    let api = api(&stub);
    api.rest_remove(
        "GitHub remove_label",
        "repos/acme/widgets/issues/7/labels/bug",
    )
    .await
    .expect("an absent label is already removed");
    let refused = api
        .rest_write(
            "GitHub add_labels",
            Method::POST,
            "repos/acme/widgets/issues/7/labels",
            None,
        )
        .await;
    assert!(refused.is_err());
}
