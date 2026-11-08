use sqlx::{PgPool, postgres::PgPoolOptions};

use super::{PgTaskDao, TaskDao};
use crate::task::test_support::seed_bare_task;
use moatless_types::access_control::ScopeContext;

#[sqlx::test]
async fn cached_task_lookup_survives_unrelated_column_changes(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let reader = PgPoolOptions::new()
        .max_connections(1)
        .max_lifetime(None)
        .idle_timeout(None)
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let dao = PgTaskDao::new(reader.clone());
    let ctx = ScopeContext::user("user-1");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&reader)
        .await
        .unwrap();

    for change in [
        "ALTER TABLE tasks ADD COLUMN rollout_metadata text",
        "ALTER TABLE tasks DROP COLUMN rollout_metadata",
    ] {
        let before = dao.find_by_id("task-1", &ctx).await.unwrap().unwrap();
        sqlx::query(change).execute(&pool).await.unwrap();
        for _ in 0..2 {
            let after = dao.find_by_id("task-1", &ctx).await.unwrap().unwrap();
            assert_eq!(after.task_id, before.task_id);
            assert_eq!(after.owner_user_id, before.owner_user_id);
            assert_eq!(after.status, before.status);
            assert!(
                dao.find_by_id("task-1", &ScopeContext::user("other-user"))
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&reader)
            .await
            .unwrap(),
        pid
    );
    reader.close().await;
}
