//! Putting rows in: workspaces, tasks, turns, sandboxes, messages, and who
//! follows what.

use crate::common::make_message_row;
use crate::common::make_task_row;
use crate::common::make_workspace_row;
use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use moatless_tasks::activities::mapping::ConversationWrite;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::models::TaskMessage;
use moatless_tasks::task::models::Task;
use moatless_tasks::task::models::Visibility;
use moatless_types::access_control::Scope;
use moatless_workspaces::compose;
use sqlx::PgPool;

// =========================================================================
// Conversation seeding
// =========================================================================

/// Insert a task in `workspace_id`, owned by `owner`.
///
/// `insert_task` auto-follows for the creator, matching every production
/// creation path; tests needing a non-follower must unfollow explicitly.
pub(super) async fn seed_task(
    pool: &PgPool,
    task_id: &str,
    owner: &str,
    workspace_id: Option<&str>,
) -> Task {
    let mut task = make_task_row(task_id, owner);
    task.workspace_id = workspace_id.map(str::to_string);
    moatless_tasks::task::test_support::seed_task(pool, &task)
        .await
        .unwrap();
    task
}

pub(super) async fn seed_public_task(
    pool: &PgPool,
    task_id: &str,
    owner: &str,
    workspace_id: Option<&str>,
) -> Task {
    let mut task = make_task_row(task_id, owner);
    task.workspace_id = workspace_id.map(str::to_string);
    task.visibility = Visibility::Public;
    moatless_tasks::task::test_support::seed_task(pool, &task)
        .await
        .unwrap();
    task
}

pub(super) async fn follow(pool: &PgPool, user_id: &str, task_id: &str) {
    moatless_tasks::task::test_support::follow_task(pool, user_id, task_id)
        .await
        .unwrap();
}

pub(super) async fn set_settled_and_snoozed(pool: &PgPool, user_id: &str, task_id: &str) {
    moatless_tasks::task::test_support::settle_task(pool, user_id, task_id, Utc::now())
        .await
        .unwrap();
    moatless_tasks::task::test_support::snooze_task(
        pool,
        user_id,
        task_id,
        Utc::now() + ChronoDuration::hours(1),
        Utc::now(),
    )
    .await
    .unwrap();
}

pub(super) async fn follower_state(
    pool: &PgPool,
    user_id: &str,
    task_id: &str,
) -> (
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
) {
    sqlx::query_as(
        r#"SELECT settled_override, settled_at, snoozed_until, snoozed_at
           FROM task_follows
           WHERE user_id = $1 AND task_id = $2"#,
    )
    .bind(user_id)
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

pub(super) async fn unfollow(pool: &PgPool, user_id: &str, task_id: &str) {
    moatless_tasks::task::test_support::unfollow_task(pool, user_id, task_id)
        .await
        .unwrap();
}

pub(super) async fn seed_turn(pool: &PgPool, task_id: &str, turn_number: i32, status: &str) {
    sqlx::query(
        r#"INSERT INTO task_turns (task_id, turn_number, status, started_at)
           VALUES ($1, $2, $3, NOW())"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

/// Seed the Sandbox an ordinary Task runs in, named after the Task, and bind
/// the Task to it.
pub(super) async fn seed_sandbox(pool: &PgPool, task_id: &str, desired_state: &str, started: bool) {
    let workspace_id: String = sqlx::query_scalar(
        "SELECT COALESCE(workspace_id, 'ws_' || repository_id) FROM tasks WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap();

    // `sandboxes.workspace_id` is an FK, and these fixtures name Workspaces that
    // `seed_workspaces` does not create. An already-seeded id conflicts, which
    // is the no-op this wants.
    let workspace = make_workspace_row(&workspace_id, "user-1");
    let _ = compose::workspaces(pool.clone())
        .insert_workspace(&workspace)
        .await;

    sqlx::query(
        r#"INSERT INTO sandboxes (sandbox_id, workspace_id, desired_state, started_at)
           VALUES ($1, $2, $3, CASE WHEN $4 THEN NOW() ELSE NULL END)
           ON CONFLICT (sandbox_id) DO UPDATE SET
             workspace_id = EXCLUDED.workspace_id,
             desired_state = EXCLUDED.desired_state,
             started_at = EXCLUDED.started_at"#,
    )
    .bind(task_id)
    .bind(&workspace_id)
    .bind(desired_state)
    .bind(started)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE tasks SET sandbox_id = $1 WHERE task_id = $1")
        .bind(task_id)
        .execute(pool)
        .await
        .unwrap();
}

/// A message from a person, which is what the activity timestamp counts.
pub(super) async fn seed_user_message(pool: &PgPool, task_id: &str, uuid: &str) {
    let msg = make_message_row(task_id, uuid);
    moatless_tasks::message::test_support::seed_message(
        pool,
        &msg,
        &[],
        &ConversationWrite::default(),
    )
    .await
    .unwrap();
}

/// A message the agent produced. Stored as `system` with the SDK's `user` role
/// for tool results, and as `assistant` for the agent's own replies — neither
/// is a person writing.
pub(super) async fn seed_agent_message(
    pool: &PgPool,
    task_id: &str,
    uuid: &str,
    turn_number: i32,
    model: Option<&str>,
) {
    let msg = TaskMessage {
        message_type: MessageType::Assistant,
        user_type: "system".into(),
        turn_number: Some(turn_number),
        assistant_message_model: model.map(str::to_string),
        ..make_message_row(task_id, uuid)
    };
    moatless_tasks::message::test_support::seed_message(
        pool,
        &msg,
        &[],
        &ConversationWrite::default(),
    )
    .await
    .unwrap();
}

/// Seeds one workspace per ownership/scope combination the listing has to
/// distinguish: the viewer's own, a global one they do not own, someone else's
/// private one, and a soft-deleted one of their own.
pub(super) async fn seed_workspaces(pool: &PgPool) {
    let repo = compose::workspaces(pool.clone());

    repo.insert_workspace(&make_workspace_row("ws-own", "user-1"))
        .await
        .unwrap();

    let mut global = make_workspace_row("ws-global", "user-2");
    global.scope = Scope::Global;
    global.owner_user_id = None;
    global.created_by = None;
    repo.insert_workspace(&global).await.unwrap();

    repo.insert_workspace(&make_workspace_row("ws-other", "user-2"))
        .await
        .unwrap();

    let mut deleted = make_workspace_row("ws-deleted", "user-1");
    deleted.deleted = true;
    repo.insert_workspace(&deleted).await.unwrap();
}

/// The one project every matrix conversation lives in. Its own readability is
/// not what these tests vary — the conversation's is.
pub(super) const MATRIX_WS: &str = "ws-matrix";

/// One conversation per combination of owner, visibility, scope, and follow the
/// two filters must distinguish.
///
/// Ids read `{owner}-{visibility}-{scope}-{follow}`, so a failing assertion
/// names the combination that broke.
pub(super) async fn seed_conversation_matrix(pool: &PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row(MATRIX_WS, "user-1"))
        .await
        .unwrap();

    for (owner_label, owner) in [("own", "user-1"), ("other", "user-2")] {
        for (visibility_label, visibility) in [
            ("private", Visibility::Private),
            ("public", Visibility::Public),
        ] {
            for (scope_label, scope) in [("user", Scope::User), ("global", Scope::Global)] {
                for (follow_label, followed) in [("followed", true), ("unfollowed", false)] {
                    let id =
                        format!("{owner_label}-{visibility_label}-{scope_label}-{follow_label}");
                    let mut task = make_task_row(&id, owner);
                    task.workspace_id = Some(MATRIX_WS.to_string());
                    task.visibility = visibility;
                    task.scope = scope;
                    // `tasks_owner_scope_chk`: a deployment-scoped row has no
                    // owner. `created_by` still records who made it.
                    task.owner_user_id = match scope {
                        Scope::User => Some(owner.to_string()),
                        Scope::Global => None,
                    };
                    moatless_tasks::task::test_support::seed_task(pool, &task)
                        .await
                        .unwrap();

                    // Creation auto-follows for the creator, so the viewer's
                    // follow is set both ways rather than only added.
                    if followed {
                        follow(pool, "user-1", &id).await;
                    } else {
                        unfollow(pool, "user-1", &id).await;
                    }
                }
            }
        }
    }
}

/// The project the delivery tests run in. Owned by `user-1`, so `user-1` reads
/// it and follows what they create in it.
pub(super) async fn seed_live_project(pool: &PgPool) {
    compose::workspaces(pool.clone())
        .insert_workspace(&make_workspace_row("ws-live", "user-1"))
        .await
        .unwrap();
}
