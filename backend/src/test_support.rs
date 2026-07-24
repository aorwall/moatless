//! Shared `#[cfg(test)]` test doubles used across module unit tests.
//!
//! Compiled only for the crate's own test build (`#[cfg(test)]` in `lib.rs`),
//! so it is never part of the shipped binary.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use moatless_events::{
    AgentSubType, SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType,
    TaskSubType, TaskTurnSubType,
};

use crate::events::EventPublisher;

/// A publish call captured by [`RecordingPublisher`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RecordedEvent {
    Sandbox(SandboxStatus),
    Task(TaskSubType),
    Agent(AgentSubType),
    TaskTurn(TaskTurnSubType),
    Server(ServerSubType),
    /// A raw event (`publish_raw_event` / `publish_raw_event_with_id`).
    Raw,
}

/// [`EventPublisher`] double that records what it was asked to publish, so
/// tests can assert the events a code path emitted without Redis.
///
/// The optional `order_log` also appends `"publish"` on each raw publish; the
/// gRPC transactional-outbox ordering tests share it with a recording turn repo
/// to assert the DB commit is logged before the publish.
#[derive(Default)]
pub(crate) struct RecordingPublisher {
    calls: Arc<Mutex<Vec<RecordedEvent>>>,
    order_log: Option<Arc<Mutex<Vec<&'static str>>>>,
}

impl RecordingPublisher {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A publisher that additionally appends `"publish"` to `order_log` on each
    /// raw publish.
    pub(crate) fn with_order_log(order_log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            order_log: Some(order_log),
        }
    }

    pub(crate) fn events(&self) -> Vec<RecordedEvent> {
        self.calls.lock().expect("recorder poisoned").clone()
    }

    /// Sandbox event statuses, in publish order.
    pub(crate) fn sandbox_statuses(&self) -> Vec<SandboxStatus> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                RecordedEvent::Sandbox(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Task event subtypes, in publish order.
    pub(crate) fn task_subtypes(&self) -> Vec<TaskSubType> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                RecordedEvent::Task(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    fn push(&self, e: RecordedEvent) {
        self.calls.lock().expect("recorder poisoned").push(e);
    }
}

#[async_trait]
impl EventPublisher for RecordingPublisher {
    async fn publish_sandbox_event(&self, _: &str, sub_type: SandboxStatus, _: &str) {
        self.push(RecordedEvent::Sandbox(sub_type));
    }
    async fn publish_sandbox_event_with_payload(
        &self,
        _: &str,
        sub_type: SandboxStatus,
        _: &SandboxEventPayload,
    ) {
        self.push(RecordedEvent::Sandbox(sub_type));
    }
    async fn publish_server_event(&self, _: &str, sub_type: ServerSubType, _: &ServerEventPayload) {
        self.push(RecordedEvent::Server(sub_type));
    }
    async fn publish_task_event(&self, _: &str, sub_type: TaskSubType, _: Option<&str>, _: Value) {
        self.push(RecordedEvent::Task(sub_type));
    }
    async fn publish_agent_event(&self, _: &str, sub_type: AgentSubType, _: Option<&str>, _: Value) {
        self.push(RecordedEvent::Agent(sub_type));
    }
    async fn publish_task_turn_event(
        &self,
        _: &str,
        sub_type: TaskTurnSubType,
        _: Option<&str>,
        _: Option<i32>,
        _: Value,
    ) {
        self.push(RecordedEvent::TaskTurn(sub_type));
    }
    async fn publish_raw_event(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<i32>,
        _: Value,
    ) {
        self.push(RecordedEvent::Raw);
    }
    async fn publish_raw_event_with_id(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<i32>,
        _: Value,
        _: bool,
    ) {
        self.push(RecordedEvent::Raw);
        if let Some(order_log) = &self.order_log {
            order_log.lock().expect("recorder poisoned").push("publish");
        }
    }
    async fn health_check(&self) -> Result<(), String> {
        Ok(())
    }
}
