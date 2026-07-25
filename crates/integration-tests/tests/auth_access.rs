//! Auth access control integration tests.
//!
//! Mirrors: tests/api/chains/auth-access-control.spec.ts
//! Spec IDs: AUTH-20 through AUTH-26

use futures_util::future::join_all;
use integration_tests::{
    admin_client, base_url, client_as_user, create_repo_request, raw_client_as_user, relogin,
    seed_test_users,
};
use moatless_api_client::ApiError;
use moatless_api_schemas::auth::CreateUserRequest;
use moatless_api_schemas::enums::{RepositoryKind, Scope, UserRole};
use moatless_api_schemas::repository::UpdateRepositoryRequest;
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_api_schemas::team::{AddTeamMemberRequest, CreateTeamRequest};
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

// AUTH-27
#[tokio::test]
async fn github_installation_settings_are_admin_only_and_reject_team_pat_writes() {
    let suffix = timestamp_suffix();
    let (admin, admin_http) =
        raw_client_as_user(&format!("installation-admin-{suffix}"), "admin").await;
    let (_, user_http) = raw_client_as_user(&format!("installation-user-{suffix}"), "user").await;
    let team = admin
        .client
        .api()
        .create_team(&CreateTeamRequest {
            name: format!("Installation Team {suffix}"),
            slug: format!("installation-team-{suffix}"),
            description: None,
        })
        .await
        .unwrap();

    let apps_url = format!("{}/api/v1/admin/settings/github/apps", base_url());
    assert_eq!(user_http.get(&apps_url).send().await.unwrap().status(), 403);
    assert_eq!(
        admin_http.get(&apps_url).send().await.unwrap().status(),
        200
    );

    let team_installation_url = format!(
        "{}/api/v1/teams/{}/settings/github/installation",
        base_url(),
        team.id
    );
    assert_eq!(
        user_http
            .get(&team_installation_url)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        admin_http
            .get(&team_installation_url)
            .send()
            .await
            .unwrap()
            .status(),
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

    let legacy_pat = admin_http
        .put(format!(
            "{}/api/v1/teams/{}/credentials/github",
            base_url(),
            team.id
        ))
        .json(&serde_json::json!({ "accessToken": "ghp_legacy" }))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy_pat.status(), 400);
}

// =============================================================================
// Chain 1: Repository Scope Filtering (AUTH-20 through AUTH-22.2)
// =============================================================================

#[tokio::test]
async fn repository_scope_filtering() {
    let users = seed_test_users().await;
    let (alice, bob, carol) = (users.alice, users.bob, users.carol);
    let suffix = timestamp_suffix();

    // Setup: Alice creates a team and adds Bob
    let team = alice
        .client
        .api()
        .create_team(&CreateTeamRequest {
            name: "Scope Team".into(),
            slug: format!("scope-{suffix}"),
            description: None,
        })
        .await
        .unwrap();
    let team_id = team.id.clone();

    alice
        .client
        .api()
        .add_member(
            &team_id,
            &AddTeamMemberRequest {
                user_id: bob.id.clone(),
                role: Some("member".into()),
            },
        )
        .await
        .unwrap();

    // Team ids are baked into the session JWT at login and never re-resolved
    // per request, so a client minted before `add_member` carries no teams.
    let alice_with_team = relogin(&alice).await;

    let mut repo_ids = Vec::new();

    // AUTH-20: Create global-scoped repository
    let mut req = create_repo_request(&format!("Global Repo {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::Global);
    let global_repo = carol.client.api().create_repository(&req).await.unwrap();
    assert_eq!(global_repo.scope, Scope::Global);
    repo_ids.push(global_repo.id.clone());

    // AUTH-20.1: Other user can see global repository
    let bob_with_team = relogin(&bob).await;
    let bob_repos = bob_with_team
        .client
        .api()
        .list_repositories()
        .await
        .unwrap();
    assert!(bob_repos.iter().any(|r| r.id == global_repo.id));

    // AUTH-21: Create team-scoped repository
    let mut req = create_repo_request(&format!("Team Repo {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::Team);
    req.team_id = Some(team_id.clone());
    let team_repo = alice_with_team
        .client
        .api()
        .create_repository(&req)
        .await
        .unwrap();
    assert_eq!(team_repo.scope, Scope::Team);
    assert_eq!(team_repo.team_id.as_deref(), Some(team_id.as_str()));
    repo_ids.push(team_repo.id.clone());

    // AUTH-21.1: Team member can see team repository
    let bob_repos = bob_with_team
        .client
        .api()
        .list_repositories()
        .await
        .unwrap();
    assert!(bob_repos.iter().any(|r| r.id == team_repo.id));

    // AUTH-21.3: A non-managing team member cannot manually verify access.
    let err = bob_with_team
        .client
        .api()
        .verify_repository_access(&team_repo.id)
        .await
        .expect_err("team member must not mutate repository access status");
    assert!(matches!(
        err,
        moatless_api_client::ApiError::Api { status: 403, .. }
    ));

    // AUTH-21.2: Non-member cannot see team repository
    let outsider = client_as_user(&format!("outsider-{suffix}"), "user").await;
    let outsider_repos = outsider.client.api().list_repositories().await.unwrap();
    assert!(!outsider_repos.iter().any(|r| r.id == team_repo.id));

    // AUTH-22: Create user-scoped repository
    let mut req = create_repo_request(&format!("User Repo {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::User);
    req.team_id = Some(team_id.clone());
    let user_repo = alice.client.api().create_repository(&req).await.unwrap();
    assert_eq!(user_repo.scope, Scope::User);
    repo_ids.push(user_repo.id.clone());

    // AUTH-22.1: Creator can see user repository
    let alice_repos = alice.client.api().list_repositories().await.unwrap();
    assert!(alice_repos.iter().any(|r| r.id == user_repo.id));

    // AUTH-22.2: Other user cannot see user repository
    let bob_repos = bob.client.api().list_repositories().await.unwrap();
    assert!(!bob_repos.iter().any(|r| r.id == user_repo.id));

    // Cleanup
    let _ = carol.client.api().delete_repository(&global_repo.id).await;
    for id in repo_ids.iter().filter(|id| *id != &global_repo.id) {
        let _ = alice.client.api().delete_repository(id).await;
    }
    let _ = alice.client.api().delete_team(&team_id).await;
}

// =============================================================================
// Chain 2: Task Scope Inheritance (AUTH-23 through AUTH-23.3)
// =============================================================================

#[tokio::test]
async fn task_scope_inheritance() {
    let users = seed_test_users().await;
    let (alice, bob) = (users.alice, users.bob);
    let suffix = timestamp_suffix();

    // Setup: Alice creates a team, adds Bob, and creates a team-scoped repo
    let team = alice
        .client
        .api()
        .create_team(&CreateTeamRequest {
            name: "Task Scope Team".into(),
            slug: format!("task-scope-{suffix}"),
            description: None,
        })
        .await
        .unwrap();
    let team_id = team.id.clone();

    alice
        .client
        .api()
        .add_member(
            &team_id,
            &AddTeamMemberRequest {
                user_id: bob.id.clone(),
                role: Some("member".into()),
            },
        )
        .await
        .unwrap();

    // Team ids are baked into the session JWT at login and never re-resolved
    // per request, so a client minted before `add_member` carries no teams.
    let alice_with_team = relogin(&alice).await;

    let mut req = create_repo_request(
        &format!("Task Scope Repo {suffix}"),
        RepositoryKind::Template,
    );
    req.scope = Some(Scope::Team);
    req.team_id = Some(team_id.clone());
    let team_repo = alice_with_team
        .client
        .api()
        .create_repository(&req)
        .await
        .unwrap();

    // AUTH-23: Create task under team-scoped repository
    let task = alice_with_team
        .client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: Some(team_repo.id.clone()),
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

    // AUTH-23.1: Task inherits team scope
    let fetched = alice_with_team
        .client
        .api()
        .get_task(&task.task_id)
        .await
        .unwrap();
    assert_eq!(fetched.scope, Scope::Team);
    assert_eq!(fetched.team_id.as_deref(), Some(team_id.as_str()));

    // AUTH-23.2: Team member can see the task
    let bob_with_team = relogin(&bob).await;
    let bob_tasks = bob_with_team
        .client
        .api()
        .list_tasks(&Default::default())
        .await
        .unwrap();
    assert!(bob_tasks.items.iter().any(|t| t.task_id == task.task_id));

    // AUTH-23.3: Non-member cannot see the task
    let outsider = client_as_user(&format!("outsider-{suffix}"), "user").await;
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

    // Cleanup
    let _ = alice.client.api().delete_task(&task.task_id).await;
    let _ = alice.client.api().delete_repository(&team_repo.id).await;
    let _ = alice.client.api().delete_team(&team_id).await;
}

// =============================================================================
// Chain 3: Admin Bypass (AUTH-24 through AUTH-24.1)
// =============================================================================

#[tokio::test]
async fn admin_bypass() {
    let users = seed_test_users().await;
    let (alice, carol) = (users.alice, users.carol);
    let suffix = timestamp_suffix();

    // Create a team (carol is NOT a member)
    let team = alice
        .client
        .api()
        .create_team(&CreateTeamRequest {
            name: "Admin Bypass Team".into(),
            slug: format!("admin-bypass-{suffix}"),
            description: None,
        })
        .await
        .unwrap();
    let team_id = team.id.clone();

    // Team ids are baked into the session JWT at login and never re-resolved
    // per request, so a client minted before `add_member` carries no teams.
    let alice_with_team = relogin(&alice).await;

    // User-scoped repo (owned by alice)
    let mut req = create_repo_request(&format!("Alice Private {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::User);
    req.team_id = Some(team_id.clone());
    let user_repo = alice.client.api().create_repository(&req).await.unwrap();

    // Team-scoped repo
    let mut req = create_repo_request(&format!("Team Only {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::Team);
    req.team_id = Some(team_id.clone());
    let team_repo = alice_with_team
        .client
        .api()
        .create_repository(&req)
        .await
        .unwrap();

    // AUTH-24: Admin can see all scoped resources
    let carol_repos = carol.client.api().list_repositories().await.unwrap();
    assert!(carol_repos.iter().any(|r| r.id == user_repo.id));
    assert!(carol_repos.iter().any(|r| r.id == team_repo.id));

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
                team_id: Some("system".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(preserved.owner_user_id, user_repo.owner_user_id);
    assert_eq!(preserved.team_id, user_repo.team_id);

    // AUTH-24.3: An ownerless team repository cannot move to user scope.
    let err = carol
        .client
        .api()
        .update_repository(
            &team_repo.id,
            &UpdateRepositoryRequest {
                scope: Some(Scope::User),
                ..Default::default()
            },
        )
        .await
        .expect_err("ownerless repository must require a future explicit owner picker");
    assert!(matches!(
        err,
        moatless_api_client::ApiError::Api { status: 400, .. }
    ));

    // Cleanup
    let _ = alice.client.api().delete_repository(&user_repo.id).await;
    let _ = alice.client.api().delete_repository(&team_repo.id).await;
    let _ = alice.client.api().delete_team(&team_id).await;
}

// =============================================================================
// Standalone: AUTH-25 (Listing repositories filters by scope)
// =============================================================================

#[tokio::test]
async fn listing_repos_filters_by_scope() {
    let users = seed_test_users().await;
    let (alice, bob, carol) = (users.alice, users.bob, users.carol);
    let suffix = timestamp_suffix();

    // Create a team
    let team = alice
        .client
        .api()
        .create_team(&CreateTeamRequest {
            name: "Filter Team".into(),
            slug: format!("filter-{suffix}"),
            description: None,
        })
        .await
        .unwrap();

    // Alice owns the team as its creator; re-login so the membership lands in
    // her session JWT.
    let alice_with_team = relogin(&alice).await;

    // Create repos with different scopes
    let mut req = create_repo_request(&format!("Global {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::Global);
    let global_repo = carol.client.api().create_repository(&req).await.unwrap();

    let mut req = create_repo_request(&format!("Team {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::Team);
    req.team_id = Some(team.id.clone());
    let team_repo = alice_with_team
        .client
        .api()
        .create_repository(&req)
        .await
        .unwrap();

    let mut req = create_repo_request(&format!("User {suffix}"), RepositoryKind::Template);
    req.scope = Some(Scope::User);
    req.team_id = Some(team.id.clone());
    let user_repo = alice.client.api().create_repository(&req).await.unwrap();

    // Bob is NOT a team member — should see global, not team or user repos
    let bob_repos = bob.client.api().list_repositories().await.unwrap();
    assert!(bob_repos.iter().any(|r| r.id == global_repo.id));
    assert!(!bob_repos.iter().any(|r| r.id == team_repo.id));
    assert!(!bob_repos.iter().any(|r| r.id == user_repo.id));

    // Bob WITH team membership — should also see team repo
    alice
        .client
        .api()
        .add_member(
            &team.id,
            &AddTeamMemberRequest {
                user_id: bob.id.clone(),
                role: Some("member".into()),
            },
        )
        .await
        .unwrap();
    let bob_with_team = relogin(&bob).await;
    let bob_team_repos = bob_with_team
        .client
        .api()
        .list_repositories()
        .await
        .unwrap();
    assert!(bob_team_repos.iter().any(|r| r.id == global_repo.id));
    assert!(bob_team_repos.iter().any(|r| r.id == team_repo.id));
    assert!(!bob_team_repos.iter().any(|r| r.id == user_repo.id));

    // Cleanup
    let _ = carol.client.api().delete_repository(&global_repo.id).await;
    let _ = alice.client.api().delete_repository(&team_repo.id).await;
    let _ = alice.client.api().delete_repository(&user_repo.id).await;
    let _ = alice.client.api().delete_team(&team.id).await;
}

// =============================================================================
// Standalone: AUTH-26 (Listing tasks filters by scope)
// =============================================================================

#[tokio::test]
async fn listing_tasks_filters_by_scope() {
    let alice = seed_test_users().await.alice;
    let suffix = timestamp_suffix();

    // Create a team
    let team = alice
        .client
        .api()
        .create_team(&CreateTeamRequest {
            name: "Task Filter Team".into(),
            slug: format!("task-filter-{suffix}"),
            description: None,
        })
        .await
        .unwrap();

    // Alice owns the team as its creator; re-login so the membership lands in
    // her session JWT.
    let alice_with_team = relogin(&alice).await;

    // Create a team-scoped repo + task
    let mut req = create_repo_request(
        &format!("Task Filter Repo {suffix}"),
        RepositoryKind::Template,
    );
    req.scope = Some(Scope::Team);
    req.team_id = Some(team.id.clone());
    let team_repo = alice_with_team
        .client
        .api()
        .create_repository(&req)
        .await
        .unwrap();

    let task = alice_with_team
        .client
        .api()
        .create_task(&CreateTaskRequest {
            repository_id: Some(team_repo.id.clone()),
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

    // Team member should see the task
    let member_tasks = alice_with_team
        .client
        .api()
        .list_tasks(&Default::default())
        .await
        .unwrap();
    assert!(member_tasks.items.iter().any(|t| t.task_id == task.task_id));

    // Cleanup
    let _ = alice.client.api().delete_task(&task.task_id).await;
    let _ = alice.client.api().delete_repository(&team_repo.id).await;
    let _ = alice.client.api().delete_team(&team.id).await;
}
