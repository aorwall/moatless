//! PostgreSQL tests for the artifact store.

use sqlx::PgPool;

use super::{ArtifactDao, PgArtifactDao};
use crate::artifact::models::DataAccess;
use crate::message::dao::{MessageDao, PgMessageDao};
use crate::task::test_support::seed_bare_task;
use moatless_types::activity::ArtifactShown;

async fn reserve(dao: &PgArtifactDao, id: &str, slug: &str, title: &str) -> (String, i32) {
    let r = dao
        .reserve_version(id, "task-1", slug, title, "user-1")
        .await
        .expect("reserve");
    (r.artifact_id, r.reserved_version)
}

#[sqlx::test]
async fn republishing_a_slug_reserves_the_next_version_of_the_same_artifact(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool);

    assert_eq!(
        reserve(&dao, "art-a", "report", "Report").await,
        ("art-a".to_string(), 1)
    );
    assert_eq!(
        reserve(&dao, "art-b", "report", "Report").await,
        ("art-a".to_string(), 2)
    );
    assert_eq!(
        reserve(&dao, "art-c", "other", "Other").await,
        ("art-c".to_string(), 1)
    );
}

#[sqlx::test]
async fn an_unpublished_reservation_is_not_listed(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool);
    reserve(&dao, "art-a", "report", "Report").await;

    assert!(dao.list_for_task("task-1").await.unwrap().is_empty());

    dao.commit_version("art-a", 1, "Report", DataAccess::default(), None)
        .await
        .unwrap()
        .expect("row");
    let listed = dao.list_for_task("task-1").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].latest_version, 1);
}

#[sqlx::test]
async fn a_late_commit_of_an_older_version_does_not_roll_back_the_latest(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool);
    reserve(&dao, "art-a", "report", "Old").await;
    reserve(&dao, "art-a", "report", "New").await;

    dao.commit_version("art-a", 2, "New", DataAccess::default(), None)
        .await
        .unwrap();
    let row = dao
        .commit_version("art-a", 1, "Old", DataAccess::default(), None)
        .await
        .unwrap()
        .expect("row");

    assert_eq!(row.latest_version, 2);
    assert_eq!(row.published_versions, [2, 1]);
    assert_eq!(row.title, "New");
}

#[sqlx::test]
async fn committing_an_unknown_artifact_answers_none(pool: PgPool) {
    let dao = PgArtifactDao(pool);
    assert!(
        dao.commit_version("missing", 1, "T", DataAccess::default(), None)
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn deleting_the_task_deletes_its_artifacts(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool.clone());
    reserve(&dao, "art-a", "report", "Report").await;

    sqlx::query("DELETE FROM tasks WHERE task_id = 'task-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(dao.find_by_id("art-a").await.unwrap().is_none());
}

fn shown(version: i32) -> ArtifactShown {
    ArtifactShown {
        artifact_id: "art-a".to_string(),
        version,
        title: "Report".to_string(),
        height: 600,
    }
}

async fn shown_activities(pool: &PgPool) -> Vec<(uuid::Uuid, String, serde_json::Value)> {
    sqlx::query_as(
        "SELECT activity_id, summary, payload FROM conversation_activities \
          WHERE task_id = 'task-1' AND kind = 'artifact_shown' ORDER BY position",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// An inline version is recorded as an `artifact_shown` task event and shown
/// in the thread from it, and a rebuild of the thread shows it again.
#[sqlx::test]
async fn an_inline_commit_shows_the_version_in_the_thread(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool.clone());
    reserve(&dao, "art-a", "report", "Report").await;
    reserve(&dao, "art-a", "report", "Report").await;

    dao.commit_version("art-a", 1, "Report", DataAccess::default(), None)
        .await
        .unwrap();
    let row = dao
        .commit_version("art-a", 2, "Report", DataAccess::default(), Some(shown(2)))
        .await
        .unwrap()
        .expect("row");

    assert_eq!(row.inline_versions, [2]);
    let events: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload FROM task_events WHERE task_id = 'task-1' AND sub_type = 'artifact_shown'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(events, [serde_json::to_value(shown(2)).unwrap()]);
    let activities = shown_activities(&pool).await;
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].1, "Report");
    assert_eq!(activities[0].2["height"], 600);

    PgMessageDao(pool.clone())
        .rebuild_conversation("task-1")
        .await
        .unwrap();
    assert_eq!(shown_activities(&pool).await, activities);
}

#[sqlx::test]
async fn the_latest_publish_decides_whether_the_page_accepts_data(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool);
    reserve(&dao, "art-a", "report", "Report").await;
    reserve(&dao, "art-a", "report", "Report").await;
    let data = DataAccess {
        accepts_data: true,
        notify_on_data: true,
    };
    let row = dao
        .commit_version("art-a", 2, "Report", data, None)
        .await
        .unwrap()
        .expect("row");
    assert!(row.accepts_data && row.notify_on_data);

    let row = dao
        .commit_version("art-a", 1, "Report", DataAccess::default(), None)
        .await
        .unwrap()
        .expect("row");
    assert!(row.accepts_data, "an older version left the flags alone");
}

#[sqlx::test]
async fn a_save_under_a_key_replaces_the_record_there(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool);
    reserve(&dao, "art-a", "report", "Report").await;
    let first = dao
        .save_record("art-a", "state", &serde_json::json!({"n": 1}), "user-1")
        .await
        .unwrap();
    let replaced = dao
        .save_record("art-a", "state", &serde_json::json!({"n": 2}), "user-2")
        .await
        .unwrap();
    assert_eq!(replaced.created_at, first.created_at);
    assert_eq!(replaced.data, serde_json::json!({"n": 2}));
    assert_eq!(replaced.submitted_by, "user-2");
    for key in ["vote:a", "vote:b"] {
        dao.save_record("art-a", key, &serde_json::json!({}), "user-1")
            .await
            .unwrap();
    }

    assert_eq!(
        dao.count_records("art-a", "state").await.unwrap(),
        (3, true)
    );
    assert_eq!(
        dao.count_records("art-a", "other").await.unwrap(),
        (3, false)
    );
    assert_eq!(
        dao.list_records("art-a", None, None).await.unwrap().len(),
        3
    );
    let since = replaced.updated_at;
    assert_eq!(
        dao.list_records("art-a", Some(since), None)
            .await
            .unwrap()
            .len(),
        2
    );
    let votes = dao
        .list_records("art-a", None, Some("vote:".to_string()))
        .await
        .unwrap();
    assert_eq!(votes.len(), 2);

    let deleted = dao.delete_record("art-a", "vote:a").await.unwrap();
    assert_eq!(deleted.map(|r| r.key).as_deref(), Some("vote:a"));
    assert!(
        dao.delete_record("art-a", "vote:a")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        dao.list_records("art-a", None, None).await.unwrap().len(),
        2
    );
}
