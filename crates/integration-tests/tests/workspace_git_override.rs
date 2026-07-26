//! Workspace git-override lifecycle integration tests.
//!
//! Spec IDs: WS-01 (read-only), WS-02/03/04 (override), WS-20/21 (reset guards).
//!
//! The reclaim paths (WS-06/07/08) need a reachable repository that actually
//! declares the workspace, so they are not covered here — the sync fetches the
//! config over the provider API. What is covered is everything that decides
//! whether an override survives: the read-only guard, that overriding keeps the
//! origin and unlocks editing, and that a reset which cannot reach git leaves
//! the workspace overridden rather than stranding it as git-managed.

use integration_tests::{create_repo_request, exec_psql, test_client};
use moatless_api_client::ApiError;
use moatless_api_schemas::{
    enums::RepositoryKind,
    workspace::{CreateWorkspaceRequest, UpdateWorkspaceRequest, WorkspaceRepoInput},
};
use pretty_assertions::assert_eq;

fn status_of(err: &ApiError) -> u16 {
    match err {
        ApiError::Api { status, .. } => *status,
        other => panic!("expected an API error, got {other}"),
    }
}

fn update_with_image(image: &str) -> UpdateWorkspaceRequest {
    UpdateWorkspaceRequest {
        docker_image: Some(image.to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn workspace_git_override_chain() {
    let client = test_client().await;
    let suffix = uuid::Uuid::new_v4();

    // A repo whose remote does not exist, so the reset's re-sync cannot reach
    // a config file — exactly the WS-21 situation.
    let mut repo_req = create_repo_request(
        &format!("ws-override-{suffix}"),
        RepositoryKind::Template,
    );
    repo_req.remote_url = Some(format!(
        "https://github.com/aorwall/ws-override-{suffix}.git"
    ));
    let repo_id = client.api().create_repository(&repo_req).await.unwrap().id;

    let created = client
        .api()
        .create_workspace(&CreateWorkspaceRequest {
            name: format!("Override target {suffix}"),
            description: None,
            scope: None,
            docker_image: Some("acme/from-git:1".to_string()),
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            plugins: None,
            resources: None,
            network_profile: None,
            repos: vec![WorkspaceRepoInput {
                repository_id: repo_id.clone(),
                branch: None,
                mount_name: None,
                is_primary: Some(true),
                position: None,
            }],
        })
        .await
        .unwrap();
    let ws_id = created.id;

    // WS-20: a workspace that was never in git has nothing to reset to.
    let err = client.api().reset_git_workspace(&ws_id).await.unwrap_err();
    assert_eq!(status_of(&err), 400);

    // Stamp the workspace as git-declared, standing in for a sync of a repo
    // that commits `.moatless/workspaces.json`.
    exec_psql(&format!(
        "UPDATE workspaces SET source = 'git', source_repository_id = '{repo_id}', \
         source_key = 'api', source_config_path = '.moatless/workspaces.json' \
         WHERE id = '{ws_id}'"
    ));

    // WS-01: git-declared workspaces are read-only.
    let err = client
        .api()
        .update_workspace(&ws_id, &update_with_image("acme/edited:1"))
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 409);
    let err = client.api().delete_workspace(&ws_id).await.unwrap_err();
    assert_eq!(status_of(&err), 409);

    // WS-02: overriding with an empty patch takes ownership, changing nothing.
    let overridden = client
        .api()
        .override_workspace(&ws_id, &UpdateWorkspaceRequest::default())
        .await
        .unwrap();
    assert_eq!(overridden.source, "manual");
    assert!(!overridden.synced_from_git);
    assert_eq!(
        overridden.docker_image.as_deref(),
        Some("acme/from-git:1"),
        "override must not alter the run-config"
    );

    // WS-03: the origin survives, which is what makes a reset possible.
    assert_eq!(overridden.source_repository_id.as_deref(), Some(repo_id.as_str()));
    assert_eq!(overridden.source_key.as_deref(), Some("api"));
    assert_eq!(
        overridden.source_config_path.as_deref(),
        Some(".moatless/workspaces.json")
    );

    // WS-04: editing is unlocked again.
    let edited = client
        .api()
        .update_workspace(&ws_id, &update_with_image("acme/edited:1"))
        .await
        .unwrap();
    assert_eq!(edited.docker_image.as_deref(), Some("acme/edited:1"));

    // WS-21: the re-sync cannot read a config from the unreachable remote, so
    // the reset reports why and the workspace stays overridden with its edits —
    // it must never be left claiming git provenance it does not have.
    let reset = client.api().reset_git_workspace(&ws_id).await.unwrap();
    assert!(!reset.resynced);
    assert!(reset.error.is_some(), "a failed reset must say why");
    assert_eq!(reset.workspace.source, "manual");
    assert_eq!(reset.workspace.docker_image.as_deref(), Some("acme/edited:1"));

    // An overridden workspace is deletable again (WS-08's precondition).
    client.api().delete_workspace(&ws_id).await.unwrap();
}
