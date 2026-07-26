//! Multi-repo workspace integration test (requires sandbox).
//!
//! Spec ID: WS-30 — a workspace with several repository placements checks out
//! every one of them into the task's sandbox.
//!
//! Both placements point at the same template repository under different mount
//! names. That is a legitimate multi-repo workspace (the same repo placed twice
//! is how you get two branches side by side) and it exercises exactly the path
//! under test — two checkouts, two mount directories, one primary — without
//! depending on a second fixture repository being reachable.
//!
//! Env gate: `SANDBOX_E2E` must be set to run.

use std::time::Duration;

use integration_tests::{ensure_template_repo, sandbox_helpers, test_client};
use moatless_api_schemas::{
    task::CreateTaskRequest,
    workspace::{CreateWorkspaceRequest, WorkspaceRepoInput},
};

const PRIMARY_MOUNT: &str = "app-primary";
const SECONDARY_MOUNT: &str = "app-secondary";

#[tokio::test]
#[ignore] // Requires SANDBOX_E2E=1
async fn multi_repo_workspace_checks_out_every_repo() {
    if std::env::var("SANDBOX_E2E").is_err() {
        eprintln!("Skipping: SANDBOX_E2E not set");
        return;
    }

    let client = test_client().await;
    let repo_id = ensure_template_repo(&client).await;
    let suffix = uuid::Uuid::new_v4();

    let workspace = client
        .api()
        .create_workspace(&CreateWorkspaceRequest {
            name: format!("multi-repo-{suffix}"),
            description: None,
            scope: None,
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            plugins: None,
            resources: None,
            network_profile: None,
            repos: vec![
                WorkspaceRepoInput {
                    repository_id: repo_id.clone(),
                    branch: None,
                    mount_name: Some(PRIMARY_MOUNT.into()),
                    is_primary: Some(true),
                    position: Some(0),
                },
                WorkspaceRepoInput {
                    repository_id: repo_id.clone(),
                    branch: None,
                    mount_name: Some(SECONDARY_MOUNT.into()),
                    is_primary: Some(false),
                    position: Some(1),
                },
            ],
        })
        .await
        .unwrap();
    assert_eq!(workspace.repos.len(), 2, "both placements must be stored");
    assert_eq!(workspace.kind, "multi");

    let created = client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: None,
            workspace_id: Some(workspace.id.clone()),
            message: None,
            name: Some("Multi-repo workspace E2E".into()),
            branch: None,
            description: None,
            agent_mode: None,
            agent_type: None,
            contexts: vec![],
            file_ids: vec![],
            model: None,
            headless: None,
            parent_task_id: None,
        })
        .await
        .unwrap();
    let task_id = &created.task_id;

    assert!(
        sandbox_helpers::wait_for_sandbox_ready(&client, task_id, Duration::from_secs(180)).await,
        "Sandbox did not become ready"
    );
    assert!(
        sandbox_helpers::wait_for_files_available(&client, task_id, Duration::from_secs(60)).await,
        "Files did not become available"
    );

    // The file browser is rooted at the workspace dir, so each placement shows
    // up as its own top-level directory named by its mount name.
    let files = client.api().list_files(task_id).await.unwrap();
    for mount in [PRIMARY_MOUNT, SECONDARY_MOUNT] {
        assert!(
            files
                .files
                .iter()
                .any(|path| path.trim_end_matches('/') == mount),
            "Expected a checkout at {mount}: {:?}",
            files.files
        );
    }

    // Each directory is a real checkout, not an empty mount point.
    for mount in [PRIMARY_MOUNT, SECONDARY_MOUNT] {
        let content = client
            .api()
            .read_file(task_id, &format!("{mount}/package.json"))
            .await
            .unwrap_or_else(|e| panic!("{mount} is not a checkout of the repository: {e}"));
        assert!(
            !content.content.is_empty(),
            "{mount}/package.json should not be empty"
        );
    }

    // Cleanup
    let _ = client.api().delete_task(task_id).await;
    let _ = client.api().delete_workspace(&workspace.id).await;
}
