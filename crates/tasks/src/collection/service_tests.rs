use std::sync::Arc;

use chrono::Utc;

use super::*;
use crate::collection::dao::MockRecordDao;
use crate::collection::test_support::records;
use crate::task::dao::MockTaskDao;
use crate::task::test_support::make_task_row;

/// `task-1` is user-1's and was fired by `loop-1`; `task-plain` is user-1's
/// with no Loop; `task-public` is user-2's and readable by everyone.
fn tasks() -> TaskCommands {
    let mut store = MockTaskDao::new();
    store
        .expect_find_by_id()
        .returning(|task_id, ctx| match task_id {
            "task-1" => {
                let mut task = make_task_row(task_id, "user-1");
                task.loop_id = Some("loop-1".to_string());
                Ok((ctx.user_id == "user-1").then_some(task))
            }
            "task-plain" => Ok((ctx.user_id == "user-1").then(|| make_task_row(task_id, "user-1"))),
            "task-public" => {
                let mut task = make_task_row(task_id, "user-2");
                task.visibility = moatless_types::task::Visibility::Public;
                Ok(Some(task))
            }
            // A gate review: a Loop's Task running as an admin-approved bot.
            "task-review" | "task-bot-manual" => {
                let mut task = make_task_row(task_id, "bot");
                task.run_as_user_id = Some("bot".to_string());
                task.loop_id = (task_id == "task-review").then(|| "loop-review".to_string());
                Ok((ctx.user_id == "bot").then_some(task))
            }
            // Another owner's private Task, which the store answers to any reader
            // and which writes for its owner and admins only.
            "task-shared" => Ok(Some(make_task_row(task_id, "user-2"))),
            _ => Ok(None),
        });
    TaskCommands::new(Arc::new(store))
}

fn collection(scope_kind: &str, scope_id: &str, name: &str) -> CollectionRow {
    CollectionRow {
        collection_id: format!("{scope_kind}-{scope_id}-{name}"),
        scope_kind: scope_kind.to_string(),
        scope_id: scope_id.to_string(),
        name: name.to_string(),
        created_at: Utc::now(),
        record_count: 0,
        last_updated_at: None,
    }
}

fn record(collection_id: &str, key: &str, data: &serde_json::Value) -> Record {
    Record {
        collection_id: collection_id.to_string(),
        key: key.to_string(),
        data: data.clone(),
        submitted_by: "user-1".to_string(),
        writer: RecordWriter::Agent,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// A store that creates any collection and saves into it, holding `count`
/// records of which one is under `runs/1`.
fn writable(count: i64) -> MockRecordDao {
    let mut store = MockRecordDao::new();
    store
        .expect_ensure_collection()
        .returning(|_, kind, id, name, _, _| Ok(collection(kind, id, name)));
    store
        .expect_count_records()
        .returning(move |_, key| Ok((count, key == "runs/1")));
    store
        .expect_save_record()
        .returning(|id, key, data, _, _| Ok(record(id, key, data)));
    store
}

fn save(key: &str) -> WriteArtifactRecordRequest {
    WriteArtifactRecordRequest::Save {
        key: key.to_string(),
        data: serde_json::json!({"outcome": "approved"}),
    }
}

fn ctx(user: &str) -> ScopeContext {
    ScopeContext::user(user)
}

#[tokio::test]
async fn an_unscoped_write_lands_in_the_loop_collection_as_the_agent() {
    let mut store = MockRecordDao::new();
    store
        .expect_ensure_collection()
        .withf(|_, kind, id, name, task, user| {
            (kind, id, name, task, user) == ("loop", "loop-1", "review-runs", "task-1", "user-1")
        })
        .times(1)
        .returning(|_, kind, id, name, _, _| Ok(collection(kind, id, name)));
    store
        .expect_count_records()
        .returning(|_, _| Ok((0, false)));
    store
        .expect_save_record()
        .withf(|id, _, _, by, writer| {
            id == "loop-loop-1-review-runs" && by == "user-1" && *writer == RecordWriter::Agent
        })
        .returning(|id, key, data, _, _| Ok(record(id, key, data)));
    let written = write(
        &tasks(),
        &records(store),
        &ctx("user-1"),
        "task-1",
        "review-runs",
        None,
        save("runs/42/abc"),
    )
    .await
    .unwrap();
    assert_eq!(
        (
            written.collection.as_str(),
            written.scope,
            written.key.as_str()
        ),
        ("review-runs", MemoryScope::Loop, "runs/42/abc")
    );
}

#[tokio::test]
async fn a_task_without_a_loop_writes_its_own_collection() {
    let written = write(
        &tasks(),
        &records(writable(0)),
        &ctx("user-1"),
        "task-plain",
        "notes",
        None,
        save("a"),
    )
    .await
    .unwrap();
    assert_eq!(written.scope, MemoryScope::Task);
}

#[tokio::test]
async fn a_visitor_writes_a_public_tasks_records() {
    let written = write(
        &tasks(),
        &records(writable(0)),
        &ctx("user-3"),
        "task-public",
        "notes",
        Some(MemoryScope::Task),
        save("a"),
    )
    .await
    .unwrap();
    assert_eq!(written.scope, MemoryScope::Task);
}

#[tokio::test]
async fn a_reader_who_cannot_change_the_task_cannot_write_its_records() {
    let err = write(
        &tasks(),
        &records(MockRecordDao::new()),
        &ctx("user-3"),
        "task-shared",
        "notes",
        Some(MemoryScope::Task),
        save("a"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CollectionError::Forbidden(_)), "{err:?}");
}

#[tokio::test]
async fn a_task_without_a_loop_has_no_loop_collection() {
    let err = write(
        &tasks(),
        &records(MockRecordDao::new()),
        &ctx("user-1"),
        "task-plain",
        "notes",
        Some(MemoryScope::Loop),
        save("a"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CollectionError::BadRequest(_)), "{err:?}");
}

#[tokio::test]
async fn a_name_that_is_not_a_plain_key_is_refused() {
    for name in ["", "a/b", "has space", &"x".repeat(64)] {
        let err = write(
            &tasks(),
            &records(MockRecordDao::new()),
            &ctx("user-1"),
            "task-1",
            name,
            None,
            save("a"),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, CollectionError::BadRequest(_)),
            "{name:?}: {err:?}"
        );
    }
}

#[tokio::test]
async fn a_full_collection_takes_a_save_only_under_a_key_it_holds() {
    let records = records(writable(MAX_RECORDS));
    let (tasks, ctx) = (tasks(), ctx("user-1"));
    let write = |key| {
        write(
            &tasks,
            &records,
            &ctx,
            "task-1",
            "review-runs",
            None,
            save(key),
        )
    };
    assert!(matches!(
        write("runs/2").await,
        Err(CollectionError::BadRequest(_))
    ));
    assert_eq!(write("runs/1").await.unwrap().key, "runs/1");
}

#[tokio::test]
async fn a_collection_never_written_lists_nothing() {
    let mut store = MockRecordDao::new();
    store.expect_find_collection().returning(|_, _, _| Ok(None));
    let items = list(
        &tasks(),
        &records(store),
        &ctx("user-1"),
        "task-1",
        "review-runs",
        None,
        RecordFilter::default(),
    )
    .await
    .unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn a_list_reads_the_collection_of_the_scope_the_task_reaches() {
    let mut store = MockRecordDao::new();
    store
        .expect_find_collection()
        .withf(|kind, id, name| (kind, id, name) == ("loop", "loop-1", "review-runs"))
        .returning(|kind, id, name| Ok(Some(collection(kind, id, name))));
    store
        .expect_list_records()
        .withf(|id, since, prefix| {
            id == "loop-loop-1-review-runs" && since.is_some() && prefix.as_deref() == Some("runs/")
        })
        .returning(|id, _, _| Ok(vec![record(id, "runs/1", &serde_json::json!({}))]));
    let items = list(
        &tasks(),
        &records(store),
        &ctx("user-1"),
        "task-1",
        "review-runs",
        None,
        RecordFilter {
            prefix: Some("runs/"),
            since: Some("2026-10-01T00:00:00Z"),
        },
    )
    .await
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].scope, MemoryScope::Loop);
}

#[tokio::test]
async fn a_since_that_is_not_rfc_3339_is_refused() {
    let err = list(
        &tasks(),
        &records(MockRecordDao::new()),
        &ctx("user-1"),
        "task-1",
        "review-runs",
        None,
        RecordFilter {
            prefix: None,
            since: Some("yesterday"),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CollectionError::BadRequest(_)), "{err:?}");
}

#[tokio::test]
async fn a_delete_from_a_collection_never_written_creates_nothing() {
    // No `expect_ensure_collection`: calling it fails the test.
    let mut store = MockRecordDao::new();
    store.expect_find_collection().returning(|_, _, _| Ok(None));
    let delete = WriteArtifactRecordRequest::Delete {
        key: "runs/1".to_string(),
    };
    let err = write(
        &tasks(),
        &records(store),
        &ctx("user-1"),
        "task-1",
        "typo-name",
        None,
        delete,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CollectionError::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn a_loop_task_writes_records_in_its_own_workspace_as_its_run_as_bot() {
    let written = write(
        &tasks(),
        &records(writable(0)),
        &ctx("bot"),
        "task-review",
        "pr-review-state",
        Some(MemoryScope::Workspace),
        save("soaplabs/widget/1"),
    )
    .await
    .unwrap();
    assert_eq!(written.scope, MemoryScope::Workspace);
}

#[tokio::test]
async fn a_task_no_loop_opened_cannot_write_a_workspace_it_cannot_write() {
    let err = write(
        &tasks(),
        &records(MockRecordDao::new()),
        &ctx("bot"),
        "task-bot-manual",
        "pr-review-state",
        Some(MemoryScope::Workspace),
        save("soaplabs/widget/1"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, CollectionError::Forbidden(_)), "{err:?}");
}
