use sqlx::AssertSqlSafe;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use super::dao::replace_git_placements;
use super::models::WorkspaceRepo;

async fn database() -> (sqlx::PgPool, sqlx::PgPool, String) {
    let url = std::env::var("MOATLESS_TEST_DATABASE_URL")
        .expect("MOATLESS_TEST_DATABASE_URL must name a disposable PostgreSQL database");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let schema = format!("placements_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE workspaces (id text PRIMARY KEY, kind text NOT NULL, deleted boolean DEFAULT false, updated_at timestamptz); \
         CREATE TABLE repositories (id text PRIMARY KEY); \
         CREATE TABLE workspace_repos (id text PRIMARY KEY, workspace_id text NOT NULL REFERENCES workspaces(id), repository_id text NOT NULL REFERENCES repositories(id), branch text, mount_name text, is_primary boolean NOT NULL, position integer NOT NULL, UNIQUE(workspace_id, mount_name)); \
         CREATE UNIQUE INDEX workspace_repos_one_primary ON workspace_repos(workspace_id) WHERE is_primary; \
         INSERT INTO workspaces(id, kind) VALUES ('ws', 'blank'); \
         INSERT INTO repositories(id) VALUES ('repo');"
    ).execute(&pool).await.unwrap();
    (pool, admin, schema)
}

async fn cleanup(pool: sqlx::PgPool, admin: sqlx::PgPool, schema: String) {
    pool.close().await;
    sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

fn placement(id: &str, position: i32) -> WorkspaceRepo {
    WorkspaceRepo {
        id: id.to_string(),
        workspace_id: "ws".to_string(),
        repository_id: "repo".to_string(),
        branch: None,
        mount_name: Some(id.to_string()),
        is_primary: false,
        position,
        created_at: None,
    }
}

async fn state(pool: &sqlx::PgPool) -> (Vec<(String, bool)>, String) {
    let rows = sqlx::query_as("SELECT id, is_primary FROM workspace_repos ORDER BY position")
        .fetch_all(pool)
        .await
        .unwrap();
    let kind = sqlx::query_scalar("SELECT kind FROM workspaces WHERE id = 'ws'")
        .fetch_one(pool)
        .await
        .unwrap();
    (rows, kind)
}

#[tokio::test]
#[ignore = "requires MOATLESS_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn failed_replacement_preserves_previous_placements_and_primary() {
    let (pool, admin, schema) = database().await;
    replace_git_placements(&pool, "ws", &[placement("old", 0)], Some("old"))
        .await
        .unwrap();
    let mut missing_repository = placement("rejected", 1);
    missing_repository.repository_id = "missing".to_string();
    let error = replace_git_placements(
        &pool,
        "ws",
        &[placement("new", 0), missing_repository],
        Some("new"),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .as_database_error()
            .unwrap()
            .is_foreign_key_violation()
    );
    assert_eq!(
        state(&pool).await,
        (vec![("old".to_string(), true)], "single".to_string())
    );
    cleanup(pool, admin, schema).await;
}

#[tokio::test]
#[ignore = "requires MOATLESS_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn concurrent_replacements_commit_one_complete_set_with_one_primary() {
    let (pool, admin, schema) = database().await;
    for _ in 0..8 {
        let first = [placement("first-0", 0), placement("first-1", 1)];
        let second = [placement("second", 0)];
        let (a, b) = tokio::join!(
            replace_git_placements(&pool, "ws", &first, Some("first-1")),
            replace_git_placements(&pool, "ws", &second, Some("second")),
        );
        a.unwrap();
        b.unwrap();
        let result = state(&pool).await;
        assert!(
            result
                == (
                    vec![
                        ("first-0".to_string(), false),
                        ("first-1".to_string(), true)
                    ],
                    "multi".to_string()
                )
                || result == (vec![("second".to_string(), true)], "single".to_string())
        );
    }
    replace_git_placements(&pool, "ws", &[], None)
        .await
        .unwrap();
    assert_eq!(state(&pool).await, (vec![], "blank".to_string()));
    cleanup(pool, admin, schema).await;
}

#[tokio::test]
#[ignore = "requires MOATLESS_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn script_write_preserves_read_grant_and_rechecks_source_deletion_and_access() {
    use super::dao::{PgWorkspaceDao, WorkspaceDao};
    use moatless_types::access_control::ScopeContext;
    use moatless_workspace_types::workspace::ScriptConfig;

    let (pool, admin, schema) = database().await;
    sqlx::raw_sql("ALTER TABLE workspaces ADD COLUMN source text NOT NULL DEFAULT 'manual', ADD COLUMN scope text NOT NULL DEFAULT 'global', ADD COLUMN owner_user_id text, ADD COLUMN created_by text, ADD COLUMN scripts jsonb; UPDATE workspaces SET owner_user_id = 'someone-else', scripts = '{}'::jsonb;")
        .execute(&pool).await.unwrap();
    let dao = PgWorkspaceDao(pool.clone());
    let viewer = ScopeContext::user("viewer");
    let target = dao
        .scripts_write_target(&viewer, "ws")
        .await
        .unwrap()
        .unwrap();
    assert!(target.scripts.is_empty());
    let next = [ScriptConfig {
        name: "build".to_string(),
        command: "make".to_string(),
        ..Default::default()
    }];
    assert!(dao.replace_scripts(&viewer, "ws", &next).await.unwrap());
    assert_eq!(
        dao.scripts_write_target(&viewer, "ws")
            .await
            .unwrap()
            .unwrap()
            .scripts,
        next
    );
    sqlx::query("UPDATE workspaces SET source = 'git' WHERE id = 'ws'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!dao.replace_scripts(&viewer, "ws", &[]).await.unwrap());
    sqlx::query("UPDATE workspaces SET source = 'manual', deleted = true WHERE id = 'ws'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!dao.replace_scripts(&viewer, "ws", &[]).await.unwrap());
    assert!(
        dao.scripts_write_target(&viewer, "ws")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE workspaces SET deleted = false, scope = 'user' WHERE id = 'ws'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!dao.replace_scripts(&viewer, "ws", &[]).await.unwrap());
    assert!(
        dao.scripts_write_target(&viewer, "ws")
            .await
            .unwrap()
            .is_none()
    );
    drop(dao);
    cleanup(pool, admin, schema).await;
}
