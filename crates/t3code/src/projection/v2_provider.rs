//! The provider session, provider thread, attempts and provider turns an
//! `OrchestrationV2ThreadProjection` carries.
//!
//! Moatless runs one agent session per Task and keeps no record of the
//! provider's own ids, so each is derived: one session and one provider thread
//! per conversation, one attempt and one provider turn per started run. The
//! client reads them to offer promote-to-steer and to know what may be forked.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::model::provider_instance;
use super::scalar::created_at;
use super::scalar::instant;
use super::session::session_status;
use super::turn::turn_is_open;
use super::v2_lifecycle::pending_background_tasks;
use crate::dao::Conversation;
use crate::nominal_path;

/// The provider thread every run of a conversation belongs to.
pub(super) fn provider_thread_id(task_id: &str) -> String {
    format!("{task_id}:provider-thread")
}

fn provider_session_id(task_id: &str) -> String {
    format!("{task_id}:provider-session")
}

/// The single attempt a started run is made of.
pub(super) fn attempt_id(run_id: &str) -> String {
    format!("{run_id}:attempt")
}

fn provider_turn_id(run_id: &str) -> String {
    format!("{run_id}:provider-turn")
}

pub(super) fn root_node_id(run_id: &str) -> String {
    format!("{run_id}:root")
}

/// What the agent in a Moatless Sandbox can do, as V2's capability record.
///
/// Restart is an interrupt then a queued send (`v2_commands`). A fork copies
/// the transcript, so it is a full handoff; so is a turn naming the other
/// instance, which switches the Task's harness. Nothing asks for approval.
fn capabilities() -> Value {
    json!({
        "sessions": {
            "supportsMultipleProviderThreadsPerSession": false,
            "supportsModelSwitchInSession": true,
            "supportsProviderSwitchingViaHandoff": true,
            "supportsRuntimeModeSwitchInSession": false,
            "pendingRequestsSurviveRestart": true
        },
        "threads": {
            "canCreateEmptyThread": true,
            "canReadThreadSnapshot": true,
            "canRollbackThread": false,
            "canForkThread": true,
            "canForkFromTurn": true,
            "canForkFromSubagentThread": false,
            "exposesNativeThreadId": false
        },
        "turns": {
            "exposesNativeTurnId": false,
            "emitsTurnStarted": true,
            "emitsTurnCompleted": true,
            "supportsInterrupt": true,
            "supportsActiveSteering": true,
            "supportsSteeringByInterruptRestart": true,
            "supportsQueuedMessages": true,
            "terminalStatusQuality": "strong"
        },
        "streaming": {
            "streamsAssistantText": false,
            "streamsReasoning": false,
            "streamsToolOutput": false,
            "streamsPlanText": false,
            "emitsMessageCompleted": true
        },
        "tools": {
            "exposesToolItemIds": true,
            "emitsToolStarted": true,
            "emitsToolCompleted": true,
            "emitsToolOutput": true,
            "supportsMcpTools": true,
            "supportsDynamicToolCallbacks": false
        },
        "approvals": {
            "supportsCommandApproval": false,
            "supportsFileReadApproval": false,
            "supportsFileChangeApproval": false,
            "supportsApplyPatchApproval": false,
            "approvalsHaveNativeRequestIds": false,
            "approvalCallbacksAreLiveOnly": false,
            "approvalsCanOriginateFromSubagents": false
        },
        "planning": {
            "emitsPlanUpdated": true,
            "emitsTodoList": true,
            "emitsProposedPlan": true,
            "supportsStructuredQuestions": true,
            "planDeltasHaveItemIds": false
        },
        "subagents": {
            "supportsSubagents": true,
            "exposesSubagentThreadIds": false,
            "emitsSubagentLifecycle": true,
            "canWaitForSubagents": false,
            "canCloseSubagents": false,
            "canForkSubagentThread": false
        },
        "context": {
            "acceptsSystemContext": false,
            "acceptsDeveloperContext": false,
            "acceptsSyntheticUserContext": true,
            "canGenerateSummaries": false,
            "canConsumeHandoffSummaries": false,
            "supportsDeltaHandoff": false,
            "supportsFullThreadHandoff": true,
            "maxRecommendedHandoffChars": null
        },
        "checkpointing": {
            "appCanCheckpointFilesystem": false,
            "supportsNestedCheckpointScopes": false,
            "providerCanRollbackConversation": false,
            "providerRollbackReturnsSnapshot": false,
            "providerCanReadConversationSnapshot": false
        },
        "identity": {
            "nativeThreadIds": "none",
            "nativeTurnIds": "none",
            "nativeItemIds": "weak",
            "nativeRequestIds": "weak"
        },
        "runtimePolicy": { "enforcement": "native" }
    })
}

/// The conversation's one provider session. `updated_at` is the projection's
/// own instant, so the session moves with everything else it is sent beside.
pub(super) fn provider_session(
    conversation: &Conversation,
    model: &Value,
    updated_at: &str,
) -> Value {
    let status = conversation
        .sandbox_desired_state()
        .map_or("stopped", |desired| session_status(conversation, desired));
    json!({
        "id": provider_session_id(&conversation.task_id),
        "driver": provider_instance(conversation),
        "providerInstanceId": provider_instance(conversation),
        "status": status,
        "cwd": nominal_path::nominal_worktree(&conversation.task_id),
        "model": model.as_str().filter(|model| !model.trim().is_empty()),
        "capabilities": capabilities(),
        "createdAt": instant(created_at(conversation)),
        "updatedAt": updated_at,
        "lastError": Value::Null
    })
}

/// The conversation's one provider thread, spanning every started run.
pub(super) fn provider_thread(
    conversation: &Conversation,
    runs: &[Value],
    updated_at: &str,
) -> Value {
    // A rolled-back run left the agent's conversation, as upstream's rollback
    // moves `lastRunOrdinal` down to its target.
    let ordinals: Vec<i64> = started(runs)
        .filter(|run| !is_rolled_back(run))
        .filter_map(|run| run.get("ordinal").and_then(Value::as_i64))
        .collect();
    json!({
        "id": provider_thread_id(&conversation.task_id),
        "driver": provider_instance(conversation),
        "providerInstanceId": provider_instance(conversation),
        "providerSessionId": provider_session_id(&conversation.task_id),
        "appThreadId": conversation.task_id,
        "ownerNodeId": Value::Null,
        "nativeThreadRef": Value::Null,
        "nativeConversationHeadRef": Value::Null,
        "status": if turn_is_open(conversation) { "active" } else { "idle" },
        "firstRunOrdinal": ordinals.iter().min(),
        "lastRunOrdinal": ordinals.iter().max(),
        "handoffIds": [],
        "forkedFrom": Value::Null,
        "pendingBackgroundTasks": pending_background_tasks(conversation),
        "createdAt": instant(created_at(conversation)),
        "updatedAt": updated_at
    })
}

/// One attempt per started run, carrying the run's own status and instants.
pub(super) fn attempts(runs: &[Value]) -> Vec<Value> {
    started(runs)
        .filter_map(|run| {
            let run_id = run.get("id")?.as_str()?;
            Some(json!({
                "id": attempt_id(run_id),
                "runId": run_id,
                "attemptOrdinal": 1,
                "rootNodeId": root_node_id(run_id),
                "providerInstanceId": run.get("providerInstanceId"),
                "providerThreadId": run.get("providerThreadId"),
                "providerTurnId": provider_turn_id(run_id),
                "reason": "initial",
                "status": execution_status(run),
                "startedAt": run.get("startedAt"),
                "completedAt": run.get("completedAt")
            }))
        })
        .collect()
}

/// One `root_turn` node per started run: the node its attempt, provider turn
/// and plans hang from.
pub(super) fn root_nodes(runs: &[Value]) -> Vec<Value> {
    started(runs)
        .filter_map(|run| {
            let run_id = run.get("id")?.as_str()?;
            Some(json!({
                "id": root_node_id(run_id),
                "threadId": run.get("threadId"),
                "runId": run_id,
                "parentNodeId": Value::Null,
                "rootNodeId": root_node_id(run_id),
                "kind": "root_turn",
                "status": if is_rolled_back(run) { "rolled_back" } else { execution_status(run) },
                "countsForRun": true,
                "providerThreadId": run.get("providerThreadId"),
                "providerTurnId": provider_turn_id(run_id),
                "nativeItemRef": Value::Null,
                "runtimeRequestId": Value::Null,
                "checkpointScopeId": Value::Null,
                "startedAt": run.get("startedAt"),
                "completedAt": run.get("completedAt")
            }))
        })
        .collect()
}

/// One provider turn per started run, the one its attempt ran as, carrying
/// the run's newest context-window reading from `token_usage` (by run id).
pub(super) fn provider_turns(runs: &[Value], token_usage: &HashMap<String, Value>) -> Vec<Value> {
    started(runs)
        .filter_map(|run| {
            let run_id = run.get("id")?.as_str()?;
            let mut turn = json!({
                "id": provider_turn_id(run_id),
                "providerThreadId": run.get("providerThreadId"),
                "nodeId": root_node_id(run_id),
                "runAttemptId": attempt_id(run_id),
                "nativeTurnRef": Value::Null,
                "ordinal": run.get("ordinal"),
                "status": execution_status(run),
                "startedAt": run.get("startedAt"),
                "completedAt": run.get("completedAt")
            });
            if let Some(usage) = token_usage.get(run_id) {
                turn["tokenUsage"] = usage.clone();
            }
            Some(turn)
        })
        .collect()
}

/// Whether a turn has taken this run: neither a queued one nor one still
/// `starting`.
pub(super) fn has_started(run: &Value) -> bool {
    !matches!(
        run.get("status").and_then(Value::as_str),
        Some("queued" | "starting")
    )
}

/// Whether a rewind cut this run. Upstream marks such a run and its root node
/// `rolled_back`, and leaves its attempt and provider turn as they ended.
fn is_rolled_back(run: &Value) -> bool {
    run.get("status").and_then(Value::as_str) == Some("rolled_back")
}

/// The runs a turn has taken.
fn started(runs: &[Value]) -> impl Iterator<Item = &Value> {
    runs.iter().filter(|run| has_started(run))
}

/// A run's status in the vocabulary attempts and provider turns share.
fn execution_status(run: &Value) -> &'static str {
    match run.get("status").and_then(Value::as_str) {
        Some("running") => "running",
        Some("failed") => "failed",
        Some("interrupted") => "interrupted",
        _ => "completed",
    }
}
