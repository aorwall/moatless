use super::*;
use std::sync::Mutex;

use crate::tag::dao::MockTagDao;
use crate::task::dao::MockTaskDao;
use crate::task::relations::MockTaskRelationDao;
use crate::task::repositories::MockTaskRepositoryDao;
use async_trait::async_trait;
use moatless_core::events::test_support::RecordingPublisher;
use moatless_ports::sandbox::{
    MockPreflightPort, MockSandboxJoiner, PreflightRejection, SandboxError,
};
use moatless_types::task::CheckoutMode;
use moatless_workspace_ports::{MockRepositoryReader, MockWorkspaceResolver};
use moatless_workspace_types::repository::RepositoryKind;
use moatless_workspaces::repository::models::Repository;

mockall::mock! {
    Starter {}
    #[async_trait]
    impl SandboxStarter for Starter {
        async fn start(&self, task_id: &str) -> Result<(), SandboxError>;
    }
}

const WORKSPACE: &str = "ws_repo1";

fn sample_repo() -> Repository {
    Repository {
        id: "repo1".into(),
        name: "repo".into(),
        kind: RepositoryKind::Remote,
        remote: None,
        default_branch: None,
        description: None,
        provider: None,
        scope: Scope::User,
        owner_user_id: None,
        created_by: None,
        deleted: false,
        created_at: None,
        updated_at: None,
    }
}

fn sample_request() -> NewTaskRequest {
    NewTaskRequest {
        sandbox_placement: crate::task::models::SandboxPlacement::default(),
        repository_id: Some("repo1".into()),
        agent_profile: AgentProfile::ClaudeCode,
        name: Some("t".into()),
        description: None,
        branch: None,
        model: None,
        created_by: Some("user1".into()),
        reporter_user_id: None,
        owner_user_id: None,
        run_as_user_id: None,
        headless: false,
        visibility: Visibility::Private,
        loop_id: None,
        task_id: None,
        provision: true,
        workspace_id: WORKSPACE.into(),
        sandbox_id: None,
        checkout_mode: None,
        template_id: None,
        vibe: None,
        append_system_prompt: None,
        ttl_minutes: 15,
        close_after_settled_days: 7,
        settle_after_hours: None,
        tag_ids: vec![],
        memory_preload: Vec::new(),
        skills: vec![],
        parent: None,
        related_to: None,
        forked_from: None,
        first_message: None,
        event_subtype: TaskSubType::Created,
        repository: None,
    }
}

/// Preflight that always passes.
fn preflight_ok() -> Arc<dyn PreflightPort> {
    let mut m = MockPreflightPort::new();
    m.expect_validate().returning(|_, _| Ok(()));
    Arc::new(m)
}

async fn empty_payload(_: Task) -> serde_json::Value {
    serde_json::json!({})
}

async fn no_message(_: String, _: FirstMessage) -> Result<(), MessageError> {
    Ok(())
}

fn first_message() -> FirstMessage {
    FirstMessage {
        text: "hello".into(),
        model: None,
        agent_mode: None,
        effort: None,
        file_ids: vec!["file-1".into()],
        contexts: vec![],
        sent_by: Some("user1".into()),
        message_uuid: None,
        skills: vec![],
        source_task_id: None,
    }
}

fn service(
    tasks: MockTaskDao,
    starter: MockStarter,
    joiner: MockSandboxJoiner,
    preflight: Arc<dyn PreflightPort>,
    publisher: Arc<RecordingPublisher>,
) -> TaskCreationService {
    service_with(
        tasks,
        joiner,
        MockTaskRelationDao::new(),
        crate::task::fork::MockTaskForkDao::new(),
        starter,
        preflight,
        publisher,
    )
}

fn service_with(
    tasks: MockTaskDao,
    joiner: MockSandboxJoiner,
    relations: MockTaskRelationDao,
    forks: crate::task::fork::MockTaskForkDao,
    starter: MockStarter,
    preflight: Arc<dyn PreflightPort>,
    publisher: Arc<RecordingPublisher>,
) -> TaskCreationService {
    let tags = MockTagDao::new();
    let mut repos = MockRepositoryReader::new();
    repos
        .expect_describe()
        .returning(|_, _| Ok(Some(sample_repo())));
    let mut workspaces = MockWorkspaceResolver::new();
    workspaces.expect_placements().returning(|_| Ok(vec![]));
    let mut task_repositories = MockTaskRepositoryDao::new();
    task_repositories
        .expect_record()
        .returning(|_, _, _| Ok(()));
    task_repositories.expect_list().returning(|_| Ok(vec![]));
    TaskCreationService::new(
        Arc::new(tasks),
        Arc::new(tags),
        Arc::new(relations),
        Arc::new(task_repositories),
        Arc::new(forks),
        Arc::new(repos),
        Arc::new(workspaces),
        Arc::new(moatless_ports::identity::MockUserResolver::new()),
        Some(publisher as Arc<dyn EventPublisher>),
        Arc::new(starter),
        Arc::new(joiner),
        preflight,
    )
}

fn joiner_never() -> MockSandboxJoiner {
    let mut joiner = MockSandboxJoiner::new();
    joiner.expect_join_task().never();
    joiner
}

#[tokio::test]
async fn happy_path_inserts_open_row_and_publishes_created_event() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| row.status == TaskStatus::Open)
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().times(1).returning(|_| Ok(()));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        publisher.clone(),
    );

    let outcome = svc
        .create(sample_request(), empty_payload, no_message)
        .await
        .expect("creation succeeds");

    assert_eq!(outcome.provision, ProvisionOutcome::Started);
    assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
    assert!(publisher.sandbox_statuses().is_empty());
}

/// The create response and the `task.created` event are built from this row
/// rather than re-read, so a row left with `None` here publishes a task
/// whose workspace no client can name.
#[tokio::test]
async fn the_persisted_and_returned_rows_carry_the_requested_workspace() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| row.workspace_id.as_deref() == Some(WORKSPACE))
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().returning(|_| Ok(()));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        publisher.clone(),
    );

    let outcome = svc
        .create(sample_request(), empty_payload, no_message)
        .await
        .expect("creation succeeds");

    assert_eq!(outcome.task.workspace_id.as_deref(), Some(WORKSPACE));
}

/// A global-scoped loop fires as its bot user; the spawned task must still
/// be `Scope::User` owned by that bot — the pod authenticates as
/// `created_by`, and the gate refuses a global or ownerless row.
#[tokio::test]
async fn a_task_defaults_to_user_scope_and_ownership_by_its_creator() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| row.scope == Scope::User && row.owner_user_id.as_deref() == Some("bot1"))
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().returning(|_| Ok(()));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);
    let mut req = sample_request();
    req.created_by = Some("bot1".into());

    let outcome = svc
        .create(req, empty_payload, no_message)
        .await
        .expect("creation succeeds");

    assert_eq!(outcome.task.scope, Scope::User);
    assert_eq!(outcome.task.owner_user_id.as_deref(), Some("bot1"));
}

#[tokio::test]
async fn a_loop_task_keeps_its_creator_and_reporter_without_changing_the_authorized_owner() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| {
            row.created_by.as_deref() == Some("bot1")
                && row.reporter_user_id.as_deref() == Some("user1")
                && row.owner_user_id.as_deref() == Some("user1")
                && row.loop_id.as_deref() == Some("loop1")
        })
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().returning(|_| Ok(()));
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        Arc::new(RecordingPublisher::new()),
    );
    let mut req = sample_request();
    req.created_by = Some("bot1".into());
    req.reporter_user_id = Some("user1".into());
    req.owner_user_id = Some("user1".into());
    req.run_as_user_id = Some("bot1".into());
    req.loop_id = Some("loop1".into());

    let outcome = svc.create(req, empty_payload, no_message).await.unwrap();

    assert_eq!(outcome.task.created_by.as_deref(), Some("bot1"));
    assert_eq!(outcome.task.reporter_user_id.as_deref(), Some("user1"));
    assert_eq!(outcome.task.owner_user_id.as_deref(), Some("user1"));
}

#[tokio::test]
async fn task_creation_persists_requested_model_on_the_task() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| row.model.as_deref() == Some("claude-opus-4-6"))
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().returning(|_| Ok(()));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);
    let mut req = sample_request();
    req.model = Some("claude-opus-4-6".to_string());

    let outcome = svc
        .create(req, empty_payload, no_message)
        .await
        .expect("creation succeeds");

    assert_eq!(outcome.task.model.as_deref(), Some("claude-opus-4-6"));
}

#[tokio::test]
async fn first_message_is_sent_before_provisioning() {
    let order = Arc::new(Mutex::new(Vec::new()));

    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));

    let mut starter = MockStarter::new();
    let start_order = Arc::clone(&order);
    starter.expect_start().times(1).returning(move |_| {
        start_order.lock().expect("order lock").push("start");
        Ok(())
    });

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);

    let mut req = sample_request();
    req.first_message = Some(FirstMessage {
        text: "hello".into(),
        model: None,
        agent_mode: None,
        effort: None,
        file_ids: vec![],
        contexts: vec![],
        sent_by: Some("user1".into()),
        message_uuid: None,
        skills: vec![],
        source_task_id: None,
    });

    let message_order = Arc::clone(&order);
    svc.create(req, empty_payload, move |_, _| {
        let message_order = Arc::clone(&message_order);
        async move {
            message_order.lock().expect("order lock").push("message");
            Ok(())
        }
    })
    .await
    .expect("creation succeeds");

    assert_eq!(
        order.lock().expect("order lock").as_slice(),
        ["message", "start"]
    );
}

#[tokio::test]
async fn a_refused_first_message_deletes_the_task_and_never_provisions() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    tasks
        .expect_delete_inserted_task()
        .times(1)
        .returning(|_| Ok(true));

    let mut starter = MockStarter::new();
    starter.expect_start().never();

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);

    let mut req = sample_request();
    req.first_message = Some(first_message());

    let err = svc
        .create(req, empty_payload, |_, _| async {
            Err(MessageError::BadRequest(
                "No uploaded file for file-1".into(),
            ))
        })
        .await
        .expect_err("a task with no brief is not created");

    assert!(matches!(
        TaskError::from(err),
        TaskError::FirstMessage(MessageError::BadRequest(_))
    ));
}

/// The message row is committed by the time the clock advance fails, and
/// the inbox has no foreign key to `tasks` — so deleting the Task here
/// would leave it behind with nothing naming it.
#[tokio::test]
async fn a_first_message_that_failed_after_persisting_marks_the_task_errored() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    tasks.expect_delete_inserted_task().never();
    tasks
        .expect_update_status_if_not_closed()
        .withf(|_, status, _| *status == TaskStatus::Error)
        .times(1)
        .returning(|_, _, _| Ok(true));

    let mut starter = MockStarter::new();
    starter.expect_start().never();

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        publisher.clone(),
    );

    let mut req = sample_request();
    req.first_message = Some(first_message());

    let err = svc
        .create(req, empty_payload, |_, _| async {
            Err(MessageError::AdvanceTaskClock(sqlx::Error::PoolClosed))
        })
        .await
        .expect_err("a task whose brief did not land is not created");

    assert!(matches!(err, TaskCreationError::FirstMessage(_)));
    assert_eq!(publisher.sandbox_statuses(), vec![SandboxStatus::Error]);
}

/// A `NotReady` join left no pod holding this Task, so a refusal after it
/// undoes the insert as cleanly as one with no join at all.
#[tokio::test]
async fn a_refused_first_message_after_a_deferred_join_still_deletes_the_task() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    tasks
        .expect_delete_inserted_task()
        .times(1)
        .returning(|_| Ok(true));
    tasks.expect_update_status_if_not_closed().never();

    let mut starter = MockStarter::new();
    starter.expect_start().never();
    let mut joiner = MockSandboxJoiner::new();
    joiner
        .expect_join_task()
        .times(1)
        .returning(|_, _, _| Err(SandboxError::NotReady("Sandbox is stopped".to_string())));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner, preflight_ok(), publisher);

    let mut req = sample_request();
    req.sandbox_id = Some("sandbox-1".to_string());
    req.first_message = Some(first_message());

    let err = svc
        .create(req, empty_payload, |_, _| async {
            Err(MessageError::BadRequest(
                "No uploaded file for file-1".into(),
            ))
        })
        .await
        .expect_err("a task with no brief is not created");

    assert!(matches!(err, TaskCreationError::FirstMessage(_)));
}

/// The shared pod registered this Task's checkout when the join landed, and
/// nothing unregisters it — so the Task stays, errored, rather than the API
/// reporting a Task that never existed while the pod still runs its context.
#[tokio::test]
async fn a_refused_first_message_after_a_join_marks_the_task_errored() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    tasks.expect_delete_inserted_task().never();
    tasks
        .expect_update_status_if_not_closed()
        .withf(|_, status, _| *status == TaskStatus::Error)
        .times(1)
        .returning(|_, _, _| Ok(true));

    let mut starter = MockStarter::new();
    starter.expect_start().never();
    let mut joiner = MockSandboxJoiner::new();
    joiner
        .expect_join_task()
        .times(1)
        .returning(|_, _, _| Ok(()));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner, preflight_ok(), publisher);

    let mut req = sample_request();
    req.sandbox_id = Some("sandbox-1".to_string());
    req.first_message = Some(first_message());

    let err = svc
        .create(req, empty_payload, |_, _| async {
            Err(MessageError::BadRequest(
                "No uploaded file for file-1".into(),
            ))
        })
        .await
        .expect_err("a task with no brief is not created");

    assert!(matches!(err, TaskCreationError::FirstMessage(_)));
}

#[tokio::test]
async fn preflight_failure_marks_errored_publishes_error_event_and_skips_provision() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| row.status == TaskStatus::Error)
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().never();

    let mut preflight = MockPreflightPort::new();
    preflight.expect_validate().returning(|_, _| {
        Err(PreflightRejection {
            user_message: "GitHub access is not configured for this repository scope.".into(),
            detail: "GitHub account not connected for user u".into(),
            throttled: false,
        })
    });

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        Arc::new(preflight),
        publisher.clone(),
    );

    let err = svc
        .create(sample_request(), empty_payload, no_message)
        .await
        .expect_err("preflight failure surfaces");

    assert!(matches!(
        err,
        TaskCreationError::Preflight {
            throttled: false,
            ..
        }
    ));
    assert_eq!(publisher.sandbox_statuses(), vec![SandboxStatus::Error]);
    assert!(publisher.task_subtypes().is_empty());
}

/// A rate-limited check can pass after the reset, so HTTP answers it 503,
/// not the 403 of a refusal that waiting cannot clear.
#[test]
fn a_throttled_preflight_is_temporarily_unavailable_over_http() {
    use axum::response::IntoResponse;

    let throttled = TaskError::from(TaskCreationError::Preflight {
        message: "rate-limited".into(),
        throttled: true,
    });
    let refused = TaskError::from(TaskCreationError::Preflight {
        message: "no access".into(),
        throttled: false,
    });

    assert_eq!(
        throttled.into_response().status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        refused.into_response().status(),
        axum::http::StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn provision_failure_still_succeeds_and_publishes_sandbox_error() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| row.status == TaskStatus::Open)
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter
        .expect_start()
        .times(1)
        .returning(|_| Err(SandboxError::Other("boom".into())));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        publisher.clone(),
    );

    let outcome = svc
        .create(sample_request(), empty_payload, no_message)
        .await
        .expect("creation still succeeds");

    assert_eq!(outcome.provision, ProvisionOutcome::Failed);
    assert_eq!(publisher.sandbox_statuses(), vec![SandboxStatus::Error]);
    assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
}

#[tokio::test]
async fn short_id_retries_on_collision() {
    let mut tasks = MockTaskDao::new();
    let mut seq = mockall::Sequence::new();
    tasks
        .expect_exists_by_short_id()
        .times(1)
        .in_sequence(&mut seq)
        .returning(|_| Ok(true));
    tasks
        .expect_exists_by_short_id()
        .times(1)
        .in_sequence(&mut seq)
        .returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().returning(|_| Ok(()));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        publisher.clone(),
    );

    let outcome = svc
        .create(sample_request(), empty_payload, no_message)
        .await
        .expect("creation succeeds after retry");

    assert_eq!(outcome.provision, ProvisionOutcome::Started);
}

/// T3 mints the Thread id, navigates to it, and keys its own state on it.
/// Minting our own here would leave the client watching a Thread that never
/// arrives.
#[tokio::test]
async fn a_supplied_task_id_is_used_verbatim() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| row.task_id == "3f1c9d42-0a7e-4b19-8c55-11d2ab6e7f30")
        .times(1)
        .returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().returning(|_| Ok(()));

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        publisher.clone(),
    );

    let mut req = sample_request();
    req.task_id = Some("3f1c9d42-0a7e-4b19-8c55-11d2ab6e7f30".into());
    let outcome = svc
        .create(req, empty_payload, no_message)
        .await
        .expect("creation succeeds");

    assert_eq!(outcome.task.task_id, "3f1c9d42-0a7e-4b19-8c55-11d2ab6e7f30");
}

/// A Thread created without a first message is a Task nobody has spoken to
/// yet. Provisioning it would burn a Sandbox on a Thread that may never be
/// used.
#[tokio::test]
async fn provisioning_is_skipped_when_the_request_does_not_ask_for_it() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    let mut starter = MockStarter::new();
    starter.expect_start().never();

    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(
        tasks,
        starter,
        joiner_never(),
        preflight_ok(),
        publisher.clone(),
    );

    let mut req = sample_request();
    req.provision = false;
    let outcome = svc
        .create(req, empty_payload, no_message)
        .await
        .expect("creation succeeds");

    assert_eq!(outcome.provision, ProvisionOutcome::Skipped);
    assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
    assert!(publisher.sandbox_statuses().is_empty());
}

#[tokio::test]
async fn shared_sandbox_create_joins_before_first_message_and_skips_provisioning() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks
        .expect_insert_task()
        .withf(|row, _| {
            row.sandbox_id.as_deref() == Some("sandbox-1")
                && row.checkout_mode == Some(CheckoutMode::Shared)
        })
        .times(1)
        .returning(|_, _| Ok(()));

    let mut starter = MockStarter::new();
    starter.expect_start().never();
    let mut joiner = MockSandboxJoiner::new();
    let join_order = Arc::clone(&order);
    joiner
        .expect_join_task()
        .withf(|task_id, checkout_mode, agent_profile| {
            !task_id.is_empty()
                && *checkout_mode == CheckoutMode::Shared
                // The joining Task's own harness, not the shared pod's.
                && *agent_profile == AgentProfile::ClaudeCode
        })
        .times(1)
        .returning(move |_, _, _| {
            join_order.lock().expect("order lock").push("join");
            Ok(())
        });
    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner, preflight_ok(), publisher.clone());

    let mut req = sample_request();
    req.sandbox_id = Some("sandbox-1".to_string());
    req.first_message = Some(FirstMessage {
        text: "hello".into(),
        model: None,
        agent_mode: None,
        effort: None,
        file_ids: vec![],
        contexts: vec![],
        sent_by: Some("user1".into()),
        message_uuid: None,
        skills: vec![],
        source_task_id: None,
    });

    let message_order = Arc::clone(&order);
    let outcome = svc
        .create(req, empty_payload, move |_, _| {
            let message_order = Arc::clone(&message_order);
            async move {
                message_order.lock().expect("order lock").push("message");
                Ok(())
            }
        })
        .await
        .expect("shared sandbox creation succeeds");

    assert_eq!(outcome.provision, ProvisionOutcome::Started);
    assert_eq!(publisher.task_subtypes(), vec![TaskSubType::Created]);
    assert_eq!(
        order.lock().expect("order lock").as_slice(),
        ["join", "message"]
    );
}

/// Creates `child-1` into `sandbox-1` under `parent-1`, whose own Sandbox is
/// `parent_sandbox`, expecting `cuts` cuts at the parent's head.
async fn create_child(parent_sandbox: &'static str, cuts: usize) {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().returning(|_, _| Ok(()));
    tasks
        .expect_find_by_id()
        .withf(|task_id, _| task_id == "parent-1")
        .returning(move |task_id, _| {
            let mut parent = crate::task::test_support::make_task_row(task_id, "user1");
            parent.sandbox_id = Some(parent_sandbox.to_string());
            Ok(Some(parent))
        });
    let mut joiner = MockSandboxJoiner::new();
    joiner.expect_join_task().returning(|_, _, _| Ok(()));
    let mut relations = MockTaskRelationDao::new();
    relations.expect_insert().returning(|_| Ok(()));
    let mut forks = crate::task::fork::MockTaskForkDao::new();
    forks
        .expect_insert_at_head()
        .withf(|task_id, source| task_id == "child-1" && source == "parent-1")
        .times(cuts)
        .returning(|_, _| Ok(true));
    let mut starter = MockStarter::new();
    starter.expect_start().never();
    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service_with(
        tasks,
        joiner,
        relations,
        forks,
        starter,
        preflight_ok(),
        publisher,
    );

    let mut req = sample_request();
    req.task_id = Some("child-1".to_string());
    req.sandbox_id = Some("sandbox-1".to_string());
    req.parent = Some(CreatedByParent {
        parent_task_id: "parent-1".to_string(),
        context: serde_json::json!({ "via": "create_task_request" }),
    });
    svc.create(req, empty_payload, |_, _| async { Ok(()) })
        .await
        .expect("child creation succeeds");
}

#[tokio::test]
async fn a_child_joining_its_parents_sandbox_is_cut_at_the_parents_head() {
    create_child("sandbox-1", 1).await;
}

#[tokio::test]
async fn a_child_joining_another_sandbox_is_not_cut() {
    create_child("sandbox-2", 0).await;
}

#[tokio::test]
async fn shared_sandbox_join_failure_cleans_up_before_first_message() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    let cleanup_order = Arc::clone(&order);
    tasks
        .expect_delete_inserted_task()
        .withf(|task_id| !task_id.is_empty())
        .times(1)
        .returning(move |_| {
            cleanup_order.lock().expect("order lock").push("cleanup");
            Ok(true)
        });

    let mut starter = MockStarter::new();
    starter.expect_start().never();
    let mut joiner = MockSandboxJoiner::new();
    let join_order = Arc::clone(&order);
    joiner
        .expect_join_task()
        .times(1)
        .returning(move |_, _, _| {
            join_order.lock().expect("order lock").push("join");
            // A genuine join failure — not `NotReady`, which defers to a start
            // (covered by `shared_sandbox_join_not_ready_starts_the_shared_sandbox`).
            Err(SandboxError::GrpcClient("connection refused".to_string()))
        });
    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner, preflight_ok(), publisher);

    let mut req = sample_request();
    req.sandbox_id = Some("sandbox-1".to_string());
    req.first_message = Some(FirstMessage {
        text: "hello".into(),
        model: None,
        agent_mode: None,
        effort: None,
        file_ids: vec![],
        contexts: vec![],
        sent_by: Some("user1".into()),
        message_uuid: None,
        skills: vec![],
        source_task_id: None,
    });

    let err = svc
        .create(req, empty_payload, |_, _| async {
            panic!("first message must not send after join failure");
        })
        .await
        .expect_err("join failure rejects create");

    assert!(matches!(err, TaskCreationError::SandboxJoin(_)));
    assert_eq!(
        order.lock().expect("order lock").as_slice(),
        ["join", "cleanup"]
    );
}

#[tokio::test]
async fn shared_sandbox_join_not_ready_starts_the_shared_sandbox() {
    // A same-sandbox create whose target has idle-stopped: the join reports
    // `NotReady` (no pod to register on), which must not fail the create. The
    // membership row is already durable, so creation starts the shared
    // sandbox — waking it — and still sends the first message.
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    // A deferred join never cleans up the inserted task.
    tasks.expect_delete_inserted_task().never();

    let mut starter = MockStarter::new();
    let start_order = Arc::clone(&order);
    starter.expect_start().times(1).returning(move |_| {
        start_order.lock().expect("order lock").push("start");
        Ok(())
    });
    let mut joiner = MockSandboxJoiner::new();
    let join_order = Arc::clone(&order);
    joiner
        .expect_join_task()
        .times(1)
        .returning(move |_, _, _| {
            join_order.lock().expect("order lock").push("join");
            Err(SandboxError::NotReady("Sandbox is stopped".to_string()))
        });
    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner, preflight_ok(), publisher.clone());

    let mut req = sample_request();
    req.sandbox_id = Some("sandbox-1".to_string());
    req.first_message = Some(FirstMessage {
        text: "hello".into(),
        model: None,
        agent_mode: None,
        effort: None,
        file_ids: vec![],
        contexts: vec![],
        sent_by: Some("user1".into()),
        message_uuid: None,
        skills: vec![],
        source_task_id: None,
    });

    let message_order = Arc::clone(&order);
    let outcome = svc
        .create(req, empty_payload, move |_, _| {
            let message_order = Arc::clone(&message_order);
            async move {
                message_order.lock().expect("order lock").push("message");
                Ok(())
            }
        })
        .await
        .expect("a not-ready shared sandbox is started, not rejected");

    assert_eq!(outcome.provision, ProvisionOutcome::Started);
    // The stopped shared sandbox produces no error event: it is being woken,
    // not failing.
    assert!(publisher.sandbox_statuses().is_empty());
    assert_eq!(
        order.lock().expect("order lock").as_slice(),
        ["join", "message", "start"]
    );
}

#[tokio::test]
async fn checkout_mode_without_sandbox_id_is_rejected_before_persistence() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().never();
    tasks.expect_insert_task().never();

    let mut starter = MockStarter::new();
    starter.expect_start().never();
    let publisher = Arc::new(RecordingPublisher::new());
    let svc = service(tasks, starter, joiner_never(), preflight_ok(), publisher);

    let mut req = sample_request();
    req.checkout_mode = Some(CheckoutMode::Shared);
    let err = svc
        .create(req, empty_payload, no_message)
        .await
        .expect_err("checkout mode without sandbox is rejected");

    assert!(matches!(
        err,
        TaskCreationError::BadRequest(msg) if msg.contains("checkout_mode requires sandbox_id")
    ));
}

#[tokio::test]
async fn a_new_task_is_given_its_own_and_its_workspaces_repositories() {
    let mut tasks = MockTaskDao::new();
    tasks.expect_exists_by_short_id().returning(|_| Ok(false));
    tasks.expect_insert_task().times(1).returning(|_, _| Ok(()));
    let mut repos = MockRepositoryReader::new();
    repos
        .expect_describe()
        .returning(|_, _| Ok(Some(sample_repo())));
    let mut workspaces = MockWorkspaceResolver::new();
    workspaces.expect_placements().returning(|_| {
        Ok(["repo2", "repo3"]
            .into_iter()
            .map(|id| moatless_workspace_ports::RepoPlacement {
                repository_id: id.into(),
                branch: None,
                mount_name: None,
                is_primary: id == "repo2",
                position: 0,
                repository: None,
            })
            .collect())
    });
    let mut task_repositories = MockTaskRepositoryDao::new();
    task_repositories
        .expect_record()
        .withf(|_, ids, source| {
            ids == [
                "repo2".to_string(),
                "repo3".to_string(),
                "repo1".to_string(),
            ] && *source == TaskRepositorySource::Workspace
        })
        .times(1)
        .returning(|_, _, _| Ok(()));
    let svc = TaskCreationService::new(
        Arc::new(tasks),
        Arc::new(MockTagDao::new()),
        Arc::new(MockTaskRelationDao::new()),
        Arc::new(task_repositories),
        Arc::new(crate::task::fork::MockTaskForkDao::new()),
        Arc::new(repos),
        Arc::new(workspaces),
        Arc::new(moatless_ports::identity::MockUserResolver::new()),
        None,
        Arc::new(MockStarter::new()),
        Arc::new(joiner_never()),
        preflight_ok(),
    );
    let mut req = sample_request();
    req.provision = false;
    svc.create(req, empty_payload, no_message)
        .await
        .expect("creation succeeds");
}
