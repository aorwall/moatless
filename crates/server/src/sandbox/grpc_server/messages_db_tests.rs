use super::*;
use std::sync::Arc;

use moatless_core::config::Config;
use moatless_ports::sandbox::SandboxError;
use moatless_tasks::message::{models::MessageType, test_support as messages};
use moatless_tasks::task::test_support as tasks;
use sandbox_proto::backend_sandbox_service_server::BackendSandboxService;
use sandbox_proto::client::{SandboxClient, SandboxConnector};
use sandbox_proto::{AppendTrajectoryRequest, PublishEventRequest, TrajectoryLine};
use serde_json::json;
use sqlx::PgPool;

use super::super::authorize::SandboxAuthorizer;

struct UnusedConnector;

#[async_trait::async_trait]
impl SandboxConnector for UnusedConnector {
    async fn sandbox_client(&self, _: &str) -> Result<SandboxClient, SandboxError> {
        panic!("system-origin messages do not change the driver")
    }
}

fn service(pool: &PgPool) -> BackendSandboxServiceImpl {
    let config = Config::test_minimal();
    let ports = crate::compose::compose(pool.clone(), &config, None);
    let mut service = crate::compose::infrastructure::grpc_service(
        &config,
        &ports,
        crate::compose::infrastructure::init_event_publisher(pool),
        None,
        None,
        Arc::new(UnusedConnector),
    );
    service.authorizer = SandboxAuthorizer::dev_unauthenticated(None, None);
    service
}

async fn pending(service: &BackendSandboxServiceImpl) -> GetPendingMessagesResponse {
    service
        .get_pending_messages(Request::new(GetPendingMessagesRequest {
            task_id: "provider-task".to_string(),
            hold_queued: false,
            ..GetPendingMessagesRequest::default()
        }))
        .await
        .expect("pending delivery")
        .into_inner()
}

async fn queue(pool: &PgPool, text: &str) {
    let mut item = messages::user_item("provider-task", uuid::Uuid::new_v4(), text);
    item.origin = moatless_tasks::inbox::InboxOrigin::System;
    messages::queue_message(pool, item).await.expect("queue");
}

async fn follow_up_after_rejection(pool: PgPool, preserved_session: Option<&str>) {
    let mut task = tasks::make_task_row("provider-task", "user-1");
    task.agent_profile = AgentProfile::Codex;
    tasks::seed_task(&pool, &task).await.expect("task");
    let original = service(&pool);
    original.append_trajectory(Request::new(AppendTrajectoryRequest {
        task_id: task.task_id.clone(), harness: "codex".to_string(),
        harness_session_id: "original-codex-session".to_string(),
        first_seq: 0,
        lines: vec![TrajectoryLine {
            kind: "response_item/message".to_string(),
            content_json: json!({"role": "assistant", "blocks": [{"type": "text", "text": "recorded history"}]}).to_string(),
            payload: "recorded history".to_string(),
            ..TrajectoryLine::default()
        }],
        ..AppendTrajectoryRequest::default()
    })).await.expect("trajectory");
    queue(&pool, "resume the session").await;
    let first = pending(&original).await;
    assert_eq!(
        first.deliveries[0].resume_session_id.as_deref(),
        Some("original-codex-session")
    );
    let mut payload =
        json!({"error": "Restore the provider configuration", "reason": "invalid_configuration"});
    if let Some(session) = preserved_session {
        payload["sessionImpact"] = json!("intact");
        payload["resumeSessionId"] = json!(session);
    }
    original
        .publish_event(Request::new(PublishEventRequest {
            task_id: task.task_id.clone(),
            event_type: sandbox_proto::EventType::Agent as i32,
            sub_type: "error".to_string(),
            payload_json: payload.to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            run_id: "rejected-run".to_string(),
            sequence: 1,
        }))
        .await
        .expect("terminal error admitted");
    original
        .acknowledge_messages(Request::new(AcknowledgeMessagesRequest {
            task_id: task.task_id.clone(),
            message_uuids: first.deliveries[0].message_uuids.clone(),
            session_id: "original-codex-session".to_string(),
            run_id: "rejected-run".to_string(),
            ..AcknowledgeMessagesRequest::default()
        }))
        .await
        .expect("terminal batch settled");
    assert!(pending(&original).await.deliveries.is_empty());
    let mut result = messages::make_message_row(&task.task_id, "provider-error-result");
    result.message_type = MessageType::Error;
    result.message_text = Some("Provider configuration must be restored".to_string());
    messages::seed_message(&pool, &result, &[], &Default::default())
        .await
        .expect("stored error result");
    queue(&pool, "provider restored, continue").await;
    let expected = preserved_session.filter(|session| *session == "original-codex-session");
    let follow_up = pending(&original).await;
    assert_eq!(follow_up.deliveries.len(), 1);
    assert_eq!(
        follow_up.deliveries[0].resume_session_id.as_deref(),
        expected
    );
    drop(original);
    let replacement = service(&pool);
    let after_replacement = pending(&replacement).await;
    assert_eq!(after_replacement.deliveries.len(), 1);
    assert_eq!(
        after_replacement.deliveries[0].resume_session_id.as_deref(),
        expected
    );
}

#[sqlx::test]
async fn intact_provider_rejection_resumes_original_session_after_replacement(pool: PgPool) {
    follow_up_after_rejection(pool, Some("original-codex-session")).await;
}

#[sqlx::test]
async fn an_error_without_an_intact_session_still_starts_fresh(pool: PgPool) {
    follow_up_after_rejection(pool, None).await;
}

#[sqlx::test]
async fn an_intact_marker_for_another_session_does_not_allow_resume(pool: PgPool) {
    follow_up_after_rejection(pool, Some("unrelated-session")).await;
}
