//! Repository CRUD integration tests.
//!
//! Mirrors: tests/api/chains/repository.spec.ts
//! Spec IDs: REPO-01 through REPO-07

use integration_tests::{create_repo_request, test_client};
use moatless_api_schemas::{
    enums::{RepositoryKind, RepositoryProvider},
    repository::UpdateRepositoryRequest,
};
use pretty_assertions::assert_eq;

// =============================================================================
// Chain: Repository CRUD (REPO-01 through REPO-05.1)
// =============================================================================

#[tokio::test]
async fn repo_crud_chain() {
    let client = test_client().await;

    // REPO-01: Create repository
    let mut req = create_repo_request("E2E Test Repository", RepositoryKind::Template);
    req.remote_url = Some("https://github.com/aorwall/hello-world-ui.git".into());
    req.description = Some("Created by API integration test".into());

    let created = client.api().create_repository(&req).await.unwrap();
    assert!(!created.id.is_empty());
    assert_eq!(created.name, "E2E Test Repository");
    assert_eq!(created.kind, RepositoryKind::Template);

    let repo_id = created.id;

    // REPO-01.1: Repository appears in listing
    let repos = client.api().list_repositories().await.unwrap();
    assert!(repos.iter().any(|r| r.id == repo_id));

    // REPO-02: Get repository by ID
    let got = client.api().get_repository(&repo_id).await.unwrap();
    assert_eq!(got.id, repo_id);
    assert_eq!(got.name, "E2E Test Repository");
    assert_eq!(got.kind, RepositoryKind::Template);
    assert!(got.created_at.is_some());

    // REPO-03: Update repository metadata
    let update = UpdateRepositoryRequest {
        name: Some("Updated E2E Repository".into()),
        description: Some("Updated by API integration test".into()),
        ..Default::default()
    };
    let updated = client
        .api()
        .update_repository(&repo_id, &update)
        .await
        .unwrap();
    assert_eq!(updated.name, "Updated E2E Repository");
    assert_eq!(
        updated.description.as_deref(),
        Some("Updated by API integration test")
    );

    // Confirm via GET
    let fetched = client.api().get_repository(&repo_id).await.unwrap();
    assert_eq!(fetched.name, "Updated E2E Repository");

    // REPO-04: List repositories
    let repos = client.api().list_repositories().await.unwrap();
    assert!(!repos.is_empty());
    let found = repos.iter().find(|r| r.id == repo_id);
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Updated E2E Repository");

    // REPO-05: Delete repository
    client.api().delete_repository(&repo_id).await.unwrap();

    // REPO-05.1: Repository removed from listing
    let repos = client.api().list_repositories().await.unwrap();
    assert!(!repos.iter().any(|r| r.id == repo_id));
}

// =============================================================================
// Standalone: Error Cases
// =============================================================================

// REPO-06: Create repository with missing required fields (422)
#[tokio::test]
async fn repo_create_missing_fields() {
    let http = reqwest::Client::new();
    let base_url = integration_tests::base_url();

    let resp = http
        .post(format!("{base_url}/api/v1/repositories"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert!((400..500).contains(&status), "expected 4xx, got {status}");
}

// REPO-07: Get non-existent repository (404)
#[tokio::test]
async fn repo_get_nonexistent() {
    let client = test_client().await;
    let fake_id = uuid::Uuid::new_v4().to_string();
    let err = client.api().get_repository(&fake_id).await.unwrap_err();

    match &err {
        moatless_api_client::ApiError::Api { status, .. } => {
            assert_eq!(*status, 404);
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

// REPO-08: Manual access verification rejects targets it cannot check.
#[tokio::test]
async fn repo_verify_access_validates_persisted_target() {
    let client = test_client().await;
    let suffix = uuid::Uuid::new_v4();

    let missing_url = client
        .api()
        .create_repository(&create_repo_request(
            &format!("Verify Missing URL {suffix}"),
            RepositoryKind::Template,
        ))
        .await
        .unwrap();
    let err = client
        .api()
        .verify_repository_access(&missing_url.id)
        .await
        .expect_err("missing URL must be rejected");
    assert!(matches!(
        err,
        moatless_api_client::ApiError::Api { status: 400, .. }
    ));

    let mut invalid_request = create_repo_request(
        &format!("Verify Invalid URL {suffix}"),
        RepositoryKind::Template,
    );
    invalid_request.remote_url = Some("not-a-github-url".into());
    invalid_request.provider = Some(RepositoryProvider::Github);
    let invalid = client
        .api()
        .create_repository(&invalid_request)
        .await
        .unwrap();
    let err = client
        .api()
        .verify_repository_access(&invalid.id)
        .await
        .expect_err("invalid URL must be rejected");
    assert!(matches!(
        err,
        moatless_api_client::ApiError::Api { status: 400, .. }
    ));

    let mut gitness_request = create_repo_request(
        &format!("Verify Gitness {suffix}"),
        RepositoryKind::Template,
    );
    gitness_request.remote_url = Some("https://gitness.example/acme/repo".into());
    gitness_request.provider = Some(RepositoryProvider::Gitness);
    let gitness = client
        .api()
        .create_repository(&gitness_request)
        .await
        .unwrap();
    let err = client
        .api()
        .verify_repository_access(&gitness.id)
        .await
        .expect_err("Gitness verification must be unavailable");
    assert!(matches!(
        err,
        moatless_api_client::ApiError::Api { status: 400, .. }
    ));

    for repository_id in [&missing_url.id, &invalid.id, &gitness.id] {
        client.api().delete_repository(repository_id).await.unwrap();
    }
}
