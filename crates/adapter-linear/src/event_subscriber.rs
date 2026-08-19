//! Mirroring a task's own event stream into its Linear agent session.
//!
//! Linear's UI shows a session as a live feed, so a bound Task's progress has to
//! be pushed there as it happens: what the assistant said, what tools it ran,
//! how the sandbox is doing, and how the turn ended. This module is the whole of
//! that reaction — it subscribes to the events it needs and nothing outside this
//! crate names Linear to make them happen.
//!
//! # The turn gate
//!
//! [`ActiveLinearTurns`] is owned here, because this is the only code that reads
//! it. It opens when an inbound webhook routes to a Task (`sessions`, `issues`)
//! or when `agent.started` arrives, and it closes on the terminal event. While
//! it is closed every emission below is dropped, so post-turn stragglers — a
//! late `sandbox.stopped`, a trailing tool result — never leak into Linear.
//!
//! Both halves of a terminal event live here, in order: emit the final activity,
//! *then* close the gate. Splitting them across two modules is what previously
//! let the gate close before the `agent.error` activity was emitted.
//!
//! # Best effort
//!
//! Every emission is a detached `tokio::spawn`. Linear being unreachable must
//! not slow the event handler or fail the run, and a notification lost to a
//! restart is not worth recovering — the next event carries the Task's state
//! again.

use std::sync::Arc;

use moatless_core::config::Config;
use moatless_core::db::DbPool;
use moatless_types::agent::AgentSubType;
use moatless_types::sandbox::SandboxStatus;
use moatless_loops::adapters::dao::AdapterConnectionDao;
use moatless_loops::adapters::events::{TaskEventSubscriber, TaskEventSubscribers};
use moatless_tasks::agent_event::{AgentEvent, EventKey};
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::task::dao::TaskDao;

use super::active_turns::ActiveLinearTurns;
use super::activity::{
    ActivityKind, notify_linear_action, notify_linear_message_activities, notify_linear_status,
    notify_linear_tool_results,
};

/// The ports the Linear event reaction reads.
pub struct LinearEventSubscriber {
    db: DbPool,
    adapter_connections: Arc<dyn AdapterConnectionDao>,
    config: Arc<Config>,
    messages: Arc<dyn MessageDao>,
    tasks: Arc<dyn TaskDao>,
    active_turns: Arc<ActiveLinearTurns>,
}

impl LinearEventSubscriber {
    pub fn new(
        db: DbPool,
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        config: Arc<Config>,
        messages: Arc<dyn MessageDao>,
        tasks: Arc<dyn TaskDao>,
        active_turns: Arc<ActiveLinearTurns>,
    ) -> Self {
        Self {
            db,
            adapter_connections,
            config,
            messages,
            tasks,
            active_turns,
        }
    }

    /// The events this Adapter reacts to.
    fn keys() -> [EventKey; 9] {
        [
            EventKey::Agent(AgentSubType::Started),
            EventKey::Agent(AgentSubType::Message),
            EventKey::Agent(AgentSubType::Completed),
            EventKey::Agent(AgentSubType::Stopped),
            EventKey::Agent(AgentSubType::Error),
            EventKey::Sandbox(SandboxStatus::Error),
            EventKey::Sandbox(SandboxStatus::Initializing),
            EventKey::Sandbox(SandboxStatus::Ready),
            EventKey::Sandbox(SandboxStatus::Stopped),
        ]
    }

    /// Assistant text and tool calls as they stream in; tool results resolved
    /// back to the tool that produced them.
    fn on_agent_message(&self, event: &AgentEvent) {
        if !self.active_turns.is_open(&event.task_id) {
            return;
        }
        let Some(message) = event.payload.get("message").cloned() else {
            return;
        };
        let role = message
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("assistant");

        // assistant → persistent `thought` text plus an ephemeral pending
        // `action` per tool_use; user → a completed `action` per tool_result,
        // looked up against the persisted tool_use row for the tool's name.
        if role == "user" {
            tokio::spawn(notify_linear_tool_results(
                self.db.clone(),
                self.adapter_connections.clone(),
                self.config.clone(),
                self.messages.clone(),
                event.task_id.clone(),
                message,
            ));
        } else {
            tokio::spawn(notify_linear_message_activities(
                self.db.clone(),
                self.adapter_connections.clone(),
                self.config.clone(),
                event.task_id.clone(),
                message,
            ));
        }
    }

    /// A terminal `response` activity carrying the last assistant message.
    ///
    /// Awaits the message read rather than spawning it: the body has to be
    /// resolved before the gate closes, and only the Linear call is detached.
    async fn on_agent_completed(&self, event: &AgentEvent) {
        // An `is_error` completion is routed through the error machinery by the
        // generic handler and posts no response activity — only the gate closes.
        let is_error = event
            .payload
            .get("isError")
            .or_else(|| event.payload.get("is_error"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !is_error {
            self.emit_completion_response(&event.task_id).await;
        }
        self.active_turns.close(&event.task_id);
    }

    async fn emit_completion_response(&self, task_id: &str) {
        let msg = match self.messages.get_last_assistant_message(task_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                tracing::warn!(
                    task_id,
                    "agent.completed: no assistant message found; skipping Linear response activity"
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    task_id,
                    error = %e,
                    "agent.completed: failed to fetch last assistant message; skipping Linear response activity"
                );
                return;
            }
        };

        let mut body = msg.message_text.clone().unwrap_or_default();
        if body.trim().is_empty() {
            tracing::warn!(
                task_id,
                message_uuid = %msg.uuid,
                "agent.completed: last assistant message has no text content; skipping Linear response activity"
            );
            return;
        }

        // Append the `Moatless task:` footer so the inbound `Comment.create`
        // echo filter (`handle_comment_created`) can recover the authoring task
        // id and drop this comment when Linear webhooks it back. Without the
        // footer the response activity re-enters the task as a fresh inbound
        // comment, the agent answers, `agent.completed` fires again, and the
        // task loops forever.
        if let Some(footer) = moatless_loops::bindings::footer::task_comment_footer(
            self.tasks.as_ref(),
            &self.config.auth.frontend_url,
            task_id,
        )
        .await
        {
            body.push_str(&footer);
        }

        self.spawn_status(task_id, ActivityKind::Response, body);
    }

    /// A persistent `error` activity, so a mid-flight failure is visible in the
    /// session rather than only in the task.
    fn on_failure(&self, event: &AgentEvent, fallback: &str) {
        if self.active_turns.is_open(&event.task_id) {
            let body = event
                .payload
                .get("error")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or(fallback)
                .to_string();
            self.spawn_status(&event.task_id, ActivityKind::Error, body);
        }
        self.active_turns.close(&event.task_id);
    }

    /// A sandbox or agent status marker, as an ephemeral `action` so each new
    /// status replaces the previous one in Linear's UI instead of piling up.
    fn spawn_action(&self, task_id: &str, action: &str, parameter: &str) {
        tokio::spawn(notify_linear_action(
            self.db.clone(),
            self.adapter_connections.clone(),
            self.config.clone(),
            task_id.to_string(),
            action.to_string(),
            parameter.to_string(),
            None,
            true,
        ));
    }

    fn spawn_status(&self, task_id: &str, kind: ActivityKind, body: String) {
        tokio::spawn(notify_linear_status(
            self.db.clone(),
            self.adapter_connections.clone(),
            self.config.clone(),
            task_id.to_string(),
            kind,
            body,
            false,
        ));
    }
}

#[async_trait::async_trait]
impl TaskEventSubscriber for LinearEventSubscriber {
    fn name(&self) -> &'static str {
        super::delivery::ADAPTER
    }

    async fn on_task_event(&self, event: &AgentEvent) {
        match event.event_key() {
            // Re-opens the gate, so a manual `moat send` against a Linear-bound
            // task re-surfaces events in Linear on the second and later turns.
            EventKey::Agent(AgentSubType::Started) => {
                self.active_turns.open(&event.task_id);
                self.spawn_action(&event.task_id, "Agent", AgentSubType::Started.as_str());
            }
            EventKey::Agent(AgentSubType::Message) => self.on_agent_message(event),
            EventKey::Agent(AgentSubType::Completed) => self.on_agent_completed(event).await,
            // A user-initiated stop emits no final activity; the gate close just
            // keeps post-stop stragglers out.
            EventKey::Agent(AgentSubType::Stopped) => self.active_turns.close(&event.task_id),
            EventKey::Agent(AgentSubType::Error) => self.on_failure(event, "Agent error"),
            EventKey::Sandbox(SandboxStatus::Error) => self.on_failure(event, "Sandbox error"),
            EventKey::Sandbox(status) if self.active_turns.is_open(&event.task_id) => {
                self.spawn_action(&event.task_id, "Sandbox", status.as_str());
            }
            _ => {}
        }
    }
}

/// Subscribe the Linear reaction to the events it needs.
///
/// One call from the composition root; a build without it leaves Linear
/// sessions without live progress and changes nothing else.
pub fn subscribe(subscribers: &mut TaskEventSubscribers, subscriber: Arc<LinearEventSubscriber>) {
    subscribers.subscribe(LinearEventSubscriber::keys(), subscriber);
}
