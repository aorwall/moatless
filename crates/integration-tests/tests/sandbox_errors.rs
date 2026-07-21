//! Sandbox error handling integration tests.
//!
//! Mirrors: tests/api/chains/sandbox-errors.spec.ts
//!
//! Tests sandbox provisioning failure scenarios:
//! - Invalid Docker image → sandbox fails to reach ready
//! - Error detection and reporting
//!
//! Env gate: `SANDBOX_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{create_repo_request, sandbox_helpers, test_client};
use moatless_api_schemas::enums::RepositoryKind;
use moatless_api_schemas::task::CreateTaskRequest;

// =============================================================================
// SANDBOX-06: Provision failure with invalid Docker image
// =============================================================================

#[tokio::test]
#[ignore] // Requires SANDBOX_E2E=1
async fn sandbox_provision_failure_invalid_image() {
    if std::env::var("SANDBOX_E2E").is_err() {
        eprintln!("Skipping: SANDBOX_E2E not set");
        return;
    }

    let client = test_client().await;

    // Repositories are git connections only (no run-config), so an unreachable
    // remote is the provisioning failure this exercises: the sandbox can't clone
    // and never reaches ready.
    let mut repo_req = create_repo_request("Invalid Repo E2E", RepositoryKind::Template);
    repo_req.remote_url =
        Some("https://github.com/aorwall/this-repo-does-not-exist-xyz-123.git".into());

    let repo = client
        .api()
        .create_repository(&repo_req)
        .await
        .expect("Failed to create repository");
    let repo_id = &repo.id;

    // Create task (no message — just provision)
    let task_req = CreateTaskRequest {
        repository_id: Some(repo_id.clone()),
        workspace_id: None,
        message: None,
        name: Some("Invalid Image Error E2E".into()),
        branch: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
        headless: None,
        parent_task_id: None,
    };
    let created = client.api().create_task(&task_req).await.unwrap();
    let task_id = &created.task_id;
    println!("Created task {task_id} with invalid Docker image");

    // Wait — sandbox should NOT become ready (timeout after 60s)
    let became_ready =
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(60)).await;
    assert!(
        !became_ready,
        "Sandbox with invalid image should NOT become ready"
    );

    // Check that the status indicates an error
    let error =
        sandbox_helpers::wait_for_sandbox_error(&client, task_id, Duration::from_secs(10)).await;

    // The sandbox may report error through status or error field
    let status = client
        .api()
        .sandbox_status(task_id)
        .await
        .expect("Failed to get sandbox status");
    println!(
        "Sandbox status: env={}, error={:?}",
        status.environment_status, status.environment_error
    );

    // Verify it's not in "ready" state
    assert_ne!(
        status.environment_status, "ready",
        "Sandbox should not be ready with invalid image"
    );

    if let Some(err) = error {
        println!("SANDBOX-06: Error detected: {err}");
    } else {
        println!(
            "SANDBOX-06: No explicit error reported, but sandbox is not ready (status={})",
            status.environment_status
        );
    }

    // Cleanup
    let _ = client.api().cleanup_sandbox(task_id).await;
    let _ = client.api().delete_task(task_id).await;
    let _ = client.api().delete_repository(repo_id).await;
    println!("Cleanup complete");
}
