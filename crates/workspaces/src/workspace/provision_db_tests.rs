//! [`provision_workspace`] against a real database: registering, reusing,
//! skipping, and serializing.

use std::sync::Arc;

use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer, MockUserResolver};
use moatless_types::access_control::{Actor, Scope, ScopeContext};
use moatless_types::git_host::GitHostAccount;
use sqlx::PgPool;

use super::*;
use crate::repository::dao::PgRepositoryDao;
use crate::repository::dao::RepositoryDao;
use crate::test_support::{make_repository_row, make_workspace_repo_row, make_workspace_row};
use crate::workspace::dao::{PgWorkspaceDao, WorkspaceDao};

/// The git sync `create_repository` runs reads `.moatless/workspaces.json`
/// through this; refusing leaves the sync a logged no-op.
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
    users: MockUserResolver,
}

impl Fixture {
    fn new(pool: PgPool) -> Self {
        Self {
            workspaces: crate::compose::workspaces(pool.clone()),
            repositories: crate::compose::repositories(pool.clone()),
            git_sync: crate::compose::git_sync(
                pool.clone(),
                Arc::new(NoGitTokens),
                moatless_core::config::HttpConfig::default().git_content,
                "http://127.0.0.1:1".to_string(),
            ),
            users: MockUserResolver::new(),
            pool,
        }
    }

    fn stores(&self) -> ProvisionStores<'_> {
        ProvisionStores {
            workspaces: &self.workspaces,
            repositories: &self.repositories,
            users: &self.users,
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

/// A global template Workspace carrying the settings a new one copies.
async fn seed_template(pool: &PgPool) {
    let mut template = make_workspace_row("ws-template", "admin-1");
    template.name = "review-template".to_string();
    template.scope = Scope::Global;
    template.owner_user_id = None;
    template.docker_image = Some("aorwall/moatless-sandbox-recipe:soaplabs".to_string());
    template.setup_commands = Some(vec!["bun install".to_string()]);
    template.default_model = Some("opus".to_string());
    template.system_prompt = Some("not copied".to_string());
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&template)
        .await
        .unwrap();
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
    sqlx::query_scalar("SELECT count(*) FROM workspaces WHERE name = 'widgets'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn an_unknown_repository_is_registered_with_a_workspace_from_the_template(pool: PgPool) {
    seed_template(&pool).await;
    let fixture = Fixture::new(pool.clone());

    let outcome = provision_workspace(
        &fixture.stores(),
        &admin(),
        &request(Some("review-template")),
    )
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
    assert_eq!(
        workspace.docker_image.as_deref(),
        Some("aorwall/moatless-sandbox-recipe:soaplabs")
    );
    assert_eq!(
        workspace.setup_commands,
        Some(vec!["bun install".to_string()])
    );
    assert_eq!(workspace.default_model.as_deref(), Some("opus"));
    assert_eq!(workspace.system_prompt, None);
    assert_eq!(workspace.run_as_user_id, None);
    let primary = PgWorkspaceDao(pool)
        .find_primary_repo(&workspace_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(primary.repository_id, repository.id);
}

#[sqlx::test]
async fn a_known_repository_gets_only_a_workspace(pool: PgPool) {
    seed_template(&pool).await;
    let repository_id = seed_repository(&pool).await;
    let fixture = Fixture::new(pool.clone());

    let outcome = provision_workspace(
        &fixture.stores(),
        &admin(),
        &request(Some("review-template")),
    )
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
    let fixture = Fixture::new(pool);

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
    let fixture = Fixture::new(pool);

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
    seed_template(&pool).await;
    let repository_id = seed_repository(&pool).await;
    PgRepositoryDao(pool.clone())
        .soft_delete(&repository_id, &admin())
        .await
        .unwrap();
    let fixture = Fixture::new(pool.clone());

    let outcome = provision_workspace(
        &fixture.stores(),
        &admin(),
        &request(Some("review-template")),
    )
    .await
    .unwrap();

    assert_eq!(outcome, Provisioned::RepositoryDeleted);
    assert_eq!(workspace_count(&pool).await, 0);
}

#[sqlx::test]
async fn no_template_creates_nothing(pool: PgPool) {
    let fixture = Fixture::new(pool.clone());

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
async fn a_missing_template_is_a_named_error(pool: PgPool) {
    seed_repository(&pool).await;
    let fixture = Fixture::new(pool);

    let error = provision_workspace(&fixture.stores(), &admin(), &request(Some("gone")))
        .await
        .unwrap_err();

    assert!(matches!(&error, ProvisionError::TemplateMissing(name) if name == "gone"));
    assert_eq!(
        error.to_string(),
        "workspace template `gone` does not exist"
    );
}

#[sqlx::test]
async fn concurrent_requests_create_one_workspace(pool: PgPool) {
    seed_template(&pool).await;
    let fixture = Fixture::new(pool.clone());
    let stores = fixture.stores();
    let ctx = admin();
    let req = request(Some("review-template"));

    let (first, second) = tokio::join!(
        provision_workspace(&stores, &ctx, &req),
        provision_workspace(&stores, &ctx, &req),
    );

    assert_eq!(first.unwrap(), second.unwrap());
    assert_eq!(workspace_count(&pool).await, 1);
}
