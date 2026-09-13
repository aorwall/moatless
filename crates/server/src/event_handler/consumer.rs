//! Redis stream consumer for agent events.
//!
//! Reads `task-events:*` under a consumer group separate from the SSE
//! consumer's, and processes `agent.*` and `task.metadata.updated` events. The
//! stream plumbing both consumers share lives in [`crate::redis_stream`]; what
//! stays here is the persistence, turn projection and dispatch an event gets.

use std::sync::Arc;

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use moatless_types::agent::AgentSubType;
use moatless_types::event::EventType;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::task::{TaskSubType, TaskTurnSubType};

use moatless_loops::adapters::events::TaskEventSubscribers;

use crate::redis_stream::{GroupStart, StreamConsumer, StreamHandler};
use crate::state::AppState;
use moatless_tasks::task_event::models::InsertTaskEvent;
use moatless_tasks::task_turn::models::classify_turn_transition;

use super::agent_handlers;
use moatless_tasks::agent_event::AgentEvent;
use moatless_tasks::agent_event::EventKey;

/// Consumer group name — separate from the SSE consumer group.
const CONSUMER_GROUP: &str = "event-handler";

/// Processes agent lifecycle events off the shared stream plumbing.
struct EventHandlerConsumer {
    state: AppState,
    /// The Adapters that react to a task's own events. Built at the composition
    /// root; this module never names a provider.
    subscribers: Arc<TaskEventSubscribers>,
}

#[async_trait]
impl StreamHandler for EventHandlerConsumer {
    async fn handle_payload(&mut self, entry_id: &str, payload: String) {
        let Ok(event) = serde_json::from_str::<AgentEvent>(&payload) else {
            // The entry is ACKed and gone from the PEL either way, so the id is
            // the only handle left on it: `XRANGE task-events:<task> <id> <id>`.
            warn!(
                entry_id = %entry_id,
                "Failed to parse event payload, ACKing to prevent redelivery"
            );
            return;
        };
        // Derived, UI-only `message.*` events are relayed to clients by the SSE
        // consumer but must never be persisted to `task_events` (server-owned
        // assembly).
        if event.event_type == EventType::Message {
            return;
        }
        persist_event(&self.state, &event).await;
        self.project_turn_state(&event).await;
        self.dispatch_event(&event).await;
    }
}

impl EventHandlerConsumer {
    /// Idempotent fallback projection of turn state into `task_turns`.
    ///
    /// The gRPC ingress writes turn state transactionally before publishing, so
    /// this is usually a no-op via `WHERE` guards; it's a backstop for the rare
    /// event that reached Redis without that transactional write. Touches only
    /// `task_turns`.
    async fn project_turn_state(&self, event: &AgentEvent) {
        let Some(transition) = classify_turn_transition(event.event_type.as_str(), &event.sub_type)
        else {
            return;
        };
        // A terminal row always belongs to the most recent in-flight turn; an
        // explicit turn_number wins when the event carries one.
        let turn_number = match event.turn_number {
            Some(t) => t,
            None => match self
                .state
                .task_turns
                .latest_turn_number(&event.task_id)
                .await
            {
                Ok(Some(t)) => t,
                Ok(None) => return,
                Err(e) => {
                    warn!(
                        task_id = %event.task_id,
                        error = %e,
                        "Turn-state fallback: failed to read current turn number"
                    );
                    return;
                }
            },
        };
        let at = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
            .map_or_else(|_| chrono::Utc::now(), |dt| dt.with_timezone(&chrono::Utc));
        if let Err(e) = self
            .state
            .task_turns
            .project_turn(
                &event.task_id,
                turn_number,
                event.agent_type.clone(),
                transition,
                at,
            )
            .await
        {
            warn!(
                task_id = %event.task_id,
                event_id = %event.event_id,
                error = %e,
                "Turn-state fallback projection failed"
            );
        }
    }

    /// Dispatch a parsed event: the generic handling this band owns, then the
    /// Adapters that subscribed to it.
    ///
    /// Subscribers run *after* the generic handler, so a reaction depending on
    /// what it persisted (e.g. the GitHub shell claim keying on the message
    /// row) sees it.
    #[tracing::instrument(
        skip_all,
        fields(task_id = %event.task_id, event_type = %event.event_type, sub_type = %event.sub_type)
    )]
    async fn dispatch_event(&self, event: &AgentEvent) {
        self.handle_event(event).await;
        self.subscribers.dispatch(event).await;
    }

    /// The generic handling every event gets, whatever Adapters are installed.
    async fn handle_event(&self, event: &AgentEvent) {
        match event.event_key() {
            EventKey::Agent(AgentSubType::Completed) => {
                // handle_agent_completed publishes task_turn.completed; external
                // notification is handled by the agent via `moat reply`.
                agent_handlers::handle_agent_completed(&self.state, event).await;
            }
            EventKey::TaskTurn(TaskTurnSubType::Completed) => {
                // External notification is handled by the agent via `moat reply`.
            }
            EventKey::Agent(AgentSubType::Message) => {
                agent_handlers::handle_agent_message(&self.state, event).await;
            }
            EventKey::Agent(AgentSubType::SessionInit) => {
                agent_handlers::handle_agent_session_init(&self.state, event).await;
            }
            EventKey::Agent(AgentSubType::Stopped) => {
                // handle_agent_stopped publishes task_turn.stopped — never
                // re-handle that downstream event or we self-amplify into a loop.
                agent_handlers::handle_agent_stopped(&self.state, event).await;
            }
            EventKey::TaskTurn(TaskTurnSubType::Stopped) => {
                // External notification is handled by the agent via `moat reply`.
            }
            EventKey::Agent(AgentSubType::Error) => {
                // handle_agent_error publishes task_turn.error; external
                // notification is handled by the agent via `moat reply`.
                agent_handlers::handle_agent_error(&(&self.state).into(), event).await;
            }
            EventKey::TaskTurn(TaskTurnSubType::Error) => {
                // External notification is handled by the agent via `moat reply`.
            }
            // Resume failed (stale session id) and the agent fell back to a
            // fresh session — persist an inline conversation notice so the
            // context loss is visible in the message history. The run
            // continues, so no task_turn.error and no Linear error activity.
            // What the harness said about its own context window — an activity of
            // its own, on no Message, so the meter has a value to read.
            EventKey::Agent(AgentSubType::ContextWindow) => {
                agent_handlers::handle_agent_context_window(&self.state, event).await;
            }
            EventKey::Agent(AgentSubType::SessionContextLost) => {
                agent_handlers::handle_agent_session_context_lost(&self.state, event).await;
            }
            EventKey::Task(TaskSubType::MetadataUpdated) => {
                agent_handlers::handle_task_metadata_updated(&self.state, event).await;
            }
            // Sandbox error — pod watcher detected a fatal crash; transcript-only
            // for the agent (auto-error reply).
            EventKey::Sandbox(SandboxStatus::Error) => {
                agent_handlers::handle_sandbox_activity(&self.state, event, "error", true).await;
                agent_handlers::handle_sandbox_fatal(&(&self.state).into(), event).await;
            }
            // Sandbox lifecycle transitions — recorded as their own activity.
            EventKey::Sandbox(
                status @ (SandboxStatus::Initializing
                | SandboxStatus::Ready
                | SandboxStatus::Stopped),
            ) => {
                agent_handlers::handle_sandbox_activity(
                    &self.state,
                    event,
                    status.as_str(),
                    false,
                )
                .await;
            }
            // Agent process started inside the sandbox. Nothing generic to do —
            // the Adapters that surface a live status subscribe to it.
            EventKey::Agent(AgentSubType::Started) => {}
            // Known SSE-only event types — forwarded to frontend but not processed here
            EventKey::Sandbox(_)
            | EventKey::Server(_)
            | EventKey::Agent(
                AgentSubType::PendingResponse
                | AgentSubType::Retrying
                | AgentSubType::Scheduled
                | AgentSubType::RecoveryStarted
                | AgentSubType::RecoveryExhausted
                // Rate-limited liveness marker from long tool calls — persisted
                // to task_events (advances the reaper's activity floor) but
                // never a conversation message.
                | AgentSubType::Activity
                // The running-command count is written onto the Sandbox row at
                // ingress, beside the admission that made it durable, so the
                // Task listing reads it without waiting on this consumer.
                | AgentSubType::CommandsChanged,
            )
            | EventKey::Task(
                TaskSubType::Open
                | TaskSubType::Created
                | TaskSubType::Closed
                | TaskSubType::Updated,
            )
            | EventKey::TaskTurn(TaskTurnSubType::Active | TaskTurnSubType::Interrupted) => {
                // `turn_interrupted` is an observability marker only — the turn
                // resumes when the pod reschedules, and the eventual terminal
                // event (or the reaper's reconcile) still closes it. No DB
                // side-effect here, just SSE fan-out.
                debug!(
                    task_id = %event.task_id,
                    event_type = %event.event_type,
                    sub_type = %event.sub_type,
                    "SSE-only event type, skipping"
                );
            }
            EventKey::Unknown(ref t, ref s) => {
                warn!(
                    task_id = %event.task_id,
                    event_type = %t,
                    sub_type = %s,
                    "Unhandled event type — event not processed"
                );
            }
        }
    }
}

/// Append an event to `task_events` (best-effort, never blocks dispatch).
///
/// `agent.message` writes no row — `task_events` is lifecycle-only, and the
/// conversation holds per-activity rows. Free rather than a method so a test
/// can reach it with a pool and no Redis.
pub async fn persist_event(state: &AppState, event: &AgentEvent) {
    if moatless_tasks::agent_event::is_agent_message(event.event_type.as_str(), &event.sub_type) {
        return;
    }
    let insert = InsertTaskEvent {
        event_id: event.event_id.clone(),
        task_id: event.task_id.clone(),
        event_type: event.event_type.as_str().to_string(),
        sub_type: event.sub_type.clone(),
        is_error: event.is_error,
        agent_type: event.agent_type.clone(),
        turn_number: event.turn_number,
        payload: event.payload.clone(),
        timestamp: event.timestamp.clone(),
    };
    if let Err(e) = state.task_events.insert_event(&insert).await {
        warn!(
            task_id = %event.task_id,
            event_id = %event.event_id,
            error = %e,
            "Failed to persist event to database"
        );
    }
}

/// Run the event consumer until cancellation.
pub async fn spawn_event_handler(
    conn: ConnectionManager,
    state: AppState,
    subscribers: Arc<TaskEventSubscribers>,
    cancel: CancellationToken,
) {
    let consumer_id = format!(
        "event-handler-{}",
        std::env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string())
    );

    info!(consumer_id = %consumer_id, "Spawning event handler consumer");

    StreamConsumer::new(
        conn,
        consumer_id,
        CONSUMER_GROUP.to_string(),
        // From new entries only: a shared group persists every event exactly
        // once, so a restarting pod must not re-read what a peer already wrote.
        GroupStart::New,
        "event-handler",
        EventHandlerConsumer { state, subscribers },
        cancel,
    )
    .run()
    .await;
}
