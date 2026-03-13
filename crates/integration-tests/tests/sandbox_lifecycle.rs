//! Sandbox lifecycle integration tests (requires sandbox).
//!
//! Mirrors: tests/api/chains/sandbox-lifecycle.spec.ts
//! Spec IDs: SANDBOX-01 through SANDBOX-05.1
//!
//! Env gate: `SANDBOX_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{task::CreateTaskRequest, sandbox::WriteFileRequestBody};
use moatless_cli::events::EventCollector;

// =============================================================================
// Chain: Sandbox Lifecycle (SANDBOX-01 through SANDBOX-05.1)
// =============================================================================

#[tokio::test]
#[ignore] // Requires SANDBOX_E2E=1
async fn sandbox_lifecycle_chain() {
    if std::env::var("SANDBOX_E2E").is_err() {
        eprintln!("Skipping: SANDBOX_E2E not set");
        return;
    }

    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;

    // Start event collector
    let collector = EventCollector::start(&client)
        .await
        .expect("Failed to start event collector");

    // Create task without message (no agent)
    let req = CreateTaskRequest {
        repository_id: repo_id.clone(),
        message: None,
        name: Some("Sandbox Lifecycle E2E".into()),
        branch: None,
        parent_task_id: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
    };
    let created = client.api().create_task(&req).await.unwrap();
    let task_id = &created.task_id;

    // SANDBOX-01: Wait for sandbox ready
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    let status = client
        .api()
        .sandbox_status(task_id)
        .await
        .unwrap();
    assert_eq!(
        status.environment_status,
        "ready",
        "environment_status should be ready"
    );

    // SANDBOX-01.1: SSE event for sandbox ready
    let sse_ready = collector
        .wait_for_event("sandbox", Some("ready"), task_id, Duration::from_secs(10))
        .await;
    assert!(sse_ready.is_some(), "Expected SSE sandbox.ready event");

    // Write a marker file for persistence checks
    let write_req = WriteFileRequestBody {
        path: "lifecycle-marker.txt".into(),
        content: "sandbox lifecycle test".into(),
    };
    client
        .api()
        .write_file(task_id, &write_req)
        .await
        .unwrap();

    // SANDBOX-02: Provision again — should be idempotent (still ready)
    let provision_resp = client
        .api()
        .provision_sandbox(task_id)
        .await
        .unwrap();
    // Verify still ready after provision
    let status = client
        .api()
        .sandbox_status(task_id)
        .await
        .unwrap();
    assert_eq!(
        status.environment_status,
        "ready",
        "Should still be ready after re-provision"
    );
    println!(
        "SANDBOX-02: Re-provision idempotent, status={}",
        provision_resp.status
    );

    // SANDBOX-03: Stop sandbox
    client.api().stop_sandbox(task_id).await.unwrap();
    assert!(
        sandbox_helpers::wait_for_sandbox_status(
            &client,
            task_id,
            "stopped",
            Duration::from_secs(60)
        )
        .await,
        "Sandbox did not reach stopped state"
    );

    // SANDBOX-03.2: Start task — should re-provision and become ready
    client.api().start_task(task_id).await.unwrap();
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready after start"
    );

    // Verify files persist after restart
    let readback = client
        .api()
        .read_file(task_id, "lifecycle-marker.txt")
        .await
        .unwrap();
    assert_eq!(
        readback.content, "sandbox lifecycle test",
        "Marker file should persist across stop/start"
    );

    // SANDBOX-04: Restart sandbox
    client.api().restart_sandbox(task_id).await.unwrap();
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready after restart"
    );

    // Verify files persist after restart
    let readback = client
        .api()
        .read_file(task_id, "lifecycle-marker.txt")
        .await
        .unwrap();
    assert_eq!(
        readback.content, "sandbox lifecycle test",
        "Marker file should persist across restart"
    );

    // SANDBOX-05: Cleanup sandbox
    client.api().cleanup_sandbox(task_id).await.unwrap();
    assert!(
        sandbox_helpers::wait_for_sandbox_status(
            &client,
            task_id,
            "not_created",
            Duration::from_secs(60)
        )
        .await,
        "Sandbox did not reach not_created state after cleanup"
    );

    // Stop collector
    collector.stop();

    // Cleanup task
    let _ = client.api().delete_task(task_id).await;
}
