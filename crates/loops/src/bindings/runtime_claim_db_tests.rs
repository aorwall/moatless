//! [`RoutingService::claim_task_creation`] against a real database: one winner
//! while the claimed Task is open, a takeover once it has closed. Also the
//! claims and bindings [`RoutingService::route_inbound_message`] makes for an
//! Ongoing Loop.

use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;

use moatless_adapter_model::error::AdapterError;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::{
    MockMessageAcceptor, MockTaskOpener, MockTaskReader, MockTranscriptAppend, TaskDescriptor,
};
use moatless_ports::task::{SandboxProvision, TaskOpened};
use moatless_types::access_control::Scope;
use moatless_types::adapter::{
    AdapterConnection, AdapterKind, Audience, InboundMessage, MessageBufferConfig,
};
use moatless_types::agent::AgentProfile;
use moatless_types::loops::{RoutingMode, SubscriptionMatch};
use moatless_types::task::TaskStatus;

use super::RoutingService;

const KIND: &str = "pull_request";
const SUBJECT: &str = "soaplabs/repo#42";

struct UnusedWaker;

#[async_trait::async_trait]
impl SandboxWaker for UnusedWaker {
    async fn wake(&self, _task_id: &str) -> Result<(), String> {
        panic!("claiming never wakes a sandbox")
    }
}

struct AcceptingWaker;

#[async_trait::async_trait]
impl SandboxWaker for AcceptingWaker {
    async fn wake(&self, _task_id: &str) -> Result<(), String> {
        Ok(())
    }
}

/// A task reader answering every Task as `status`, created by `created_by`.
fn tasks(status: TaskStatus, created_by: fn(&str) -> &'static str) -> MockTaskReader {
    let mut tasks = MockTaskReader::new();
    tasks.expect_describe().returning(move |task_id, _| {
        Ok(Some(TaskDescriptor {
            task_id: task_id.to_string(),
            status,
            name: None,
            owner_user_id: None,
            workspace_id: None,
            created_by: Some(created_by(task_id).to_string()),
            reporter_user_id: None,
            run_as_user_id: None,
            loop_id: None,
            ttl_minutes: 0,
        }))
    });
    tasks
}

/// A router whose task reader answers every Task as `status`.
fn routing(pool: PgPool, status: TaskStatus) -> Arc<RoutingService> {
    router(
        pool,
        tasks(status, |_| "alice"),
        MockTaskOpener::new(),
        Arc::new(UnusedWaker),
    )
}

fn router(
    pool: PgPool,
    tasks: MockTaskReader,
    task_creation: MockTaskOpener,
    sandbox_waker: Arc<dyn SandboxWaker>,
) -> Arc<RoutingService> {
    let mut message_send = MockMessageAcceptor::new();
    message_send.expect_accept().returning(|_| Ok(()));
    let connections = moatless_adapter_model::compose::adapter_connections(pool.clone());
    crate::compose::loop_graph(pool).routing(crate::compose::RoutingInputs {
        adapter_connections: connections.routing(),
        own_events: Arc::new(crate::bindings::own_events::OwnEvents::new(
            Arc::new(MockTranscriptAppend::new()),
            None,
        )),
        tasks: Arc::new(tasks),
        task_creation: Arc::new(task_creation),
        message_send: Arc::new(message_send),
        sandbox_waker,
        adapters: None,
        message_buffer: MessageBufferConfig::default(),
    })
}

fn connection() -> AdapterConnection {
    AdapterConnection {
        id: "pc_test".into(),
        adapter_kind: AdapterKind::Github,
        connection_kind: "org".into(),
        scope_value: "soaplabs".into(),
        credential_ref: None,
        webhook_secret: None,
        transport_config: serde_json::json!({}),
        created_by: "alice".into(),
        scope: Scope::Global,
        owner_user_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn subscription() -> SubscriptionMatch {
    SubscriptionMatch {
        name: "gate-review".to_string(),
        id: "tr_claim".into(),
        adapter_kind: AdapterKind::Github,
        connection_id: "pc_test".into(),
        source_matcher: "soaplabs/repo".into(),
        source_name: None,
        routing_mode: RoutingMode::BySubject,
        event_filter: serde_json::json!([]),
        workspace_id: "ws-1".into(),
        prompt_template: None,
        task_name: None,
        tag_ids: vec![],
        memory_preload: Vec::new(),
        agent_properties: Default::default(),
        skills: Vec::new(),
        run_as_user_id: Some("alice".into()),
        agent_profile: AgentProfile::ClaudeCode,
        model: None,
        close_after_settled_days: None,
        settle_after_hours: None,
        node_selector: None,
        stop_when_done: None,
        room: None,
        message_buffer_seconds: None,
        allow_direct_send: false,
        adapter_config: None,
        ongoing_task_id: None,
        close_task_on_close: false,
        notify_on_success: false,
        active: true,
        created_by: "alice".into(),
        scope: Scope::Global,
        owner_user_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn claim(routing: &RoutingService, event: &str) -> Result<(), AdapterError> {
    routing
        .claim_task_creation(
            &connection(),
            &subscription(),
            KIND,
            SUBJECT,
            "github",
            Some(event),
        )
        .await
}

#[sqlx::test]
async fn a_second_claim_loses_while_the_claimed_task_is_open(pool: PgPool) {
    let routing = routing(pool, TaskStatus::Open);
    claim(&routing, "e1").await.expect("the first claim wins");
    routing
        .record_created_task("tr_claim", KIND, SUBJECT, "task-1")
        .await
        .unwrap();
    assert!(matches!(
        claim(&routing, "e2").await,
        Err(AdapterError::CreationClaimed { .. })
    ));
}

/// What lets a check-run trigger on a PR whose Task closed open a new one.
#[sqlx::test]
async fn a_claim_whose_task_closed_is_taken_over(pool: PgPool) {
    let routing = routing(pool, TaskStatus::Closed);
    claim(&routing, "e1").await.expect("the first claim wins");
    routing
        .record_created_task("tr_claim", KIND, SUBJECT, "task-1")
        .await
        .unwrap();
    claim(&routing, "e2")
        .await
        .expect("a closed Task's claim is taken over");
}

#[sqlx::test]
async fn a_released_claim_can_be_claimed_again(pool: PgPool) {
    let routing = routing(pool, TaskStatus::Open);
    claim(&routing, "e1").await.expect("the first claim wins");
    routing
        .release_task_creation("tr_claim", KIND, SUBJECT)
        .await
        .unwrap();
    claim(&routing, "e2")
        .await
        .expect("a released claim is free");
}

const THREAD_A: &str = "T1:C1:1700000000.000100";
const THREAD_B: &str = "T1:C1:1700000001.000100";

fn ongoing(ongoing_task_id: Option<&str>) -> SubscriptionMatch {
    SubscriptionMatch {
        routing_mode: RoutingMode::Ongoing,
        ongoing_task_id: ongoing_task_id.map(str::to_string),
        ..subscription()
    }
}

fn message(text: &str) -> InboundMessage {
    InboundMessage {
        adapter_kind: AdapterKind::Slack,
        source_id: "C1".into(),
        thread_id: None,
        message_thread_id: "1700000000.000100".into(),
        user_id: "alice".into(),
        author: None,
        sender_external_id: None,
        requester: None,
        text: text.into(),
        file_ids: Vec::new(),
        attachments: Vec::new(),
        thread_properties: serde_json::json!({}),
        adapter_context: serde_json::json!({}),
        external_event_id: None,
        source_task_id: None,
        audience: Audience::Everyone,
        replies_to: Vec::new(),
        event: None,
    }
}

/// Write the connection and Loop rows a Binding references.
async fn seed(pool: &PgPool, routing: &RoutingService) {
    sqlx::query(
        r#"INSERT INTO adapter_connections
               (id, adapter_kind, connection_kind, external_account_id, created_by, scope)
           VALUES ('pc_test', 'slack', 'workspace', 'T1', 'alice', 'global')"#,
    )
    .execute(pool)
    .await
    .unwrap();
    routing
        .loops
        .upsert_subscription(&ongoing(None))
        .await
        .unwrap();
}

async fn route(
    routing: &RoutingService,
    subscription: &SubscriptionMatch,
    thread: &str,
) -> Result<Option<String>, AdapterError> {
    routing
        .route_inbound_message(
            &connection(),
            subscription,
            message("alert"),
            "slack",
            "thread",
            thread,
            serde_json::json!({}),
        )
        .await
}

async fn bound_tasks(routing: &RoutingService, thread: &str) -> Vec<String> {
    let mut tasks: Vec<String> = routing
        .task_bindings
        .for_subject("slack", "thread", thread)
        .await
        .unwrap()
        .into_iter()
        .filter(|b| b.active)
        .map(|b| b.task_id)
        .collect();
    tasks.sort();
    tasks
}

/// Two alerts in different threads that both read the Loop before either set
/// its ongoing Task: the second loses the Loop's claim instead of opening a
/// second ongoing Task, and its retry reaches the first's.
#[sqlx::test]
async fn events_in_two_threads_open_one_ongoing_task(pool: PgPool) {
    let mut opener = MockTaskOpener::new();
    opener.expect_open().times(1).returning(|_| {
        Ok(TaskOpened {
            task_id: "task-ongoing".into(),
            provision: SandboxProvision::Started,
        })
    });
    let routing = router(
        pool.clone(),
        tasks(TaskStatus::Open, |_| "alice"),
        opener,
        Arc::new(AcceptingWaker),
    );
    seed(&pool, &routing).await;
    let stale = ongoing(None);

    let first = route(&routing, &stale, THREAD_A).await.unwrap();
    let second = route(&routing, &stale, THREAD_B).await;

    assert_eq!(first.as_deref(), Some("task-ongoing"));
    assert!(
        matches!(second, Err(AdapterError::CreationClaimed { .. })),
        "{second:?}"
    );
    assert!(bound_tasks(&routing, THREAD_B).await.is_empty());
}

/// Bind `task-a` to [`THREAD_A`], route an event there through an Ongoing Loop
/// whose ongoing Task is `task-ongoing`, and name the Tasks then bound.
async fn bound_after_an_event_on_a_held_thread(
    pool: PgPool,
    created_by: fn(&str) -> &'static str,
) -> Vec<String> {
    let routing = router(
        pool.clone(),
        tasks(TaskStatus::Open, created_by),
        MockTaskOpener::new(),
        Arc::new(AcceptingWaker),
    );
    seed(&pool, &routing).await;
    let subscription = ongoing(Some("task-ongoing"));
    routing
        .task_bindings
        .bind(
            &connection(),
            Some(&subscription),
            "task-a",
            "slack",
            "thread",
            THREAD_A,
            serde_json::json!({}),
            super::ORIGIN_ADAPTER_EVENT,
            None,
            &Default::default(),
        )
        .await
        .unwrap();

    route(&routing, &subscription, THREAD_A).await.unwrap();
    bound_tasks(&routing, THREAD_A).await
}

#[sqlx::test]
async fn the_ongoing_task_does_not_join_a_thread_its_user_holds(pool: PgPool) {
    assert_eq!(
        bound_after_an_event_on_a_held_thread(pool, |_| "alice").await,
        ["task-a"]
    );
}

#[sqlx::test]
async fn the_ongoing_task_joins_a_thread_another_user_holds(pool: PgPool) {
    let by_task = |task_id: &str| if task_id == "task-a" { "bob" } else { "alice" };
    assert_eq!(
        bound_after_an_event_on_a_held_thread(pool, by_task).await,
        ["task-a", "task-ongoing"]
    );
}
