//! File operations integration tests (requires sandbox).
//!
//! Mirrors: tests/api/chains/file-operations.spec.ts
//! Spec IDs: FILE-01 through FILE-05
//!
//! Env gate: `SANDBOX_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{
    ensure_template_repo, sandbox_helpers, test_client,
};
use moatless_api_client::types::{
    CreateTaskRequest, SearchByNameRequest, WriteFileRequestBody,
};
use pretty_assertions::assert_eq;

// =============================================================================
// Chain: File Operations (FILE-01 through FILE-05)
// =============================================================================

#[tokio::test]
#[ignore] // Requires SANDBOX_E2E=1
async fn file_operations_chain() {
    if std::env::var("SANDBOX_E2E").is_err() {
        eprintln!("Skipping: SANDBOX_E2E not set");
        return;
    }

    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;

    // Create task without message (no agent execution)
    let req = CreateTaskRequest {
        repository_id: repo_id.clone(),
        message: None,
        name: Some("File Operations E2E".into()),
        branch: None,
        parent_task_id: None,
        description: None,
        agent_mode: None,
        agent_type: None,
        contexts: vec![],
        file_ids: vec![],
        model: None,
    };
    let created = client.create_task(&req).await.unwrap().into_inner();
    let task_id = &created.task_id;

    // Wait for sandbox to be ready
    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(120)).await,
        "Sandbox did not become ready"
    );

    // FILE-01: List files, poll until non-empty, assert package.json
    assert!(
        sandbox_helpers::wait_for_files_available(&client, task_id, Duration::from_secs(30)).await,
        "Files did not become available"
    );
    let files = client
        .list_files_handler(task_id, None, None)
        .await
        .unwrap()
        .into_inner();
    assert!(
        files.files.iter().any(|f| f.contains("package.json")),
        "Expected package.json in file list: {:?}",
        files.files
    );

    // FILE-02: Read package.json, assert non-empty content
    let content = client
        .read_file_handler(task_id, None, "package.json")
        .await
        .unwrap()
        .into_inner();
    assert!(
        !content.content.is_empty(),
        "package.json content should not be empty"
    );

    // FILE-03: File tree, assert has entries
    let tree = client
        .file_tree_handler(task_id, None)
        .await
        .unwrap()
        .into_inner();
    assert!(
        !tree.tree.is_empty(),
        "File tree should have entries"
    );

    // FILE-04: Search by name "package.json", assert found
    let search_req = SearchByNameRequest {
        query: "package.json".into(),
        case_sensitive: None,
        max_results: None,
    };
    let search_result = client
        .search_by_name_handler(task_id, &search_req)
        .await
        .unwrap()
        .into_inner();
    assert!(
        search_result.total_matches > 0,
        "Expected to find package.json via search"
    );

    // FILE-05: Write file and read back to verify
    let write_req = WriteFileRequestBody {
        path: "e2e-test-file.txt".into(),
        content: "E2E file operations test content".into(),
    };
    client
        .write_file_handler(task_id, &write_req)
        .await
        .unwrap();

    let readback = client
        .read_file_handler(task_id, None, "e2e-test-file.txt")
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        readback.content, "E2E file operations test content",
        "Written content should match read content"
    );

    // Cleanup
    let _ = client.delete_task(task_id).await;
}
