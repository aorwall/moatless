//! What a run's ending and the agent's own context work add to a V2 thread:
//! failures and their class, interrupts, background commands, context usage,
//! compactions, agent switches as handoffs, and child-outcome notices.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::model::provider_instance;
use super::scalar::instant;
use super::transcript::activity_anchor;
use super::turn::turn_id;
use super::turn::turn_is_open;
use super::turn::turn_state;
use super::v2_provider::provider_thread_id;
use super::v2_provider::root_node_id;
use crate::dao::Conversation;
use crate::providers;
use crate::transcript::{Transcript, TranscriptMessage, TurnOutcome};
use moatless_types::activity::{Activity, ActivityKind};
use moatless_types::agent::AgentProfile;

/// `OrchestrationV2ProviderFailure.message`'s ceiling, in characters.
const FAILURE_MESSAGE_MAX_CHARS: usize = 4_096;

/// `OrchestrationV2ProviderFailure.code`'s ceiling, in characters.
const FAILURE_CODE_MAX_CHARS: usize = 128;

/// `OrchestrationV2ProviderFailureClass` for an `agent.error` reason, which
/// the Sandbox tags from its `ErrorClass`. A failure with no reason is
/// `unknown`.
pub(super) fn failure_class(reason: Option<&str>) -> &'static str {
    match reason {
        Some("usage_limit") => "usage_limit",
        Some("authentication") => "permission_error",
        Some("connectivity") => "transport_error",
        Some("invalid_configuration" | "invalid_input") => "validation_error",
        Some(
            "rate_limit"
            | "provider_unavailable"
            | "unavailable_session"
            | "native_protocol"
            | "process_exit"
            | "unresponsive_process",
        ) => "provider_error",
        _ => "unknown",
    }
}

/// Whether retrying the same request can succeed, where the class says.
fn retryable(class: &str) -> Value {
    match class {
        "transport_error" | "provider_error" => json!(true),
        "permission_error" | "validation_error" => json!(false),
        _ => Value::Null,
    }
}

/// A `resetsAt` as the wire's instant; `None` when absent or unparseable.
fn reset_at(resets_at: Option<&str>) -> Option<String> {
    let parsed = DateTime::parse_from_rfc3339(resets_at?).ok()?;
    Some(instant(parsed.with_timezone(&Utc)))
}

/// The shell's `lastErrorClass` and `usageLimitResetAt`: set only while the
/// newest turn ended in an error.
pub(super) fn shell_error_fields(conversation: &Conversation) -> (Value, Value) {
    let failed = conversation.turn_status.as_deref().and_then(turn_state) == Some("error");
    if !failed || turn_is_open(conversation) {
        return (Value::Null, Value::Null);
    }
    let reason = conversation.turn_error_reason.as_deref();
    let class = failure_class(reason);
    let reset = (class == "usage_limit")
        .then(|| reset_at(conversation.turn_error_resets_at.as_deref()))
        .flatten();
    (json!(class), reset.map_or(Value::Null, Value::String))
}

/// An `OrchestrationV2ProviderFailure` for a turn that ended in an error.
fn failure(outcome: &TurnOutcome) -> Value {
    let reason = outcome.reason.as_deref();
    let class = failure_class(reason);
    let message: String = outcome
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
        .unwrap_or("The agent stopped with an error.")
        .chars()
        .take(FAILURE_MESSAGE_MAX_CHARS)
        .collect();
    let code = reason
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(|code| {
            code.chars()
                .take(FAILURE_CODE_MAX_CHARS)
                .collect::<String>()
        });
    let mut failure = json!({
        "class": class,
        "message": message.trim(),
        "code": code,
        "retryable": retryable(class)
    });
    if class == "usage_limit" {
        failure["resetAt"] =
            reset_at(outcome.resets_at.as_deref()).map_or(Value::Null, Value::String);
    }
    failure
}

/// How each turn ended, by turn number. The newest turn is read off the
/// conversation, the same row the shell and `runs` report it from.
pub(super) fn turn_outcomes(
    conversation: &Conversation,
    transcript: &Transcript,
) -> HashMap<i32, TurnOutcome> {
    let mut outcomes: HashMap<i32, TurnOutcome> = transcript
        .turn_outcomes
        .iter()
        .map(|outcome| (outcome.turn_number, outcome.clone()))
        .collect();
    if let (Some(turn_number), Some(status)) = (
        conversation.turn_number,
        conversation.turn_status.as_deref(),
    ) {
        outcomes.insert(
            turn_number,
            TurnOutcome {
                turn_number,
                status: status.to_string(),
                started_at: conversation.turn_started_at,
                ended_at: conversation.turn_ended_at,
                error: conversation.turn_error.clone(),
                reason: conversation.turn_error_reason.clone(),
                resets_at: conversation.turn_error_resets_at.clone(),
            },
        );
    }
    outcomes
}

/// A settled turn's run status: `failed` for an error, `interrupted` for a
/// stop, else `completed`. A turn older than the newest is never open.
pub(super) fn settled_run_status(outcome: Option<&TurnOutcome>) -> &'static str {
    match outcome.and_then(|outcome| turn_state(&outcome.status)) {
        Some("error") => "failed",
        Some("interrupted") => "interrupted",
        _ => "completed",
    }
}

/// The items a run's ending adds on its root node: an `error` with the full
/// failure for a failed run, which the error and usage-limit banners read, and
/// a `run_interrupt_result` for a stopped one. Each pairs with the instant it
/// sorts at. `turns` are the turns `runs` reports as runs.
pub(super) fn run_ending_items(
    conversation: &Conversation,
    turns: &[i32],
    outcomes: &HashMap<i32, TurnOutcome>,
) -> Vec<(String, Value)> {
    turns
        .iter()
        .filter_map(|turn_number| {
            let outcome = outcomes.get(turn_number)?;
            let status = settled_run_status(Some(outcome));
            if status == "completed" {
                return None;
            }
            let run_id = turn_id(&conversation.task_id, *turn_number);
            let at = instant(
                outcome
                    .ended_at
                    .or(conversation.updated_at)
                    .unwrap_or_else(|| super::scalar::created_at(conversation)),
            );
            let base = json!({
                "threadId": conversation.task_id,
                "runId": run_id,
                "nodeId": root_node_id(&run_id),
                "providerThreadId": provider_thread_id(&conversation.task_id),
                "providerTurnId": Value::Null,
                "nativeItemRef": Value::Null,
                "parentItemId": Value::Null,
                "startedAt": at,
                "completedAt": at,
                "updatedAt": at
            });
            let fields = if status == "failed" {
                json!({
                    "id": format!("item:error:{run_id}"),
                    "status": "failed",
                    "title": Value::Null,
                    "type": "error",
                    "failure": failure(outcome)
                })
            } else {
                json!({
                    "id": format!("item:interrupt-result:{run_id}"),
                    "status": "interrupted",
                    "title": "Interrupted",
                    "type": "run_interrupt_result",
                    // No "by user": `reconcile_superseded_turns` also settles a turn a pod
                    // restart cut off as `stopped`, and the row cannot tell the two apart.
                    "message": "Run stopped before it finished"
                })
            };
            Some((at, super::v2_items::layered(base, fields)))
        })
        .collect()
}

/// A `run_interrupt_request` for the open turn whose agent did not confirm a
/// Stop, with the instant it sorts at. Its `unresponsiveAt` is what makes the
/// client offer Force stop; a confirmed Stop closes the turn instead, which
/// [`run_ending_items`] reports.
pub(super) fn unresponsive_stop_item(conversation: &Conversation) -> Option<(String, Value)> {
    let at = conversation.turn_stop_unresponsive_at?;
    let turn_number = conversation.turn_number?;
    if !turn_is_open(conversation) {
        return None;
    }
    let at = instant(at);
    let run_id = turn_id(&conversation.task_id, turn_number);
    let item = json!({
        "id": format!("item:interrupt-request:{run_id}"),
        "threadId": conversation.task_id,
        "runId": run_id,
        "nodeId": root_node_id(&run_id),
        "providerThreadId": provider_thread_id(&conversation.task_id),
        "providerTurnId": Value::Null,
        "nativeItemRef": Value::Null,
        "parentItemId": Value::Null,
        "status": "running",
        "title": "Stop requested",
        "startedAt": at,
        "completedAt": Value::Null,
        "updatedAt": at,
        "type": "run_interrupt_request",
        "message": "The agent did not confirm the stop",
        "unresponsiveAt": at
    });
    Some((at, item))
}

/// The `moat cmd` commands that will wake this Task once its Turn has ended,
/// which the client shows as Waiting. Children are deliberately left out
/// rather than inherited: a parent would read Waiting while any child works.
/// `background_task` rather than `command`: the client lets a `command` finish
/// the thread. A command whose label has not been mirrored yet goes unnamed.
pub(super) fn pending_background_tasks(conversation: &Conversation) -> Value {
    if turn_is_open(conversation) {
        return json!([]);
    }
    let count = usize::try_from(conversation.running_commands).unwrap_or(0);
    let commands = (0..count).map(|index| {
        let label = conversation
            .running_command_labels
            .get(index)
            .map(|label| label.trim())
            .filter(|label| !label.is_empty())
            .unwrap_or("Background command");
        json!({
            "taskId": format!("{}:command:{}", conversation.task_id, index + 1),
            "description": label,
            "kind": "background_task"
        })
    });
    Value::Array(commands.collect())
}

/// One context-window reading as V2's token usage. `None` for a reading with
/// no usable count.
fn token_usage(activity: &Activity) -> Option<Value> {
    let used = activity.payload.get("usedTokens")?.as_i64()?;
    if used < 0 {
        return None;
    }
    let mut usage = json!({ "usedTokens": used });
    if let Some(max) = activity
        .payload
        .get("maxTokens")
        .and_then(Value::as_i64)
        .filter(|max| *max > 0)
    {
        usage["maxTokens"] = json!(max);
    }
    Some(usage)
}

fn context_readings(transcript: &Transcript) -> impl Iterator<Item = &Activity> {
    transcript
        .activities
        .iter()
        .filter(|activity| activity.kind == ActivityKind::ContextWindowUpdated)
}

/// The newest context-window reading per run, as each provider turn's
/// `tokenUsage`, keyed by run id.
pub(super) fn token_usage_by_run(
    conversation: &Conversation,
    transcript: &Transcript,
) -> HashMap<String, Value> {
    let mut newest: HashMap<String, &Activity> = HashMap::new();
    for activity in context_readings(transcript) {
        let (run, _) = activity_anchor(conversation, transcript, activity);
        let Some(run) = run.as_str() else {
            continue;
        };
        newest
            .entry(run.to_string())
            .and_modify(|held| {
                if (held.position, held.ordinal) <= (activity.position, activity.ordinal) {
                    *held = activity;
                }
            })
            .or_insert(activity);
    }
    newest
        .into_iter()
        .filter_map(|(run, activity)| {
            let mut usage = token_usage(activity)?;
            usage["updatedAt"] = json!(instant(activity.created_at));
            Some((run, usage))
        })
        .collect()
}

/// The conversation's newest context-window reading, as the provider thread's
/// `contextUsage`; `null` before the agent reported one.
pub(super) fn context_usage(transcript: &Transcript) -> Value {
    context_readings(transcript)
        .max_by_key(|activity| (activity.position, activity.ordinal))
        .and_then(token_usage)
        .unwrap_or(Value::Null)
}

/// A `compaction` item's own fields.
pub(super) fn compaction_fields(conversation: &Conversation, activity: &Activity) -> Value {
    let mut fields = json!({ "type": "compaction", "driver": provider_instance(conversation) });
    if let Some(summary) = activity
        .payload
        .get("summary")
        .and_then(Value::as_str)
        .filter(|summary| !summary.trim().is_empty())
    {
        fields["summary"] = json!(summary);
    }
    fields
}

/// The handoff an agent switch stands for.
fn handoff_id(conversation: &Conversation, activity: &Activity) -> String {
    format!("{}:handoff:{}", conversation.task_id, activity.activity_id)
}

/// The provider instance an Agent profile in a switch's payload runs on.
fn switched_instance(activity: &Activity, key: &str) -> Option<&'static str> {
    let profile = AgentProfile::try_from(activity.payload.get(key)?.as_str()?).ok()?;
    Some(providers::instance_for(profile.harness()))
}

/// The turn an agent switch hands the conversation to: the first that started
/// at or after it, since a turn running at the switch finishes on the agent it
/// started with. `None` while no turn has.
pub(super) fn handoff_turn(
    conversation: &Conversation,
    transcript: &Transcript,
    activity: &Activity,
) -> Option<i32> {
    turn_outcomes(conversation, transcript)
        .into_values()
        .filter(|outcome| {
            outcome
                .started_at
                .is_some_and(|started| started >= activity.created_at)
        })
        .map(|outcome| outcome.turn_number)
        .min()
}

/// A `handoff` item's own fields. Moatless carries the whole history to the
/// new harness, so every switch is a `full_thread_summary`, on the one
/// provider thread the conversation has.
pub(super) fn handoff_fields(conversation: &Conversation, activity: &Activity) -> Value {
    let thread = provider_thread_id(&conversation.task_id);
    let from: Vec<&str> = switched_instance(activity, "from").into_iter().collect();
    let to = switched_instance(activity, "to").unwrap_or_else(|| provider_instance(conversation));
    let mut fields = json!({
        "type": "handoff",
        "contextHandoffId": handoff_id(conversation, activity),
        "fromProviderThreadIds": [thread],
        "toProviderThreadId": thread,
        "fromProviderInstanceIds": from,
        "toProviderInstanceId": to,
        "strategy": "full_thread_summary"
    });
    if !activity.summary.trim().is_empty() {
        fields["summary"] = json!(activity.summary);
    }
    fields
}

/// One `OrchestrationV2ContextHandoff` per agent switch a later run took,
/// `ready` since the pod transcodes the history before that run starts.
pub(super) fn context_handoffs(
    conversation: &Conversation,
    transcript: &Transcript,
    runs: &[Value],
) -> Vec<Value> {
    let thread = provider_thread_id(&conversation.task_id);
    transcript
        .activities
        .iter()
        .filter(|activity| activity.kind == ActivityKind::AgentSwitched)
        .filter_map(|activity| {
            let turn = handoff_turn(conversation, transcript, activity)?;
            // A switch before the first run hands no history over.
            if turn <= 1 {
                return None;
            }
            let target = turn_id(&conversation.task_id, turn);
            let ordinal = runs
                .iter()
                .find(|run| run.get("id").and_then(Value::as_str) == Some(target.as_str()))?
                .get("ordinal")?
                .as_i64()?;
            let at = instant(activity.created_at);
            Some(json!({
                "id": handoff_id(conversation, activity),
                "threadId": conversation.task_id,
                "targetRunId": target,
                "fromProviderThreadIds": [thread],
                "toProviderThreadId": thread,
                "coveredRunOrdinals": { "from": 1, "to": ordinal - 1 },
                "strategy": "full_thread_summary",
                "status": "ready",
                "summaryMessageId": Value::Null,
                "summaryText": activity.summary,
                "detailInTurnItem": true,
                "createdByProviderInstanceId": Value::Null,
                "createdAt": at,
                "updatedAt": at
            }))
        })
        .collect()
}

/// A child-outcome notice's `notification` fields: its first line is the
/// summary, the rest its detail. `None` for any other Message.
pub(super) fn notification(message: &TranscriptMessage, text: &str) -> Option<Value> {
    let outcome = match message.child_outcome.as_deref()? {
        "completed" => "completed",
        "errored" => "failed",
        _ => "unknown",
    };
    let text = text.trim();
    let (summary, detail) = text.split_once('\n').unwrap_or((text, ""));
    let summary = Some(summary.trim())
        .filter(|summary| !summary.is_empty())
        .unwrap_or("Child task finished");
    let mut source = json!({ "kind": "delegated_task", "taskIds": [] });
    if let Some(child) = message.source_task_id.as_deref() {
        source["childThreadId"] = json!(child);
    }
    let mut notification = json!({ "source": source, "outcome": outcome, "summary": summary });
    if !detail.trim().is_empty() {
        notification["detail"] = json!(detail.trim());
    }
    Some(notification)
}
