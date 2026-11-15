//! [`RoutingService::claim_task_creation`] against a real database: one winner
//! while the claimed Task is open, a takeover once it has closed.

use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;

use moatless_adapter_model::error::AdapterError;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::{
    MockMessageAcceptor, MockTaskOpener, MockTaskReader, MockTranscriptAppend, TaskDescriptor,
};
use moatless_types::access_control::Scope;
use moatless_types::adapter::{AdapterConnection, AdapterKind, MessageBufferConfig};
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

/// A router whose task reader answers every Task as `status`.
fn routing(pool: PgPool, status: TaskStatus) -> Arc<RoutingService> {
    let mut tasks = MockTaskReader::new();
    tasks.expect_describe().returning(move |task_id, _| {
        Ok(Some(TaskDescriptor {
            task_id: task_id.to_string(),
            status,
            name: None,
            owner_user_id: None,
            workspace_id: None,
            created_by: None,
            reporter_user_id: None,
            run_as_user_id: None,
            loop_id: None,
            ttl_minutes: 0,
        }))
    });
    let connections = moatless_adapter_model::compose::adapter_connections(pool.clone());
    crate::compose::loop_graph(pool).routing(crate::compose::RoutingInputs {
        adapter_connections: connections.routing(),
        own_events: Arc::new(crate::bindings::own_events::OwnEvents::new(
            Arc::new(MockTranscriptAppend::new()),
            None,
        )),
        tasks: Arc::new(tasks),
        task_creation: Arc::new(MockTaskOpener::new()),
        message_send: Arc::new(MockMessageAcceptor::new()),
        sandbox_waker: Arc::new(UnusedWaker),
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
