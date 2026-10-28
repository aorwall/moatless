//! PostgreSQL tests for the record store.

use sqlx::PgPool;

use super::{KEPT_CHANGES, PgRecordDao, RecordDao};
use crate::artifact::dao::{ArtifactDao, PgArtifactDao};
use crate::task::test_support::seed_bare_task;
use moatless_api_schemas::artifact::RecordWriter;

async fn reserve(dao: &PgArtifactDao, id: &str, slug: &str, title: &str) {
    dao.reserve_version(id, "task-1", slug, title, "user-1")
        .await
        .expect("reserve");
}

#[sqlx::test]
async fn reserving_an_artifact_creates_its_own_collection_which_its_task_takes_along(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool.clone());
    let records = PgRecordDao(pool.clone());
    reserve(&dao, "art-a", "report", "Report").await;
    reserve(&dao, "art-a", "report", "Report").await;
    let own = records
        .find_collection("artifact", "art-a", "")
        .await
        .unwrap()
        .expect("collection");
    assert_eq!(own.collection_id, "art-a");
    records
        .save_record(
            "art-a",
            "a",
            &serde_json::json!({}),
            "user-1",
            RecordWriter::Page,
        )
        .await
        .unwrap();

    sqlx::query("DELETE FROM tasks WHERE task_id = 'task-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        records
            .find_collection("artifact", "art-a", "")
            .await
            .unwrap()
            .is_none()
    );
    assert!(records.find_record("art-a", "a").await.unwrap().is_none());
}

#[sqlx::test]
async fn a_scoped_collection_is_created_once_and_outlives_the_task_that_wrote_it(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let records = PgRecordDao(pool.clone());
    let first = records
        .ensure_collection("c-1", "loop", "loop-1", "review-runs", "task-1", "user-1")
        .await
        .unwrap();
    let again = records
        .ensure_collection("c-2", "loop", "loop-1", "review-runs", "task-2", "user-2")
        .await
        .unwrap();
    assert_eq!(
        (first.collection_id.as_str(), again.collection_id.as_str()),
        ("c-1", "c-1")
    );
    records
        .ensure_collection("c-3", "loop", "loop-2", "review-runs", "task-1", "user-1")
        .await
        .unwrap();
    records
        .save_record(
            "c-1",
            "runs/1",
            &serde_json::json!({"ok": true}),
            "user-1",
            RecordWriter::Agent,
        )
        .await
        .unwrap();

    sqlx::query("DELETE FROM tasks WHERE task_id = 'task-1'")
        .execute(&pool)
        .await
        .unwrap();
    let listed = records.list_collections("loop", "loop-1").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].name.as_str(), listed[0].record_count),
        ("review-runs", 1)
    );
    assert!(listed[0].last_updated_at.is_some());
    assert_eq!(
        records.record_heads(&["c-1".to_string()]).await.unwrap(),
        vec![("c-1".to_string(), 1)]
    );
}

#[sqlx::test]
async fn a_save_under_a_key_replaces_the_record_there(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool.clone());
    let records = PgRecordDao(pool);
    reserve(&dao, "art-a", "report", "Report").await;
    let first = records
        .save_record(
            "art-a",
            "state",
            &serde_json::json!({"n": 1}),
            "user-1",
            RecordWriter::Page,
        )
        .await
        .unwrap();
    let replaced = records
        .save_record(
            "art-a",
            "state",
            &serde_json::json!({"n": 2}),
            "user-2",
            RecordWriter::Page,
        )
        .await
        .unwrap();
    assert_eq!(replaced.created_at, first.created_at);
    assert_eq!(replaced.data, serde_json::json!({"n": 2}));
    assert_eq!(replaced.submitted_by, "user-2");
    for key in ["vote:a", "vote:b"] {
        records
            .save_record(
                "art-a",
                key,
                &serde_json::json!({}),
                "user-1",
                RecordWriter::Page,
            )
            .await
            .unwrap();
    }

    assert_eq!(
        records.count_records("art-a", "state").await.unwrap(),
        (3, true)
    );
    assert_eq!(
        records.count_records("art-a", "other").await.unwrap(),
        (3, false)
    );
    assert_eq!(
        records
            .list_records("art-a", None, None)
            .await
            .unwrap()
            .len(),
        3
    );
    let since = replaced.updated_at;
    assert_eq!(
        records
            .list_records("art-a", Some(since), None)
            .await
            .unwrap()
            .len(),
        2
    );
    let votes = records
        .list_records("art-a", None, Some("vote:".to_string()))
        .await
        .unwrap();
    assert_eq!(votes.len(), 2);

    let deleted = records.delete_record("art-a", "vote:a").await.unwrap();
    assert_eq!(deleted.map(|r| r.key).as_deref(), Some("vote:a"));
    assert!(
        records
            .delete_record("art-a", "vote:a")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        records
            .list_records("art-a", None, None)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[sqlx::test]
async fn an_insert_leaves_a_saved_record_alone_and_keeps_its_writer(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool.clone());
    let records = PgRecordDao(pool);
    reserve(&dao, "art-a", "report", "Report").await;
    let first = records
        .insert_record(
            "art-a",
            "req/1",
            &serde_json::json!({"n": 1}),
            "user-1",
            RecordWriter::Page,
        )
        .await
        .unwrap()
        .expect("inserted");
    assert_eq!(first.writer, RecordWriter::Page);
    let second = records
        .insert_record(
            "art-a",
            "req/1",
            &serde_json::json!({"n": 2}),
            "user-2",
            RecordWriter::Page,
        )
        .await
        .unwrap();
    assert!(second.is_none());
    let stored = records
        .find_record("art-a", "req/1")
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(
        (stored.data, stored.submitted_by.as_str()),
        (serde_json::json!({"n": 1}), "user-1")
    );

    let agent = records
        .save_record(
            "art-a",
            "req/1",
            &serde_json::json!({}),
            "user-3",
            RecordWriter::Agent,
        )
        .await
        .unwrap();
    assert_eq!(agent.writer, RecordWriter::Agent);
    assert!(
        records
            .find_record("art-a", "req/2")
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test]
async fn a_conditional_write_keeps_the_owner_and_misses_a_record_that_changed(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool.clone());
    let records = PgRecordDao(pool);
    reserve(&dao, "art-a", "report", "Report").await;
    let stored = records
        .save_record(
            "art-a",
            "plan/1",
            &serde_json::json!({"n": 1}),
            "user-1",
            RecordWriter::Agent,
        )
        .await
        .unwrap();

    let updated = records
        .update_record(
            "art-a",
            "plan/1",
            &serde_json::json!({"n": 2}),
            stored.updated_at,
        )
        .await
        .unwrap()
        .expect("updated");
    assert_eq!(
        (
            updated.data.clone(),
            updated.submitted_by.as_str(),
            updated.writer
        ),
        (serde_json::json!({"n": 2}), "user-1", RecordWriter::Agent)
    );
    assert!(updated.updated_at > stored.updated_at);

    let stale = records
        .update_record(
            "art-a",
            "plan/1",
            &serde_json::json!({"n": 3}),
            stored.updated_at,
        )
        .await
        .unwrap();
    assert!(stale.is_none());
    let stale = records
        .delete_record_at("art-a", "plan/1", stored.updated_at)
        .await
        .unwrap();
    assert!(stale.is_none());

    let deleted = records
        .delete_record_at("art-a", "plan/1", updated.updated_at)
        .await
        .unwrap()
        .expect("deleted");
    assert_eq!(deleted.data, serde_json::json!({"n": 2}));
}

#[sqlx::test]
async fn every_record_write_is_logged_in_order_and_the_log_keeps_the_latest(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    let dao = PgArtifactDao(pool.clone());
    let records = PgRecordDao(pool);
    reserve(&dao, "art-a", "report", "Report").await;
    let saved = records
        .save_record(
            "art-a",
            "a",
            &serde_json::json!({}),
            "user-1",
            RecordWriter::Page,
        )
        .await
        .unwrap();
    records
        .insert_record(
            "art-a",
            "b",
            &serde_json::json!({}),
            "user-1",
            RecordWriter::Page,
        )
        .await
        .unwrap();
    records
        .update_record("art-a", "a", &serde_json::json!({"n": 1}), saved.updated_at)
        .await
        .unwrap();
    records.delete_record("art-a", "b").await.unwrap();
    let unchanged = records
        .insert_record(
            "art-a",
            "a",
            &serde_json::json!({}),
            "user-2",
            RecordWriter::Page,
        )
        .await
        .unwrap();
    assert!(unchanged.is_none());
    records.delete_record("art-a", "never-saved").await.unwrap();

    let (head, changes) = records.record_changes("art-a", 1).await.unwrap();
    assert_eq!(head, 4);
    assert_eq!(
        changes,
        vec![
            (2, "b".to_string()),
            (3, "a".to_string()),
            (4, "b".to_string())
        ]
    );
    assert_eq!(
        records
            .record_heads(&["art-a".to_string(), "missing".to_string()])
            .await
            .unwrap(),
        vec![("art-a".to_string(), 4)]
    );
    let current = records
        .find_records("art-a", &["a".to_string(), "b".to_string()])
        .await
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].data, serde_json::json!({"n": 1}));

    for _ in 0..KEPT_CHANGES {
        records
            .save_record(
                "art-a",
                "a",
                &serde_json::json!({}),
                "user-1",
                RecordWriter::Page,
            )
            .await
            .unwrap();
    }
    let (head, changes) = records.record_changes("art-a", 0).await.unwrap();
    assert_eq!(head, 4 + KEPT_CHANGES);
    assert_eq!(changes.len(), usize::try_from(KEPT_CHANGES).unwrap());
    assert_eq!(changes[0].0, 5);
}
