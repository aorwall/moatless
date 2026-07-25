//! Auth access control integration tests.
//!
//! Mirrors: tests/api/chains/auth-access-control.spec.ts
//! Spec IDs: AUTH-20 through AUTH-26. Team-scope scenarios (AUTH-21.x,
//! AUTH-23.x team parts, AUTH-27 team writes) are retired — the scope model
//! is two-valued (`global` | `user`) and teams no longer grant access.

use futures_util::future::join_all;
use integration_tests::{
    admin_client, base_url, client_as_user, create_repo_request, raw_client_as_user,
    seed_test_users,
};
use moatless_api_client::ApiError;
use moatless_api_schemas::auth::CreateUserRequest;
use moatless_api_schemas::enums::{RepositoryKind, Scope, UserRole};
use moatless_api_schemas::repository::UpdateRepositoryRequest;
use moatless_api_schemas::task::CreateTaskRequest;
use pretty_assertions::assert_eq;

fn timestamp_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

#[tokio::test]
async fn concurrent_local_user_creation_returns_conflict_not_internal_error() {
    let login = format!("concurrent-user-{}", timestamp_suffix());
    let results = join_all((0..8).map(|_| {
        let login = login.clone();
        async move {
            admin_client()
                .await
                .api()
                .create_user(&CreateUserRequest {
                    is_bot: false,
                    github_app_key: None,
                    login: Some(login),
                    password: Some("integration-test-password".to_string()),
                    role: Some(UserRole::User),
                    must_change_password: Some(false),
                })
                .await
        }
    }))
    .await;

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for error in results.into_iter().filter_map(Result::err) {
        assert!(
            matches!(error, ApiError::Api { status: 409, .. }),
            "racing local-user creation returned {error}"
        );
    }
}

// AUTH-27 (rewritten): the global GitHub installation binding is admin-only.
#[tokio::test]
async fn github_installation_settings_are_admin_only() {
    let suffix = timestamp_suffix();
    let (_, admin_http) =
        raw_client_as_user(&format!("installation-admin-{suffix}"), "admin").await;
    let (_, user_http) = raw_client_as_user(&format!("installation-user-{suffix}"), "user").await;

    let apps_url = format!("{}/api/v1/admin/settings/github/apps", base_url());
    assert_eq!(user_http.get(&apps_url).send().await.unwrap().status(), 403);
    assert_eq!(
        admin_http.get(&apps_url).send().await.unwrap().status(),
        200
    );

    let invalid_binding = admin_http
        .put(format!(
            "{}/api/v1/admin/settings/github/installation",
            base_url()
        ))
        .json(&serde_json::json!({
            "githubAppKey": "not-configured",
            "installationId": 42,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_binding.status(), 400);
}

// =============================================================================
// Chain 1: Repository Scope Filtering (AUTH-20 through AUTH-22.2)
// =============================================================================

#[tokio::test]
async fn repository_scope_filtering() {
    let users = seed_test_users().await;
    let (alice, bob, carol) = (users.alice, users.bob, users.carol);
    let suffix = timestamp_suffix();

    // AUTH-20: Create global-scoped repository (carol is admin)
    let mut req = create_repo_request(&format!("Global Repo {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::Global);
    let global_repo = carol.client.api().create_repository(&req).await.unwrap();
    assert_eq!(global_repo.scope, Scope::Global);

    // AUTH-20.1: Other user can see global repository
    let bob_repos = bob.client.api().list_repositories().await.unwrap();
    assert!(bob_repos.iter().any(|r| r.id == global_repo.id));

    // AUTH-20.2: Non-admin cannot create global-scoped repositories
    let mut req = create_repo_request(
        &format!("Global Denied {suffix}"),
        RepositoryKind::Template,
    );
    req.scope = Some(Scope::Global);
    let err = bob
        .client
        .api()
        .create_repository(&req)
        .await
        .expect_err("non-admin must not create global repositories");
    assert!(matches!(err, ApiError::Api { status: 403, .. }));

    // AUTH-20.3: Non-admin cannot mutate a global repository — 404, not 403,
    // so control denials never confirm existence semantics beyond read.
    let err = bob
        .client
        .api()
        .update_repository(
            &global_repo.id,
            &UpdateRepositoryRequest {
                description: Some("hijack".into()),
                ..Default::default()
            },
        )
        .await
        .expect_err("non-admin must not mutate global repositories");
    assert!(matches!(err, ApiError::Api { status: 404, .. }));

    // AUTH-22: Create user-scoped repository (the default)
    let mut req = create_repo_request(&format!("User Repo {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::User);
    let user_repo = alice.client.api().create_repository(&req).await.unwrap();
    assert_eq!(user_repo.scope, Scope::User);

    // AUTH-22.1: Creator can see user repository
    let alice_repos = alice.client.api().list_repositories().await.unwrap();
    assert!(alice_repos.iter().any(|r| r.id == user_repo.id));

    // AUTH-22.2: Other user cannot see user repository
    let bob_repos = bob.client.api().list_repositories().await.unwrap();
    assert!(!bob_repos.iter().any(|r| r.id == user_repo.id));

    // AUTH-22.3: Other user gets 404 (not 403) fetching it by id
    let err = bob
        .client
        .api()
        .get_repository(&user_repo.id)
        .await
        .expect_err("someone else's user-scoped repository must be hidden");
    assert!(matches!(err, ApiError::Api { status: 404, .. }));

    // Cleanup
    let _ = carol.client.api().delete_repository(&global_repo.id).await;
    let _ = alice.client.api().delete_repository(&user_repo.id).await;
}

// =============================================================================
// Chain 2: Task Scope Inheritance (AUTH-23, rewritten for user scope)
// =============================================================================

#[tokio::test]
async fn task_scope_inheritance() {
    let users = seed_test_users().await;
    let (alice, bob) = (users.alice, users.bob);
    let suffix = timestamp_suffix();

    let req = create_repo_request(
        &format!("Task Scope Repo {suffix}"),
        RepositoryKind::Template,
    );
    let repo = alice.client.api().create_repository(&req).await.unwrap();

    // AUTH-23: Create task under a user-scoped repository
    let task = alice
        .client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: Some(repo.id.clone()),
            workspace_id: None,
            message: Some("Test task for scope inheritance".into()),
            name: Some(format!("Scoped Task {suffix}")),
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
    assert!(!task.task_id.is_empty());

    // AUTH-23.1: Task is user-scoped and owned by its creator
    let fetched = alice.client.api().get_task(&task.task_id).await.unwrap();
    assert_eq!(fetched.scope, Scope::User);
    assert_eq!(fetched.owner_user_id.as_deref(), Some(alice.id.as_str()));

    // AUTH-23.2: Owner can see the task in listings
    let alice_tasks = alice
        .client
        .api()
        .list_tasks(&Default::default())
        .await
        .unwrap();
    assert!(alice_tasks.items.iter().any(|t| t.task_id == task.task_id));

    // AUTH-23.3: Another user cannot see the task
    let bob_tasks = bob.client.api().list_tasks(&Default::default()).await.unwrap();
    assert!(!bob_tasks.items.iter().any(|t| t.task_id == task.task_id));

    // Cleanup
    let _ = alice.client.api().delete_task(&task.task_id).await;
    let _ = alice.client.api().delete_repository(&repo.id).await;
}

// =============================================================================
// Chain 3: Admin Bypass (AUTH-24 through AUTH-24.2)
// =============================================================================

#[tokio::test]
async fn admin_bypass() {
    let users = seed_test_users().await;
    let (alice, carol) = (users.alice, users.carol);
    let suffix = timestamp_suffix();

    // User-scoped repo (owned by alice)
    let mut req = create_repo_request(&format!("Alice Private {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::User);
    let user_repo = alice.client.api().create_repository(&req).await.unwrap();

    // AUTH-24: Admin can see all scoped resources
    let carol_repos = carol.client.api().list_repositories().await.unwrap();
    assert!(carol_repos.iter().any(|r| r.id == user_repo.id));

    // AUTH-24.1: Admin can fetch user-scoped repository by ID
    let repo = carol
        .client
        .api()
        .get_repository(&user_repo.id)
        .await
        .unwrap();
    assert_eq!(repo.id, user_repo.id);
    assert!(repo.name.contains("Alice Private"));

    // AUTH-24.2: An admin re-submitting user scope cannot take ownership.
    let preserved = carol
        .client
        .api()
        .update_repository(
            &user_repo.id,
            &UpdateRepositoryRequest {
                scope: Some(Scope::User),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(preserved.owner_user_id, user_repo.owner_user_id);

    // Cleanup
    let _ = alice.client.api().delete_repository(&user_repo.id).await;
}

// =============================================================================
// Standalone: AUTH-25 (Listing repositories filters by scope)
// =============================================================================

#[tokio::test]
async fn listing_repos_filters_by_scope() {
    let users = seed_test_users().await;
    let (alice, bob, carol) = (users.alice, users.bob, users.carol);
    let suffix = timestamp_suffix();

    // Create repos with different scopes
    let mut req = create_repo_request(&format!("Global {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::Global);
    let global_repo = carol.client.api().create_repository(&req).await.unwrap();

    let mut req = create_repo_request(&format!("User {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::User);
    let user_repo = alice.client.api().create_repository(&req).await.unwrap();

    // Bob sees the global repo but not alice's user repo
    let bob_repos = bob.client.api().list_repositories().await.unwrap();
    assert!(bob_repos.iter().any(|r| r.id == global_repo.id));
    assert!(!bob_repos.iter().any(|r| r.id == user_repo.id));

    // Alice sees both her own and the global repo
    let alice_repos = alice.client.api().list_repositories().await.unwrap();
    assert!(alice_repos.iter().any(|r| r.id == global_repo.id));
    assert!(alice_repos.iter().any(|r| r.id == user_repo.id));

    // Cleanup
    let _ = carol.client.api().delete_repository(&global_repo.id).await;
    let _ = alice.client.api().delete_repository(&user_repo.id).await;
}

// =============================================================================
// Standalone: AUTH-26 (Listing tasks filters by scope)
// =============================================================================

#[tokio::test]
async fn listing_tasks_filters_by_scope() {
    let alice = seed_test_users().await.alice;
    let suffix = timestamp_suffix();

    // Create a user-scoped repo + task
    let req = create_repo_request(
        &format!("Task Filter Repo {suffix}"),
        RepositoryKind::Template,
    );
    let repo = alice.client.api().create_repository(&req).await.unwrap();

    let task = alice
        .client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: Some(repo.id.clone()),
            workspace_id: None,
            message: Some("Scoped task test".into()),
            name: Some(format!("Scoped Task {suffix}")),
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

    // Outsider should NOT see the task
    let outsider = client_as_user(&format!("outsider2-{suffix}"), "user").await;
    let outsider_tasks = outsider
        .client
        .api()
        .list_tasks(&Default::default())
        .await
        .unwrap();
    assert!(!outsider_tasks
        .items
        .iter()
        .any(|t| t.task_id == task.task_id));

    // Owner should see the task
    let owner_tasks = alice
        .client
        .api()
        .list_tasks(&Default::default())
        .await
        .unwrap();
    assert!(owner_tasks.items.iter().any(|t| t.task_id == task.task_id));

    // Cleanup
    let _ = alice.client.api().delete_task(&task.task_id).await;
    let _ = alice.client.api().delete_repository(&repo.id).await;
}
