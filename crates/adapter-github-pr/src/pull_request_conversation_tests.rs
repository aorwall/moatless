use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

use super::*;

type Responder = fn(&Value) -> Value;

/// A GraphQL endpoint answering each request's variables with `respond`, and
/// the variables every request carried.
async fn graphql_stub(respond: Responder) -> (GithubPrApi, Arc<Mutex<Vec<Value>>>) {
    async fn handle(
        State((respond, seen)): State<(Responder, Arc<Mutex<Vec<Value>>>)>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let variables = body["variables"].clone();
        let data = respond(&variables);
        seen.lock().unwrap().push(variables);
        Json(json!({ "data": data }))
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/graphql", post(handle))
        .with_state((respond, seen.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client =
        super::super::api::http_client(moatless_core::config::HttpConfig::default().github);
    (
        GithubPrApi::new(client, "ghp_test", format!("http://{address}")),
        seen,
    )
}

fn actor(login: &str) -> Value {
    json!({ "__typename": "User", "login": login, "avatarUrl": null, "name": null })
}

fn comment(id: &str, at: &str) -> Value {
    json!({ "id": id, "body": id, "createdAt": at, "lastEditedAt": null, "url": null, "author": actor("alice") })
}

fn page(has_next: bool, cursor: &str) -> Value {
    json!({ "hasNextPage": has_next, "endCursor": cursor })
}

fn two_page_activity(variables: &Value) -> Value {
    let pr = if variables["head"] == json!(true) {
        json!({
            "author": actor("alice"),
            "reviewRequests": { "nodes": [
                { "requestedReviewer": { "__typename": "Team", "combinedSlug": "acme/core", "name": "Core", "avatarUrl": null } },
                { "requestedReviewer": { "__typename": "User", "login": "bob", "name": "Bob", "avatarUrl": null } },
            ] },
            "latestReviews": { "nodes": [{ "author": actor("bob") }, { "author": actor("carol") }] },
            "commits": { "nodes": [{ "commit": {
                "oid": "abc", "messageHeadline": "Add widgets", "committedDate": "2026-10-01T08:00:00Z",
                "additions": 3, "deletions": 1,
                "authors": { "nodes": [{ "name": "Alice", "avatarUrl": null, "user": { "login": "alice" } }, { "name": "Dan", "avatarUrl": null, "user": null }] }
            } }] },
            "comments": { "totalCount": 2, "pageInfo": page(true, "c1"), "nodes": [comment("ic-2", "2026-10-01T12:00:00Z")] },
            "reviews": { "pageInfo": page(false, "r1"), "nodes": [
                { "id": "rv-empty", "body": "", "state": "COMMENTED", "submittedAt": "2026-10-01T10:00:00Z", "lastEditedAt": null, "url": null, "author": actor("bob") },
                { "id": "rv-ok", "body": "", "state": "APPROVED", "submittedAt": "2026-10-01T11:00:00Z", "lastEditedAt": null, "url": null, "author": actor("carol") },
                { "id": "rv-pending", "body": "draft", "state": "PENDING", "submittedAt": null, "lastEditedAt": null, "url": null, "author": actor("bob") },
            ] },
            "reviewThreads": { "pageInfo": page(false, "t1"), "nodes": [{
                "id": "th-1", "isResolved": false, "isOutdated": false, "path": "src/lib.rs", "line": 4, "diffSide": "LEFT",
                "comments": { "totalCount": 3, "pageInfo": page(true, "tc1"), "nodes": [comment("rc-1", "2026-10-01T09:00:00Z")] }
            }] }
        })
    } else {
        json!({
            "comments": { "totalCount": 2, "pageInfo": page(false, "c2"), "nodes": [comment("ic-1", "2026-10-01T07:00:00Z")] }
        })
    };
    json!({ "repository": { "pullRequest": pr } })
}

#[tokio::test]
async fn activity_pages_only_the_connections_with_more_to_give() {
    let (api, seen) = graphql_stub(two_page_activity).await;
    let activity = read_activity(&api, "acme", "app", 7).await.unwrap();

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1]["head"], json!(false));
    assert_eq!(seen[1]["commentsAfter"], json!("c1"));
    assert_eq!(seen[1]["withReviews"], json!(false));
    assert_eq!(seen[1]["withThreads"], json!(false));

    let ids: Vec<&str> = activity
        .remarks
        .iter()
        .map(|remark| remark.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["ic-1", "rc-1", "rv-ok", "ic-2"],
        "oldest first, empty and pending reviews dropped"
    );
    assert_eq!(
        activity.remarks[1].kind,
        PullRequestRemarkKind::ReviewComment
    );
    assert_eq!(activity.remarks[1].path.as_deref(), Some("src/lib.rs"));
    assert_eq!(
        activity.remarks[2].review_state.as_deref(),
        Some("APPROVED")
    );
    // The thread holds two comments beyond its first page.
    assert_eq!(activity.remark_count, 6);
    assert!(activity.remarks_truncated);
    assert!(!activity.review_threads_truncated);

    let thread = &activity.review_threads[0];
    assert_eq!(thread.side, DiffSide::Left);
    assert_eq!(thread.comment_count, 3);
    assert_eq!(thread.next_comments_cursor.as_deref(), Some("tc1"));

    let reviewers: Vec<&str> = activity
        .reviewers
        .iter()
        .map(|actor| actor.login.as_str())
        .collect();
    assert_eq!(reviewers, ["acme/core", "bob", "carol"]);
    let authors: Vec<&str> = activity.commits[0]
        .authors
        .iter()
        .map(|actor| actor.login.as_str())
        .collect();
    assert_eq!(authors, ["alice", "Dan"]);
    assert_eq!(activity.author.unwrap().login, "alice");
}

fn thread_on_another_pull_request(_: &Value) -> Value {
    json!({ "node": {
        "pullRequest": { "number": 8, "repository": { "nameWithOwner": "acme/app" } },
        "comments": { "pageInfo": page(false, "x"), "nodes": [comment("rc-9", "2026-10-01T09:00:00Z")] }
    } })
}

#[tokio::test]
async fn a_thread_of_another_pull_request_is_refused() {
    let (api, _) = graphql_stub(thread_on_another_pull_request).await;
    let error = read_thread_comments(&api, "acme", "app", 7, "th-9", Some("tc1"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not on acme/app#7"), "{error}");
}

fn status(_: &Value) -> Value {
    json!({ "repository": { "pullRequest": {
        "labels": { "nodes": [{ "name": "bug", "color": "d73a4a" }] },
        "reviewRequests": { "nodes": [{ "requestedReviewer": { "__typename": "Bot", "login": "copilot", "avatarUrl": null } }] },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": { "nodes": [
            { "__typename": "CheckRun", "name": "test", "status": "IN_PROGRESS", "conclusion": null, "detailsUrl": "https://ci/1", "title": null },
            { "__typename": "CheckRun", "name": "lint", "status": "COMPLETED", "conclusion": "TIMED_OUT", "detailsUrl": null, "title": "Lint" },
            { "__typename": "StatusContext", "context": "deploy", "state": "SUCCESS", "targetUrl": "https://deploy", "description": "Deployed" },
        ] } } } }] }
    } } })
}

#[tokio::test]
async fn status_reads_labels_reviewers_and_checks() {
    let (api, _) = graphql_stub(status).await;
    let status = read_status(&api, "acme", "app", 7).await.unwrap();
    assert_eq!(status.labels[0].color.as_deref(), Some("d73a4a"));
    assert!(status.requested_reviewers[0].is_bot);
    let checks: Vec<(&str, PullRequestCheckStatus)> = status
        .checks
        .iter()
        .map(|check| (check.name.as_str(), check.status))
        .collect();
    assert_eq!(
        checks,
        [
            ("test", PullRequestCheckStatus::Pending),
            ("lint", PullRequestCheckStatus::Failure),
            ("deploy", PullRequestCheckStatus::Success),
        ]
    );
    assert_eq!(status.checks[2].description.as_deref(), Some("Deployed"));
}
