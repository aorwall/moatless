//! [`OrgSync::sync_org`] against a real database and a fake GitHub.

use std::sync::Arc;

use moatless_core::config::WorkspaceSyncConfig;
use moatless_core::workspace_identity_grants::{WorkspaceIdentityGrant, WorkspaceIdentityGrants};
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_types::access_control::{Actor, Scope, ScopeContext};
use moatless_types::git_host::GitHostAccount;
use serde_json::json;
use sqlx::PgPool;

use super::*;
use crate::fake_github::FakeGithub;
use crate::repository::dao::{PgRepositoryDao, RepositoryDao};
use crate::test_support::make_repository_row;
use crate::workspace::dao::{PgWorkspaceDao, WorkspaceDao};

struct FixedToken;

#[async_trait::async_trait]
impl GitHostTokenIssuer for FixedToken {
    async fn token(&self, _: &Actor, _: &GitHostAccount) -> Result<String, GitHostTokenError> {
        Ok("t".to_string())
    }
    async fn github_is_usable(&self, _: &Actor) -> Result<(), GitHostTokenError> {
        Ok(())
    }
}

fn listed(name: &str, archived: bool) -> serde_json::Value {
    owned("acme", name, archived, false)
}

fn owned(owner: &str, name: &str, archived: bool, private: bool) -> serde_json::Value {
    json!({
        "name": name,
        "html_url": format!("https://github.com/{owner}/{name}"),
        "archived": archived,
        "disabled": false,
        "private": private,
        "owner": { "login": owner },
        "default_branch": "main",
        "description": null,
    })
}

fn org_sync(pool: &PgPool, github: &FakeGithub, grants: WorkspaceIdentityGrants) -> OrgSync {
    let git_sync = crate::compose::git_sync(
        pool.clone(),
        Arc::new(FixedToken),
        moatless_core::config::HttpConfig::default().git_content,
        github.base.clone(),
        grants,
    );
    crate::compose::org_sync(
        pool.clone(),
        crate::compose::repositories(pool.clone()),
        git_sync,
        Arc::new(FixedToken),
        moatless_core::config::HttpConfig::default().git_content,
        github.base.clone(),
        &WorkspaceSyncConfig {
            github_orgs: vec!["acme".to_string()],
            github_org_sync_user_id: Some("admin-1".to_string()),
            identity_grants: WorkspaceIdentityGrants::default(),
        },
    )
    .expect("an organization is configured")
}

fn admin() -> ScopeContext {
    ScopeContext::admin("admin-1")
}

async fn seed_global(pool: &PgPool, id: &str, url: &str) {
    let mut repo = make_repository_row(id, "admin-1");
    repo.remote = Some(super::super::remote::resolve_remote(
        Some(RepositoryProvider::Github),
        url.to_string(),
    ));
    repo.provider = Some(RepositoryProvider::Github);
    repo.scope = Scope::Global;
    repo.owner_user_id = None;
    PgRepositoryDao(pool.clone()).insert(&repo).await.unwrap();
}

#[sqlx::test]
async fn a_listing_registers_new_repositories_and_removes_departed_ones(pool: PgPool) {
    let github = FakeGithub::start().await;
    github.set_org("acme", vec![listed("api", false), listed("old", true)]);
    github.set_repo("acme/gone", owned("other", "gone", false, false));
    seed_global(&pool, "repo-gone", "https://github.com/acme/gone").await;
    seed_global(&pool, "repo-old", "https://github.com/acme/old").await;
    let sync = org_sync(&pool, &github, WorkspaceIdentityGrants::default());

    let summary = sync.sync_org("acme").await.unwrap();

    assert_eq!((summary.registered, summary.removed), (1, 2), "{summary:?}");
    assert!(summary.failed.is_empty(), "{summary:?}");
    let repos = PgRepositoryDao(pool.clone());
    let api = repos
        .find_by_normalized_remote_url("https://github.com/acme/api", Scope::Global, None)
        .await
        .unwrap()
        .expect("api was registered");
    assert_eq!(api.created_by.as_deref(), Some("admin-1"));
    let workspaces = PgWorkspaceDao(pool.clone())
        .find_by_repository(&api.id)
        .await
        .unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].source_key.as_deref(), Some("default"));
    for gone in ["repo-gone", "repo-old"] {
        assert!(
            repos.find_by_id(gone, &admin()).await.unwrap().is_none(),
            "{gone}"
        );
    }

    let again = sync.sync_org("acme").await.unwrap();
    assert_eq!(again, OrgSyncSummary::default());
}

/// A listing narrowed by SSO, an App's repository selection or lost team
/// access answers 200 without the repositories it hides.
#[sqlx::test]
async fn a_repository_a_narrowed_listing_omits_is_kept(pool: PgPool) {
    let github = FakeGithub::start().await;
    // The listing shows a private repository, yet the token cannot see
    // `hidden`, so its 404 proves nothing.
    github.set_org("acme", vec![owned("acme", "api", false, true)]);
    github.set_repo("acme/live", owned("acme", "live", false, true));
    seed_global(&pool, "repo-live", "https://github.com/acme/live").await;
    seed_global(&pool, "repo-hidden", "https://github.com/acme/hidden").await;
    seed_global(&pool, "repo-api", "https://github.com/acme/api").await;

    let summary = org_sync(&pool, &github, WorkspaceIdentityGrants::default())
        .sync_org("acme")
        .await
        .unwrap();

    assert_eq!(summary.removed, 0, "{summary:?}");
    let mut kept: Vec<&str> = summary.kept.iter().map(|(id, _)| id.as_str()).collect();
    kept.sort_unstable();
    assert_eq!(kept, ["repo-hidden", "repo-live"]);
}

/// `/users/{account}/repos` lists public repositories only, so a private one
/// is read through `/user/repos` when the token is the account's.
#[sqlx::test]
async fn a_user_account_lists_its_private_repositories_through_its_own_token(pool: PgPool) {
    let github = FakeGithub::start().await;
    github.set_user(
        "acme",
        vec![
            owned("acme", "secret", false, true),
            owned("other", "shared", false, true),
        ],
    );
    github.set_repo("acme/old", owned("acme", "old", true, true));
    seed_global(&pool, "repo-old", "https://github.com/acme/old").await;

    let summary = org_sync(&pool, &github, WorkspaceIdentityGrants::default())
        .sync_org("acme")
        .await
        .unwrap();

    assert_eq!((summary.registered, summary.removed), (1, 1), "{summary:?}");
    let repos = PgRepositoryDao(pool);
    assert!(
        repos
            .find_by_normalized_remote_url("https://github.com/other/shared", Scope::Global, None)
            .await
            .unwrap()
            .is_none(),
        "another owner's repository is not the account's"
    );
}

#[sqlx::test]
async fn another_users_public_listing_removes_nothing(pool: PgPool) {
    let github = FakeGithub::start().await;
    github.set_user("someone-else", Vec::new());
    github.set_account("acme", vec![listed("api", false)]);
    seed_global(&pool, "repo-private", "https://github.com/acme/private").await;

    let summary = org_sync(&pool, &github, WorkspaceIdentityGrants::default())
        .sync_org("acme")
        .await
        .unwrap();

    assert_eq!((summary.registered, summary.removed), (1, 0), "{summary:?}");
    assert_eq!(summary.kept.len(), 1, "{summary:?}");
}

/// A removed repository's soft-deleted row keeps it removed.
#[sqlx::test]
async fn a_soft_deleted_repository_is_not_registered_again(pool: PgPool) {
    let github = FakeGithub::start().await;
    github.set_org("acme", vec![listed("api", false)]);
    seed_global(&pool, "repo-api", "https://github.com/acme/api").await;
    PgRepositoryDao(pool.clone())
        .soft_delete("repo-api", &admin())
        .await
        .unwrap();

    let summary = org_sync(&pool, &github, WorkspaceIdentityGrants::default())
        .sync_org("acme")
        .await
        .unwrap();

    assert_eq!(summary, OrgSyncSummary::default());
}

#[sqlx::test]
async fn a_granted_identity_in_git_reaches_the_registered_workspace(pool: PgPool) {
    let github = FakeGithub::start().await;
    github.set_org("acme", vec![listed("ops", false)]);
    github.put_file(
        "acme/ops",
        ".moatless/workspaces.json",
        r#"{"workspaces":[{"key":"default","sandboxServiceAccount":"cluster-reader",
            "networkProfile":"cluster-reader"}]}"#,
    );
    let grants = WorkspaceIdentityGrants::new(vec![WorkspaceIdentityGrant {
        repository: "acme/ops".to_string(),
        workspace: "default".to_string(),
        sandbox_service_account: Some("cluster-reader".to_string()),
        owner_user_id: Some("admin-1".to_string()),
        ..Default::default()
    }]);

    org_sync(&pool, &github, grants)
        .sync_org("acme")
        .await
        .unwrap();

    let ops = PgRepositoryDao(pool.clone())
        .find_by_normalized_remote_url("https://github.com/acme/ops", Scope::Global, None)
        .await
        .unwrap()
        .unwrap();
    let workspace = PgWorkspaceDao(pool)
        .find_by_repository(&ops.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        workspace.sandbox_service_account.as_deref(),
        Some("cluster-reader")
    );
    // Not granted, so dropped.
    assert_eq!(workspace.network_profile, None);
    assert_eq!(workspace.scope, Scope::User);
    assert_eq!(workspace.owner_user_id.as_deref(), Some("admin-1"));
}
