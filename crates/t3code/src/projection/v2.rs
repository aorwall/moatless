//! Orchestration V2 projections, built from the V1 row builders beside it.
//!
//! Only `thread`, `runs`, `turnItems`, `messages`, `runtimeRequests` and
//! `lineage` carry Moatless state; every other array has no Moatless
//! counterpart and is always empty. Turn items cover messages, reasoning,
//! shell commands, questions the agent asked and the fork marker; other
//! activity kinds are left out of the timeline rather than guessed at.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::model::model_selection;
use super::model::provider_instance;
use super::origin::OriginContext;
use super::project::project;
use super::scalar::created_at;
use super::scalar::instant;
use super::scalar::non_empty;
use super::scalar::nullable_instant;
use super::shell::conversation_row;
use super::transcript::activity_anchor;
use super::transcript::message_bubble;
use super::transcript::message_instant;
use super::turn::turn_id;
use super::turn::turn_is_open;
use super::turn::turn_reference;
use super::turn::turn_state;
use crate::AgentModelDefaults;
use crate::dao::Conversation;
use crate::transcript_dao::Transcript;
use crate::transcript_dao::TranscriptMessage;
use crate::workspace::UiRpcListedWorkspace;
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityStatus;
use moatless_types::activity::item_type;
use moatless_types::activity::result_text;
use moatless_types::message::MessageType;
use moatless_types::message::TOOL_RESPONSE_USER_TYPE;
use moatless_types::task::TaskRelationType;

/// Every V2 thread reports the contract's default interaction mode: Moatless
/// has no state distinguishing a "plan" mode from the agent's own tool use.
const DEFAULT_INTERACTION_MODE: &str = "default";

/// One thread's resolved ancestry, read once per request rather than per shell
/// row — see [`Lineage::none`] for the listing-scope default.
#[derive(Clone)]
pub(crate) struct Lineage {
    parent_task_id: Option<String>,
    /// `Some("fork")` or `Some("subagent")`; `None` for a thread with no
    /// parent edge.
    relationship: Option<&'static str>,
    root_task_id: String,
    /// The turn the fork cut from, read off `task_relations.context`. `None`
    /// means "the whole thread as of the fork", V2's `latest_stable`.
    fork_at_turn: Option<i32>,
    created_by: &'static str,
    creation_source: &'static str,
}

impl Lineage {
    /// No ancestry resolved: the default for shell-listing rows, where
    /// resolving every row's parent would cost one relations query per
    /// conversation on every poll. The single-thread surfaces
    /// (`getThreadProjection`/`subscribeThread`/`launchThread`) call
    /// [`Lineage::root`] or [`Lineage::child`] instead.
    pub(crate) fn none(task_id: &str) -> Self {
        Self {
            parent_task_id: None,
            relationship: None,
            root_task_id: task_id.to_string(),
            fork_at_turn: None,
            created_by: "user",
            creation_source: "web",
        }
    }

    /// A thread with no parent edge, lineage fully resolved: it is its own
    /// root.
    pub(crate) fn root(task_id: &str) -> Self {
        Self::none(task_id)
    }

    /// A thread with a resolved parent edge.
    pub(crate) fn child(
        parent_task_id: String,
        relation: TaskRelationType,
        root_task_id: String,
        fork_at_turn: Option<i32>,
    ) -> Self {
        let (relationship, created_by, creation_source) = match relation {
            TaskRelationType::ForkedFrom => ("fork", "user", "web"),
            // A delegated child is spawned by the agent. `"mcp"` is what makes
            // T3 treat it as message-able rather than a provider-native subagent.
            _ => ("subagent", "agent", "mcp"),
        };
        Self {
            parent_task_id: Some(parent_task_id),
            relationship: Some(relationship),
            root_task_id,
            fork_at_turn,
            created_by,
            creation_source,
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "parentThreadId": self.parent_task_id,
            "relationshipToParent": self.relationship,
            "rootThreadId": self.root_task_id
        })
    }

    fn forked_from_value(&self) -> Value {
        let Some(parent) = self.parent_task_id.as_deref() else {
            return Value::Null;
        };
        if self.relationship != Some("fork") {
            return Value::Null;
        }
        let at_turn = self.fork_at_turn.unwrap_or(1).max(1);
        json!({ "type": "run", "threadId": parent, "runId": turn_id(parent, at_turn) })
    }
}

/// The fields `OrchestrationV2AppThread` and `OrchestrationV2ThreadShell`
/// share, layered on top of [`conversation_row`]. A V2 row keeps the V1 row's
/// extra `session` key, which the V2 contract ignores.
fn v2_common_fields(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    lineage: &Lineage,
) -> Map<String, Value> {
    let mut row = conversation_row(conversation, project_id, defaults);
    row.insert("createdBy".into(), json!(lineage.created_by));
    row.insert("creationSource".into(), json!(lineage.creation_source));
    row.insert(
        "providerInstanceId".into(),
        json!(provider_instance(conversation)),
    );
    row.insert("interactionMode".into(), json!(DEFAULT_INTERACTION_MODE));
    // No Moatless state names the provider's own thread id separately from
    // the Task.
    row.insert("activeProviderThreadId".into(), Value::Null);
    row.insert("lineage".into(), lineage.to_value());
    row.insert("forkedFrom".into(), lineage.forked_from_value());
    row.insert("lastVisitedAt".into(), Value::Null);
    row.insert("deletedAt".into(), Value::Null);
    row
}

/// The thread half of a single-thread `OrchestrationV2ThreadProjection`.
pub(crate) fn app_thread(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    lineage: &Lineage,
) -> Value {
    Value::Object(v2_common_fields(
        conversation,
        project_id,
        defaults,
        lineage,
    ))
}

/// One conversation's row in the V2 shell listing.
///
/// `item_count` is `0` at listing scope: counting a conversation's turn items
/// would mean loading its transcript on every poll of every row, which the
/// shell listing — unlike a single open thread — cannot afford. Only
/// [`thread_projection`] reports a real count.
pub(crate) fn thread_shell_v2(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    lineage: &Lineage,
) -> Value {
    let mut row = v2_common_fields(conversation, project_id, defaults, lineage);
    row.insert("status".into(), json!(shell_status(conversation)));
    row.insert("pendingRuntimeRequest".into(), Value::Null);
    row.insert("latestVisibleMessage".into(), Value::Null);
    row.insert(
        "lastError".into(),
        conversation
            .turn_error
            .as_deref()
            .map_or(Value::Null, |error| json!(error)),
    );
    row.insert(
        "hasActionableProposedPlan".into(),
        json!(conversation.pending_plan),
    );
    row.insert(
        "latestUserMessageAt".into(),
        nullable_instant(conversation.latest_user_message_at),
    );
    let (latest_run_id, active_run_id) = run_ids(conversation);
    row.insert("latestRunId".into(), latest_run_id);
    row.insert("activeRunId".into(), active_run_id);
    row.insert("itemCount".into(), json!(0));
    row.insert("visibleItemCount".into(), json!(0));
    row.insert(
        "ownerName".into(),
        conversation
            .owner_name
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
    );
    Value::Object(row)
}

/// One conversation's row changing, as a V2 shell stream item. A closed Task
/// moves to the archive rather than leaving the listing.
pub(crate) fn thread_updated_v2(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    sequence: i64,
) -> Value {
    let location = if conversation.closed_at.is_some() {
        "archive"
    } else {
        "active"
    };
    json!({
        "kind": "thread.updated",
        "sequence": sequence,
        "location": location,
        "thread": thread_shell_v2(conversation, project_id, defaults, &Lineage::none(&conversation.task_id))
    })
}

/// One conversation leaving the V2 listing. Built under the same rule as
/// [`super::thread_removed`]: only for a conversation the viewer was sent.
pub(crate) fn thread_removed_v2(task_id: &str, sequence: i64) -> Value {
    json!({
        "kind": "thread.removed",
        "sequence": sequence,
        "location": "active",
        "threadId": task_id
    })
}

/// `OrchestrationV2ShellThreadStatus`: `idle` plus the run-status vocabulary.
fn shell_status(conversation: &Conversation) -> &'static str {
    let Some(status) = conversation.turn_status.as_deref() else {
        return "idle";
    };
    let Some(state) = turn_state(status) else {
        return "idle";
    };
    if turn_is_open(conversation) {
        return "running";
    }
    match state {
        "error" => "failed",
        "interrupted" => "interrupted",
        _ => "idle",
    }
}

/// The newest turn's run id, twice: once unconditionally
/// (`latestRunId`), once only while it is still open (`activeRunId`).
fn run_ids(conversation: &Conversation) -> (Value, Value) {
    let Some(turn_number) = conversation.turn_number else {
        return (Value::Null, Value::Null);
    };
    let run_id = json!(turn_id(&conversation.task_id, turn_number));
    if turn_is_open(conversation) {
        (run_id.clone(), run_id)
    } else {
        (run_id, Value::Null)
    }
}

/// The V2 shell snapshot: open and archived rows share one listing, split by
/// which array a project's threads are dropped into.
pub(crate) fn shell_snapshot_v2(
    workspaces: &[UiRpcListedWorkspace],
    conversations: &[Conversation],
    archived: &[Conversation],
    defaults: &AgentModelDefaults,
    sequence: i64,
) -> Value {
    json!({
        "schemaVersion": 1,
        "snapshotSequence": sequence,
        "projects": workspaces.iter().map(project).collect::<Vec<_>>(),
        "threads": listed_threads_v2(workspaces, conversations, defaults),
        "archivedThreads": listed_threads_v2(workspaces, archived, defaults)
    })
}

/// The archived-only snapshot `getArchivedShellSnapshot`/
/// `subscribeArchivedShell` answer with.
pub(crate) fn archived_shell_snapshot_v2(
    workspaces: &[UiRpcListedWorkspace],
    archived: &[Conversation],
    defaults: &AgentModelDefaults,
    sequence: i64,
) -> Value {
    json!({
        "schemaVersion": 1,
        "snapshotSequence": sequence,
        "projects": workspaces.iter().map(project).collect::<Vec<_>>(),
        "threads": listed_threads_v2(workspaces, archived, defaults)
    })
}

pub(crate) fn listed_threads_v2(
    workspaces: &[UiRpcListedWorkspace],
    conversations: &[Conversation],
    defaults: &AgentModelDefaults,
) -> Vec<Value> {
    let projects: HashSet<&str> = workspaces
        .iter()
        .map(|listed| listed.workspace.id.as_str())
        .collect();
    conversations
        .iter()
        .filter_map(|conversation| {
            let project_id = conversation
                .workspace_id
                .as_deref()
                .filter(|id| projects.contains(id))?;
            Some(thread_shell_v2(
                conversation,
                project_id,
                defaults,
                &Lineage::none(&conversation.task_id),
            ))
        })
        .collect()
}

/// A queued message's run id. Deliberately not `{taskId}:{n}`: the client's
/// settled-run rule parses that form as a turn that already ran.
pub(crate) fn queued_run_id(task_id: &str, message_uuid: &str) -> String {
    format!("{task_id}:queued:{message_uuid}")
}

/// The message a [`queued_run_id`] names.
pub(crate) fn queued_message_uuid(run_id: &str) -> Option<&str> {
    run_id
        .split_once(":queued:")
        .map(|(_, uuid)| uuid)
        .filter(|uuid| !uuid.is_empty())
}

/// The Moatless turn number a run id names; `None` for a queued run or an id
/// this server did not mint.
pub(crate) fn turn_number_of_run(run_id: &str) -> Option<i32> {
    if queued_message_uuid(run_id).is_some() {
        return None;
    }
    run_id
        .rsplit_once(':')?
        .1
        .parse()
        .ok()
        .filter(|turn| *turn > 0)
}

/// A message the person sent that no turn has taken yet.
fn is_queued(message: &TranscriptMessage) -> bool {
    message.turn_number.is_none()
        && message.message_type == MessageType::User
        && message.user_type != TOOL_RESPONSE_USER_TYPE
        && !message.sandbox_wake
}

/// One `OrchestrationV2Run` per Moatless turn in the window plus the current
/// turn, then one `queued` run per message no turn has taken yet. Every turn
/// older than the current one reports `completed`: a turn's own status is not
/// kept once a newer turn starts.
pub(crate) fn runs(
    conversation: &Conversation,
    transcript: &Transcript,
    defaults: &AgentModelDefaults,
) -> Vec<Value> {
    let mut turn_numbers: Vec<i32> = transcript
        .messages
        .iter()
        .filter_map(|message| message.turn_number)
        .collect();
    turn_numbers.sort_unstable();
    turn_numbers.dedup();
    if let Some(current) = conversation.turn_number {
        if !turn_numbers.contains(&current) {
            turn_numbers.push(current);
        }
    }
    let last_turn = turn_numbers.last().copied().unwrap_or(0);
    let mut runs: Vec<Value> = turn_numbers
        .into_iter()
        .map(|turn_number| run_for_turn(conversation, transcript, defaults, turn_number))
        .collect();
    let queued = transcript
        .messages
        .iter()
        .filter(|message| is_queued(message));
    for (position, message) in queued.enumerate() {
        let at = instant(message_instant(conversation, message));
        let position = i32::try_from(position).unwrap_or(i32::MAX - 1) + 1;
        runs.push(json!({
            "id": queued_run_id(&conversation.task_id, &message.uuid),
            "threadId": conversation.task_id,
            "ordinal": last_turn.max(0).saturating_add(position),
            "providerInstanceId": provider_instance(conversation),
            "modelSelection": model_selection(conversation, defaults),
            "providerThreadId": Value::Null,
            "userMessageId": message.uuid,
            "rootNodeId": Value::Null,
            "activeAttemptId": Value::Null,
            "status": "queued",
            "queuePosition": position,
            "requestedAt": at,
            "startedAt": Value::Null,
            "completedAt": Value::Null,
            "checkpointId": Value::Null,
            "contextHandoffId": Value::Null
        }));
    }
    runs
}

fn run_for_turn(
    conversation: &Conversation,
    transcript: &Transcript,
    defaults: &AgentModelDefaults,
    turn_number: i32,
) -> Value {
    let in_turn: Vec<&TranscriptMessage> = transcript
        .messages
        .iter()
        .filter(|message| message.turn_number == Some(turn_number))
        .collect();
    let instants: Vec<chrono::DateTime<chrono::Utc>> = in_turn
        .iter()
        .map(|message| message_instant(conversation, message))
        .collect();
    let first_at = instants.iter().min().copied();
    let last_at = instants.iter().max().copied();
    let is_latest = conversation.turn_number == Some(turn_number);
    let status = if is_latest {
        match conversation.turn_status.as_deref().and_then(turn_state) {
            Some("running") => "running",
            Some("error") => "failed",
            Some("interrupted") => "interrupted",
            _ => "completed",
        }
    } else {
        "completed"
    };
    let user_message_id = in_turn
        .iter()
        .find(|message| {
            message.message_type == MessageType::User
                && message.user_type != TOOL_RESPONSE_USER_TYPE
                && !message.sandbox_wake
        })
        .map_or_else(
            || format!("{}:{turn_number}:user-message", conversation.task_id),
            |message| message.uuid.clone(),
        );
    let requested_at = first_at.unwrap_or_else(|| created_at(conversation));
    let completed_at = if is_latest {
        nullable_instant(conversation.turn_ended_at)
    } else {
        last_at.map_or(Value::Null, |at| json!(instant(at)))
    };
    json!({
        "id": turn_id(&conversation.task_id, turn_number),
        "threadId": conversation.task_id,
        "ordinal": turn_number.max(1),
        "providerInstanceId": provider_instance(conversation),
        "modelSelection": model_selection(conversation, defaults),
        "providerThreadId": Value::Null,
        "userMessageId": user_message_id,
        "rootNodeId": Value::Null,
        "activeAttemptId": Value::Null,
        "status": status,
        "requestedAt": instant(requested_at),
        "startedAt": instant(requested_at),
        "completedAt": completed_at,
        "checkpointId": Value::Null,
        "contextHandoffId": Value::Null
    })
}

/// A shell command's exit code, derived rather than stored: Moatless records
/// whether a tool call settled and whether it errored, not the process's own
/// exit status. `0`/`1` are a defensible stand-in while the call is settled;
/// `null` while it is still running, matching the contract's "fetch it with
/// getTurnItem" note for output withheld rather than unknown.
fn command_exit_code(activity: &Activity) -> Value {
    match activity.status {
        ActivityStatus::InProgress => Value::Null,
        ActivityStatus::Completed => json!(0),
        ActivityStatus::Failed => json!(1),
    }
}

fn activity_item_status(activity: &Activity) -> &'static str {
    match activity.status {
        ActivityStatus::InProgress => "running",
        ActivityStatus::Completed => "completed",
        ActivityStatus::Failed => "failed",
    }
}

/// Lay `extra`'s fields over `base`'s. A non-object `extra` adds nothing.
fn layered(base: Value, extra: Value) -> Value {
    match (base, extra) {
        (Value::Object(mut base), Value::Object(extra)) => {
            base.extend(extra);
            Value::Object(base)
        }
        (base, _) => base,
    }
}

/// Where a turn item sorts: its instant, then an activity written with a
/// message ahead of that message, then window order.
type TimelineKey = (String, u8, usize);

/// The turn items and conversation messages a transcript window renders, in
/// the scope this module documents at its head. Items are ordered by time,
/// messages and activities interleaved, and numbered in that order.
pub(crate) fn turn_items_and_messages(
    conversation: &Conversation,
    transcript: &Transcript,
    origins: &OriginContext<'_>,
) -> (Vec<Value>, Vec<Value>) {
    let mut timeline: Vec<(TimelineKey, Value)> = Vec::new();
    let mut messages = Vec::new();

    for (index, message) in transcript.messages.iter().enumerate() {
        let turn = if is_queued(message) {
            json!(queued_run_id(&conversation.task_id, &message.uuid))
        } else {
            turn_reference(conversation, message)
        };
        let at = instant(message_instant(conversation, message));
        let Some(bubble) = message_bubble(message, &turn, &at, origins) else {
            continue;
        };
        let role = bubble["role"].as_str().unwrap_or("assistant").to_string();
        let text = bubble["text"].as_str().unwrap_or_default().to_string();
        let attachments = bubble
            .get("attachments")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let origin = bubble.get("origin").cloned().unwrap_or(Value::Null);
        let (created_by, creation_source) = match role.as_str() {
            "user" => ("user", "web"),
            _ => ("agent", "provider"),
        };
        let base = json!({
            "id": format!("item:message:{}", message.uuid),
            "threadId": conversation.task_id,
            "runId": turn,
            "nodeId": Value::Null,
            "providerThreadId": Value::Null,
            "providerTurnId": Value::Null,
            "nativeItemRef": Value::Null,
            "parentItemId": Value::Null,
            "status": "completed",
            "title": Value::Null,
            "startedAt": at,
            "completedAt": at,
            "updatedAt": at
        });
        let item = match role.as_str() {
            "user" => layered(
                base,
                json!({
                    "createdBy": created_by,
                    "creationSource": creation_source,
                    "type": "user_message",
                    "messageId": message.uuid,
                    "inputIntent": "turn_start",
                    "text": text,
                    "origin": origin,
                    "attachments": attachments
                }),
            ),
            "assistant" => layered(
                base,
                json!({
                    "type": "assistant_message",
                    "messageId": message.uuid,
                    "text": text,
                    "attachments": attachments,
                    "streaming": false
                }),
            ),
            _ => layered(base, json!({ "type": "system_notice", "message": text })),
        };
        timeline.push(((at.clone(), 1, index), item));
        messages.push(json!({
            "createdBy": created_by,
            "creationSource": creation_source,
            "id": message.uuid,
            "threadId": conversation.task_id,
            "runId": turn,
            "nodeId": Value::Null,
            "role": role,
            "text": text,
            "origin": origin,
            "attachments": attachments,
            "streaming": false,
            "createdAt": at,
            "updatedAt": at
        }));
    }

    for (index, activity) in transcript.activities.iter().enumerate() {
        let (turn, at) = activity_anchor(conversation, transcript, activity);
        // Anchored to its message's instant: reasoning and tool calls come
        // before the reply they were written with.
        let anchored = activity.message_uuid.as_deref().is_some_and(|uuid| {
            transcript
                .messages
                .iter()
                .any(|message| message.uuid == uuid)
        });
        let completed_at = if activity.status == ActivityStatus::InProgress {
            Value::Null
        } else {
            json!(at)
        };
        let base = json!({
            "id": format!("item:activity:{}", activity.activity_id),
            "threadId": conversation.task_id,
            "runId": turn,
            "nodeId": Value::Null,
            "providerThreadId": Value::Null,
            "providerTurnId": Value::Null,
            "nativeItemRef": Value::Null,
            "parentItemId": Value::Null,
            "status": activity_item_status(activity),
            "title": Value::Null,
            "startedAt": at,
            "completedAt": completed_at,
            "updatedAt": at
        });
        let item = match activity.kind {
            ActivityKind::Thinking => {
                let Some(text) = activity.payload.get("text").and_then(Value::as_str) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                layered(
                    base,
                    json!({ "type": "reasoning", "text": text, "streaming": false }),
                )
            }
            ActivityKind::ToolCall if item_type(&activity.summary) == "command_execution" => {
                let input = activity
                    .tool_input()
                    .get("command")
                    .and_then(Value::as_str)
                    .map_or_else(|| activity.tool_input().to_string(), str::to_string);
                let output = activity.result().and_then(result_text).map(str::to_string);
                layered(
                    base,
                    json!({
                        "type": "command_execution",
                        "input": input,
                        "output": output,
                        "outputIndicatesFailure": activity.is_error,
                        "exitCode": command_exit_code(activity)
                    }),
                )
            }
            ActivityKind::UserInputRequested => {
                let Some(request_id) = request_id(activity) else {
                    continue;
                };
                let status = match request_status(conversation, transcript, activity, request_id) {
                    RequestStatus::Pending => "waiting",
                    RequestStatus::Resolved(_) => "completed",
                    RequestStatus::Expired => "cancelled",
                };
                let mut item = layered(
                    base,
                    json!({
                        "type": "user_input_request",
                        "requestId": request_id,
                        "questions": activity.payload.get("questions").cloned().unwrap_or_else(|| json!([]))
                    }),
                );
                item["status"] = json!(status);
                item
            }
            _ => continue,
        };
        let rank = if anchored { 0 } else { 2 };
        timeline.push(((at, rank, index), item));
    }

    timeline.sort_by(|(left, _), (right, _)| left.cmp(right));
    let turn_items = timeline
        .into_iter()
        .enumerate()
        .map(|(ordinal, (_, mut item))| {
            item["ordinal"] = json!(ordinal);
            item
        })
        .collect();
    (turn_items, messages)
}

fn request_id(activity: &Activity) -> Option<&str> {
    activity
        .payload
        .get("requestId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

enum RequestStatus<'a> {
    Pending,
    /// Carries the `UserInputResolved` row that answered it.
    Resolved(&'a Activity),
    /// Never answered, and its turn is over: nobody is waiting on it now.
    Expired,
}

fn request_status<'a>(
    conversation: &Conversation,
    transcript: &'a Transcript,
    asked: &Activity,
    request_id: &str,
) -> RequestStatus<'a> {
    let resolved = transcript.activities.iter().find(|activity| {
        activity.kind == ActivityKind::UserInputResolved
            && self::request_id(activity) == Some(request_id)
    });
    if let Some(resolved) = resolved {
        return RequestStatus::Resolved(resolved);
    }
    let (turn, _) = activity_anchor(conversation, transcript, asked);
    let current = conversation
        .turn_number
        .map(|number| json!(turn_id(&conversation.task_id, number)));
    if turn_is_open(conversation) && Some(&turn) == current.as_ref() {
        RequestStatus::Pending
    } else {
        RequestStatus::Expired
    }
}

/// One `OrchestrationV2RuntimeRequest` per question the agent asked, joined to
/// its `user_input_request` turn item by `requestId`. `responseCapability` is
/// `live` because an answer reaches the agent while its turn waits; the session
/// id is a stand-in the client only compares, never dials.
pub(crate) fn runtime_requests(conversation: &Conversation, transcript: &Transcript) -> Vec<Value> {
    transcript
        .activities
        .iter()
        .filter(|activity| activity.kind == ActivityKind::UserInputRequested)
        .filter_map(|activity| {
            let request_id = request_id(activity)?;
            let (_, at) = activity_anchor(conversation, transcript, activity);
            let (status, resolved_at, answers) =
                match request_status(conversation, transcript, activity, request_id) {
                    RequestStatus::Pending => ("pending", Value::Null, None),
                    RequestStatus::Resolved(resolved) => (
                        "resolved",
                        json!(instant(resolved.created_at)),
                        resolved.payload.get("answers").cloned(),
                    ),
                    RequestStatus::Expired => ("expired", json!(at), None),
                };
            let mut row = json!({
                "id": request_id,
                "nodeId": format!("node:{request_id}"),
                "providerTurnId": Value::Null,
                "nativeRequestRef": Value::Null,
                "kind": "user_input",
                "status": status,
                "responseCapability": {
                    "type": "live",
                    "providerSessionId": format!("moatless:{}", conversation.task_id)
                },
                "createdAt": at,
                "resolvedAt": resolved_at
            });
            if let Some(answers) = answers.filter(Value::is_object) {
                row["answers"] = answers;
            }
            Some(row)
        })
        .collect()
}

/// The turn item a forked thread's timeline opens with, naming the run it was
/// cut from. `None` for a thread that is not a fork.
fn fork_turn_item(conversation: &Conversation, lineage: &Lineage) -> Option<Value> {
    let parent = lineage.parent_task_id.as_deref()?;
    if lineage.relationship != Some("fork") {
        return None;
    }
    let at_turn = lineage.fork_at_turn.unwrap_or(1).max(1);
    let at = instant(created_at(conversation));
    Some(json!({
        "id": format!("item:fork:{}", conversation.task_id),
        "threadId": conversation.task_id,
        "runId": Value::Null,
        "nodeId": Value::Null,
        "providerThreadId": Value::Null,
        "providerTurnId": Value::Null,
        "nativeItemRef": Value::Null,
        "parentItemId": Value::Null,
        "ordinal": 0,
        "status": "completed",
        "title": Value::Null,
        "startedAt": at,
        "completedAt": at,
        "updatedAt": at,
        "type": "fork",
        "source": { "type": "run", "threadId": parent, "runId": turn_id(parent, at_turn) },
        "targetThreadId": conversation.task_id
    }))
}

fn visible_turn_items(thread_id: &str, turn_items: &[Value]) -> Vec<Value> {
    turn_items
        .iter()
        .filter_map(|item| {
            let position = item.get("ordinal")?.clone();
            let id = item.get("id")?.clone();
            Some(json!({
                "position": position,
                "visibility": "local",
                "sourceThreadId": thread_id,
                "sourceItemId": id,
                "item": item
            }))
        })
        .collect()
}

/// The whole `OrchestrationV2ThreadProjection` for one conversation.
pub(crate) fn thread_projection(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    transcript: &Transcript,
    origins: &OriginContext<'_>,
    lineage: &Lineage,
) -> Value {
    let thread = app_thread(conversation, project_id, defaults, lineage);
    let runs = runs(conversation, transcript, defaults);
    let (mut turn_items, messages) = turn_items_and_messages(conversation, transcript, origins);
    if let Some(fork_item) = fork_turn_item(conversation, lineage) {
        for item in &mut turn_items {
            if let Some(ordinal) = item.get("ordinal").and_then(Value::as_i64) {
                item["ordinal"] = json!(ordinal + 1);
            }
        }
        turn_items.insert(0, fork_item);
    }
    let visible_turn_items = visible_turn_items(&conversation.task_id, &turn_items);
    json!({
        "thread": thread,
        "runs": runs,
        "attempts": [],
        "nodes": [],
        "subagents": [],
        "providerSessions": [],
        "providerThreads": [],
        "providerTurns": [],
        "runtimeRequests": runtime_requests(conversation, transcript),
        "messages": messages,
        "plans": [],
        "turnItems": turn_items,
        "checkpointScopes": [],
        "checkpoints": [],
        "contextHandoffs": [],
        "contextTransfers": [],
        "visibleTurnItems": visible_turn_items,
        "updatedAt": instant(conversation.updated_at.unwrap_or_else(|| created_at(conversation)))
    })
}

#[cfg(test)]
mod tests;
