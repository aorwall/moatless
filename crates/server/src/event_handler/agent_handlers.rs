//! Agent event handlers — process agent.completed, agent.stopped, agent.error
//! events.

use std::sync::Arc;

use moatless_core::events::EventPublisher;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::{TaskRelationType, TaskSubType, TaskTurnSubType};

use crate::sandbox::lifecycle::Lifecycle;
use crate::state::AppState;
use moatless_ports::sandbox::SandboxWaker;
use moatless_tasks::message::models::MessageRole;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::models::TaskMessage;
use moatless_tasks::message::models::TaskMessageContext;
use moatless_tasks::message::send::MessageSendService;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::relations::TaskRelations;
use moatless_tasks::task_activities::mapping::BlockContext;
use moatless_tasks::task_activities::mapping::ConversationWrite;
use moatless_tasks::task_activities::mapping::map_content;

use super::models::SdkResultMessage;
use moatless_tasks::agent_event::AgentEvent;

/// Handle `agent.completed` event.
///
/// No separate result row is persisted: the run's usage goes onto the turn's
/// last assistant row, before updating cost, emitting follow-ups, and
/// finalizing any active Linear session.
pub async fn handle_agent_completed(state: &AppState, event: &AgentEvent) {
    // reason: agent-lifecycle event handler runs as background system with no caller identity
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let sdk_result: SdkResultMessage = match serde_json::from_value(event.payload.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                error = %e,
                "Failed to parse agent.completed payload — skipping"
            );
            return;
        }
    };

    // Defensive safety net: if the sandbox publishes an `is_error: true` result
    // under `agent.completed` (older code paths or future agent runtimes), route
    // it through the same error-message machinery as `handle_agent_error` so the
    // failure is visible in chat instead of disappearing into status flips.
    if sdk_result.is_error.unwrap_or(false) {
        handle_agent_error(&AgentErrorPorts::from(state), event).await;
        return;
    }

    // The run's USD cost: the transport's own figure where it priced the run
    // (Claude), otherwise priced here from the model and token counts (Codex,
    // tmux report no cost). This single value feeds the cost metric, the turn
    // metrics, and the task-total recompute below.
    let run_cost_usd = sdk_result.total_cost_usd.or_else(|| {
        let model = sdk_result.model.as_deref()?;
        let usage = sdk_result.usage.as_ref()?;
        super::pricing::estimate_cost_usd(model, usage)
    });

    // Prometheus: this is a successful terminal run (error completions
    // short-circuit above). Record outcome, cost, tokens, turn latency, turn
    // count, and any permission denials from the SDK result. The tmux backend
    // reports 0-valued durations on some paths — those are filtered out.
    {
        let agent_type = event.agent_type.as_deref();
        super::metrics::record_run("completed", agent_type);
        if let Some(cost) = run_cost_usd {
            super::metrics::record_cost(agent_type, cost);
        }
        if let Some(usage) = sdk_result.usage.as_ref() {
            super::metrics::record_tokens(agent_type, "input", usage.input_tokens);
            super::metrics::record_tokens(agent_type, "output", usage.output_tokens);
            super::metrics::record_tokens(
                agent_type,
                "cache_creation",
                usage.cache_creation_input_tokens,
            );
            super::metrics::record_tokens(agent_type, "cache_read", usage.cache_read_input_tokens);
        }
        if let Some(ms) = sdk_result.duration_ms.filter(|&ms| ms > 0) {
            let context =
                super::metrics::context_bucket(sdk_result.usage.as_ref(), sdk_result.num_turns);
            super::metrics::record_turn_duration(
                agent_type,
                "completed",
                context,
                ms as f64 / 1000.0,
            );
        }
        if let Some(ms) = sdk_result.duration_api_ms.filter(|&ms| ms > 0) {
            super::metrics::record_turn_api_duration(agent_type, ms as f64 / 1000.0);
        }
        if let Some(n) = sdk_result.num_turns {
            super::metrics::record_turns(agent_type, n);
        }
        for denial in &sdk_result.permission_denials {
            if let Some(name) = denial.tool_name.as_deref() {
                super::metrics::record_permission_denial(super::metrics::bucket_tool(name));
            }
        }
    }

    // Fallback: set real Claude Code session_id on user messages
    // (in case session_init event was missed or arrived before messages were fetched)
    if let Some(ref sid) = sdk_result.session_id {
        if let Err(e) = state
            .messages
            .set_session_id_for_task(&event.task_id, sid)
            .await
        {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to set session_id on user messages (fallback)");
        }
    }

    // Touch the activity timestamp. Status is flipped separately via
    // `update_status_if_not_closed` so a task closed mid-run (e.g. its PR was
    // closed) is never resurrected to Open by this automated handler. Cost is
    // NOT incremented here: an at-least-once event stream would double-count a
    // redelivered completion, so `total_cost_usd` is recomputed from the turn
    // metrics below instead, which is idempotent.
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let patch = TaskPatch {
        last_message_at: Some(now),
        ..Default::default()
    };

    if let Err(e) = state
        .tasks
        .update_task(&event.task_id, &patch, &sys_ctx)
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after completion");
    }

    // Store the run's authoritative totals on the turn. Claude Code's per-message
    // `output_tokens` is a partial value sampled before the response finished
    // (the final count only reaches the transcript), so the terminal result is
    // the only trustworthy source — and it is per-run, which is what a turn is.
    let turn_number = resolve_turn_number(state, &event.task_id, event.turn_number).await;
    if let Some(turn_number) = turn_number {
        let metrics = serde_json::json!({
            "usage": event.payload.get("usage"),
            "modelUsage": event.payload.get("modelUsage"),
            // The only model name a Codex turn carries: it reports one model
            // per run and no `modelUsage` breakdown, so usage reporting has
            // nothing else to group its tokens by.
            "model": sdk_result.model,
            "totalCostUsd": run_cost_usd,
            "durationMs": sdk_result.duration_ms,
            "durationApiMs": sdk_result.duration_api_ms,
            "numTurns": sdk_result.num_turns,
            "permissionDenials": event.payload.get("permission_denials"),
        });
        if let Err(e) = state
            .task_turns
            .set_turn_metrics(&event.task_id, turn_number, &metrics)
            .await
        {
            tracing::warn!(
                task_id = %event.task_id,
                turn_number,
                error = %e,
                "Failed to store turn metrics from completion result"
            );
        }
    }

    // Recompute the task's total cost from the per-turn metrics just written.
    // Summing the durable turn rows is idempotent under at-least-once delivery,
    // where a blind `total_cost_usd + cost` would drift on every redelivery.
    if let Err(e) = state.tasks.recompute_total_cost(&event.task_id).await {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to recompute task total cost");
    }

    // The only writer of an assistant row's usage columns, for every harness:
    // `handle_agent_message` leaves them null. The raw `usage` object is kept
    // whole (e.g. Codex `reasoning_output_tokens`) alongside the typed columns.
    if let (Some(turn_number), Some(usage)) = (turn_number, sdk_result.usage.as_ref()) {
        let raw_usage = event.payload.get("usage");
        match state
            .messages
            .backfill_last_assistant_usage(
                &event.task_id,
                turn_number,
                usage.input_tokens.map(|v| v as i32),
                usage.output_tokens.map(|v| v as i32),
                usage.cache_creation_input_tokens.map(|v| v as i32),
                usage.cache_read_input_tokens.map(|v| v as i32),
                raw_usage,
            )
            .await
        {
            Ok(true) => tracing::debug!(
                task_id = %event.task_id,
                turn_number,
                "Backfilled assistant-message usage from completion result"
            ),
            Ok(false) => {}
            Err(e) => tracing::warn!(
                task_id = %event.task_id,
                turn_number,
                error = %e,
                "Failed to backfill assistant usage after completion"
            ),
        }
    }

    // Reopen only if the task isn't already Closed. A Closed task is terminal
    // against automated re-open — the returned `reopened` gates the
    // `TaskSubType::Open` SSE event below so a closed task emits no phantom Open.
    let reopened = match state
        .tasks
        .update_status_if_not_closed(&event.task_id, TaskStatus::Open, &sys_ctx)
        .await
    {
        Ok(changed) => changed,
        Err(e) => {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task status after completion");
            false
        }
    };

    // Emit events. Error completions short-circuit through `handle_agent_error`
    // above, so this path is success-only.
    if let Some(ref events) = state.events {
        events
            .publish_task_turn_event(
                &event.task_id,
                TaskTurnSubType::Completed,
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;

        // Only announce Open when this handler actually (re)opened the task.
        // A task closed mid-run stays Closed and must not emit a phantom Open.
        if reopened {
            events
                .publish_task_event(
                    &event.task_id,
                    TaskSubType::Open,
                    event.agent_type.as_deref(),
                    serde_json::json!({}),
                )
                .await;
        }
    }

    stop_sandbox_when_done(&state.tasks, &state.sandbox, &event.task_id).await;
}

/// Release the pod of a `stop_when_done` Sandbox now that its run has finished.
///
/// What makes dedicated capacity affordable for a Loop: the node is billed for
/// the whole idle TTL otherwise, and nothing else scales it down until the
/// reaper's next cycle. Only the success path calls this — an errored run keeps
/// its pod for the idle TTL, which is what leaves a failure inspectable.
async fn stop_sandbox_when_done(tasks: &TaskCommands, sandbox: &Lifecycle, task_id: &str) {
    let sandbox_id = match tasks.sandbox_to_stop_when_done(task_id).await {
        Ok(Some(sandbox_id)) => sandbox_id,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to read stop-when-done sandbox");
            return;
        }
    };

    tracing::info!(
        task_id = %task_id,
        sandbox_id = %sandbox_id,
        "Agent run finished on a stop-when-done sandbox, stopping it"
    );
    if let Err(e) = sandbox
        .stop(
            &sandbox_id,
            moatless_types::sandbox::SandboxStopReason::System,
        )
        .await
    {
        // The pod survives every failure path here, so the idle reaper is still
        // the backstop that stops it.
        tracing::warn!(
            task_id = %task_id,
            sandbox_id = %sandbox_id,
            error = %e,
            "Failed to stop a stop-when-done sandbox"
        );
    }
}

/// Handle `agent.session_init` event — the agent's session opened.
///
/// Records the run's native turn id (Codex only) on the turn so a later fork
/// can cut the thread there; Claude carries none and skips it.
pub async fn handle_agent_session_init(state: &AppState, event: &AgentEvent) {
    let session_id = if let Some(sid) = event.payload.get("session_id").and_then(|v| v.as_str()) {
        sid
    } else {
        tracing::warn!(task_id = %event.task_id, "agent.session_init missing session_id");
        return;
    };

    tracing::info!(
        task_id = %event.task_id,
        session_id = %session_id,
        "Agent session initialized"
    );

    // Codex names a turn to fork at by its `turn/start` id, reported here. Store
    // it on the turn this run opened; a run without one (Claude) has nothing to
    // record.
    let agent_turn_id = event
        .payload
        .get("agent_turn_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    if let Some(agent_turn_id) = agent_turn_id {
        if let Some(turn_number) =
            resolve_turn_number(state, &event.task_id, event.turn_number).await
        {
            if let Err(e) = state
                .task_turns
                .set_agent_turn_id(&event.task_id, turn_number, agent_turn_id)
                .await
            {
                tracing::warn!(task_id = %event.task_id, error = %e, "Failed to record agent turn id");
            }
        }
    }
}

/// Handle `agent.stopped` event — the agent was stopped before completing.
///
/// Nothing about a stop enters the conversation — the task's status and turn
/// state already say so, and that terminal status is what the next run reads to
/// decide it needs a fresh session.
pub async fn handle_agent_stopped(state: &AppState, event: &AgentEvent) {
    // reason: agent-lifecycle event handler runs as background system with no caller identity
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    // Idempotency: drop duplicate stop fan-out. Both the sandbox and the
    // /stop HTTP handler can publish `agent.stopped` for a single user
    // action, and a hammered /stop endpoint must never re-run the stop. The
    // event log is the guard — a routine stop leaves no transcript row behind
    // to recognise, so an already-terminal run means this event has nothing to
    // add.
    match state
        .task_events
        .agent_run_already_terminal(&event.task_id, &event.event_id)
        .await
    {
        Ok(true) => {
            tracing::debug!(
                task_id = %event.task_id,
                event_id = %event.event_id,
                "Dropping duplicate agent.stopped — run already terminal"
            );
            return;
        }
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to check run state; proceeding with stop");
        }
    }

    // Prometheus: count the stopped run once, after the duplicate-stop guard.
    super::metrics::record_run("stopped", event.agent_type.as_deref());

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    if let Err(e) = state
        .tasks
        .update_task(
            &event.task_id,
            &TaskPatch {
                last_message_at: Some(now),
                ..Default::default()
            },
            &sys_ctx,
        )
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after stop");
    }

    // Reopen only if the task isn't already Closed. A task closed mid-run
    // (e.g. its PR was closed) stays Closed — this automated handler must
    // not resurrect it.
    if let Err(e) = state
        .tasks
        .update_status_if_not_closed(&event.task_id, TaskStatus::Open, &sys_ctx)
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task status after stop");
    }

    if let Some(ref events) = state.events {
        events
            .publish_task_turn_event(
                &event.task_id,
                TaskTurnSubType::Stopped,
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
    }
}

/// The ports an errored run is closed over.
///
/// Two callers reach this path: the event consumer, which projects it out of
/// `AppState`, and the task reaper, which holds the same ports directly.
#[derive(Clone)]
pub struct AgentErrorPorts {
    pub tasks: Arc<TaskCommands>,
    pub task_relations: Arc<TaskRelations>,
    pub message_send: Arc<MessageSendService>,
    pub sandbox_waker: Arc<dyn SandboxWaker>,
    pub events: Option<Arc<dyn EventPublisher>>,
}

/// Handle `agent.error` or `sandbox.fatal` event.
pub async fn handle_agent_error(ports: &AgentErrorPorts, event: &AgentEvent) {
    // reason: agent-lifecycle event handler runs as background system with no caller identity
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    // Prometheus: count the errored run. Reached both from the `agent.error`
    // dispatch arm and from `handle_agent_completed`'s `is_error` short-circuit
    // (which returns before recording a `completed` outcome), so an error run is
    // never double-counted as completed.
    super::metrics::record_run("error", event.agent_type.as_deref());

    let error_msg = event
        .payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown agent error");

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    // No transcript row. A run outcome is not something somebody said, and the
    // one failure worth reading — the Sandbox dying under the agent — already
    // reaches the conversation as a `sandbox_error` activity carrying the exit
    // code, terminated reason, disruption cause and container logs
    // (`sandbox/k8s/pod_watcher.rs:803`). What is left here is the turn's own
    // status and the Events tab, which keeps every one of these.

    if let Err(e) = ports
        .tasks
        .update_task(
            &event.task_id,
            &TaskPatch {
                last_message_at: Some(now),
                ..Default::default()
            },
            &sys_ctx,
        )
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task after error");
    }

    // Reopen only if the task isn't already Closed. This is the path the
    // orphaned-turn reaper routes through (`close_orphaned_turn`), so this
    // single guard also keeps the reaper from re-opening a closed task: the
    // synthetic error still publishes `taskTurn.error` (the UI unblocks), but
    // the status stays Closed.
    let child_not_closed = match ports
        .tasks
        .update_status_if_not_closed(&event.task_id, TaskStatus::Open, &sys_ctx)
        .await
    {
        Ok(changed) => changed,
        Err(e) => {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task status after error");
            false
        }
    };

    if let Some(ref events) = ports.events {
        events
            .publish_task_turn_event(
                &event.task_id,
                TaskTurnSubType::Error,
                event.agent_type.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
    }

    if child_not_closed {
        notify_parent_task_of_child_error(ports, event, error_msg).await;
    }
}

/// Tell the parent task that this child errored; a completing child reports its
/// own result, so only errors are forwarded here.
///
/// `created_by` only — a fork's errors stay in its own conversation since
/// nobody awaits a fork's result.
async fn notify_parent_task_of_child_error(
    ports: &AgentErrorPorts,
    event: &AgentEvent,
    error_msg: &str,
) {
    let relation = match ports.task_relations.find_parent(&event.task_id).await {
        Ok(Some(relation)) if relation.relation_type == TaskRelationType::CreatedBy => relation,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                error = %e,
                "Failed to load parent task relation for child error notification"
            );
            return;
        }
    };

    let message = format!(
        "Child task `{}` errored.\n\nError: {}",
        event.task_id, error_msg
    );

    match ports
        .message_send
        .send_message_flow(
            &relation.to_task_id,
            &message,
            None,
            None,
            None,
            &[],
            &[],
            true,
            moatless_types::task::ExternalOrigin {
                sender_id: None,
                source_task_id: Some(&event.task_id),
                ..moatless_types::task::ExternalOrigin::none()
            },
            false,
            None,
            &[],
            None,
        )
        .await
    {
        Ok(_message_uuid) => {
            if let Err(error_msg) = ports.sandbox_waker.wake(&relation.to_task_id).await {
                ports
                    .message_send
                    .persist_error_result(&relation.to_task_id, &error_msg)
                    .await;
            }
        }
        Err(moatless_tasks::message::error::MessageError::TaskClosed(_)) => {
            tracing::debug!(
                task_id = %event.task_id,
                parent_task_id = %relation.to_task_id,
                "Skipping child error notification because parent task is closed"
            );
        }
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                parent_task_id = %relation.to_task_id,
                error = %e,
                "Failed to notify parent task about child error"
            );
        }
    }
}

/// Handle `sandbox.fatal` event — delegates to `handle_agent_error` to create
/// an error result message when the sandbox crashes fatally (OOM, crash loop,
/// etc.).
pub async fn handle_sandbox_fatal(ports: &AgentErrorPorts, event: &AgentEvent) {
    handle_agent_error(ports, event).await;
}

/// Persist a Sandbox lifecycle change as its own activity, unless it restates
/// the one before it.
///
/// `coarse_status`/`coarse_is_error` label the row only when the payload lacks
/// a `lifecycleEvent`; an unparseable payload falls back to this coarse label
/// too.
pub async fn handle_sandbox_activity(
    state: &AppState,
    event: &AgentEvent,
    coarse_status: &str,
    coarse_is_error: bool,
) {
    let payload: moatless_types::sandbox::SandboxEventPayload =
        serde_json::from_value(event.payload.clone()).unwrap_or_default();
    let activity = moatless_tasks::task_activities::mapping::sandbox_activity(
        &event.event_id,
        &event.task_id,
        coarse_status,
        coarse_is_error,
        &payload,
    );
    if let Err(e) = state.messages.record_sandbox_activity(&activity).await {
        tracing::warn!(
            task_id = %event.task_id,
            event_id = %event.event_id,
            error = %e,
            "sandbox lifecycle activity could not be recorded"
        );
    }
}

/// Persist a context-window reading as its own activity.
///
/// A reading with no `usedTokens` is dropped rather than written as a row that
/// shadows the last usable one.
pub async fn handle_agent_context_window(state: &AppState, event: &AgentEvent) {
    let Some(used_tokens) = event
        .payload
        .get("usedTokens")
        .and_then(serde_json::Value::as_i64)
        .filter(|used| *used >= 0)
    else {
        return;
    };
    let max_tokens = event
        .payload
        .get("maxTokens")
        .and_then(serde_json::Value::as_i64)
        .filter(|max| *max > 0);

    let activity = moatless_tasks::task_activities::mapping::context_window_updated(
        &event.event_id,
        &event.task_id,
        used_tokens,
        max_tokens,
    );
    if let Err(e) = state.messages.record_standalone_activity(&activity).await {
        tracing::warn!(
            task_id = %event.task_id,
            event_id = %event.event_id,
            error = %e,
            "context-window reading could not be recorded"
        );
    }
}

pub async fn handle_agent_session_context_lost(_state: &AppState, event: &AgentEvent) {
    let resume_error = event
        .payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown resume error");

    tracing::warn!(
        task_id = %event.task_id,
        error = %resume_error,
        "Previous session could not be resumed; continuing with a fresh session"
    );
}

/// Confirm delivery of the ingress-outbox record whose payload built this
/// message, deleting it.
///
/// The stranded record's next re-publication converges on the same
/// idempotent upsert and deletes itself, so this warns and moves on.
async fn release_ingress_record(state: &AppState, task_id: &str, event_id: &str) {
    match state.event_ingress.delivered(event_id).await {
        Ok(true) => {
            metrics::counter!("event_ingress_delivered_total").increment(1);
        }
        // No record: this event reached the consumer without an ingress-outbox
        // spool to release.
        Ok(false) => {}
        Err(e) => tracing::warn!(
            task_id,
            event_id,
            error = %e,
            "message committed but its ingress record could not be released; the relay will retry it"
        ),
    }
}

/// Handle `agent.message` event — persist SDK messages and auto-tag skills.
///
/// Persists both assistant messages (text + `tool_use`) and SDK user messages
/// (`tool_result`), then extracts skill tags from assistant messages.
pub async fn handle_agent_message(state: &AppState, event: &AgentEvent) {
    // 1. Determine message type from payload role
    //    SDK assistant → stored as "assistant"
    //    SDK user (tool results) → stored as "system" (distinguishes from frontend user messages)
    let msg_role = event
        .payload
        .get("message")
        .and_then(|m| m.get("role"))
        .and_then(|v| v.as_str())
        .unwrap_or("assistant");

    let (message_type, message_role) = match msg_role {
        "user" => (MessageType::System, MessageRole::User),
        _ => (MessageType::Assistant, MessageRole::Assistant),
    };

    // 2. Extract fields from payload
    let uuid = event
        .payload
        .get("uuid")
        .and_then(|v| v.as_str())
        .unwrap_or(&event.event_id);

    let session_id = event
        .payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let parent_uuid = event
        .payload
        .get("parent_tool_use_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let content = event
        .payload
        .get("message")
        .and_then(|m| m.get("content"))
        .cloned()
        .unwrap_or(serde_json::json!([]));

    // Normalize content to array if it's a string (safety net)
    let content = if content.is_string() {
        serde_json::json!([{ "type": "text", "text": content.as_str().unwrap_or("") }])
    } else {
        content
    };

    // Emit tool-call metrics: cache tool_use_id→tool on the assistant message,
    // and count the completed call (by tool + status) on the matching user
    // tool_result message. Fire-and-forget — never blocks persistence.
    //
    // Metrics observe the *incoming* block(s), not the assembled Message, so a
    // per-block source event records its own `tool_use`/`tool_result` exactly
    // once as it arrives rather than re-counting the whole Message every block.
    record_tool_call_metrics(state, event, &content, msg_role == "user").await;

    // Per-block source events carry `blockIndex` plus a single-block content
    // array. The block index is the stable identity for conversation mapping:
    // it determines the activity id and text slot without reading the current
    // Message row back first. Coalesced events carry the whole content array,
    // whose first block is at index 0.
    let block_index = event
        .payload
        .get("blockIndex")
        .and_then(as_block_index)
        .unwrap_or(0);

    // Only a turn's terminal event carries a final count. A per-block snapshot
    // is sampled at `message_start`, so Claude's `output_tokens` reads 1-9
    // whatever the reply; `handle_agent_completed` writes the row's usage.
    let usage = event
        .payload
        .get("message")
        .and_then(|m| m.get("usage"))
        .filter(|_| carries_turn_result(&content));

    // 3. Inherit agent_mode from the last user message (same pattern as result messages)
    let agent_mode = match state.messages.get_last_user_message(&event.task_id).await {
        Ok(Some(user_msg)) => user_msg.agent_mode(),
        _ => None,
    };

    // 4. Extract API message fields and build sdk_metadata
    let api_message = event.payload.get("message");

    let (
        assistant_message_id,
        assistant_message_model,
        assistant_stop_reason,
        assistant_stop_sequence,
    ) = if message_type == MessageType::Assistant {
        (
            api_message
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            api_message
                .and_then(|m| m.get("model"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            api_message
                .and_then(|m| m.get("stop_reason"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            api_message
                .and_then(|m| m.get("stop_sequence"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
        )
    } else {
        (None, None, None, None)
    };

    let sdk_metadata = if message_type == MessageType::Assistant {
        build_sdk_metadata(
            &event.payload,
            &["isApiErrorMessage", "apiError", "requestId"],
        )
    } else {
        build_sdk_metadata(
            &event.payload,
            &[
                "isMeta",
                "isCompactSummary",
                "isVisibleInTranscriptOnly",
                "planContent",
                "thinkingMetadata",
                "todos",
                "summarizeMetadata",
                "sourceToolAssistantUUID",
                "sourceToolUseID",
                "permissionMode",
            ],
        )
    };

    let tool_use_result = if message_type == MessageType::Assistant {
        None
    } else {
        event
            .payload
            .get("toolUseResult")
            .or_else(|| event.payload.get("tool_use_result"))
            .cloned()
    };

    // 5. Resolve turn number — events may carry one; otherwise inherit from
    //    the current task turn so the row groups with its user message.
    let turn_number = resolve_turn_number(state, &event.task_id, event.turn_number).await;

    // 6. Build TaskMessage
    let msg_row = TaskMessage {
        id: 0,
        task_id: event.task_id.clone(),
        uuid: uuid.to_string(),
        message_type,
        user_type: "system".to_string(),
        parent_uuid,
        is_sidechain: None,
        cwd: None,
        session_id,
        turn_number,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: event.timestamp.clone(),
        message_role,
        // Left unset: the text is spliced in block by block by the append
        // itself, from `conversation.text`, and the insert must not overwrite
        // what earlier blocks already put there.
        message_text: None,
        assistant_message_id,
        assistant_message_type: None,
        assistant_message_model,
        assistant_stop_reason,
        assistant_stop_sequence,
        // Written by `handle_agent_completed` from the run's result.
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result,
        sdk_metadata,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode_raw: agent_mode.map(|m| m.as_str().to_string()),
        model: None,
        // Effort is a choice on a person's turn; an agent-produced row carries
        // none, and nothing reads one off these rows.
        effort: None,
        skills: Vec::new(),
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_subject_key: None,
        external_source_name: None,
        external_author: None,
        external_sender_id: None,
        source_task_id: None,
        start_fresh: false,
        sent_by: None,
        // Only a person's answer to a stored question carries one, and this row
        // is the agent's own output.
        tool_response: None,
    };

    // 7. Decide what each arriving block becomes.
    let mut conversation = map_content(
        BlockContext {
            task_id: &event.task_id,
            message_uuid: uuid,
            parent_uuid: msg_row.parent_uuid.as_deref(),
            message_type,
            block_index,
        },
        &content,
    );

    // The transport's structured view of an outcome is a field of the *Message*
    // the result arrived on, not of the block, so the mapping cannot see it and
    // the caller attaches it here. Only where the Message carries exactly one
    // result: `toolUseResult` is one object, and a coalesced event carrying two
    // results gives no way to say which it describes.
    if let ([outcome], Some(typed)) = (
        conversation.outcomes.as_mut_slice(),
        &msg_row.tool_use_result,
    ) {
        outcome.typed_result = Some(typed.clone());
    }

    // Usage is admitted against the Message's uuid rather than its row, so it
    // survives a turn that said nothing and therefore has no row. All four tiers
    // are kept: Codex's cache-read is its largest, and dropping it lost most of
    // the run's input.
    conversation.usage = usage.map(|usage| {
        let field = |name: &str| {
            usage
                .get(name)
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0) as i32
        };
        moatless_tasks::task_activities::mapping::AdmittedUsage {
            input_tokens: field("input_tokens"),
            output_tokens: field("output_tokens"),
            cache_creation_input_tokens: field("cache_creation_input_tokens"),
            cache_read_input_tokens: field("cache_read_input_tokens"),
        }
    });

    // A result changes the Message that *called* for it, not the carrier row it
    // arrived on — which is why the carrier is not rendered at all. Resolved
    // before the write so the conversation change records the Message a reader
    // will actually re-read.
    let anchor_uuid = anchor_message_uuid(state, &event.task_id, uuid, &conversation).await;

    // 8. Persist. A Message row exists when somebody said something: a turn that
    //    only called tools, and a carrier row that only relays a result, write
    //    their activities and no Message.
    let persisted = if conversation.text.is_empty() {
        state
            .messages
            .append_activities(&event.task_id, &anchor_uuid, &conversation)
            .await
    } else {
        let contexts: Vec<TaskMessageContext> = vec![];
        state
            .messages
            .append(&msg_row, &contexts, &conversation)
            .await
            .map(|_| ())
    };

    match persisted {
        Ok(()) => {
            tracing::debug!(
                task_id = %event.task_id,
                uuid = %uuid,
                message_type = %message_type,
                "Agent message persisted"
            );
            // The content is now in its canonical tables, so the durable ingress
            // record that carried it here has done its job. This is
            // the system's only confirmation of delivery and therefore its only
            // delete: a record that survives is content the conversation never
            // got, and the relay keeps re-publishing it until this line runs.
            release_ingress_record(state, &event.task_id, &event.event_id).await;
            // Server-owned assembly (server-owned assembly and skeleton-message model). A subagent row (carrying a
            // `parent_uuid` = the spawning `Agent` tool-use id) rides a
            // lightweight `subagentProgress` event and never streams transcript
            // content; a top-level row re-assembles the affected parent Message
            // and publishes a derived `message.upsert` for the live Chat.
            if let Some(parent_tool_use_id) = msg_row.parent_uuid.as_deref() {
                moatless_tasks::message::hooks::emit_subagent_progress(
                    state.events.as_ref(),
                    &state.messages,
                    &event.task_id,
                    parent_tool_use_id,
                )
                .await;
            } else {
                moatless_tasks::message::hooks::emit_message_upsert_from_daos(
                    state.events.as_ref(),
                    &state.messages,
                    Some(&state.files),
                    &event.task_id,
                    &anchor_uuid,
                )
                .await;
            }
        }
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                uuid = %uuid,
                error = %e,
                "Failed to persist agent message"
            );
        }
    }

    // 9. Skill tagging (only for assistant messages)
    if message_type == MessageType::Assistant {
        let skill_names = extract_skill_names(&event.payload);
        if skill_names.is_empty() {
            return;
        }

        let tag_names: Vec<String> = skill_names
            .into_iter()
            .map(|s| format!("skill-{s}"))
            .collect();

        state.tags.propagate_tags(&event.task_id, &tag_names).await;
    }
}

/// Handle `task.metadata.updated` event.
pub async fn handle_task_metadata_updated(state: &AppState, event: &AgentEvent) {
    // reason: agent-lifecycle event handler runs as background system with no caller identity
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let name = event.payload.get("name").and_then(|v| v.as_str());
    let description = event.payload.get("description").and_then(|v| v.as_str());

    if name.is_none() && description.is_none() {
        return;
    }

    let patch = TaskPatch {
        name: name.map(std::string::ToString::to_string),
        description: description.map(std::string::ToString::to_string),
        ..Default::default()
    };

    if let Err(e) = state
        .tasks
        .update_task(&event.task_id, &patch, &sys_ctx)
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to update task metadata");
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Parse a `blockIndex` payload field into a native block position. Accepts an
/// integer; anything else (absent, negative, non-numeric) yields `None`, which
/// means the event carries a whole content array starting at 0.
fn as_block_index(value: &serde_json::Value) -> Option<usize> {
    value.as_u64().and_then(|v| usize::try_from(v).ok())
}

/// Whether `content` holds Codex's `turn_result` block: the one per-block event
/// whose usage is a final count.
fn carries_turn_result(content: &serde_json::Value) -> bool {
    content.as_array().is_some_and(|blocks| {
        blocks.iter().any(|block| {
            block.get("type").and_then(serde_json::Value::as_str) == Some("turn_result")
        })
    })
}

/// Which Message a reader will see change.
///
/// A tool result arrives on a *later* Message than the call it answers but
/// renders as that call's outcome, so an event with only outcomes names the
/// calling Message instead; unresolvable calls fall back to the arriving uuid.
async fn anchor_message_uuid(
    state: &AppState,
    task_id: &str,
    message_uuid: &str,
    conversation: &ConversationWrite,
) -> String {
    if !conversation.text.is_empty() || !conversation.activities.is_empty() {
        return message_uuid.to_string();
    }
    let Some(outcome) = conversation.outcomes.first() else {
        return message_uuid.to_string();
    };
    match state
        .messages
        .find_tool_call(task_id, &outcome.tool_use_id)
        .await
    {
        Ok(Some(call)) if !call.parent_message_uuid.is_empty() => call.parent_message_uuid,
        Ok(_) => message_uuid.to_string(),
        Err(e) => {
            tracing::debug!(
                task_id,
                tool_use_id = %outcome.tool_use_id,
                error = %e,
                "could not resolve the Message a tool result answers; naming the carrier"
            );
            message_uuid.to_string()
        }
    }
}

/// Emit per-tool-call Prometheus metrics from an `agent.message` content array.
///
/// A `tool_result` block carries no tool name, only `tool_use_id`, so the
/// bucketed tool name is cached at the matching `tool_use` block and recovered
/// later (cache, then DB fallback). See [`super::metrics`].
async fn record_tool_call_metrics(
    state: &AppState,
    event: &AgentEvent,
    content: &serde_json::Value,
    is_user: bool,
) {
    let Some(blocks) = content.as_array() else {
        return;
    };

    if is_user {
        // tool_result blocks → count the completed call by tool + outcome.
        for block in blocks {
            if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = block.get("tool_use_id").and_then(|v| v.as_str()) else {
                continue;
            };
            let status = if block.get("is_error").and_then(serde_json::Value::as_bool) == Some(true)
            {
                "error"
            } else {
                "ok"
            };
            let tool = resolve_tool_label(state, &event.task_id, tool_use_id).await;
            super::metrics::record_tool_call(tool, event.agent_type.as_deref(), status);
        }
    } else {
        // assistant tool_use blocks → cache the bucketed tool name so the later
        // tool_result event can label its outcome without a DB hit.
        for block in blocks {
            if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                continue;
            }
            let (Some(id), Some(name)) = (
                block.get("id").and_then(|v| v.as_str()),
                block.get("name").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            state
                .tool_call_names
                .insert(id.to_string(), super::metrics::bucket_tool(name))
                .await;
        }
    }
}

/// Resolve the bucketed tool label for a `tool_use_id`: in-memory cache first,
/// then a persisted-row lookup, finally `"unknown"`.
///
/// The DB fallback covers cross-process/post-restart misses where the
/// `tool_use` block was never seen by this process.
async fn resolve_tool_label(state: &AppState, task_id: &str, tool_use_id: &str) -> &'static str {
    if let Some(label) = state.tool_call_names.get(tool_use_id).await {
        return label;
    }
    match state.messages.find_tool_call(task_id, tool_use_id).await {
        Ok(Some(row)) => {
            let label = super::metrics::bucket_tool(&row.tool_name);
            state
                .tool_call_names
                .insert(tool_use_id.to_string(), label)
                .await;
            label
        }
        Ok(None) => "unknown",
        Err(e) => {
            tracing::debug!(
                task_id,
                tool_use_id,
                error = %e,
                "tool-call metric: tool name lookup failed; labelling as unknown"
            );
            "unknown"
        }
    }
}

/// Resolve which turn a result/error/stopped row belongs to.
///
/// The gRPC ingress stamps `event.turn_number` from the event's run id, so the
/// fallback to the task's latest turn only covers events that reached the feed
/// without passing through that ingress.
async fn resolve_turn_number(
    state: &AppState,
    task_id: &str,
    event_turn_number: Option<i32>,
) -> Option<i32> {
    if let Some(t) = event_turn_number {
        return Some(t);
    }
    match state.task_turns.latest_turn_number(task_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to read current turn number");
            None
        }
    }
}

/// Build `sdk_metadata` JSON object from selected keys in a payload. Returns
/// `None` if no keys matched or all were null.
fn build_sdk_metadata(payload: &serde_json::Value, keys: &[&str]) -> Option<serde_json::Value> {
    let mut meta = serde_json::Map::new();
    for key in keys {
        if let Some(v) = payload.get(*key) {
            if !v.is_null() {
                meta.insert((*key).to_string(), v.clone());
            }
        }
    }
    if meta.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(meta))
    }
}

/// Extract skill names from `tool_use` blocks in agent message payload.
fn extract_skill_names(payload: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();

    // Look for content blocks array in payload
    let content = payload
        .get("content")
        .or_else(|| payload.get("message_content"));

    let blocks = match content {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return names,
    };

    for block in blocks {
        let block_type = block.get("type").and_then(|v| v.as_str());
        if block_type != Some("tool_use") {
            continue;
        }

        let tool_name = block.get("name").and_then(|v| v.as_str());
        if tool_name != Some("Skill") {
            continue;
        }

        // Extract skill name from tool input
        if let Some(input) = block.get("input") {
            if let Some(skill) = input.get("skill").and_then(|v| v.as_str()) {
                names.push(skill.to_string());
            }
        }
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── extract_skill_names ──────────────────────────────────────────

    #[test]
    fn extract_single_skill() {
        let payload = json!({
            "content": [{
                "type": "tool_use",
                "name": "Skill",
                "input": { "skill": "commit" }
            }]
        });
        assert_eq!(extract_skill_names(&payload), vec!["commit"]);
    }

    #[test]
    fn extract_multiple_skills() {
        let payload = json!({
            "content": [
                { "type": "tool_use", "name": "Skill", "input": { "skill": "commit" } },
                { "type": "tool_use", "name": "Skill", "input": { "skill": "review-pr" } }
            ]
        });
        assert_eq!(extract_skill_names(&payload), vec!["commit", "review-pr"]);
    }

    #[test]
    fn extract_ignores_non_skill_tools() {
        let payload = json!({
            "content": [
                { "type": "tool_use", "name": "Read", "input": { "path": "/tmp" } },
                { "type": "tool_use", "name": "Skill", "input": { "skill": "deploy" } }
            ]
        });
        assert_eq!(extract_skill_names(&payload), vec!["deploy"]);
    }

    #[test]
    fn extract_ignores_non_tool_use_blocks() {
        let payload = json!({
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "tool_use", "name": "Skill", "input": { "skill": "test" } }
            ]
        });
        assert_eq!(extract_skill_names(&payload), vec!["test"]);
    }

    #[test]
    fn extract_empty_content() {
        let payload = json!({ "content": [] });
        assert!(extract_skill_names(&payload).is_empty());
    }

    #[test]
    fn extract_no_content_key() {
        let payload = json!({ "other": "data" });
        assert!(extract_skill_names(&payload).is_empty());
    }

    #[test]
    fn extract_from_message_content_key() {
        let payload = json!({
            "message_content": [{
                "type": "tool_use",
                "name": "Skill",
                "input": { "skill": "plan" }
            }]
        });
        assert_eq!(extract_skill_names(&payload), vec!["plan"]);
    }

    #[test]
    fn extract_skill_missing_input() {
        let payload = json!({
            "content": [{
                "type": "tool_use",
                "name": "Skill"
            }]
        });
        assert!(extract_skill_names(&payload).is_empty());
    }

    // ── per-block assembly ───────────────────────────────────────────

    #[test]
    fn as_block_index_parses_only_non_negative_integers() {
        assert_eq!(as_block_index(&json!(0)), Some(0));
        assert_eq!(as_block_index(&json!(3)), Some(3));
        // A coalesced event carries no blockIndex, and neither does a
        // malformed one. Both fall back to index 0 at the call site.
        assert_eq!(as_block_index(&json!(-1)), None);
        assert_eq!(as_block_index(&json!("2")), None);
        assert_eq!(as_block_index(&json!(null)), None);
    }

    #[test]
    fn carries_turn_result_matches_only_the_turn_result_block() {
        assert!(carries_turn_result(&json!([{ "type": "turn_result" }])));
        assert!(!carries_turn_result(
            &json!([{ "type": "text", "text": "done" }])
        ));
        assert!(!carries_turn_result(&json!([])));
        assert!(!carries_turn_result(&json!("turn_result")));
    }
}
