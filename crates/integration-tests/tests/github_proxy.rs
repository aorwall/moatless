//! E2E tests for the typed `/api/v1/github/*` proxy endpoints behind
//! `moat gh` (PR create + PR/issue comments + threaded review-comment
//! replies).
//!
//! Two tiers:
//! - Unconditional (db tier): task-access and token-resolution error paths —
//!   no GitHub connectivity needed.
//! - Gated (`GITHUB_PROXY_E2E=1`, `#[ignore]`): full happy path against an
//!   in-process mock GitHub API. The backend must be started with
//!   `GITHUB_API_BASE_URL` pointing at the mock (the docker-compose CI
//!   overlay passes it through) and a non-empty `GITHUB_CHANNEL_TOKEN`:
//!
//!   ```bash
//!   GITHUB_PROXY_E2E=1 \
//!   GITHUB_API_BASE_URL=http://host.docker.internal:18092 \
//!   GITHUB_CHANNEL_TOKEN=mock-token \
//!     docker-compose/test-runner.sh --test github_proxy
//!   ```

use std::time::Duration;

use integration_tests::mock_github::{
    MockGithub, MOCK_COMMENT_ID, MOCK_DEFAULT_BRANCH, MOCK_PR_NUMBER, MOCK_REPLY_ID,
};
use integration_tests::{ensure_template_repo, test_client};
use moatless_api_client::ApiError;
use moatless_api_schemas::adapters::CreateAdapterConnectionRequest;
use moatless_api_schemas::enums::{AdapterKind, RoutingMode};
use moatless_api_schemas::github::{
    GithubCommentRequest, GithubCreatePrRequest, GithubReplyRequest,
};
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_api_schemas::triggers::{
    CreateAdapterEventTriggerSource, CreateTriggerRequest, CreateTriggerRule, CreateTriggerSource,
    TriggerKind,
};
use moatless_cli::client::MoatClient;
use pretty_assertions::assert_eq;

/// Mock port must be fixed: the backend reads `GITHUB_API_BASE_URL` at
/// container start, before the test binds the listener.
const DEFAULT_MOCK_PORT: u16 = 18092;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

fn api_status(err: ApiError) -> u16 {
    match err {
        ApiError::Api { status, .. } => status,
        other => panic!("expected ApiError::Api, got: {other}"),
    }
}

async fn create_bare_task(client: &MoatClient, repo_id: &str) -> String {
    client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: repo_id.to_string(),
            workspace_id: None,
            message: Some("github-proxy fixture task".into()),
            name: Some("github-proxy-test".into()),
            branch: None,
            description: None,
            agent_mode: None,
            agent_type: None,
            contexts: vec![],
            file_ids: vec![],
            model: None,
            command_name: None,
            headless: None,
            task_template_id: None,
            parent_task_id: None,
        })
        .await
        .unwrap()
        .task_id
}

async fn cleanup_task(client: &MoatClient, task_id: &str) {
    let _ = client.api().stop_task(task_id).await;
    let _ = client.api().delete_task(task_id).await;
}

// =============================================================================
// Unconditional error paths (db tier)
// =============================================================================

#[tokio::test]
async fn create_pr_with_unknown_task_returns_404() {
    let client = test_client();
    let err = client
        .api()
        .github_create_pr(
            "no-such-owner",
            "no-such-repo",
            &GithubCreatePrRequest {
                title: "t".into(),
                body: None,
                head: "feat/x".into(),
                base: Some("main".into()),
                draft: None,
                task_id: Some("task-does-not-exist".into()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(api_status(err), 404, "task access is checked first");
}

#[tokio::test]
async fn comment_with_unknown_task_returns_404() {
    let client = test_client();
    let err = client
        .api()
        .github_post_issue_comment(
            "no-such-owner",
            "no-such-repo",
            1,
            &GithubCommentRequest {
                body: "hi".into(),
                task_id: Some("task-does-not-exist".into()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(api_status(err), 404, "task access is checked first");
}

#[tokio::test]
async fn reply_with_unknown_task_returns_404() {
    let client = test_client();
    let err = client
        .api()
        .github_reply_review_comment(
            "no-such-owner",
            "no-such-repo",
            1,
            99,
            &GithubReplyRequest {
                body: "hi".into(),
                task_id: Some("task-does-not-exist".into()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(api_status(err), 404, "task access is checked first");
}

#[tokio::test]
async fn create_pr_without_github_token_returns_502() {
    // Only meaningful when the backend has no usable GitHub credentials and
    // talks to the real github.com; under the mocked E2E setup the call
    // would succeed instead.
    if std::env::var("GITHUB_PROXY_E2E").is_ok() {
        eprintln!("Skipping: GITHUB_PROXY_E2E set (mock would answer)");
        return;
    }
    let client = test_client();
    let err = client
        .api()
        .github_create_pr(
            "no-such-owner",
            "no-such-repo",
            &GithubCreatePrRequest {
                title: "t".into(),
                body: None,
                head: "feat/x".into(),
                base: Some("main".into()),
                draft: None,
                task_id: None,
            },
        )
        .await
        .unwrap_err();
    // Both failure shapes are AdapterError::AdapterApi → 502: no token →
    // "No GitHub token available"; an (invalid) token from the environment →
    // GitHub rejects the call upstream.
    assert_eq!(api_status(err), 502);
}

// =============================================================================
// Gated happy path against the in-process mock GitHub
// =============================================================================

#[tokio::test]
#[ignore] // Requires GITHUB_PROXY_E2E=1 + GITHUB_API_BASE_URL on the backend
async fn pr_create_and_comment_bind_task_via_mock_github() {
    if std::env::var("GITHUB_PROXY_E2E").is_err() {
        eprintln!("Skipping: GITHUB_PROXY_E2E not set");
        return;
    }

    let mock_port: u16 = std::env::var("GITHUB_MOCK_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MOCK_PORT);
    let mock = MockGithub::start(mock_port).await;

    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;
    let suffix = timestamp_suffix();
    let owner = format!("mockorg{suffix}");
    let gh_repo = "mockrepo";

    // connect_github_binding requires a `github` connection scoped to the
    // owner and an adapter-event trigger matching `owner/repo`.
    let connection = client
        .api()
        .create_adapter_connection(&CreateAdapterConnectionRequest {
            adapter_kind: AdapterKind::Github,
            connection_kind: Some("github_org".into()),
            external_account_id: owner.clone(),
            webhook_secret: None,
            source_name: None,
            transport_config: None,
            team_id: None,
            scope: None,
        })
        .await
        .unwrap();
    let trigger = client
        .api()
        .create_trigger(&CreateTriggerRequest {
            name: Some(format!("github-proxy-{suffix}")),
            kind: TriggerKind::AdapterEvent,
            active: Some(true),
            scope: None,
            team_id: None,
            rule: CreateTriggerRule {
                repository_id: repo_id.clone(),
                routing_mode: RoutingMode::BySubject,
                ongoing_task_id: None,
                task_name: None,
                prompt: None,
                tag_ids: None,
                run_as_user_id: None,
                agent_type: None,
                inactive_ttl_days: None,
            },
            source: CreateTriggerSource::AdapterEvent(CreateAdapterEventTriggerSource {
                adapter_kind: AdapterKind::Github,
                connection_id: connection.id.clone(),
                source_matcher: format!("{owner}/{gh_repo}"),
                source_name: Some(format!("{owner}/{gh_repo}")),
                event_filter: None,
                adapter_config: None,
                allow_direct_send: Some(false),
                close_task_on_close: Some(false),
                notify_on_success: Some(false),
            }),
        })
        .await
        .unwrap();

    let task_id = create_bare_task(&client, &repo_id).await;

    // ---- PR create: base omitted (mock serves default branch), body gets
    // the Moatless task footer, binding is upserted. ----
    let pr = client
        .api()
        .github_create_pr(
            &owner,
            gh_repo,
            &GithubCreatePrRequest {
                title: "feat: mock pr".into(),
                body: Some("PR body from e2e test".into()),
                head: "feat/mock".into(),
                base: None,
                draft: None,
                task_id: Some(task_id.clone()),
            },
        )
        .await
        .unwrap();

    assert_eq!(pr.number, MOCK_PR_NUMBER);
    assert!(
        pr.html_url.ends_with(&format!("/pull/{MOCK_PR_NUMBER}")),
        "unexpected html_url: {}",
        pr.html_url
    );
    assert!(
        pr.binding_id.is_some(),
        "PR create with task_id must bind the task"
    );

    // The mock saw the create-PR request with the resolved default base
    // branch and the task footer baked into the body.
    let create_req = mock
        .wait_for(Duration::from_secs(5), |c| {
            c.method == "POST" && c.path.ends_with("/pulls")
        })
        .await
        .expect("mock never received POST /pulls");
    assert_eq!(create_req.body["base"].as_str(), Some(MOCK_DEFAULT_BRANCH));
    assert_eq!(create_req.body["head"].as_str(), Some("feat/mock"));
    let sent_body = create_req.body["body"].as_str().unwrap_or_default();
    assert!(
        sent_body.starts_with("PR body from e2e test"),
        "original body must be preserved: {sent_body}"
    );
    assert!(
        sent_body.contains("Moatless task:") && sent_body.contains(&task_id),
        "footer with task URL must be baked in: {sent_body}"
    );

    let bindings = client.api().list_task_bindings(&task_id).await.unwrap();
    assert_eq!(bindings.len(), 1, "exactly one binding after PR create");
    assert_eq!(bindings[0].subject_kind, "github_pr");
    assert_eq!(
        bindings[0].destination_config["pr_number"].as_u64(),
        Some(MOCK_PR_NUMBER)
    );
    assert_eq!(
        bindings[0].destination_config["owner"].as_str(),
        Some(owner.as_str())
    );

    // ---- Comment on the PR: same endpoint serves issues and PRs; binding
    // upsert is idempotent. ----
    let comment = client
        .api()
        .github_post_issue_comment(
            &owner,
            gh_repo,
            MOCK_PR_NUMBER,
            &GithubCommentRequest {
                body: "comment from e2e test".into(),
                task_id: Some(task_id.clone()),
            },
        )
        .await
        .unwrap();
    assert_eq!(comment.id, MOCK_COMMENT_ID);
    assert!(
        comment
            .html_url
            .ends_with(&format!("#issuecomment-{MOCK_COMMENT_ID}")),
        "unexpected comment url: {}",
        comment.html_url
    );

    let comment_req = mock
        .wait_for(Duration::from_secs(5), |c| {
            c.method == "POST" && c.path.contains("/issues/") && c.path.ends_with("/comments")
        })
        .await
        .expect("mock never received POST /issues/{n}/comments");
    assert_eq!(
        comment_req.body["body"].as_str(),
        Some("comment from e2e test")
    );

    let bindings = client.api().list_task_bindings(&task_id).await.unwrap();
    assert_eq!(bindings.len(), 1, "binding upsert must be idempotent");

    // ---- Threaded reply to a review comment: routes through the pulls
    // replies endpoint, binds the PR (not the comment), dedup-notes the new
    // review comment id. ----
    let reply = client
        .api()
        .github_reply_review_comment(
            &owner,
            gh_repo,
            MOCK_PR_NUMBER,
            99,
            &GithubReplyRequest {
                body: "reply from e2e test".into(),
                task_id: Some(task_id.clone()),
            },
        )
        .await
        .unwrap();
    assert_eq!(reply.id, MOCK_REPLY_ID);
    assert!(
        reply.html_url.ends_with(&format!(
            "/pull/{MOCK_PR_NUMBER}#discussion_r{MOCK_REPLY_ID}"
        )),
        "unexpected reply url: {}",
        reply.html_url
    );
    assert!(
        reply.binding_id.is_some(),
        "reply with task_id must bind the task"
    );

    let reply_req = mock
        .wait_for(Duration::from_secs(5), |c| {
            c.method == "POST" && c.path.ends_with("/comments/99/replies")
        })
        .await
        .expect("mock never received POST .../comments/99/replies");
    assert_eq!(reply_req.body["body"].as_str(), Some("reply from e2e test"));

    let bindings = client.api().list_task_bindings(&task_id).await.unwrap();
    assert_eq!(
        bindings.len(),
        1,
        "reply binds the same PR — still one binding"
    );

    // Cleanup
    cleanup_task(&client, &task_id).await;
    let _ = client.api().delete_trigger(&trigger.id).await;
}
