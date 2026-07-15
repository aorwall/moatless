//! Per-task connections endpoint integration tests.
//!
//! Covers `GET /api/v1/tasks/{task_id}/connections` — the non-admin,
//! scope-checked counterpart to the admin task-connections list. The decode
//! logic itself is unit-tested in `backend/src/task/connections_handlers.rs`;
//! these exercise the wired-up HTTP endpoint end to end (empty-state and
//! access control).

use integration_tests::{
    client_as_user, client_as_user_with_teams, create_repo_request, ensure_template_repo,
    seed_test_users, test_client,
};
use moatless_api_client::ApiError;
use moatless_api_schemas::enums::{RepositoryKind, Scope};
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_api_schemas::team::CreateTeamRequest;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

fn task_request(repository_id: String, message: &str) -> CreateTaskRequest {
    CreateTaskRequest {
        repository_id,
        message: Some(message.into()),
        name: None,
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
    }
}

// A freshly created task has no external connections wired to it.
#[tokio::test]
async fn task_connections_empty_when_no_bindings() {
    let client = test_client();
    let repo_id = ensure_template_repo(&client).await;

    let created = client
        .api()
        .create_task(&task_request(repo_id, "Connections empty-state task"))
        .await
        .unwrap();
    let task_id = created.task_id;

    let connections = client.api().get_task_connections(&task_id).await.unwrap();
    assert!(
        connections.is_empty(),
        "expected no connections, got {connections:?}"
    );

    // Cleanup
    let _ = client.api().delete_task(&task_id).await;
}

// The endpoint is scope-checked like `get_task` (not admin-gated): the task
// owner can load it, but a user who cannot see the task gets a 404.
#[tokio::test]
async fn task_connections_scope_respected() {
    seed_test_users().await;

    let alice = client_as_user("alice-test", "alice", "user");
    let suffix = timestamp_suffix();

    let team = alice
        .api()
        .create_team(&CreateTeamRequest {
            name: "Connections Scope Team".into(),
            slug: format!("conn-scope-{suffix}"),
            description: None,
        })
        .await
        .unwrap();
    let team_id = team.id.clone();

    let alice_with_team = client_as_user_with_teams("alice-test", "alice", "user", &[&team_id]);

    let mut req = create_repo_request(
        &format!("Connections Scope Repo {suffix}"),
        RepositoryKind::Template,
    );
    req.scope = Some(Scope::Team);
    req.team_id = Some(team_id.clone());
    let team_repo = alice_with_team.api().create_repository(&req).await.unwrap();

    let task = alice_with_team
        .api()
        .create_task(&task_request(
            team_repo.id.clone(),
            "Scoped connections task",
        ))
        .await
        .unwrap();
    let task_id = task.task_id;

    // Owner (team member) can load the endpoint — empty list, not admin-gated.
    let owner_view = alice_with_team
        .api()
        .get_task_connections(&task_id)
        .await
        .unwrap();
    assert!(owner_view.is_empty());

    // Non-member is denied with 404, mirroring `get_task`.
    let outsider = client_as_user("outsider-user", "outsider", "user");
    let err = outsider
        .api()
        .get_task_connections(&task_id)
        .await
        .expect_err("outsider must not read another scope's connections");
    match err {
        ApiError::Api { status, .. } => assert_eq!(status, 404, "expected 404, got {status}"),
        other => panic!("expected 404 ApiError, got {other:?}"),
    }

    // Cleanup
    let _ = alice.api().delete_task(&task_id).await;
    let _ = alice.api().delete_repository(&team_repo.id).await;
    let _ = alice.api().delete_team(&team_id).await;
}
