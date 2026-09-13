//! gRPC event publisher for sandbox agent events.
//!
//! Publishes events to the backend via gRPC, which records them on the feed
//! for SSE consumption. See [`GrpcEventPublisher`] for the enqueue-only,
//! at-least-once delivery contract.

use moatless_types::agent::AgentSubType;
use moatless_types::event::EventType;
use moatless_types::task::{TaskSubType, TaskTurnSubType};
use sandbox_proto::PublishEventRequest;
use tracing::{debug, warn};

use crate::outbox::EventOutbox;

/// Publishes sandbox events to the backend via the at-least-once outbox.
///
/// All publish operations are non-blocking; delivery, retry, ordering and
/// overflow handling live in [`crate::outbox`].
#[derive(Clone)]
pub struct GrpcEventPublisher {
    outbox: EventOutbox,
    /// Set on run-scoped clones (see [`GrpcEventPublisher::for_run`]); every
    /// event published through one carries it so the backend can attribute the
    /// event to that run's turn regardless of delivery order.
    run_id: Option<String>,
}

impl GrpcEventPublisher {
    /// Create a new publisher backed by the given outbox.
    pub fn new(outbox: EventOutbox) -> Self {
        Self {
            outbox,
            run_id: None,
        }
    }

    /// A clone of this publisher that stamps `run_id` on everything it emits.
    /// Used by the coordinator service for the lifetime of one agent run.
    pub fn for_run(&self, run_id: &str) -> Self {
        Self {
            outbox: self.outbox.clone(),
            run_id: Some(run_id.to_string()),
        }
    }

    /// Access the underlying outbox (for shutdown flush and metrics).
    pub fn outbox(&self) -> &EventOutbox {
        &self.outbox
    }

    /// Enqueue a pre-built request, assigning a stable `event_id` when empty;
    /// for events assembled outside the typed helpers below.
    ///
    /// Returns whether the outbox took it; see [`EventOutbox::enqueue`].
    pub fn enqueue(&self, mut req: PublishEventRequest) -> bool {
        if req.event_id.is_empty() {
            req.event_id = uuid::Uuid::new_v4().to_string();
        }
        if sanitize_payload_json_nuls(&mut req.payload_json) {
            warn!(
                task_id = %req.task_id,
                sub_type = %req.sub_type,
                "Sanitized NUL bytes from event payload before publishing"
            );
        }
        self.outbox.enqueue(req)
    }

    /// Publish a task lifecycle event (e.g. status changes, metadata updates).
    pub async fn publish_task_event(
        &self,
        task_id: &str,
        sub_type: TaskSubType,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, EventType::Task, sub_type.as_str(), payload)
            .await;
    }

    /// Routes as `event_type="agent"` (`"agent.completed"`, etc.) to the
    /// backend consumer.
    ///
    /// Returns whether it was enqueued; most callers ignore it, but the prompt
    /// announcement must not record itself as published when refused.
    pub async fn publish_agent_event(
        &self,
        task_id: &str,
        sub_type: AgentSubType,
        payload: serde_json::Value,
    ) -> bool {
        self.publish_event(task_id, EventType::Agent, sub_type.as_str(), payload)
            .await
    }

    /// Publish a sandbox lifecycle event (e.g. ready, error, fatal).
    pub async fn publish_sandbox_event(
        &self,
        task_id: &str,
        sub_type: moatless_types::sandbox::SandboxStatus,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, EventType::Sandbox, sub_type.as_str(), payload)
            .await;
    }

    /// Publish a task turn event (e.g. turn active, completed, failed).
    pub async fn publish_task_turn_event(
        &self,
        task_id: &str,
        sub_type: TaskTurnSubType,
        payload: serde_json::Value,
    ) {
        self.publish_event(task_id, EventType::TaskTurn, sub_type.as_str(), payload)
            .await;
    }

    /// Internal: build a generic event and enqueue it for delivery. Returns
    /// whether it was enqueued.
    async fn publish_event(
        &self,
        task_id: &str,
        event_type: EventType,
        sub_type: &str,
        payload: serde_json::Value,
    ) -> bool {
        let payload_json = match serde_json::to_string(&payload) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to serialize event payload");
                return false;
            }
        };

        // Narrowing onto the wire enum is partial: the domain enum carries
        // backend-derived types the protocol cannot express. Every caller above
        // passes a sandbox-originated type, so a rejection here is a bug in this
        // module, not a runtime condition — drop the event rather than publish it
        // under a different type. The drop goes through the outbox's accounting
        // seam so it is measured and announced, not only logged.
        let proto_event_type = match sandbox_proto::EventType::try_from(event_type) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(
                    task_id = %task_id,
                    sub_type = %sub_type,
                    error = %e,
                    "Refusing to publish event over the sandbox protocol"
                );
                self.outbox.record_unrepresentable();
                return false;
            }
        };

        debug!(
            task_id = %task_id,
            event_type = %event_type,
            sub_type = %sub_type,
            "Enqueueing event for delivery"
        );

        self.enqueue(PublishEventRequest {
            task_id: task_id.to_string(),
            event_type: proto_event_type as i32,
            sub_type: sub_type.to_string(),
            payload_json,
            event_id: uuid::Uuid::new_v4().to_string(),
            sequence: 0,
            run_id: self.run_id.clone().unwrap_or_default(),
        })
    }
}

fn sanitize_payload_json_nuls(payload_json: &mut String) -> bool {
    if !payload_json.contains('\0') && !payload_json.contains("\\u0000") {
        return false;
    }

    let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(payload_json) else {
        return false;
    };
    if !sanitize_value_nuls(&mut payload) {
        return false;
    }

    match serde_json::to_string(&payload) {
        Ok(sanitized) => {
            *payload_json = sanitized;
            true
        }
        Err(e) => {
            warn!(error = %e, "Failed to reserialize sanitized event payload");
            false
        }
    }
}

fn sanitize_value_nuls(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => {
            if s.contains('\0') {
                *s = s.replace('\0', "\\0");
                true
            } else {
                false
            }
        }
        serde_json::Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= sanitize_value_nuls(item);
            }
            changed
        }
        serde_json::Value::Object(map) => {
            let mut changed = false;
            for item in map.values_mut() {
                changed |= sanitize_value_nuls(item);
            }
            changed
        }
        _ => false,
    }
}

/// A `BackendClient` that keeps what it was asked to publish.
///
/// Lives here rather than in a test module, so the agent's own tests can build
/// a publisher without restating the trait.
#[cfg(test)]
pub(crate) mod test_backend {
    use std::sync::Mutex;

    use sandbox_proto::{
        DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery,
        PublishEventRequest,
    };
    use tonic::Status;

    use crate::backend_client::BackendClient;
    use crate::commands::JournaledCommand;

    #[derive(Default)]
    pub(crate) struct CapturingBackend {
        pub(crate) received: Mutex<Vec<PublishEventRequest>>,
        /// Every `record_unreported_commands` call, oldest first, with its
        /// revision.
        pub(crate) unreported_commands: Mutex<Vec<(i64, Vec<JournaledCommand>)>>,
    }

    #[async_trait::async_trait]
    impl BackendClient for CapturingBackend {
        async fn publish_event(&self, req: PublishEventRequest) -> Result<(), Status> {
            self.received.lock().expect("lock").push(req);
            Ok(())
        }

        async fn record_unreported_commands(
            &self,
            _task_id: &str,
            commands: &[JournaledCommand],
            revision: i64,
        ) {
            self.unreported_commands
                .lock()
                .expect("lock")
                .push((revision, commands.to_vec()));
        }

        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<PendingAgentDelivery>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _message_uuids: Vec<String>,
            _session_id: &str,
            _run_id: &str,
            _answered_prompt: Option<&str>,
        ) -> Result<(), Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn get_credentials(&self, _task_id: &str) -> Result<GetCredentialsResponse, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn update_credential(
            &self,
            _task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<DownloadUploadResponse, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn recover_interrupted_run(
            &self,
            _task_id: &str,
        ) -> Result<Option<InterruptedTurn>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn upload_transcript(
            &self,
            _task_id: &str,
            _tar_data: Vec<u8>,
        ) -> Result<(), Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, Status> {
            Err(Status::unimplemented("not used"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use tokio_util::sync::CancellationToken;

    use super::test_backend::CapturingBackend;
    use crate::backend_client::BackendClient;

    #[test]
    fn sanitize_payload_json_nuls_rewrites_nested_string_values() {
        let mut payload = serde_json::json!({
            "text": "before\0after",
            "items": ["x\0y"],
            "literal": "\\u0000"
        })
        .to_string();

        assert!(sanitize_payload_json_nuls(&mut payload));

        let parsed: serde_json::Value = serde_json::from_str(&payload).expect("sanitized JSON");
        assert_eq!(parsed["text"], "before\\0after");
        assert_eq!(parsed["items"][0], "x\\0y");
        assert_eq!(parsed["literal"], "\\u0000");
    }

    #[test]
    fn sanitize_payload_json_nuls_leaves_malformed_payloads_unchanged() {
        let mut payload = r#"{"text":"before\u0000after""#.to_string();
        let original = payload.clone();

        assert!(!sanitize_payload_json_nuls(&mut payload));
        assert_eq!(payload, original);
    }

    #[tokio::test]
    async fn typed_publish_agent_event_sanitizes_payload_before_outbox_delivery() {
        let backend = Arc::new(CapturingBackend::default());
        let cancel = CancellationToken::new();
        let (outbox, _drainer) = EventOutbox::spawn(
            Arc::clone(&backend) as Arc<dyn BackendClient>,
            "task-1".to_string(),
            cancel.clone(),
        );
        let publisher = GrpcEventPublisher::new(outbox);

        publisher
            .publish_agent_event(
                "task-1",
                AgentSubType::Message,
                serde_json::json!({ "text": "before\0after" }),
            )
            .await;

        for _ in 0..20 {
            if !backend.received.lock().expect("lock").is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }

        let received = backend.received.lock().expect("lock");
        let req = received.first().expect("event delivered");
        let payload: serde_json::Value =
            serde_json::from_str(&req.payload_json).expect("payload JSON");
        assert_eq!(payload["text"], "before\\0after");

        cancel.cancel();
    }
}
