//! PostgreSQL tests for the memory store.

use sqlx::PgPool;

use super::{MemoryDao, PgMemoryDao};
use crate::memory::models::{
    ClaimOutcome, ListFilter, PutEntry, PutOutcome, ScopeRef, TtlChange, Writer,
};
use crate::task::test_support::seed_bare_task;
use moatless_types::memory::MemoryScope;

fn loop_scope() -> ScopeRef {
    ScopeRef {
        scope: MemoryScope::Loop,
        id: "loop-1".to_string(),
    }
}

fn workspace_scope() -> ScopeRef {
    ScopeRef {
        scope: MemoryScope::Workspace,
        id: "ws-1".to_string(),
    }
}

fn writer(task_id: &str) -> Writer {
    Writer {
        task_id: task_id.to_string(),
        loop_id: Some("loop-1".to_string()),
        user_id: "user-1".to_string(),
    }
}

fn entry(key: &str, summary: Option<&str>, body: Option<&str>) -> PutEntry {
    PutEntry {
        scope: loop_scope(),
        key: key.to_string(),
        summary: summary.map(str::to_string),
        body: body.map(str::to_string),
        data: None,
        tags: None,
        citations: None,
        ttl: TtlChange::Keep,
        stale_after_seconds: None,
        if_version: None,
        writer: writer("task-1"),
    }
}

async fn written(dao: &PgMemoryDao, put: &PutEntry) -> crate::memory::models::MemoryEntryRow {
    match dao.put(put).await.expect("put") {
        PutOutcome::Written(row) => *row,
        other => panic!("expected a write, got {other:?}"),
    }
}

#[sqlx::test]
async fn a_put_creates_then_patches_and_bumps_the_version(pool: PgPool) {
    let dao = PgMemoryDao(pool);
    let first = written(&dao, &entry("ci:flaky", Some("retry once"), Some("v1"))).await;
    assert_eq!(first.version, 1);
    assert!(
        first.expires_at.is_some(),
        "a new entry gets the default TTL"
    );

    let second = written(&dao, &entry("ci:flaky", None, Some("v2"))).await;
    assert_eq!(second.version, 2);
    assert_eq!(second.summary, "retry once", "an omitted summary is kept");
    assert_eq!(second.body, "v2");

    let log = dao.log(&first.entry_id, 10).await.expect("log");
    assert_eq!(
        log.iter().map(|e| e.version).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[sqlx::test]
async fn a_new_key_without_a_summary_is_refused(pool: PgPool) {
    let dao = PgMemoryDao(pool);
    let outcome = dao.put(&entry("a", None, Some("x"))).await.expect("put");
    assert!(matches!(outcome, PutOutcome::SummaryRequired));
}

#[sqlx::test]
async fn if_version_guards_the_write(pool: PgPool) {
    let dao = PgMemoryDao(pool);
    let mut create = entry("a", Some("s"), None);
    create.if_version = Some(0);
    written(&dao, &create).await;

    let again = dao.put(&create).await.expect("put");
    assert!(matches!(
        again,
        PutOutcome::VersionMismatch { current: Some(1) }
    ));

    let mut stale = entry("a", None, Some("x"));
    stale.if_version = Some(5);
    let outcome = dao.put(&stale).await.expect("put");
    assert!(matches!(
        outcome,
        PutOutcome::VersionMismatch { current: Some(1) }
    ));
}

#[sqlx::test]
async fn reads_cover_only_the_scopes_given(pool: PgPool) {
    let dao = PgMemoryDao(pool);
    written(&dao, &entry("a", Some("in loop"), None)).await;
    let mut other = entry("a", Some("in workspace"), None);
    other.scope = workspace_scope();
    written(&dao, &other).await;

    let found = dao.find(&[loop_scope()], "a").await.expect("find");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].summary, "in loop");

    let both = dao
        .find(&[loop_scope(), workspace_scope()], "a")
        .await
        .expect("find");
    assert_eq!(both.len(), 2);
}

#[sqlx::test]
async fn list_filters_by_prefix_and_search_matches_bodies(pool: PgPool) {
    let dao = PgMemoryDao(pool);
    written(
        &dao,
        &entry("triage:SP-1", Some("first"), Some("the flaky checkout")),
    )
    .await;
    written(&dao, &entry("triage:SP-2", Some("second"), None)).await;
    written(&dao, &entry("ci:runner", Some("runner"), None)).await;

    let filter = ListFilter {
        prefix: Some("triage:".to_string()),
        limit: 10,
        ..ListFilter::default()
    };
    let listed = dao.list(&[loop_scope()], &filter).await.expect("list");
    assert_eq!(listed.len(), 2);

    let hits = dao
        .search(&[loop_scope()], "checkout", 10)
        .await
        .expect("search");
    assert_eq!(
        hits.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
        vec!["triage:SP-1"]
    );

    let preloaded = dao
        .preload(
            &[loop_scope()],
            &["ci:runner".to_string()],
            &["triage:".to_string()],
            10,
        )
        .await
        .expect("preload");
    assert_eq!(preloaded.len(), 3);
}

#[sqlx::test]
async fn an_expired_entry_reads_as_absent_and_a_put_restarts_it(pool: PgPool) {
    let dao = PgMemoryDao(pool.clone());
    let row = written(&dao, &entry("a", Some("old"), Some("old body"))).await;
    sqlx::query("UPDATE memory_entries SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .expect("expire");

    assert!(
        dao.find(&[loop_scope()], "a")
            .await
            .expect("find")
            .is_empty()
    );

    let outcome = dao.put(&entry("a", None, Some("x"))).await.expect("put");
    assert!(
        matches!(outcome, PutOutcome::SummaryRequired),
        "an expired entry is gone, so the put is a create"
    );
    let fresh = written(&dao, &entry("a", Some("new"), None)).await;
    assert_eq!(fresh.version, 1);
    assert_ne!(fresh.entry_id, row.entry_id);
}

#[sqlx::test]
async fn no_expiry_clears_the_deadline_and_verify_renews_it(pool: PgPool) {
    let dao = PgMemoryDao(pool);
    let mut forever = entry("a", Some("s"), None);
    forever.ttl = TtlChange::NoExpiry;
    let row = written(&dao, &forever).await;
    assert!(row.expires_at.is_none());

    let mut timed = entry("b", Some("s"), None);
    timed.ttl = TtlChange::Set(60);
    let row = written(&dao, &timed).await;
    let verified = dao
        .verify(&row.entry_id, &writer("task-1"))
        .await
        .expect("verify")
        .expect("entry");
    assert!(verified.verified_at.is_some());
    assert!(verified.expires_at >= row.expires_at);
}

#[sqlx::test]
async fn a_claim_holds_against_open_tasks_only(pool: PgPool) {
    seed_bare_task(&pool, "task-1", "open").await;
    seed_bare_task(&pool, "task-2", "open").await;
    let dao = PgMemoryDao(pool.clone());
    let row = written(&dao, &entry("triage:SP-1", Some("s"), None)).await;

    let first = dao
        .claim(&row.entry_id, 3600, &writer("task-1"))
        .await
        .expect("claim");
    assert!(matches!(first, Some(ClaimOutcome::Claimed(_))));
    let second = dao
        .claim(&row.entry_id, 3600, &writer("task-2"))
        .await
        .expect("claim");
    assert!(matches!(second, Some(ClaimOutcome::HeldBy(holder)) if holder == "task-1"));

    sqlx::query("UPDATE tasks SET status = 'closed' WHERE task_id = 'task-1'")
        .execute(&pool)
        .await
        .expect("close");
    let third = dao
        .claim(&row.entry_id, 3600, &writer("task-2"))
        .await
        .expect("claim");
    assert!(matches!(third, Some(ClaimOutcome::Claimed(_))));

    assert!(
        !dao.release(&row.entry_id, &writer("task-1"))
            .await
            .expect("release")
    );
    assert!(
        dao.release(&row.entry_id, &writer("task-2"))
            .await
            .expect("release")
    );
}

#[sqlx::test]
async fn reads_are_recorded_once_per_version(pool: PgPool) {
    let dao = PgMemoryDao(pool.clone());
    let row = written(&dao, &entry("a", Some("s"), None)).await;
    let reads = [(row.entry_id.clone(), 1)];
    dao.record_reads("task-1", &reads).await.expect("read");
    dao.record_reads("task-1", &reads)
        .await
        .expect("read again");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_reads")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn delete_removes_the_entry_and_its_history(pool: PgPool) {
    let dao = PgMemoryDao(pool.clone());
    let row = written(&dao, &entry("a", Some("s"), None)).await;
    assert!(dao.delete(&row.entry_id).await.expect("delete"));
    assert!(!dao.delete(&row.entry_id).await.expect("delete again"));
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(events, 0);
}
