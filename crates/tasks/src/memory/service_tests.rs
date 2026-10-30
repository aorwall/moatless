use std::sync::{Arc, Mutex};

use chrono::Utc;

use super::*;
use crate::memory::dao::MockMemoryDao;
use crate::task::dao::MockTaskDao;
use crate::task::test_support::make_task_row;
use moatless_types::access_control::Scope;
use moatless_workspace_ports::{MockRepositoryReader, MockWorkspaceResolver};
use moatless_workspaces::test_support::{make_repository_row, make_workspace_row};

fn ctx(user: &str) -> ScopeContext {
    ScopeContext::user(user)
}

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
            // Reads to everyone and writes for admins only.
            "task-shared" => {
                let mut task = make_task_row(task_id, "user-2");
                task.scope = Scope::Global;
                Ok(Some(task))
            }
            _ => Ok(None),
        });
    TaskCommands::new(Arc::new(store))
}

fn memories(store: MockMemoryDao) -> Memories {
    memories_over(store, Scope::User)
}

/// Memories over `store` whose fixture Workspace and repository are user-1's
/// when `shared` is `User`, and global when it is `Global`.
fn memories_over(store: MockMemoryDao, shared: Scope) -> Memories {
    let owner = (shared == Scope::User).then(|| "user-1".to_string());
    let mut repositories = MockRepositoryReader::new();
    let repo_owner = owner.clone();
    repositories.expect_describe().returning(move |_, id| {
        let mut repo = make_repository_row(id, "user-1");
        repo.scope = shared;
        repo.owner_user_id = repo_owner.clone();
        Ok(Some(repo))
    });
    let mut workspaces = MockWorkspaceResolver::new();
    workspaces.expect_describe().returning(move |_, id| {
        let mut workspace = make_workspace_row(id, "user-1");
        workspace.scope = shared;
        workspace.owner_user_id = owner.clone();
        Ok(Some(workspace))
    });
    Memories::new(
        Arc::new(store),
        Arc::new(repositories),
        Arc::new(workspaces),
    )
}

fn row(key: &str, scope: MemoryScope, scope_id: &str) -> MemoryEntryRow {
    let now = Utc::now();
    MemoryEntryRow {
        entry_id: format!("{scope}-{key}"),
        scope_kind: scope.as_str().to_string(),
        scope_id: scope_id.to_string(),
        key: key.to_string(),
        summary: "summary".to_string(),
        body: "body".to_string(),
        data: None,
        tags: Vec::new(),
        citations: Vec::new(),
        version: 3,
        ttl_seconds: Some(60),
        expires_at: Some(now),
        stale_after_seconds: None,
        verified_at: None,
        claimed_by_task_id: None,
        claim_expires_at: None,
        updated_by_task_id: Some("task-1".to_string()),
        created_by_task_id: None,
        created_by_loop_id: None,
        created_by_user_id: None,
        created_at: now,
        updated_at: now,
        stale: false,
    }
}

fn put_req(key: &str) -> PutMemoryRequest {
    PutMemoryRequest {
        key: key.to_string(),
        summary: Some("one line".to_string()),
        ..PutMemoryRequest::default()
    }
}

/// A put that lands in `scope`/`id` and answers with the row it wrote.
fn store_expecting_put(scope: MemoryScope, id: &'static str) -> MockMemoryDao {
    let mut store = MockMemoryDao::new();
    store.expect_put().returning(move |entry| {
        assert_eq!(entry.scope.scope, scope);
        assert_eq!(entry.scope.id, id);
        assert_eq!(entry.writer.task_id, "task-1");
        Ok(PutOutcome::Written(Box::new(row(&entry.key, scope, id))))
    });
    store
}

#[tokio::test]
async fn a_put_without_a_scope_lands_in_the_tasks_loop() {
    let entry = put(
        &tasks(),
        &memories(store_expecting_put(MemoryScope::Loop, "loop-1")),
        &ctx("user-1"),
        "task-1",
        put_req("ci:flaky"),
    )
    .await
    .map_err(|e| e.to_string());
    assert_eq!(entry.map(|e| e.scope), Ok(MemoryScope::Loop));
}

#[tokio::test]
async fn a_put_without_a_scope_or_a_loop_lands_in_the_task() {
    let mut store = MockMemoryDao::new();
    store.expect_put().returning(|entry| {
        assert_eq!(entry.scope.scope, MemoryScope::Task);
        assert_eq!(entry.scope.id, "task-plain");
        Ok(PutOutcome::Written(Box::new(row(
            &entry.key,
            MemoryScope::Task,
            "task-plain",
        ))))
    });
    let result = put(
        &tasks(),
        &memories(store),
        &ctx("user-1"),
        "task-plain",
        put_req("a"),
    )
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn global_memory_is_refused_to_a_user_task() {
    let req = PutMemoryRequest {
        scope: Some(MemoryScope::Global),
        ..put_req("a")
    };
    let result = put(
        &tasks(),
        &memories(MockMemoryDao::new()),
        &ctx("user-1"),
        "task-1",
        req,
    )
    .await;
    assert!(matches!(result, Err(MemoryError::Forbidden(_))));
}

#[tokio::test]
async fn a_put_naming_a_scope_lands_only_in_that_scope() {
    let req = PutMemoryRequest {
        scope: Some(MemoryScope::Loop),
        ..put_req("state")
    };
    let entry = put(
        &tasks(),
        &memories(store_expecting_put(MemoryScope::Loop, "loop-1")),
        &ctx("user-1"),
        "task-1",
        req,
    )
    .await
    .map_err(|e| e.to_string());
    assert_eq!(entry.map(|e| e.scope), Ok(MemoryScope::Loop));
}

#[tokio::test]
async fn a_scope_the_task_lacks_is_a_bad_request() {
    let req = PutMemoryRequest {
        scope: Some(MemoryScope::Loop),
        ..put_req("state")
    };
    let result = put(
        &tasks(),
        &memories(MockMemoryDao::new()),
        &ctx("user-1"),
        "task-plain",
        req,
    )
    .await;
    assert!(matches!(result, Err(MemoryError::BadRequest(_))));
}

#[tokio::test]
async fn repository_memory_on_a_shared_repository_is_refused_to_a_user() {
    let req = PutMemoryRequest {
        scope: Some(MemoryScope::Repository),
        ..put_req("a")
    };
    let result = put(
        &tasks(),
        &memories_over(MockMemoryDao::new(), Scope::Global),
        &ctx("user-1"),
        "task-1",
        req,
    )
    .await;
    assert!(matches!(result, Err(MemoryError::Forbidden(_))));
}

#[tokio::test]
async fn a_visitor_writes_a_public_tasks_memory() {
    let mut store = MockMemoryDao::new();
    store.expect_put().returning(|entry| {
        Ok(PutOutcome::Written(Box::new(row(
            &entry.key,
            MemoryScope::Task,
            "task-public",
        ))))
    });
    let result = put(
        &tasks(),
        &memories(store),
        &ctx("user-1"),
        "task-public",
        put_req("a"),
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test]
async fn a_reader_of_someone_elses_task_cannot_write_its_memory() {
    let store = MockMemoryDao::new();
    let result = put(
        &tasks(),
        &memories(store),
        &ctx("user-1"),
        "task-shared",
        put_req("a"),
    )
    .await;
    assert!(matches!(result, Err(MemoryError::Forbidden(_))));
}

#[tokio::test]
async fn a_stale_if_version_is_a_conflict() {
    let mut store = MockMemoryDao::new();
    store
        .expect_put()
        .returning(|_| Ok(PutOutcome::VersionMismatch { current: Some(4) }));
    let req = PutMemoryRequest {
        if_version: Some(3),
        ..put_req("a")
    };
    let result = put(&tasks(), &memories(store), &ctx("user-1"), "task-1", req).await;
    assert!(matches!(result, Err(MemoryError::Conflict(m)) if m.contains("version 4")));
}

#[tokio::test]
async fn ttl_and_no_expiry_together_are_refused() {
    let req = PutMemoryRequest {
        ttl_seconds: Some(60),
        no_expiry: true,
        ..put_req("a")
    };
    let result = put(
        &tasks(),
        &memories(MockMemoryDao::new()),
        &ctx("user-1"),
        "task-1",
        req,
    )
    .await;
    assert!(matches!(result, Err(MemoryError::BadRequest(_))));
}

#[tokio::test]
async fn a_get_reads_one_scope_the_loop_by_default_and_records_the_read() {
    for (scope, expected) in [
        (
            Some(MemoryScope::Loop),
            ScopeRef {
                scope: MemoryScope::Loop,
                id: "loop-1".to_string(),
            },
        ),
        (
            None,
            ScopeRef {
                scope: MemoryScope::Loop,
                id: "loop-1".to_string(),
            },
        ),
        (
            Some(MemoryScope::Global),
            ScopeRef {
                scope: MemoryScope::Global,
                id: String::new(),
            },
        ),
    ] {
        let reads: Arc<Mutex<Vec<(String, i32)>>> = Arc::default();
        let mut store = MockMemoryDao::new();
        let want = expected.clone();
        store.expect_find().returning(move |scopes, key| {
            assert_eq!(scopes, std::slice::from_ref(&want));
            Ok(vec![row(key, want.scope, &want.id)])
        });
        let seen = reads.clone();
        store.expect_record_reads().returning(move |task_id, r| {
            assert_eq!(task_id, "task-1");
            seen.lock().map(|mut s| s.extend_from_slice(r)).ok();
            Ok(())
        });
        let got = get(
            &tasks(),
            &memories(store),
            &ctx("user-1"),
            "task-1",
            "state",
            scope,
        )
        .await
        .map_err(|e| e.to_string());
        let scopes: Option<Vec<MemoryScope>> =
            got.ok().map(|v| v.iter().map(|e| e.scope).collect());
        assert_eq!(scopes, Some(vec![expected.scope]));
        let recorded = reads.lock().map(|r| r.clone()).unwrap_or_default();
        assert_eq!(recorded, vec![(format!("{}-state", expected.scope), 3)]);
    }
}

#[tokio::test]
async fn a_reader_of_someone_elses_task_does_not_reach_the_owners_user_memory() {
    let params = ListParams {
        scope: Some(MemoryScope::User),
        ..ListParams::default()
    };
    let result = list(
        &tasks(),
        &memories(MockMemoryDao::new()),
        &ctx("user-1"),
        "task-public",
        params,
    )
    .await;
    assert!(matches!(result, Err(MemoryError::Forbidden(_))));
}

#[tokio::test]
async fn a_scoped_append_writes_only_the_entry_in_that_scope() {
    let mut store = MockMemoryDao::new();
    store.expect_find().returning(|scopes, key| {
        assert_eq!(
            scopes,
            [ScopeRef {
                scope: MemoryScope::Loop,
                id: "loop-1".to_string(),
            }]
        );
        Ok(vec![row(key, MemoryScope::Loop, "loop-1")])
    });
    store.expect_append().returning(|entry_id, _, _| {
        assert_eq!(entry_id, "loop-log");
        Ok(Some(row("log", MemoryScope::Loop, "loop-1")))
    });
    let req = AppendMemoryRequest {
        key: "log".to_string(),
        scope: Some(MemoryScope::Loop),
        text: "run 3".to_string(),
        summary: None,
    };
    let result = append(&tasks(), &memories(store), &ctx("user-1"), "task-1", req).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn a_claim_held_by_another_task_is_a_conflict() {
    let mut store = MockMemoryDao::new();
    store
        .expect_find()
        .returning(|_, key| Ok(vec![row(key, MemoryScope::Loop, "loop-1")]));
    store
        .expect_claim()
        .returning(|_, _, _| Ok(Some(ClaimOutcome::HeldBy("task-9".to_string()))));
    let req = ClaimMemoryRequest {
        key: "triage:SP-1".to_string(),
        scope: None,
        ttl_seconds: 3600,
    };
    let result = claim(&tasks(), &memories(store), &ctx("user-1"), "task-1", req).await;
    assert!(matches!(result, Err(MemoryError::Conflict(m)) if m.contains("task-9")));
}

#[tokio::test]
async fn a_claim_on_a_missing_key_creates_the_entry_first() {
    let puts = Arc::new(Mutex::new(Vec::new()));
    let mut store = store_missing_until_put(
        Arc::clone(&puts),
        PutOutcome::Written(Box::new(row("ticket:SP-1", MemoryScope::Loop, "loop-1"))),
    );
    store.expect_claim().times(1).returning(|entry_id, _, _| {
        assert_eq!(entry_id, "loop-ticket:SP-1");
        Ok(Some(ClaimOutcome::Claimed(Box::new(row(
            "ticket:SP-1",
            MemoryScope::Loop,
            "loop-1",
        )))))
    });
    let req = ClaimMemoryRequest {
        key: "ticket:SP-1".to_string(),
        scope: None,
        ttl_seconds: 3600,
    };
    let result = claim(&tasks(), &memories(store), &ctx("user-1"), "task-1", req).await;
    assert!(result.is_ok(), "{:?}", result.err());
    let puts = puts.lock().unwrap();
    assert_eq!(puts[0].summary.as_deref(), Some("ticket:SP-1"));
    assert_eq!(puts[0].if_version, Some(0));
}

#[test]
fn an_invalid_key_is_a_bad_request() {
    assert!(validate_key("triage:SP-123").is_ok());
    assert!(matches!(
        validate_key("has space"),
        Err(MemoryError::BadRequest(_))
    ));
}

#[test]
fn a_summary_is_one_short_line() {
    assert!(validate_summary("fine").is_ok());
    assert!(validate_summary("two\nlines").is_err());
    assert!(validate_summary(&"x".repeat(MAX_SUMMARY_CHARS + 1)).is_err());
}

/// A store with no `key` until its first `put`, recording each put.
fn store_missing_until_put(puts: Arc<Mutex<Vec<PutEntry>>>, put: PutOutcome) -> MockMemoryDao {
    let created = Arc::new(Mutex::new(false));
    let mut store = MockMemoryDao::new();
    let seen = Arc::clone(&created);
    store.expect_find().returning(move |_, key| {
        let found = *seen.lock().unwrap();
        Ok(if found {
            vec![row(key, MemoryScope::Loop, "loop-1")]
        } else {
            Vec::new()
        })
    });
    let put = Mutex::new(Some(put));
    store.expect_put().times(1).returning(move |entry| {
        puts.lock().unwrap().push(entry.clone());
        *created.lock().unwrap() = true;
        Ok(put.lock().unwrap().take().unwrap())
    });
    store
}

/// [`store_missing_until_put`], expecting one append.
fn store_creating_on_put(puts: Arc<Mutex<Vec<PutEntry>>>, put: PutOutcome) -> MockMemoryDao {
    let mut store = store_missing_until_put(puts, put);
    store
        .expect_append()
        .times(1)
        .returning(|entry_id, text, _| {
            // `row` names an entry `{scope}-{key}`: the one `find` returned.
            let key = entry_id.strip_prefix("loop-").expect("a found entry");
            let mut appended = row(key, MemoryScope::Loop, "loop-1");
            appended.body = text.to_string();
            Ok(Some(appended))
        });
    store
}

fn append_req(summary: Option<&str>) -> AppendMemoryRequest {
    append_req_for("admin:changes", summary)
}

fn append_req_for(key: &str, summary: Option<&str>) -> AppendMemoryRequest {
    AppendMemoryRequest {
        key: key.to_string(),
        scope: None,
        text: "a line".to_string(),
        summary: summary.map(str::to_string),
    }
}

#[tokio::test]
async fn an_append_to_a_missing_key_creates_the_entry_first() {
    let puts = Arc::new(Mutex::new(Vec::new()));
    let store = store_creating_on_put(
        Arc::clone(&puts),
        PutOutcome::Written(Box::new(row("admin:changes", MemoryScope::Loop, "loop-1"))),
    );
    let entry = append(
        &tasks(),
        &memories(store),
        &ctx("user-1"),
        "task-1",
        append_req(None),
    )
    .await
    .map_err(|e| e.to_string());
    assert_eq!(entry.map(|e| e.key), Ok("admin:changes".to_string()));
    let puts = puts.lock().unwrap();
    assert_eq!(puts[0].summary.as_deref(), Some("admin:changes"));
    assert_eq!(puts[0].if_version, Some(0));
    assert_eq!(puts[0].scope.scope, MemoryScope::Loop);
}

#[tokio::test]
async fn an_append_that_loses_the_race_to_create_appends_to_the_winner() {
    let puts = Arc::new(Mutex::new(Vec::new()));
    let store = store_creating_on_put(
        Arc::clone(&puts),
        PutOutcome::VersionMismatch { current: Some(1) },
    );
    let req = append_req(Some("Log of admin changes"));
    let result = append(&tasks(), &memories(store), &ctx("user-1"), "task-1", req).await;
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(
        puts.lock().unwrap()[0].summary.as_deref(),
        Some("Log of admin changes")
    );
}

#[tokio::test]
async fn an_append_creating_a_key_longer_than_a_summary_cuts_the_summary() {
    let puts = Arc::new(Mutex::new(Vec::new()));
    let store = store_creating_on_put(
        Arc::clone(&puts),
        PutOutcome::Written(Box::new(row("admin:changes", MemoryScope::Loop, "loop-1"))),
    );
    let key = format!("log:{}", "k".repeat(MAX_SUMMARY_CHARS));
    let req = append_req_for(&key, None);
    let result = append(&tasks(), &memories(store), &ctx("user-1"), "task-1", req).await;
    assert!(result.is_ok(), "{:?}", result.err());
    let summary = puts.lock().unwrap()[0].summary.clone().unwrap_or_default();
    assert_eq!(summary.chars().count(), MAX_SUMMARY_CHARS);
}
