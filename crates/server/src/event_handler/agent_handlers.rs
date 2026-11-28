//! Agent event handlers — process agent.completed, agent.stopped, agent.error
//! events.

use std::sync::Arc;

use moatless_core::events::EventPublisher;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::{TaskRelationType, TaskSubType, TaskTurnSubType};

use crate::sandbox::lifecycle::Lifecycle;
use crate::state::AppState;
use moatless_ports::sandbox::SandboxWaker;
use moatless_tasks::message::send::MessageSendService;
use moatless_tasks::task::creation::TASK_ERRORED_LIFECYCLE_EVENTS;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task::relations::TaskRelations;
use moatless_tasks::task_turn::operations::TaskTurns;

use super::models::SdkResultMessage;
use moatless_tasks::agent_event::AgentEvent;

/// Handle `agent.completed` event.
///
/// No separate result row is persisted: the run's usage goes onto the turn's
/// last assistant row, before updating cost, emitting follow-ups, and
/// finalizing any active Linear session.
pub async fn handle_agent_completed(state: &AppState, event: &AgentEvent) {
    #[expect(
        clippy::disallowed_methods,
        reason = "agent-lifecycle event handler runs as background system with no caller identity"
    )]
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

    // Defensive: an `is_error: true` under `agent.completed` (older or
    // future runtimes) routes through `handle_agent_error` so it surfaces in
    // chat instead of vanishing into a status flip.
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
        let agent_profile = event.agent_profile.as_deref();
        super::metrics::record_run("completed", agent_profile);
        if let Some(cost) = run_cost_usd {
            super::metrics::record_cost(agent_profile, cost);
        }
        if let Some(usage) = sdk_result.usage.as_ref() {
            super::metrics::record_tokens(agent_profile, "input", usage.input_tokens);
            super::metrics::record_tokens(agent_profile, "output", usage.output_tokens);
            super::metrics::record_tokens(
                agent_profile,
                "cache_creation",
                usage.cache_creation_input_tokens,
            );
            super::metrics::record_tokens(
                agent_profile,
                "cache_read",
                usage.cache_read_input_tokens,
            );
        }
        if let Some(ms) = sdk_result.duration_ms.filter(|&ms| ms > 0) {
            let context =
                super::metrics::context_bucket(sdk_result.usage.as_ref(), sdk_result.num_turns);
            super::metrics::record_turn_duration(
                agent_profile,
                "completed",
                context,
                ms as f64 / 1000.0,
            );
        }
        if let Some(ms) = sdk_result.duration_api_ms.filter(|&ms| ms > 0) {
            super::metrics::record_turn_api_duration(agent_profile, ms as f64 / 1000.0);
        }
        if let Some(n) = sdk_result.num_turns {
            super::metrics::record_turns(agent_profile, n);
        }
        for denial in &sdk_result.permission_denials {
            if let Some(name) = denial.tool_name.as_deref() {
                super::metrics::record_permission_denial(super::metrics::bucket_tool(name));
            }
        }
    }

    // Touch the activity timestamp only. Status flips separately via
    // `update_status_if_not_closed`, so a task closed mid-run isn't
    // resurrected. Cost isn't added here either: at-least-once delivery would
    // double-count it, so `total_cost_usd` is recomputed below instead.
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
                event.agent_profile.as_deref(),
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
                    event.agent_profile.as_deref(),
                    serde_json::json!({}),
                )
                .await;
        }
    }

    // A task closed mid-run tells its parent nothing, as in `handle_agent_error`.
    // Nor does a turn that left `moat cmd` commands owed a wake, which also
    // keeps a `stop_when_done` sandbox: the wake's turn does both.
    let background_commands = event
        .payload
        .get("backgroundCommands")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if background_commands > 0 {
        // Deliberately before the stop: `running_commands` misses an exited
        // command the agent has not read, and stopping would kill its watch.
        return;
    }
    if reopened {
        notify_parent_of_child_outcome(
            &AgentErrorPorts::from(state),
            event,
            ChildOutcome::Completed { turn_number },
        )
        .await;
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

/// Handle `agent.started` event: a run began, so an `Error` left by a failed
/// create or start no longer describes the Task.
///
/// Only an errored Task is written, so a normal run start adds no listing
/// change. Without this the status waits for the run to end, and the T3 sidebar
/// hides the Task for the whole run.
pub async fn handle_agent_started(tasks: &TaskCommands, event: &AgentEvent) {
    #[expect(
        clippy::disallowed_methods,
        reason = "agent-lifecycle event handler runs as background system with no caller identity"
    )]
    let sys_ctx = ScopeContext::system();
    let status = match tasks.find_by_id(&event.task_id, &sys_ctx).await {
        Ok(Some(task)) => task.status,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(task_id = %event.task_id, error = %e, "Failed to read task status on agent start");
            return;
        }
    };
    if status != TaskStatus::Error {
        return;
    }
    // `_if_not_closed`: a close that lands between the read and this write wins.
    if let Err(e) = tasks
        .update_status_if_not_closed(&event.task_id, TaskStatus::Open, &sys_ctx)
        .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to reopen errored task on agent start");
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
    if let Some(agent_turn_id) = agent_turn_id
        && let Some(turn_number) =
            resolve_turn_number(state, &event.task_id, event.turn_number).await
        && let Err(e) = state
            .task_turns
            .set_agent_turn_id(&event.task_id, turn_number, agent_turn_id)
            .await
    {
        tracing::warn!(task_id = %event.task_id, error = %e, "Failed to record agent turn id");
    }
}

/// Handle `agent.stopped` event — the agent was stopped before completing.
///
/// Nothing about a stop enters the conversation, nor a `created_by` parent's:
/// the turn state already says so, the next run reads that status to decide it
/// needs a fresh session, and whoever stopped it — often the parent — knows.
pub async fn handle_agent_stopped(state: &AppState, event: &AgentEvent) {
    #[expect(
        clippy::disallowed_methods,
        reason = "agent-lifecycle event handler runs as background system with no caller identity"
    )]
    let sys_ctx = ScopeContext::system();
    // Idempotency: the sandbox and the /stop handler can both publish
    // `agent.stopped` for one user action, and a hammered /stop must never
    // re-run it. The event log is the guard: an already-terminal run means
    // this event adds nothing.
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
    super::metrics::record_run("stopped", event.agent_profile.as_deref());

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
                event.agent_profile.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
    }
}

/// The ports an errored run is closed over, and a child's notice to its parent
/// goes through.
///
/// Two callers reach the error path: the event consumer, which projects it out of
/// `AppState`, and the task reaper, which holds the same ports directly.
#[derive(Clone)]
pub struct AgentErrorPorts {
    pub tasks: Arc<TaskCommands>,
    pub task_relations: Arc<TaskRelations>,
    pub message_send: Arc<MessageSendService>,
    pub sandbox_waker: Arc<dyn SandboxWaker>,
    pub events: Option<Arc<dyn EventPublisher>>,
    pub task_turns: Arc<TaskTurns>,
}

/// Handle `agent.error` event.
pub async fn handle_agent_error(ports: &AgentErrorPorts, event: &AgentEvent) {
    finish_errored_run(ports, event, true).await;
}

/// Record an errored run; `reopen` returns the task to `Open` and tells a
/// `created_by` parent.
async fn finish_errored_run(ports: &AgentErrorPorts, event: &AgentEvent, reopen: bool) {
    #[expect(
        clippy::disallowed_methods,
        reason = "agent-lifecycle event handler runs as background system with no caller identity"
    )]
    let sys_ctx = ScopeContext::system();
    // Prometheus: count the errored run. Reached both from the `agent.error`
    // dispatch arm and from `handle_agent_completed`'s `is_error` short-circuit
    // (which returns before recording a `completed` outcome), so an error run is
    // never double-counted as completed.
    super::metrics::record_run("error", event.agent_profile.as_deref());

    let error_msg = event
        .payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown agent error");

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    // No transcript row: a run outcome isn't something somebody said. The one
    // failure worth reading — the sandbox dying under the agent — already
    // reaches the conversation as a `sandbox_error` activity with exit code,
    // reason and logs; this leaves only the turn status and Events tab.

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
    let child_not_closed = reopen
        && match ports
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
                event.agent_profile.as_deref(),
                event.turn_number,
                event.payload.clone(),
            )
            .await;
    }

    if child_not_closed {
        notify_parent_of_child_outcome(ports, event, ChildOutcome::Errored { error: error_msg })
            .await;
    }
}

/// How a `created_by` child's run ended, as its parent is told.
enum ChildOutcome<'a> {
    Completed { turn_number: Option<i32> },
    Errored { error: &'a str },
}

/// The child turn this outcome ends: the event's own, else the child's latest.
async fn outcome_turn(
    ports: &AgentErrorPorts,
    event: &AgentEvent,
    outcome: &ChildOutcome<'_>,
) -> Option<i32> {
    let turn_number = match outcome {
        ChildOutcome::Completed { turn_number } => *turn_number,
        ChildOutcome::Errored { .. } => event.turn_number,
    };
    match turn_number {
        Some(turn) => Some(turn),
        None => ports
            .task_turns
            .latest_turn_number(&event.task_id)
            .await
            .ok()
            .flatten(),
    }
}

/// Whether the parent triggered `turn_number`. A turn that cannot be resolved,
/// or whose trigger cannot be read, still notifies: a lost notice strands a
/// waiting parent, a spurious one only wakes it.
async fn turn_triggered_by(
    ports: &AgentErrorPorts,
    event: &AgentEvent,
    turn_number: Option<i32>,
    parent_task_id: &str,
) -> bool {
    let Some(turn_number) = turn_number else {
        return true;
    };
    match ports
        .task_turns
        .triggered_by_task_id(&event.task_id, turn_number)
        .await
    {
        Ok(trigger) => trigger.as_deref() == Some(parent_task_id),
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                turn_number,
                error = %e,
                "Failed to read turn trigger for child notice"
            );
            true
        }
    }
}

/// Send the parent task a notice that this child's run ended, and wake it.
///
/// `created_by` only — a fork's outcomes stay in its own conversation since
/// nobody awaits a fork's result. A closed parent is skipped, and so is a turn
/// the parent did not trigger (see [`TaskTurns::triggered_by_task_id`]). The
/// notice says only how the run ended; the child still reports its own result.
async fn notify_parent_of_child_outcome(
    ports: &AgentErrorPorts,
    event: &AgentEvent,
    outcome: ChildOutcome<'_>,
) {
    let relation = match ports.task_relations.find_parent(&event.task_id).await {
        Ok(Some(relation)) if relation.relation_type == TaskRelationType::CreatedBy => relation,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                error = %e,
                "Failed to load parent task relation for child notice"
            );
            return;
        }
    };

    let outcome_turn = outcome_turn(ports, event, &outcome).await;
    if !turn_triggered_by(ports, event, outcome_turn, &relation.to_task_id).await {
        return;
    }

    // Each notice is keyed on its Turn rather than on its text. A completion's
    // text carries the child's current name, which a rename can change between
    // two deliveries of one event; one Sandbox failure publishes several
    // errors (`pod_evicted`, then `fatal`), and the parent needs only the first.
    let turn_key = |kind: &str, turn_number: Option<i32>| {
        let turn = turn_number.map_or_else(|| event.event_id.clone(), |t| t.to_string());
        let key = format!(
            "child-{kind}\u{1f}{}\u{1f}{}\u{1f}{turn}",
            event.task_id, relation.to_task_id
        );
        Some(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, key.as_bytes()).to_string())
    };
    let (message, message_uuid, delivered_outcome) = match outcome {
        ChildOutcome::Errored { error } => (
            format!("Child task `{}` errored.\n\nError: {error}", event.task_id),
            turn_key("errored", outcome_turn),
            moatless_tasks::inbox::ChildOutcome::Errored,
        ),
        ChildOutcome::Completed { turn_number } => (
            completion_notice(ports, &event.task_id, turn_number).await,
            turn_key("completed", turn_number),
            moatless_tasks::inbox::ChildOutcome::Completed,
        ),
    };

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
            message_uuid,
            // Notices that land during one parent turn open the next as one
            // delivery, rather than each steering into the turn.
            true,
            Some(delivered_outcome),
            None,
        )
        .await
    {
        // `Ok(None)` is a redelivery whose notice is already queued; its
        // persisted row, not a second wake, is what the reconciler acts on.
        Ok(None) => {}
        Ok(Some(_message_uuid)) => {
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
                "Skipping child notice because parent task is closed"
            );
        }
        Err(e) => {
            tracing::warn!(
                task_id = %event.task_id,
                parent_task_id = %relation.to_task_id,
                error = %e,
                "Failed to notify parent task about child outcome"
            );
        }
    }
}

/// The text of a child's completion notice: which child, which Turn, and
/// nothing of what it produced.
async fn completion_notice(
    ports: &AgentErrorPorts,
    child_task_id: &str,
    turn_number: Option<i32>,
) -> String {
    #[expect(
        clippy::disallowed_methods,
        reason = "agent-lifecycle event handler runs as background system with no caller identity"
    )]
    let sys_ctx = ScopeContext::system();
    let name = match ports.tasks.find_by_id(child_task_id, &sys_ctx).await {
        Ok(task) => task.and_then(|t| t.name).filter(|n| !n.trim().is_empty()),
        Err(e) => {
            tracing::warn!(task_id = %child_task_id, error = %e, "Failed to read child task name");
            None
        }
    };
    let child = match name {
        Some(name) => format!("Child task `{child_task_id}` ({name})"),
        None => format!("Child task `{child_task_id}`"),
    };
    let turn = turn_number.map_or_default(|t| format!(" turn {t}"));
    format!(
        "{child} completed{turn}. This notice is automatic and carries no result; \
         read the child's conversation if you need one. It needs no reply."
    )
}

/// Handle a `sandbox.error` event: close the run as errored, unless a pod died
/// that its replacement will resume (see [`POD_REPLACED_LIFECYCLE_EVENTS`]).
pub async fn handle_sandbox_fatal(ports: &AgentErrorPorts, event: &AgentEvent) {
    let lifecycle_event = event
        .payload
        .get("lifecycleEvent")
        .and_then(serde_json::Value::as_str);
    if lifecycle_event.is_some_and(|e| POD_REPLACED_LIFECYCLE_EVENTS.contains(&e))
        && sandbox_still_desired_running(ports, &event.task_id).await
    {
        tracing::info!(
            task_id = %event.task_id,
            lifecycle_event = ?lifecycle_event,
            "Sandbox pod died under a running intent; leaving the turn for its replacement"
        );
        return;
    }
    let marked_errored =
        lifecycle_event.is_some_and(|e| TASK_ERRORED_LIFECYCLE_EVENTS.contains(&e));
    finish_errored_run(ports, event, !marked_errored).await;
}

/// `lifecycleEvent`s for a pod that died while its Deployment schedules a
/// replacement. Closing the turn on these deliberately does not happen: the
/// replacement would find no turn to replay or note, and the resumed agent
/// reads its cut-off tool call as the user interrupting. `auto_stopped` and the
/// reaper's recovery window close a turn no replacement resumes.
const POD_REPLACED_LIFECYCLE_EVENTS: &[&str] = &["pod_failed", "pod_evicted", "fatal"];

/// Whether `task_id`'s Sandbox has an explicit `running` intent. A failed read
/// answers `false`, so the turn closes as it would without this check.
async fn sandbox_still_desired_running(ports: &AgentErrorPorts, task_id: &str) -> bool {
    match ports.tasks.get_sandbox_intents(&[task_id.to_owned()]).await {
        Ok(mut intents) => intents
            .remove(task_id)
            .filter(moatless_tasks::task::models::SandboxIntentView::has_explicit_intent)
            .is_some_and(|intent| {
                intent.desired_state == moatless_types::sandbox::SandboxDesiredState::Running
            }),
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to read Sandbox intent after a pod failure");
            false
        }
    }
}

pub fn handle_agent_session_context_lost(_state: &AppState, event: &AgentEvent) {
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

/// Handle `task.metadata.updated` event.
pub async fn handle_task_metadata_updated(state: &AppState, event: &AgentEvent) {
    #[expect(
        clippy::disallowed_methods,
        reason = "agent-lifecycle event handler runs as background system with no caller identity"
    )]
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
