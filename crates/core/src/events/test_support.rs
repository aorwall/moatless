// This module compiles only for test builds (`#[cfg(test)]`) or downstream test
// tiers (`feature = "test-support"`), never a shipping build, so the recorder's
// mutex `.expect()` calls are test scaffolding — allow them the way the crate's
// own `#[cfg(test)]` code is allowed (the crate-level allow is `test`-gated and
// does not cover the feature-gated build).
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Shared [`EventPublisher`] test double used across the workspace's test tiers.
//!
//! Gated `#[cfg(any(test, feature = "test-support"))]`, so it is compiled for
//! this crate's own test build and, off the `test-support` feature, for the
//! test builds of downstream crates (the tasks band and the backend) that need
//! to assert what events a code path emitted without a live Redis. No shipping
//! build enables the feature, so it never reaches the released binary.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use moatless_events::{
    AgentSubType, SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType,
    TaskSubType, TaskTurnSubType,
};

use crate::events::{EventPublisher, PublishError};

/// A publish call captured by [`RecordingPublisher`].
#[derive(Debug, Clone, PartialEq)]
pub enum RecordedEvent {
    Sandbox(SandboxStatus),
    Task(TaskSubType),
    Agent(AgentSubType),
    TaskTurn(TaskTurnSubType),
    Server(ServerSubType),
    /// A raw event (`publish_raw_event` / `publish_raw_event_with_id`).
    Raw,
}

/// One `publish_raw_event_with_id` call, captured in full.
#[derive(Debug, Clone)]
pub struct RawPublish {
    pub event_id: String,
    pub payload: Value,
    /// The flag deciding whether an oversized payload may be truncated.
    pub payload_recoverable: bool,
}

/// [`EventPublisher`] double that records what it was asked to publish, so
/// tests can assert the events a code path emitted without Redis.
///
/// The optional `order_log` also appends `"publish"` on each raw publish; the
/// ingress ordering tests share it with a recording repository to assert the DB
/// commit is logged before the publish. `failing` simulates a transport outage
/// for a fixed number of raw publishes, so fail-then-recover paths are testable.
#[derive(Default)]
pub struct RecordingPublisher {
    calls: Arc<Mutex<Vec<RecordedEvent>>>,
    raw: Arc<Mutex<Vec<RawPublish>>>,
    order_log: Option<Arc<Mutex<Vec<&'static str>>>>,
    fail_next: Arc<Mutex<usize>>,
}

impl RecordingPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// A publisher that additionally appends `"publish"` to `order_log` on each
    /// raw publish.
    pub fn with_order_log(order_log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            order_log: Some(order_log),
            ..Self::default()
        }
    }

    /// A publisher whose next `fail_next` raw publishes fail as a transport
    /// error, and which succeeds from then on.
    pub fn failing(fail_next: usize) -> Self {
        Self {
            fail_next: Arc::new(Mutex::new(fail_next)),
            ..Self::default()
        }
    }

    /// Raw publishes, in order.
    pub fn raw_publishes(&self) -> Vec<RawPublish> {
        self.raw.lock().expect("recorder poisoned").clone()
    }

    pub fn events(&self) -> Vec<RecordedEvent> {
        self.calls.lock().expect("recorder poisoned").clone()
    }

    /// Sandbox event statuses, in publish order.
    pub fn sandbox_statuses(&self) -> Vec<SandboxStatus> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                RecordedEvent::Sandbox(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Task event subtypes, in publish order.
    pub fn task_subtypes(&self) -> Vec<TaskSubType> {
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
    async fn publish_agent_event(
        &self,
        _: &str,
        sub_type: AgentSubType,
        _: Option<&str>,
        _: Value,
    ) {
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
        event_id: &str,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<i32>,
        payload: Value,
        payload_recoverable: bool,
    ) -> Result<(), PublishError> {
        self.push(RecordedEvent::Raw);
        if let Some(order_log) = &self.order_log {
            order_log.lock().expect("recorder poisoned").push("publish");
        }
        {
            let mut remaining = self.fail_next.lock().expect("recorder poisoned");
            if *remaining > 0 {
                *remaining -= 1;
                return Err(PublishError::Transport {
                    attempts: 3,
                    source: redis::RedisError::from((
                        redis::ErrorKind::Io,
                        "injected transport failure",
                    )),
                });
            }
        }
        self.raw
            .lock()
            .expect("recorder poisoned")
            .push(RawPublish {
                event_id: event_id.to_string(),
                payload,
                payload_recoverable,
            });
        Ok(())
    }
    async fn health_check(&self) -> Result<(), String> {
        Ok(())
    }
}
