#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! Integration tests for `feedback_dao`.

use crate::common;

use chrono::Utc;
use common::{make_repository_row, make_task_row, user_scope};
use moatless_backend::feedback::dao::{FeedbackDao, FeedbackFilter, PgFeedbackDao};
use moatless_backend::message::dao as message_dao;
use moatless_backend::task_activities::mapping::ConversationWrite;
use moatless_backend::feedback::models::Feedback;
use sqlx::PgPool;

const USER: &str = "user-1";

/// Seed the repo + task that the listing query joins to.
async fn seed_task(pool: &PgPool, task_id: &str, repo_id: &str, workspace_id: Option<&str>) {
    let repo = make_repository_row(repo_id, USER);
    moatless_backend::repository::dao::insert(pool, &repo)
        .await
        .expect("seed repo");

    let mut task = make_task_row(task_id, USER);
    task.repository_id = repo_id.to_string();
    task.workspace_id = workspace_id.map(str::to_string);
    moatless_backend::task::dao::insert_task(pool, &task)
        .await
        .expect("seed task");
}

/// Insert a minimal `task_activities` row directly: the domain writer
/// (`upsert`) needs a transaction and an allocated conversation position,
/// which is more machinery than a fixture needs to prove a read.
async fn seed_activity(pool: &PgPool, task_id: &str, position: i64, message_uuid: &str, tool_use_id: Option<&str>) {
    sqlx::query(
        r#"INSERT INTO task_activities
               (activity_id, task_id, position, message_uuid, kind, status, tool_use_id, summary)
           VALUES ($1, $2, $3, $4, 'tool_call', 'completed', $5, 'Bash')"#,
    )
    .bind(format!("{message_uuid}:{position}"))
    .bind(task_id)
    .bind(position)
    .bind(message_uuid)
    .bind(tool_use_id)
    .execute(pool)
    .await
    .expect("seed activity");
}

/// Insert a minimal `task_messages` row, so `latest_activity`'s
/// message-only fallback has a row to find.
async fn seed_message(pool: &PgPool, task_id: &str, uuid: &str) {
    let msg = common::make_message_row(task_id, uuid);
    message_dao::append(pool, &msg, &[], &ConversationWrite::default())
        .await
        .expect("seed message");
}

fn filter() -> FeedbackFilter {
    FeedbackFilter {
        scope: user_scope(USER),
        workspace_id: None,
        repository_id: None,
        task_id: None,
        source: None,
        surface: None,
        category: None,
        created_by: None,
        since: None,
        limit: 50,
        offset: 0,
    }
}

fn sample(feedback_id: &str, task_id: &str, surface: &str) -> Feedback {
    Feedback {
        feedback_id: feedback_id.to_string(),
        task_id: task_id.to_string(),
        message_uuid: None,
        tool_use_id: None,
        source: "user".to_string(),
        surface: surface.to_string(),
        category: None,
        comment: "looks good".to_string(),
        metadata: None,
        created_by: "user-1".to_string(),
        created_at: Utc::now(),
    }
}

#[sqlx::test]
async fn insert_and_find_round_trips(pool: PgPool) {
    let dao = PgFeedbackDao(pool);
    let mut row = sample("fb-1", "task-1", "task");
    row.category = Some("environment".to_string());
    dao.insert(&row).await.expect("insert");

    let fetched = dao.find_by_id("fb-1").await.expect("find").expect("row");
    assert_eq!(fetched.feedback_id, "fb-1");
    assert_eq!(fetched.category.as_deref(), Some("environment"));
    assert_eq!(fetched.task_id, "task-1");
    assert_eq!(fetched.surface, "task");
    assert_eq!(fetched.comment, "looks good");
    assert_eq!(fetched.source, "user");
    assert!(fetched.message_uuid.is_none());
    assert!(fetched.tool_use_id.is_none());
}

#[sqlx::test]
async fn list_for_task_filters_and_orders(pool: PgPool) {
    let dao = PgFeedbackDao(pool);

    let mut a1 = sample("fb-a1", "task-A", "task");
    a1.created_at = Utc::now() - chrono::Duration::seconds(60);
    dao.insert(&a1).await.unwrap();

    let mut a2 = sample("fb-a2", "task-A", "message");
    a2.message_uuid = Some("msg-1".to_string());
    dao.insert(&a2).await.unwrap();

    let b1 = sample("fb-b1", "task-B", "task");
    dao.insert(&b1).await.unwrap();

    let listed = dao.list_for_task("task-A").await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].feedback_id, "fb-a1");
    assert_eq!(listed[1].feedback_id, "fb-a2");

    let other = dao.list_for_task("task-B").await.unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].feedback_id, "fb-b1");
}

#[sqlx::test]
async fn metadata_round_trips(pool: PgPool) {
    let dao = PgFeedbackDao(pool);
    let mut row = sample("fb-meta", "task-meta", "preview_failure");
    row.metadata = Some(serde_json::json!({"preview_error": "boom"}));
    dao.insert(&row).await.unwrap();

    let fetched = dao.find_by_id("fb-meta").await.unwrap().unwrap();
    assert_eq!(
        fetched.metadata.unwrap()["preview_error"],
        serde_json::json!("boom")
    );
}

#[sqlx::test]
async fn list_returns_newest_first_with_task_context(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", Some("ws-a")).await;
    let dao = PgFeedbackDao(pool);

    let mut older = sample("fb-old", "task-A", "task");
    older.created_at = Utc::now() - chrono::Duration::seconds(60);
    dao.insert(&older).await.unwrap();
    dao.insert(&sample("fb-new", "task-A", "task")).await.unwrap();

    let (items, total) = dao.list(&filter()).await.unwrap();

    assert_eq!(total, 2);
    assert_eq!(items[0].feedback.feedback_id, "fb-new");
    assert_eq!(items[1].feedback.feedback_id, "fb-old");
    assert_eq!(items[0].repository_id, "repo-a");
    // `repositories` is owned by `crates/workspaces`; the DAO leaves the name
    // for the handler to resolve via `RepositoryReader`.
    assert_eq!(items[0].repository_name, None);
    assert_eq!(items[0].workspace_id.as_deref(), Some("ws-a"));
    assert_eq!(items[0].task_title, "Test task task-A");
}

#[sqlx::test]
async fn list_filters_by_workspace_repository_and_task(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", Some("ws-a")).await;
    seed_task(&pool, "task-B", "repo-b", Some("ws-b")).await;
    let dao = PgFeedbackDao(pool);
    dao.insert(&sample("fb-a", "task-A", "task")).await.unwrap();
    dao.insert(&sample("fb-b", "task-B", "task")).await.unwrap();

    let by_workspace = FeedbackFilter {
        workspace_id: Some("ws-a".to_string()),
        ..filter()
    };
    let (items, total) = dao.list(&by_workspace).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].feedback.feedback_id, "fb-a");

    let by_repo = FeedbackFilter {
        repository_id: Some("repo-b".to_string()),
        ..filter()
    };
    let (items, _) = dao.list(&by_repo).await.unwrap();
    assert_eq!(items[0].feedback.feedback_id, "fb-b");

    let by_task = FeedbackFilter {
        task_id: Some("task-A".to_string()),
        ..filter()
    };
    let (items, _) = dao.list(&by_task).await.unwrap();
    assert_eq!(items[0].feedback.feedback_id, "fb-a");
}

#[sqlx::test]
async fn list_filters_by_source_surface_category_submitter_and_age(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", None).await;
    let dao = PgFeedbackDao(pool);

    let mut agent = sample("fb-agent", "task-A", "message");
    agent.source = "agent".to_string();
    agent.message_uuid = Some("msg-1".to_string());
    agent.created_by = "user-2".to_string();
    agent.created_at = Utc::now() - chrono::Duration::days(30);
    agent.category = Some("environment".to_string());
    dao.insert(&agent).await.unwrap();
    dao.insert(&sample("fb-user", "task-A", "task")).await.unwrap();

    let by_source = FeedbackFilter {
        source: Some("agent".to_string()),
        ..filter()
    };
    let (items, _) = dao.list(&by_source).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].feedback.feedback_id, "fb-agent");

    let by_surface = FeedbackFilter {
        surface: Some("task".to_string()),
        ..filter()
    };
    let (items, _) = dao.list(&by_surface).await.unwrap();
    assert_eq!(items[0].feedback.feedback_id, "fb-user");

    let by_category = FeedbackFilter {
        category: Some("environment".to_string()),
        ..filter()
    };
    let (items, total) = dao.list(&by_category).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].feedback.feedback_id, "fb-agent");

    let by_submitter = FeedbackFilter {
        created_by: Some("user-2".to_string()),
        ..filter()
    };
    let (items, _) = dao.list(&by_submitter).await.unwrap();
    assert_eq!(items[0].feedback.feedback_id, "fb-agent");

    let recent = FeedbackFilter {
        since: Some(Utc::now() - chrono::Duration::days(1)),
        ..filter()
    };
    let (items, total) = dao.list(&recent).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].feedback.feedback_id, "fb-user");
}

/// A user with no access to the task must not read its feedback, and the total
/// must not count it either — a total that outruns the page leaks existence.
#[sqlx::test]
async fn list_hides_feedback_on_tasks_the_caller_cannot_read(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", None).await;
    let dao = PgFeedbackDao(pool);
    dao.insert(&sample("fb-a", "task-A", "task")).await.unwrap();

    let stranger = FeedbackFilter {
        scope: user_scope("user-other"),
        ..filter()
    };
    let (items, total) = dao.list(&stranger).await.unwrap();
    assert!(items.is_empty());
    assert_eq!(total, 0);
}

/// The total counts the whole filtered set, so an offset past the end still
/// reports it rather than coming back as zero.
#[sqlx::test]
async fn list_pages_without_losing_the_total(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", None).await;
    let dao = PgFeedbackDao(pool);
    for i in 0..3 {
        let mut row = sample(&format!("fb-{i}"), "task-A", "task");
        row.created_at = Utc::now() - chrono::Duration::seconds(i);
        dao.insert(&row).await.unwrap();
    }

    let page = FeedbackFilter {
        limit: 2,
        ..filter()
    };
    let (items, total) = dao.list(&page).await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(total, 3);

    let past_end = FeedbackFilter {
        limit: 2,
        offset: 10,
        ..filter()
    };
    let (items, total) = dao.list(&past_end).await.unwrap();
    assert!(items.is_empty());
    assert_eq!(total, 3);
}

#[sqlx::test]
async fn latest_activity_prefers_the_latest_tool_call(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", None).await;
    seed_message(&pool, "task-A", "msg-1").await;
    seed_activity(&pool, "task-A", 1, "msg-1", Some("tool-1")).await;
    seed_activity(&pool, "task-A", 2, "msg-1", Some("tool-2")).await;
    let dao = PgFeedbackDao(pool);

    let found = dao.latest_activity("task-A").await.unwrap().unwrap();
    assert_eq!(found.message_uuid, "msg-1");
    assert_eq!(found.tool_use_id.as_deref(), Some("tool-2"));
}

#[sqlx::test]
async fn latest_activity_falls_back_to_the_latest_message_with_no_tool_call(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", None).await;
    seed_message(&pool, "task-A", "msg-1").await;
    seed_message(&pool, "task-A", "msg-2").await;
    let dao = PgFeedbackDao(pool);

    let found = dao.latest_activity("task-A").await.unwrap().unwrap();
    assert_eq!(found.message_uuid, "msg-2");
    assert!(found.tool_use_id.is_none());
}

#[sqlx::test]
async fn latest_activity_is_none_for_a_task_with_no_messages(pool: PgPool) {
    seed_task(&pool, "task-A", "repo-a", None).await;
    let dao = PgFeedbackDao(pool);

    assert!(dao.latest_activity("task-A").await.unwrap().is_none());
}
