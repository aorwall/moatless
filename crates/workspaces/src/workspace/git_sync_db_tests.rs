//! PostgreSQL tests for the git-declared workspace sync.
//!
//! Reconcile-delete only runs against a desired set git actually declared.

use std::sync::Arc;

use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_types::access_control::{Actor, ScopeContext};
use moatless_workspace_types::repository::RepositoryProvider;
use moatless_workspace_types::workspace::WorkspaceSource;
use sqlx::PgPool;

use crate::test_support::{make_repository_row, make_workspace_row};
use crate::workspace::dao::{PgWorkspaceDao, WorkspaceDao};
use crate::workspace::git_sync::SyncForce;

const USER: &str = "user-1";

fn scope_ctx(user_id: &str) -> ScopeContext {
    ScopeContext::user(user_id)
}

/// An issuer holding no credential, which is what `resolve_sync_token` sees for
/// a host the deployment never configured: it probes unauthenticated.
struct NoGitTokens;

#[async_trait::async_trait]
impl GitHostTokenIssuer for NoGitTokens {
    async fn token(
        &self,
        actor: &Actor,
        repo: &moatless_types::git_host::GitHostAccount,
    ) -> Result<String, GitHostTokenError> {
        Err(GitHostTokenError::NoCredential {
            actor: actor.clone(),
            provider: repo.provider(),
            host: None,
        })
    }

    async fn github_is_usable(&self, actor: &Actor) -> Result<(), GitHostTokenError> {
        Err(GitHostTokenError::NoCredential {
            actor: actor.clone(),
            provider: RepositoryProvider::Github,
            host: None,
        })
    }
}

#[sqlx::test]
async fn a_repository_shipping_no_workspaces_json_removes_nothing(pool: PgPool) {
    // Gitness resolves no config file, so the sync reaches the "no file" return
    // without a provider call.
    let mut repo = make_repository_row("repo-1", USER);
    repo.provider = Some(RepositoryProvider::Gitness);
    repo.remote = Some(crate::repository::remote::resolve_remote(
        repo.provider,
        "https://gitness.example/acme/repo.git".to_string(),
    ));
    crate::compose::seed_repository(&pool, &repo)
        .await
        .expect("seed repo");

    let workspaces = PgWorkspaceDao(pool.clone());
    let mut declared = make_workspace_row("ws-git", USER);
    declared.source = WorkspaceSource::Git;
    declared.source_repository_id = Some("repo-1".to_string());
    declared.source_key = Some("ws-git".to_string());
    workspaces.insert_workspace(&declared).await.unwrap();

    let git_sync = crate::compose::git_sync(
        pool.clone(),
        Arc::new(NoGitTokens),
        moatless_core::config::HttpConfig::default().git_content,
    );
    let summary = git_sync
        .sync_repo_workspaces_for(&repo, &scope_ctx(USER), &SyncForce::RespectOverrides)
        .await
        .expect("sync succeeds");

    assert_eq!(summary.removed, 0);
    let survivor = workspaces
        .find_by_id("ws-git", &scope_ctx(USER))
        .await
        .unwrap()
        .expect("workspace still present");
    assert!(
        !survivor.deleted,
        "an undeclared desired set tombstoned the repository's workspace"
    );
}
