use super::*;
use moatless_types::access_control::ScopeContext;

use crate::task::dao::MockTaskDao;
use crate::task::models::{SandboxDesiredState, SandboxStopReason, Visibility};
use moatless_types::store::StoreError;
use uuid::Uuid;

struct RecordingSandbox {
    log: Arc<Mutex<Vec<&'static str>>>,
    fail_removal: bool,
}

#[async_trait::async_trait]
impl SandboxLifecycle for RecordingSandbox {
    async fn start(&self, _task_id: &str) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
        unimplemented!("not reachable from delete_task ordering test")
    }

    async fn stop(
        &self,
        _task_id: &str,
        _reason: SandboxStopReason,
    ) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
        unimplemented!("not reachable from delete_task ordering test")
    }

    async fn stop_agent(&self, _task_id: &str, _reason: SandboxStopReason) {
        unimplemented!("not reachable from delete_task ordering test")
    }

    async fn remove(
        &self,
        _task_id: &str,
        _retention: ArchiveRetention,
    ) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
        self.log.lock().expect("log").push("remove");
        if self.fail_removal {
            return Err(moatless_ports::sandbox::SandboxOpError::Internal(
                "remove failed".to_string(),
            ));
        }
        Ok(())
    }

    async fn archive_storage(
        &self,
        _task_id: &str,
    ) -> Result<(), moatless_ports::sandbox::SandboxOpError> {
        unimplemented!("not reachable from delete_task ordering test")
    }

    async fn stopped_with_storage(
        &self,
        _task_ids: &[String],
    ) -> Result<
        std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
        moatless_ports::sandbox::SandboxOpError,
    > {
        unimplemented!("not reachable from delete_task ordering test")
    }

    async fn desired_state(
        &self,
        _task_id: &str,
    ) -> Result<SandboxDesiredState, moatless_ports::sandbox::SandboxOpError> {
        unimplemented!("not reachable from delete_task ordering test")
    }
}

/// Fails every read fast, so a close's best-effort disposal only logs.
fn unreachable_inbox() -> crate::inbox::InboxService {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(100))
        .connect_lazy("postgres://inbox:inbox@127.0.0.1:1/inbox")
        .expect("connect_lazy only parses the URL");
    crate::inbox::InboxService::new(pool)
}

/// `TaskBindings` fake answering only `deactivate_for_task`: records
/// requested task ids and replays a queued outcome — everything
/// [`deactivate_task_bindings`] can observe. Other methods panic.
struct FakeBindings {
    outcome: Mutex<std::collections::VecDeque<Result<u64, StoreError>>>,
    deactivated: Mutex<Vec<String>>,
}

impl FakeBindings {
    fn returning(outcome: Result<u64, StoreError>) -> Self {
        Self {
            outcome: Mutex::new([outcome].into()),
            deactivated: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.deactivated.lock().expect("fake poisoned").clone()
    }
}

#[async_trait::async_trait]
impl TaskBindings for FakeBindings {
    async fn deactivate_for_task(&self, task_id: &str) -> Result<u64, StoreError> {
        self.deactivated
            .lock()
            .expect("fake poisoned")
            .push(task_id.to_string());
        self.outcome
            .lock()
            .expect("fake poisoned")
            .pop_front()
            .expect("deactivate_for_task called more times than the test queued outcomes")
    }

    async fn summaries(&self, _task_id: &str) -> Result<Vec<TaskBindingSummary>, StoreError> {
        unimplemented!("not reachable from deactivate_task_bindings")
    }

    async fn list_summaries(
        &self,
        _task_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<TaskBindingSummary>>, StoreError> {
        unimplemented!("not reachable from deactivate_task_bindings")
    }
}

/// The close/delete cleanup reaches `task_bindings` only through the
/// injected port — no pool, no concrete repository.
#[tokio::test]
async fn deactivate_task_bindings_goes_through_the_injected_port() {
    let bindings = FakeBindings::returning(Ok(2));

    deactivate_task_bindings(&bindings, "task-1").await;

    assert_eq!(bindings.calls(), vec!["task-1".to_string()]);
}

/// Deactivation is best-effort: a repository failure is logged, not
/// propagated, so closing or deleting a task still completes.
#[tokio::test]
async fn deactivate_task_bindings_swallows_a_repository_error() {
    let bindings = FakeBindings::returning(Err(StoreError::new("pool closed")));

    deactivate_task_bindings(&bindings, "task-2").await;

    assert_eq!(bindings.calls(), vec!["task-2".to_string()]);
}

#[tokio::test]
async fn deleting_a_parent_removes_its_side_chat_before_the_parent() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_find_by_id().returning(|id, ctx| {
        if id == "side" {
            assert!(ctx.is_admin);
        }
        Ok(Some(super::super::test_support::make_task_row(
            id,
            if id == "parent" { "owner" } else { "other" },
        )))
    });
    tasks.expect_side_chat_ids().returning(|id| {
        Ok(if id == "parent" {
            vec!["side".into()]
        } else {
            vec![]
        })
    });
    tasks
        .expect_update_task()
        .times(2)
        .returning(|_, _, _| Ok(true));
    let deleted = Arc::new(Mutex::new(Vec::new()));
    let captured = deleted.clone();
    tasks.expect_delete_task().times(2).returning(move |id, _| {
        captured.lock().unwrap().push(id.to_string());
        Ok(true)
    });
    let bindings = FakeBindings::returning(Ok(0));
    bindings.outcome.lock().unwrap().push_back(Ok(0));
    let sandbox = RecordingSandbox {
        log: Arc::new(Mutex::new(Vec::new())),
        fail_removal: false,
    };
    super::super::removal::delete_task(
        &sandbox,
        &bindings,
        &TaskCommands::new(Arc::new(tasks)),
        &artifacts(None),
        &ScopeContext::user("owner"),
        "parent".into(),
    )
    .await
    .unwrap();
    assert_eq!(*deleted.lock().unwrap(), ["side", "parent"]);
    assert_eq!(bindings.calls(), ["side", "parent"]);
}

#[tokio::test]
async fn closing_a_parent_closes_side_chats_but_keeps_ordinary_forks() {
    use super::super::relations::{MockTaskRelationDao, TaskRelation, TaskRelationType};
    let mut tasks = MockTaskDao::new();
    tasks
        .expect_find_by_id()
        .returning(|id, _| Ok(Some(super::super::test_support::make_task_row(id, "owner"))));
    let closed = Arc::new(Mutex::new(Vec::new()));
    let captured = closed.clone();
    tasks
        .expect_update_task()
        .times(2)
        .returning(move |id, _, _| {
            captured.lock().unwrap().push(id.to_string());
            Ok(true)
        });
    let mut relations = MockTaskRelationDao::new();
    relations
        .expect_list_incoming()
        .withf(|id, kind| id == "parent" && *kind == Some(TaskRelationType::ForkedFrom))
        .once()
        .returning(|_, _| {
            Ok(vec![
                TaskRelation::new(
                    "side",
                    "parent",
                    TaskRelationType::ForkedFrom,
                    serde_json::json!({"side_chat": true}),
                ),
                TaskRelation::new(
                    "ordinary",
                    "parent",
                    TaskRelationType::ForkedFrom,
                    serde_json::json!({}),
                ),
            ])
        });
    let bindings = FakeBindings::returning(Ok(0));
    bindings.outcome.lock().unwrap().push_back(Ok(0));
    let closer = TaskCloser::new(
        Arc::new(tasks),
        Arc::new(relations),
        Arc::new(moatless_core::config::Config::test_minimal()),
        Arc::new(RecordingSandbox {
            log: Arc::new(Mutex::new(Vec::new())),
            fail_removal: false,
        }),
        Arc::new(bindings),
        None,
        unreachable_inbox(),
    );
    closer
        .close_for(&ScopeContext::user("owner"), "parent")
        .await
        .unwrap();
    assert_eq!(*closed.lock().unwrap(), ["side", "parent"]);
}

#[tokio::test]
async fn delete_closes_task_before_lifecycle_remove() {
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = MockTaskDao::new();
    expect_owned_row(&mut tasks, "00000000-0000-0000-0000-000000000001");
    let update_log = Arc::clone(&log);
    tasks
        .expect_update_task()
        .withf(|task_id, patch, _ctx| {
            task_id == "task-1" && patch.status == Some(TaskStatus::Closed)
        })
        .once()
        .returning(move |_, _, _| {
            update_log.lock().expect("log").push("close");
            Ok(true)
        });
    let delete_log = Arc::clone(&log);
    tasks
        .expect_delete_task()
        .withf(|task_id, _ctx| task_id == "task-1")
        .once()
        .returning(move |_, _| {
            delete_log.lock().expect("log").push("delete");
            Ok(true)
        });

    let status = delete_task(
        State(Arc::new(RecordingSandbox {
            log: Arc::clone(&log),
            fail_removal: false,
        }) as Arc<dyn SandboxLifecycle>),
        State(Arc::new(FakeBindings::returning(Ok(0))) as Arc<dyn TaskBindings>),
        State(Arc::new(TaskCommands::new(Arc::new(tasks)))),
        State(Arc::new(artifacts(None))),
        AuthUser {
            user_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid"),
            login: "user-1".to_string(),
            role: None,
            provider: None,
            group_id: None,
            task_id: None,
            is_agent_context: false,
        },
        Path("task-1".to_string()),
    )
    .await
    .expect("delete succeeds");

    assert_eq!(status, axum::http::StatusCode::NO_CONTENT);
    assert_eq!(*log.lock().expect("log"), vec!["close", "remove", "delete"]);
}

#[tokio::test]
async fn failed_removal_retains_the_closed_task_row() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = MockTaskDao::new();
    expect_owned_row(&mut tasks, "owner");
    let updated = log.clone();
    tasks
        .expect_update_task()
        .once()
        .withf(|_, patch, _| patch.status == Some(TaskStatus::Closed))
        .returning(move |_, _, _| {
            updated.lock().unwrap().push("close");
            Ok(true)
        });
    let sandbox = RecordingSandbox {
        log: log.clone(),
        fail_removal: true,
    };
    let bindings = FakeBindings::returning(Ok(1));
    let ctx = moatless_types::access_control::ScopeContext::user("owner");
    let tasks = TaskCommands::new(Arc::new(tasks));
    let result = super::super::removal::delete_task(
        &sandbox,
        &bindings,
        &tasks,
        &artifacts(None),
        &ctx,
        "task-1".to_string(),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(*log.lock().unwrap(), vec!["close", "remove"]);
    assert_eq!(*bindings.deactivated.lock().unwrap(), vec!["task-1"]);
}

/// `Artifacts` over `endpoint`'s object storage, or over none.
fn artifacts(endpoint: Option<&str>) -> Artifacts {
    let s3 = endpoint.map(|endpoint| {
        moatless_s3::S3Storage::at_endpoint(
            endpoint,
            "us-east-1",
            moatless_s3::S3Credentials {
                access_key_id: "test".to_string(),
                secret_access_key: "test".to_string(),
                session_token: None,
            },
        )
    });
    Artifacts::new(
        Arc::new(crate::artifact::dao::MockArtifactDao::new()),
        s3,
        Arc::new(moatless_core::config::S3Config {
            bucket_name: Some("bucket".to_string()),
            key_prefix: "uploads".to_string(),
            transcript_prefix: "transcripts".to_string(),
            workspace_prefix: "workspaces".to_string(),
            artifact_prefix: "artifacts".to_string(),
            region: "us-east-1".to_string(),
        }),
        "https://moatless.example",
        None,
        Arc::new(crate::collection::test_support::records(
            crate::collection::dao::MockRecordDao::new(),
        )),
    )
}

/// An S3 endpoint answering each request with the next of `responses`
/// (status, body). Returns its URL and the request line plus body of
/// every request it answered.
async fn s3_endpoint(
    responses: Vec<(&'static str, &'static str)>,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.expect("read"));
            }
            let head = String::from_utf8_lossy(&request).to_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map_or(0, |length| length.trim().parse::<usize>().expect("length"));
            let mut payload = vec![0; length];
            socket.read_exact(&mut payload).await.expect("read body");
            let line = head.lines().next().unwrap_or_default().to_string();
            requests.push(format!("{line}\n{}", String::from_utf8_lossy(&payload)));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.expect("write");
        }
        requests
    });
    (url, server)
}

/// `find_by_id` answering with a private row `owner` owns.
fn expect_owned_row(tasks: &mut MockTaskDao, owner: &'static str) {
    tasks.expect_side_chat_ids().returning(|_| Ok(Vec::new()));
    tasks.expect_find_by_id().returning(move |task_id, _| {
        Ok(Some(super::super::test_support::make_task_row(
            task_id, owner,
        )))
    });
}

fn deletable_tasks() -> TaskCommands {
    let mut tasks = MockTaskDao::new();
    expect_owned_row(&mut tasks, "owner");
    tasks
        .expect_update_task()
        .once()
        .returning(|_, _, _| Ok(true));
    tasks.expect_delete_task().once().returning(|_, _| Ok(true));
    TaskCommands::new(Arc::new(tasks))
}

#[tokio::test]
async fn delete_removes_the_task_artifact_pages() {
    let (url, server) = s3_endpoint(vec![
        (
            "200 OK",
            "<ListBucketResult><IsTruncated>false</IsTruncated>\
             <Contents><Key>artifacts/task-1/art-1/v1.html</Key></Contents>\
             </ListBucketResult>",
        ),
        ("200 OK", "<DeleteResult/>"),
    ])
    .await;
    let sandbox = RecordingSandbox {
        log: Arc::new(Mutex::new(Vec::new())),
        fail_removal: false,
    };

    super::super::removal::delete_task(
        &sandbox,
        &FakeBindings::returning(Ok(0)),
        &deletable_tasks(),
        &artifacts(Some(&url)),
        &moatless_types::access_control::ScopeContext::user("owner"),
        "task-1".to_string(),
    )
    .await
    .expect("delete succeeds");

    let requests = server.await.expect("endpoint");
    assert!(
        requests[0].contains("prefix=artifacts%2ftask-1%2f"),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("post /bucket/?delete"),
        "{}",
        requests[1]
    );
    assert!(
        requests[1].contains("<Key>artifacts/task-1/art-1/v1.html</Key>"),
        "{}",
        requests[1]
    );
}

/// A storage outage leaves the pages orphaned rather than the Task
/// undeletable.
#[tokio::test]
async fn delete_completes_when_artifact_storage_fails() {
    let (url, server) = s3_endpoint(vec![("403 Forbidden", "")]).await;
    let sandbox = RecordingSandbox {
        log: Arc::new(Mutex::new(Vec::new())),
        fail_removal: false,
    };

    super::super::removal::delete_task(
        &sandbox,
        &FakeBindings::returning(Ok(0)),
        &deletable_tasks(),
        &artifacts(Some(&url)),
        &moatless_types::access_control::ScopeContext::user("owner"),
        "task-1".to_string(),
    )
    .await
    .expect("delete succeeds despite the storage failure");

    assert_eq!(server.await.expect("endpoint").len(), 1);
}

/// A public Task's visitor may write to it but not delete it, and is
/// refused before the close and the Sandbox removal, not after.
#[tokio::test]
async fn a_visitor_cannot_delete_a_public_task() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_find_by_id().returning(|task_id, _| {
        let mut row = super::super::test_support::make_task_row(task_id, "owner");
        row.visibility = Visibility::Public;
        Ok(Some(row))
    });
    tasks.expect_update_task().never();
    tasks.expect_delete_task().never();
    let log = Arc::new(Mutex::new(Vec::new()));
    let sandbox = RecordingSandbox {
        log: log.clone(),
        fail_removal: false,
    };

    let result = super::super::removal::delete_task(
        &sandbox,
        &FakeBindings::returning(Ok(0)),
        &TaskCommands::new(Arc::new(tasks)),
        &artifacts(None),
        &moatless_types::access_control::ScopeContext::user("visitor"),
        "task-1".to_string(),
    )
    .await;

    assert!(matches!(result, Err(TaskError::Forbidden(_))));
    assert!(log.lock().unwrap().is_empty());
}
