//! [`provision_workspace`] against a real database: registering, reusing,
//! skipping, and serializing.

use std::sync::Arc;

use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_types::access_control::{Actor, Scope, ScopeContext};
use moatless_types::git_host::GitHostAccount;
use sqlx::PgPool;

use super::*;
use crate::fake_github::FakeGithub;
use crate::repository::dao::PgRepositoryDao;
use crate::repository::dao::RepositoryDao;
use crate::test_support::{make_repository_row, make_workspace_repo_row, make_workspace_row};
use crate::workspace::dao::{PgWorkspaceDao, WorkspaceDao};

/// Refusing every token makes the git sync read the fake GitHub
/// unauthenticated.
struct NoGitTokens;

#[async_trait::async_trait]
impl GitHostTokenIssuer for NoGitTokens {
    async fn token(&self, actor: &Actor, _: &GitHostAccount) -> Result<String, GitHostTokenError> {
        Err(GitHostTokenError::NoCredential {
            actor: actor.clone(),
            provider: RepositoryProvider::Github,
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

struct Fixture {
    pool: PgPool,
    workspaces: Arc<Workspaces>,
    repositories: Arc<Repositories>,
    git_sync: Arc<GitSyncService>,
}

impl Fixture {
    /// Reads repository files from `github`, which serves none until a test
    /// puts one.
    fn new(pool: PgPool, github: &FakeGithub) -> Self {
        Self {
            workspaces: crate::compose::workspaces(pool.clone()),
            repositories: crate::compose::repositories(pool.clone()),
            git_sync: crate::compose::git_sync(
                pool.clone(),
                Arc::new(NoGitTokens),
                moatless_core::config::HttpConfig::default().git_content,
                github.base.clone(),
                Default::default(),
            ),
            pool,
        }
    }

    fn stores(&self) -> ProvisionStores<'_> {
        ProvisionStores {
            workspaces: &self.workspaces,
            repositories: &self.repositories,
            git_sync: &self.git_sync,
            pool: &self.pool,
        }
    }
}

fn admin() -> ScopeContext {
    ScopeContext::admin("admin-1")
}

fn request(template: Option<&str>) -> ProvisionRequest {
    ProvisionRequest {
        owner: "soaplabs".to_string(),
        repo: "widgets".to_string(),
        default_branch: Some("trunk".to_string()),
        description: Some("Widgets".to_string()),
        template: template.map(str::to_string),
        preferred_workspace_id: None,
    }
}

/// A global repository already registered for `soaplabs/widgets`.
async fn seed_repository(pool: &PgPool) -> String {
    let mut repo = make_repository_row("repo-known", "admin-1");
    repo.remote = Some(crate::repository::remote::resolve_remote(
        Some(RepositoryProvider::Github),
        "https://github.com/soaplabs/widgets.git".to_string(),
    ));
    repo.scope = Scope::Global;
    repo.owner_user_id = None;
    PgRepositoryDao(pool.clone()).insert(&repo).await.unwrap();
    repo.id
}

async fn workspace_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM workspaces WHERE NOT deleted")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn an_unknown_repository_is_registered_with_its_default_workspace(pool: PgPool) {
    let github = FakeGithub::start().await;
    let fixture = Fixture::new(pool.clone(), &github);

    let outcome = provision_workspace(&fixture.stores(), &admin(), &request(Some("any")))
        .await
        .unwrap();

    let Provisioned::Workspace(workspace_id) = outcome else {
        panic!("expected a workspace, got {outcome:?}");
    };
    let repository = PgRepositoryDao(pool.clone())
        .find_by_normalized_remote_url("https://github.com/soaplabs/widgets", Scope::Global, None)
        .await
        .unwrap()
        .expect("the repository was registered");
    assert_eq!(repository.default_branch.as_deref(), Some("trunk"));
    assert_eq!(repository.description.as_deref(), Some("Widgets"));

    let workspace = PgWorkspaceDao(pool.clone())
        .find_by_id(&workspace_id, &admin())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(workspace.name, "widgets");
    assert_eq!(workspace.scope, Scope::Global);
    assert_eq!(workspace.source_key.as_deref(), Some("default"));
    let primary = PgWorkspaceDao(pool)
        .find_primary_repo(&workspace_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(primary.repository_id, repository.id);
}

/// The repository's own `default` entry overrides the synthesized one.
#[sqlx::test]
async fn a_default_entry_in_git_overrides_the_default_workspace(pool: PgPool) {
    let github = FakeGithub::start().await;
    github.put_file(
        "soaplabs/widgets",
        ".moatless/workspaces.json",
        r#"{"workspaces":[{"key":"default","dockerImage":"img:1","codeExecution":{"setupCommands":["bun install"]}}]}"#,
    );
    let fixture = Fixture::new(pool.clone(), &github);

    let Provisioned::Workspace(workspace_id) =
        provision_workspace(&fixture.stores(), &admin(), &request(Some("any")))
            .await
            .unwrap()
    else {
        panic!("expected a workspace");
    };

    let workspace = PgWorkspaceDao(pool.clone())
        .find_by_id(&workspace_id, &admin())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(workspace.name, "widgets");
    assert_eq!(workspace.docker_image.as_deref(), Some("img:1"));
    assert_eq!(
        workspace.setup_commands,
        Some(vec!["bun install".to_string()])
    );
    assert_eq!(workspace_count(&pool).await, 1);
}

/// A repository registered before every repository had a default Workspace.
#[sqlx::test]
async fn a_known_repository_without_a_workspace_is_synced_once(pool: PgPool) {
    let repository_id = seed_repository(&pool).await;
    let github = FakeGithub::start().await;
    let fixture = Fixture::new(pool.clone(), &github);

    let outcome = provision_workspace(&fixture.stores(), &admin(), &request(None))
        .await
        .unwrap();

    assert!(matches!(outcome, Provisioned::Workspace(_)));
    let repositories: i64 = sqlx::query_scalar("SELECT count(*) FROM repositories")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(repositories, 1);
    let placed = PgWorkspaceDao(pool)
        .find_by_repository(&repository_id)
        .await
        .unwrap();
    assert_eq!(placed.len(), 1);
}

#[sqlx::test]
async fn an_existing_workspace_is_reused(pool: PgPool) {
    let repository_id = seed_repository(&pool).await;
    let dao = PgWorkspaceDao(pool.clone());
    dao.insert_workspace(&make_workspace_row("ws-existing", "admin-1"))
        .await
        .unwrap();
    dao.add_repo(
        &make_workspace_repo_row("wsr-1", "ws-existing", &repository_id, true, 0),
        &admin(),
    )
    .await
    .unwrap();
    let github = FakeGithub::start().await;
    let fixture = Fixture::new(pool, &github);

    // No template: finding needs none.
    let outcome = provision_workspace(&fixture.stores(), &admin(), &request(None))
        .await
        .unwrap();

    assert_eq!(outcome, Provisioned::Workspace("ws-existing".to_string()));
}

/// A loop's named Workspace wins over the repository's other Workspaces.
#[sqlx::test]
async fn the_preferred_workspace_wins_when_it_holds_the_repository(pool: PgPool) {
    let repository_id = seed_repository(&pool).await;
    let dao = PgWorkspaceDao(pool.clone());
    for (n, id) in ["ws-first", "ws-loop"].into_iter().enumerate() {
        dao.insert_workspace(&make_workspace_row(id, "admin-1"))
            .await
            .unwrap();
        dao.add_repo(
            &make_workspace_repo_row(&format!("wsr-{n}"), id, &repository_id, true, 0),
            &admin(),
        )
        .await
        .unwrap();
    }
    let github = FakeGithub::start().await;
    let fixture = Fixture::new(pool, &github);

    for (preferred, expected) in [
        ("ws-loop", "ws-loop"),
        ("ws-first", "ws-first"),
        ("ws-elsewhere", "ws-first"),
    ] {
        let request = ProvisionRequest {
            preferred_workspace_id: Some(preferred.to_string()),
            ..request(None)
        };
        let outcome = provision_workspace(&fixture.stores(), &admin(), &request)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            Provisioned::Workspace(expected.to_string()),
            "{preferred}"
        );
    }
}

#[sqlx::test]
async fn a_soft_deleted_repository_is_skipped(pool: PgPool) {
    let repository_id = seed_repository(&pool).await;
    PgRepositoryDao(pool.clone())
        .soft_delete(&repository_id, &admin())
        .await
        .unwrap();
    let github = FakeGithub::start().await;
    let fixture = Fixture::new(pool.clone(), &github);

    let outcome = provision_workspace(&fixture.stores(), &admin(), &request(Some("any")))
        .await
        .unwrap();

    assert_eq!(outcome, Provisioned::RepositoryDeleted);
    assert_eq!(workspace_count(&pool).await, 0);
}

#[sqlx::test]
async fn no_template_creates_nothing(pool: PgPool) {
    let github = FakeGithub::start().await;
    let fixture = Fixture::new(pool.clone(), &github);

    let outcome = provision_workspace(&fixture.stores(), &admin(), &request(None))
        .await
        .unwrap();

    assert_eq!(outcome, Provisioned::NoTemplate);
    let repositories: i64 = sqlx::query_scalar("SELECT count(*) FROM repositories")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(repositories, 0);
}

#[sqlx::test]
async fn concurrent_requests_create_one_workspace(pool: PgPool) {
    let github = FakeGithub::start().await;
    let fixture = Fixture::new(pool.clone(), &github);
    let stores = fixture.stores();
    let ctx = admin();
    let req = request(Some("any"));

    let (first, second) = tokio::join!(
        provision_workspace(&stores, &ctx, &req),
        provision_workspace(&stores, &ctx, &req),
    );

    assert_eq!(first.unwrap(), second.unwrap());
    assert_eq!(workspace_count(&pool).await, 1);
}
