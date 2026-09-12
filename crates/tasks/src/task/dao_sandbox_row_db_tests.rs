//! Database tests for the Sandbox row a Task's own writes maintain, and for
//! the cleanup a failed bootstrap gets.

use super::*;
use crate::task::test_support::seed_bare_task;
use sqlx::PgPool;

async fn seed_sandbox_member(pool: &PgPool, task_id: &str, sandbox_id: &str) {
    sqlx::query(
        r#"INSERT INTO sandboxes (sandbox_id, workspace_id, desired_state, desired_state_updated_at)
           SELECT $1, COALESCE(workspace_id, 'ws_' || repository_id), 'stopped', NOW()
           FROM tasks WHERE task_id = $2
           ON CONFLICT (sandbox_id) DO NOTHING"#,
    )
    .bind(sandbox_id)
    .bind(task_id)
    .execute(pool)
    .await
    .expect("seed sandbox");
    sqlx::query("UPDATE tasks SET sandbox_id = $1 WHERE task_id = $2")
        .bind(sandbox_id)
        .bind(task_id)
        .execute(pool)
        .await
        .expect("bind task to sandbox");
}

fn owner_ctx() -> ScopeContext {
    ScopeContext {
        user_id: "user-1".to_string(),
        is_admin: false,
    }
}

fn task_row(task_id: &str) -> Task {
    let now = Utc::now();
    Task {
        task_id: task_id.to_string(),
        short_id: Some(format!("s-{task_id}")),
        created_at: now,
        last_message_at: now,
        agent_type: AgentType(AgentType::CLAUDE_CODE.into()),
        repository_id: "repo-x".to_string(),
        status: TaskStatus::Open,
        total_cost_usd: 0.0,
        site_url: None,
        template_id: None,
        vibe: None,
        name: None,
        description: None,
        site_id: None,
        site_provider: None,
        branch: None,
        model: None,
        created_by: Some("user-1".to_string()),
        visibility: Visibility::Private,
        scope: Scope::User,
        owner_user_id: Some("user-1".to_string()),
        run_as_user_id: None,
        updated_at: None,
        append_system_prompt: None,
        headless: false,
        ttl_minutes: 15,
        inactive_ttl_days: 7,
        loop_id: None,
        workspace_id: Some("ws_repo-x".to_string()),
        sandbox_id: None,
        checkout_mode: None,
        skills: Vec::new(),
    }
}

#[derive(sqlx::FromRow)]
struct SandboxShape {
    workspace_id: String,
    scope: String,
    owner_user_id: Option<String>,
    run_as_user_id: Option<String>,
    branch: Option<String>,
    headless: bool,
}

async fn sandbox_shape(pool: &PgPool, sandbox_id: &str) -> Option<SandboxShape> {
    sqlx::query_as(
        "SELECT workspace_id, scope, owner_user_id, run_as_user_id, branch, headless
         FROM sandboxes WHERE sandbox_id = $1",
    )
    .bind(sandbox_id)
    .fetch_optional(pool)
    .await
    .expect("read sandbox")
}

/// Task creation writes the Sandbox row, and its shape comes off the Task
/// that created it. This is the invariant migration 0142 backfilled for
/// Sandboxes that predate it, so the two cannot disagree.
#[sqlx::test]
async fn insert_task_writes_the_sandbox_row_from_the_task(pool: PgPool) {
    // Seeds the team and the `ws_repo-x` workspace the row points at.
    seed_bare_task(&pool, "seed", "open").await;

    let mut row = task_row("t1");
    row.branch = Some("feature/x".to_string());
    row.headless = true;
    store_over(&pool).insert_task(&row).await.unwrap();

    let shape = sandbox_shape(&pool, "t1").await.expect("sandbox row");
    assert_eq!(shape.workspace_id, "ws_repo-x");
    assert_eq!(shape.scope, "user");
    assert_eq!(shape.owner_user_id.as_deref(), Some("user-1"));
    assert_eq!(shape.branch.as_deref(), Some("feature/x"));
    assert!(shape.headless);

    let bound: Option<String> =
        sqlx::query_scalar("SELECT sandbox_id FROM tasks WHERE task_id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bound.as_deref(), Some("t1"), "the Task names its Sandbox");
}

/// `run_as_user_id` follows `Actor::run_as`: the Workspace's account
/// outranks the owner of the work, and falls back to that owner.
#[sqlx::test]
async fn the_workspace_run_as_account_outranks_the_task_owner(pool: PgPool) {
    seed_bare_task(&pool, "seed", "open").await;
    let dao = store_over(&pool);

    dao.insert_task(&task_row("t1")).await.unwrap();
    assert_eq!(
        sandbox_shape(&pool, "t1")
            .await
            .expect("sandbox row")
            .run_as_user_id
            .as_deref(),
        Some("user-1"),
        "no workspace account: the owner of the work"
    );

    sqlx::query("UPDATE workspaces SET run_as_user_id = 'bot-9' WHERE id = 'ws_repo-x'")
        .execute(&pool)
        .await
        .unwrap();
    dao.insert_task(&task_row("t2")).await.unwrap();
    assert_eq!(
        sandbox_shape(&pool, "t2")
            .await
            .expect("sandbox row")
            .run_as_user_id
            .as_deref(),
        Some("bot-9"),
        "the workspace names the account its tasks act as"
    );
}

/// A Task naming its own run-as outranks the Workspace's account. This is
/// what `moat tasks create --run-as` writes; `service::normalize` is where
/// the admin-and-bot check that admits it lives.
#[sqlx::test]
async fn the_task_run_as_account_outranks_the_workspace(pool: PgPool) {
    seed_bare_task(&pool, "seed", "open").await;
    let dao = store_over(&pool);
    sqlx::query("UPDATE workspaces SET run_as_user_id = 'bot-9' WHERE id = 'ws_repo-x'")
        .execute(&pool)
        .await
        .unwrap();

    let mut row = task_row("t1");
    row.run_as_user_id = Some("bot-3".to_string());
    dao.insert_task(&row).await.unwrap();

    assert_eq!(
        sandbox_shape(&pool, "t1")
            .await
            .expect("sandbox row")
            .run_as_user_id
            .as_deref(),
        Some("bot-3"),
        "the Task's own choice wins over the workspace account"
    );
}

/// Joining an existing Sandbox writes no second row: the joiner adopts the
/// shape the initial member gave it.
#[sqlx::test]
async fn joining_a_sandbox_writes_no_second_row(pool: PgPool) {
    seed_bare_task(&pool, "seed", "open").await;
    let dao = store_over(&pool);
    dao.insert_task(&task_row("t1")).await.unwrap();

    let mut joiner = task_row("t2");
    joiner.sandbox_id = Some("t1".to_string());
    joiner.checkout_mode = Some(CheckoutMode::Shared);
    dao.insert_task(&joiner).await.unwrap();

    assert!(sandbox_shape(&pool, "t2").await.is_none(), "no second row");
    let bound: Option<String> =
        sqlx::query_scalar("SELECT sandbox_id FROM tasks WHERE task_id = 't2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bound.as_deref(), Some("t1"));
}

/// A Task whose `sandboxes` row is gone projects as converged Stopped at
/// the no-intent epoch — the absence semantic this task-response projection
/// owns. Deleting the projection would silently drop it.
#[sqlx::test]
async fn no_sandbox_row_projects_as_converged_stopped(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    // `tasks_sandbox_id_fkey` is ON DELETE SET NULL, so the Task survives
    // its Sandbox row and still has to project.
    sqlx::query("DELETE FROM sandboxes WHERE sandbox_id = 't1'")
        .execute(&pool)
        .await
        .unwrap();

    let views = load_sandbox_intent_views(&pool, &[String::from("t1")])
        .await
        .unwrap();
    let view = views.get("t1").expect("existing task has a projection");
    assert_eq!(view.desired_state, SandboxDesiredState::Stopped);
    assert!(
        !view.has_explicit_intent(),
        "a no-row task must not look like an explicit stop"
    );
}

#[sqlx::test]
async fn delete_task_deletes_the_last_member_sandbox_row(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_sandbox_member(&pool, "t1", "t1").await;
    let dao = store_over(&pool);

    assert!(dao.delete_task("t1", &owner_ctx()).await.unwrap());

    let sandbox_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sandboxes WHERE sandbox_id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sandbox_count, 0);
}

#[sqlx::test]
async fn delete_task_keeps_sandbox_row_when_members_remain(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_bare_task(&pool, "t2", "open").await;
    seed_sandbox_member(&pool, "t1", "t1").await;
    seed_sandbox_member(&pool, "t2", "t1").await;
    let dao = store_over(&pool);

    assert!(dao.delete_task("t1", &owner_ctx()).await.unwrap());

    let sandbox_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sandboxes WHERE sandbox_id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sandbox_count, 1);
    let remaining_member_sandbox: Option<String> =
        sqlx::query_scalar("SELECT sandbox_id FROM tasks WHERE task_id = 't2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining_member_sandbox.as_deref(), Some("t1"));
}

/// The reaper folds a Sandbox's decision over this result, so a member the
/// query drops is a member whose live turn the reaper closes.
#[sqlx::test]
async fn list_sandbox_members_returns_the_owner_and_its_active_forks(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    seed_bare_task(&pool, "t2", "open").await;
    seed_bare_task(&pool, "t3", "error").await;
    seed_bare_task(&pool, "t4", "closed").await;
    seed_bare_task(&pool, "t5", "open").await;
    for fork in ["t2", "t3", "t4"] {
        seed_sandbox_member(&pool, fork, "t1").await;
    }

    let mut members = store_over(&pool)
        .list_sandbox_members(&["t1".to_string()])
        .await
        .unwrap();
    members.sort();

    assert_eq!(
        members,
        vec![
            ("t1".to_string(), "t1".to_string()),
            ("t2".to_string(), "t1".to_string()),
            ("t3".to_string(), "t1".to_string()),
        ],
        "the owner and its open and error forks, without the closed fork or t5"
    );
}

async fn seed_message(pool: &PgPool, task_id: &str) {
    sqlx::query(
        r#"INSERT INTO task_messages (task_id, uuid, message_type, user_type,
                                      timestamp, message_role)
           VALUES ($1, $2, 'user', 'human', NOW()::text, 'user')"#,
    )
    .bind(task_id)
    .bind(format!("{task_id}-m1"))
    .execute(pool)
    .await
    .expect("seed message");
}

async fn seed_activity(pool: &PgPool, task_id: &str) {
    sqlx::query(
        r#"INSERT INTO task_activities (activity_id, task_id, position, kind, status,
                                        summary, is_error)
           VALUES ($1 || '-a1', $1, 1, 'sandbox_error', 'failed',
                   'preflight failed', true)"#,
    )
    .bind(task_id)
    .execute(pool)
    .await
    .expect("seed activity");
}

async fn task_exists(pool: &PgPool, task_id: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM tasks WHERE task_id = $1)")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The retry the T3 draft makes under the id its first send wedged. Without
/// this the row stays and every retry fails the unique index.
#[sqlx::test]
async fn delete_errored_empty_task_frees_a_failed_bootstrap(pool: PgPool) {
    seed_bare_task(&pool, "t1", "error").await;
    seed_activity(&pool, "t1").await;
    let dao = store_over(&pool);

    assert!(
        dao.delete_errored_empty_task("t1", &owner_ctx())
            .await
            .unwrap()
    );

    assert!(!task_exists(&pool, "t1").await);
    let sandbox_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sandboxes WHERE sandbox_id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sandbox_count, 0);
    // The failed incarnation's error activity would otherwise open the
    // conversation the retry creates.
    let activity_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM task_activities WHERE task_id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(activity_count, 0);
}

/// An errored task somebody has already talked to is a conversation, not a
/// bootstrap that never happened.
#[sqlx::test]
async fn delete_errored_empty_task_keeps_an_errored_task_with_messages(pool: PgPool) {
    seed_bare_task(&pool, "t1", "error").await;
    seed_message(&pool, "t1").await;
    let dao = store_over(&pool);

    assert!(
        !dao.delete_errored_empty_task("t1", &owner_ctx())
            .await
            .unwrap()
    );
    assert!(task_exists(&pool, "t1").await);
}

/// A live thread is never taken over by a create that names its id.
#[sqlx::test]
async fn delete_errored_empty_task_keeps_a_live_task(pool: PgPool) {
    seed_bare_task(&pool, "t1", "open").await;
    let dao = store_over(&pool);

    assert!(
        !dao.delete_errored_empty_task("t1", &owner_ctx())
            .await
            .unwrap()
    );
    assert!(task_exists(&pool, "t1").await);
}

/// The control rule rides in the statement, so an id somebody else's failed
/// bootstrap holds is not reusable even when the caller can read the row.
#[sqlx::test]
async fn delete_errored_empty_task_keeps_another_users_task(pool: PgPool) {
    seed_bare_task(&pool, "t1", "error").await;
    let stranger = ScopeContext {
        user_id: "user-2".to_string(),
        is_admin: false,
    };
    let dao = store_over(&pool);

    assert!(
        !dao.delete_errored_empty_task("t1", &stranger)
            .await
            .unwrap()
    );
    assert!(task_exists(&pool, "t1").await);
}
