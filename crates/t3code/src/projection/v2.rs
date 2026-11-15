//! Orchestration V2 projections, built from the V1 row builders beside it.
//!
//! Only `thread`, `runs`, `turnItems`, `messages`, `runtimeRequests`,
//! `subagents` and `lineage` carry Moatless state; every other array has no
//! Moatless counterpart and is always empty. Turn items cover messages,
//! reasoning, shell commands, questions the agent asked, the fork marker and
//! the Tasks the thread delegated to; other activity kinds are left out of the
//! timeline rather than guessed at.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use super::model::model_selection;
use super::model::provider_instance;
use super::model::turn_model_selections;
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
use super::turn::running_inputs;
use super::turn::turn_id;
use super::turn::turn_is_open;
use super::turn::turn_reference;
use super::turn::turn_state;
use super::v2_items::html_render_item;
use super::v2_items::layered;
use super::v2_items::plan_nodes;
use super::v2_items::plans;
use super::v2_items::secret_request_item;
use super::v2_items::todo_list_item;
use super::v2_items::tool_item;
use super::v2_lifecycle::compaction_fields;
use super::v2_lifecycle::context_handoffs;
use super::v2_lifecycle::context_usage;
use super::v2_lifecycle::handoff_fields;
use super::v2_lifecycle::handoff_turn;
use super::v2_lifecycle::notification;
use super::v2_lifecycle::pending_background_tasks;
use super::v2_lifecycle::run_ending_items;
use super::v2_lifecycle::settled_run_status;
use super::v2_lifecycle::shell_error_fields;
use super::v2_lifecycle::token_usage_by_run;
use super::v2_lifecycle::turn_outcomes;
use super::v2_provider::attempt_id;
use super::v2_provider::attempts;
use super::v2_provider::provider_session;
use super::v2_provider::provider_thread;
use super::v2_provider::provider_thread_id;
use super::v2_provider::provider_turns;
use super::v2_provider::root_node_id;
use super::v2_provider::root_nodes;
use super::v2_subagents::link_delegating_commands;
use super::v2_subagents::subagents;
use super::v2_subagents::thread_created_items;
use super::v2_subagents::with_thread_created;
use crate::AgentModelDefaults;
use crate::dao::Conversation;
use crate::transcript::RewoundTurn;
use crate::transcript::Transcript;
use crate::transcript::TranscriptMessage;
use crate::transcript::TurnOutcome;
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityStatus;
use moatless_types::message::MessageType;
use moatless_types::message::TOOL_RESPONSE_USER_TYPE;
use moatless_types::running::{Working, derive_running};
use moatless_types::task::TaskRelationType;
use moatless_workspaces::workspace::models::ListedWorkspace;

/// Every V2 thread reports the contract's default interaction mode: Moatless
/// has no state distinguishing a "plan" mode from the agent's own tool use.
const DEFAULT_INTERACTION_MODE: &str = "default";

/// One thread's ancestry: resolved per request for a single thread, read off
/// the listing's own columns for a shell row (see [`Lineage::listed`]).
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
    pub(crate) side_chat: bool,
    created_by: &'static str,
    creation_source: &'static str,
}

impl Lineage {
    /// A listing row's lineage. `parents` maps listed Tasks to their parents;
    /// the root is the furthest ancestor it reaches, else the parent, which is
    /// what a single-row read's empty map gives. Nesting reads only the parent.
    pub(crate) fn listed(conversation: &Conversation, parents: &HashMap<&str, &str>) -> Self {
        let task_id = &conversation.task_id;
        let edge = conversation.parent_task_id.as_deref().zip(
            conversation
                .parent_relation
                .as_deref()
                .and_then(|relation| TaskRelationType::try_from(relation).ok()),
        );
        let Some((parent, relation)) = edge else {
            return Self::root(task_id);
        };
        let mut root = parent;
        for _ in 0..parents.len() {
            match parents.get(root) {
                Some(next) if *next != task_id => root = next,
                _ => break,
            }
        }
        let mut lineage = Self::child(
            parent.to_string(),
            relation,
            root.to_string(),
            conversation.fork_at_turn,
        );
        lineage.side_chat = conversation.side_chat;
        lineage
    }

    /// A thread with no parent edge, lineage fully resolved: it is its own
    /// root.
    pub(crate) fn root(task_id: &str) -> Self {
        Self {
            parent_task_id: None,
            relationship: None,
            root_task_id: task_id.to_string(),
            fork_at_turn: None,
            side_chat: false,
            created_by: "user",
            creation_source: "web",
        }
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
            side_chat: false,
            created_by,
            creation_source,
        }
    }

    fn to_value(&self) -> Value {
        let mut value = json!({
            "parentThreadId": self.parent_task_id,
            "relationshipToParent": self.relationship,
            "rootThreadId": self.root_task_id
        });
        if self.side_chat {
            value["sideChat"] = json!(true);
        }
        value
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
    row.insert(
        "lastVisitedAt".into(),
        nullable_instant(conversation.last_read_at),
    );
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
    row.insert(
        "pendingRuntimeRequest".into(),
        pending_runtime_request(conversation),
    );
    row.insert("latestVisibleMessage".into(), Value::Null);
    row.insert(
        "lastError".into(),
        conversation
            .turn_error
            .as_deref()
            .map_or(Value::Null, |error| json!(error)),
    );
    let (error_class, usage_limit_reset_at) = shell_error_fields(conversation);
    row.insert("lastErrorClass".into(), error_class);
    row.insert("usageLimitResetAt".into(), usage_limit_reset_at);
    row.insert(
        "pendingBackgroundTasks".into(),
        pending_background_tasks(conversation),
    );
    // The same set `subtasks.list` answers with; the sidebar badge opens it.
    row.insert("childCount".into(), json!(conversation.child_count));
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
    let open = turn_is_open(conversation);
    let started = conversation.turn_number.and(conversation.turn_started_at);
    row.insert("latestRunRequestedAt".into(), nullable_instant(started));
    row.insert("latestRunStartedAt".into(), nullable_instant(started));
    row.insert(
        "latestRunCompletedAt".into(),
        // A settled turn without an end reads as settled at the row's update,
        // as the client assumes when the field is absent.
        nullable_instant(match (open, started) {
            (false, Some(_)) => conversation.turn_ended_at.or(conversation.updated_at),
            _ => None,
        }),
    );
    row.insert(
        "activityRunStartedAt".into(),
        nullable_instant(activity_started_at(conversation)),
    );
    row.insert(
        "activityRunStatus".into(),
        activity_run_status(conversation).map_or(Value::Null, |status| json!(status)),
    );
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

/// The shell's summary of a question the agent is waiting on, which the
/// sidebar shows as Input. The client reads only `kind`; the listing has no
/// transcript, so `id` names the thread rather than the question.
fn pending_runtime_request(conversation: &Conversation) -> Value {
    if !conversation.pending_question {
        return Value::Null;
    }
    let at = conversation
        .updated_at
        .unwrap_or_else(|| created_at(conversation));
    json!({
        "id": format!("{}:pending-question", conversation.task_id),
        "kind": "user_input",
        "createdAt": instant(at)
    })
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
        "thread": thread_shell_v2(conversation, project_id, defaults, &Lineage::listed(conversation, &HashMap::new()))
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

/// One project, re-read and sent whole, at the position that carried it.
pub(crate) fn project_updated_v2(workspace: &ListedWorkspace, sequence: i64) -> Value {
    json!({
        "kind": "project.updated",
        "sequence": sequence,
        "project": project(workspace)
    })
}

/// `OrchestrationV2ShellThreadStatus`: `idle` plus the run-status vocabulary.
pub(super) fn shell_status(conversation: &Conversation) -> &'static str {
    if turn_is_open(conversation) {
        return "running";
    }
    // Before the early `idle`: a first send has no turn status at all.
    if awaits_turn(conversation) {
        return "starting";
    }
    // `idle` only for a thread that never ran: the client reads `idle` on a
    // thread with runs as parked on background work and labels it Waiting.
    let Some(state) = conversation.turn_status.as_deref().and_then(turn_state) else {
        return "idle";
    };
    match state {
        "error" => "failed",
        "interrupted" => "interrupted",
        _ => "completed",
    }
}

/// When the work a working row's sidebar timer counts from began: the open
/// turn's start, or for a send owed a turn the newest inbox item that owes it.
fn activity_started_at(conversation: &Conversation) -> Option<DateTime<Utc>> {
    if turn_is_open(conversation) {
        return conversation.turn_started_at;
    }
    if awaits_turn(conversation) {
        return conversation
            .latest_queued_message_at
            .or(conversation.latest_user_message_at);
    }
    None
}

/// What the work [`activity_started_at`] times is doing, which the Lineage
/// panel reads as a child's live status; `None` while no work is under way.
fn activity_run_status(conversation: &Conversation) -> Option<&'static str> {
    if turn_is_open(conversation) {
        return Some(if conversation.pending_question {
            "waiting"
        } else {
            "running"
        });
    }
    awaits_turn(conversation).then_some("starting")
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
    workspaces: &[ListedWorkspace],
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
    workspaces: &[ListedWorkspace],
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
    workspaces: &[ListedWorkspace],
    conversations: &[Conversation],
    defaults: &AgentModelDefaults,
) -> Vec<Value> {
    let projects: HashSet<&str> = workspaces
        .iter()
        .map(|listed| listed.workspace.id.as_str())
        .collect();
    let parents: HashMap<&str, &str> = conversations
        .iter()
        .filter_map(|conversation| {
            let parent = conversation.parent_task_id.as_deref()?;
            Some((conversation.task_id.as_str(), parent))
        })
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
                &Lineage::listed(conversation, &parents),
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

fn checkpoint_id(task_id: &str, ordinal: i64) -> String {
    format!("{task_id}:checkpoint:{ordinal}")
}

/// The turn count a [`checkpoint_id`] rewinds to: `0` for the thread's start.
pub(crate) fn turn_count_of_checkpoint(checkpoint_id: &str) -> Option<i32> {
    checkpoint_id
        .rsplit_once(":checkpoint:")?
        .1
        .parse()
        .ok()
        .filter(|count| *count >= 0)
}

/// The points `checkpoint.rollback` can rewind the conversation to: the
/// thread's start, then the end of each settled run, keyed by run `ordinal`.
/// They restore no files: `files` is empty and a rollback that asks for files
/// is refused (see `v2_commands`).
fn checkpoints(conversation: &Conversation, runs: &[Value]) -> Vec<Value> {
    let task_id = &conversation.task_id;
    let scope = format!("{task_id}:checkpoints");
    let start = instant(created_at(conversation));
    let mut checkpoints = vec![json!({
        "id": checkpoint_id(task_id, 0),
        "threadId": task_id,
        "scopeId": scope,
        "runId": Value::Null,
        "nodeId": format!("{task_id}:checkpoint:0:node"),
        "parentCheckpointId": Value::Null,
        "ordinalWithinScope": 0,
        "appRunOrdinal": Value::Null,
        "ref": checkpoint_id(task_id, 0),
        "status": "ready",
        "files": [],
        "capturedAt": start
    })];
    for run in runs {
        let status = match run["status"].as_str() {
            Some("completed" | "interrupted") => "ready",
            Some("failed") => "error",
            _ => continue,
        };
        let (Some(ordinal), Some(node)) = (run["ordinal"].as_i64(), run.get("rootNodeId")) else {
            continue;
        };
        let parent = checkpoints.last().map(|previous| previous["id"].clone());
        let captured = run
            .get("completedAt")
            .filter(|at| !at.is_null())
            .or_else(|| run.get("requestedAt"))
            .cloned()
            .unwrap_or_else(|| json!(start));
        checkpoints.push(json!({
            "id": checkpoint_id(task_id, ordinal),
            "threadId": task_id,
            "scopeId": scope,
            "runId": run["id"],
            "nodeId": node,
            "parentCheckpointId": parent,
            "ordinalWithinScope": ordinal,
            "appRunOrdinal": ordinal,
            "ref": checkpoint_id(task_id, ordinal),
            "status": status,
            "files": [],
            "capturedAt": captured
        }));
    }
    checkpoints
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

/// The run a send waits in while its turn is still owed. Like
/// [`queued_run_id`], never `{taskId}:{n}`.
fn starting_run_id(task_id: &str, message_uuid: &str) -> String {
    format!("{task_id}:starting:{message_uuid}")
}

/// A message the person sent that no turn has taken yet. A child-outcome
/// notice is a notification, never a queued run.
fn is_pending(message: &TranscriptMessage) -> bool {
    message.turn_number.is_none() && is_user_message(message)
}

/// A message somebody sent the agent: not a tool response, a Sandbox wake or
/// a child-outcome notice.
fn is_user_message(message: &TranscriptMessage) -> bool {
    message.message_type == MessageType::User
        && message.user_type != TOOL_RESPONSE_USER_TYPE
        && !message.sandbox_wake
        && message.child_outcome.is_none()
}

/// Whether the agent is working with no turn open yet: V1's session status
/// reads the same rule, so a send shows as work before the Sandbox opens
/// the turn that takes it.
fn awaits_turn(conversation: &Conversation) -> bool {
    !turn_is_open(conversation)
        && conversation.sandbox_desired_state().is_some_and(|desired| {
            matches!(
                derive_running(&running_inputs(conversation, desired)),
                Working::Running | Working::Waiting | Working::Starting
            )
        })
}

/// The run a pending, unheld send belongs to: the open turn it steers into,
/// or the `starting` run of the turn it is owed. `None` when the agent is
/// stopped, and the send waits as a queued run instead.
fn steered_run_id(conversation: &Conversation, transcript: &Transcript) -> Option<String> {
    if turn_is_open(conversation) {
        return conversation
            .turn_number
            .map(|turn| turn_id(&conversation.task_id, turn));
    }
    if !awaits_turn(conversation) {
        return None;
    }
    transcript
        .messages
        .iter()
        .find(|message| is_pending(message) && !message.held)
        .map(|message| starting_run_id(&conversation.task_id, &message.uuid))
}

/// The run a pending message is shown under, or `None` for a queued one. Only
/// a send held for the active turn to end (or one a stopped agent cannot take)
/// is queued: any other steers into the open turn or opens the next one.
fn pending_run(message: &TranscriptMessage, steered: Option<&str>) -> Option<String> {
    if message.held {
        return None;
    }
    steered.map(str::to_string)
}

/// The turns [`runs`] reports as runs of their own: each in the window, plus
/// the current one.
fn run_turns(conversation: &Conversation, transcript: &Transcript) -> Vec<i32> {
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
    turn_numbers
}

/// One `OrchestrationV2Run` per Moatless turn in the window plus the current
/// turn and a `rolled_back` one per turn a rewind cut, in `ordinal` order, then
/// a `starting` run for a turn an unheld send is owed and a `queued` run per
/// send no turn will take (see [`pending_run`]). A turn older than the current
/// one reports its own `task_turns` status.
pub(crate) fn runs(
    conversation: &Conversation,
    transcript: &Transcript,
    defaults: &AgentModelDefaults,
) -> Vec<Value> {
    let turn_numbers = run_turns(conversation, transcript);
    let outcomes = turn_outcomes(conversation, transcript);
    let rewound: Vec<&RewoundTurn> = transcript
        .rewound_turns
        .iter()
        .filter(|turn| !turn_numbers.contains(&turn.turn_number))
        .collect();
    let mut last_turn = turn_numbers
        .iter()
        .copied()
        .chain(rewound.iter().map(|turn| turn.turn_number))
        .max()
        .unwrap_or(0)
        .max(0);
    let mut runs: Vec<Value> = turn_numbers
        .into_iter()
        .map(|turn_number| {
            let outcome = outcomes.get(&turn_number);
            run_for_turn(conversation, transcript, defaults, turn_number, outcome)
        })
        .chain(
            rewound
                .into_iter()
                .map(|turn| rolled_back_run(conversation, defaults, turn)),
        )
        .collect();
    runs.sort_by_key(|run| run["ordinal"].as_i64());
    let steered = steered_run_id(conversation, transcript);
    let starting = transcript.messages.iter().find(|message| {
        steered.as_deref() == Some(&starting_run_id(&conversation.task_id, &message.uuid))
    });
    if let Some(message) = starting {
        last_turn = last_turn.saturating_add(1);
        let at = instant(message_instant(conversation, message));
        runs.push(json!({
            "id": starting_run_id(&conversation.task_id, &message.uuid),
            "threadId": conversation.task_id,
            "ordinal": last_turn,
            "providerInstanceId": provider_instance(conversation),
            "modelSelection": model_selection(conversation, defaults),
            "providerThreadId": provider_thread_id(&conversation.task_id),
            "userMessageId": message.uuid,
            "rootNodeId": Value::Null,
            "activeAttemptId": Value::Null,
            "status": "starting",
            "requestedAt": at,
            "startedAt": Value::Null,
            "completedAt": Value::Null,
            "checkpointId": Value::Null,
            "contextHandoffId": Value::Null
        }));
    }
    let mut queued: Vec<&TranscriptMessage> = transcript
        .messages
        .iter()
        .filter(|message| is_pending(message))
        .filter(|message| pending_run(message, steered.as_deref()).is_none())
        .collect();
    // Stable, so arrival order stands among the ones nobody moved.
    queued.sort_by_key(|message| (message.queue_order.is_none(), message.queue_order));
    // A stop leaves the queue waiting for the next send
    // (`dispatcher::dispatch_when_turn_ends`); `queue.resume` releases it.
    let held = !turn_is_open(conversation)
        && conversation.turn_status.as_deref().and_then(turn_state) == Some("interrupted");
    for (position, message) in queued.into_iter().enumerate() {
        let at = instant(message_instant(conversation, message));
        let position = i32::try_from(position).unwrap_or(i32::MAX - 1) + 1;
        runs.push(json!({
            "id": queued_run_id(&conversation.task_id, &message.uuid),
            "threadId": conversation.task_id,
            "ordinal": last_turn.saturating_add(position),
            "providerInstanceId": provider_instance(conversation),
            "modelSelection": model_selection(conversation, defaults),
            "providerThreadId": Value::Null,
            "userMessageId": message.uuid,
            "rootNodeId": Value::Null,
            "activeAttemptId": Value::Null,
            "status": "queued",
            "queuePosition": position,
            "queueHeld": held,
            "requestedAt": at,
            "startedAt": Value::Null,
            "completedAt": Value::Null,
            "checkpointId": Value::Null,
            "contextHandoffId": Value::Null
        }));
    }
    runs
}

/// A turn a rewind cut, as upstream T3 leaves a rolled-back run: under its
/// own id, completed at the rewind. Its Messages are gone, so its user message
/// id is the same placeholder [`run_for_turn`] uses for a turn without one.
fn rolled_back_run(
    conversation: &Conversation,
    defaults: &AgentModelDefaults,
    turn: &RewoundTurn,
) -> Value {
    let turn_number = turn.turn_number;
    let run_id = turn_id(&conversation.task_id, turn_number);
    json!({
        "id": run_id,
        "threadId": conversation.task_id,
        "ordinal": turn_number.max(1),
        "providerInstanceId": provider_instance(conversation),
        "modelSelection": model_selection(conversation, defaults),
        "providerThreadId": provider_thread_id(&conversation.task_id),
        "userMessageId": format!("{}:{turn_number}:user-message", conversation.task_id),
        "rootNodeId": root_node_id(&run_id),
        "activeAttemptId": attempt_id(&run_id),
        "status": "rolled_back",
        "requestedAt": instant(turn.started_at),
        "startedAt": instant(turn.started_at),
        "completedAt": instant(turn.rewound_at),
        "checkpointId": Value::Null,
        "contextHandoffId": Value::Null
    })
}

fn run_for_turn(
    conversation: &Conversation,
    transcript: &Transcript,
    defaults: &AgentModelDefaults,
    turn_number: i32,
    outcome: Option<&TurnOutcome>,
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
        settled_run_status(outcome)
    };
    let user_message_id = in_turn
        .iter()
        .find(|message| is_user_message(message))
        .map_or_else(
            || format!("{}:{turn_number}:user-message", conversation.task_id),
            |message| message.uuid.clone(),
        );
    let requested_at = first_at.unwrap_or_else(|| created_at(conversation));
    let completed_at = if is_latest {
        nullable_instant(conversation.turn_ended_at)
    } else {
        outcome
            .and_then(|outcome| outcome.ended_at)
            .or(last_at)
            .map_or(Value::Null, |at| json!(instant(at)))
    };
    let run_id = turn_id(&conversation.task_id, turn_number);
    json!({
        "id": run_id,
        "threadId": conversation.task_id,
        "ordinal": turn_number.max(1),
        "providerInstanceId": provider_instance(conversation),
        "modelSelection": model_selection(conversation, defaults),
        "providerThreadId": provider_thread_id(&conversation.task_id),
        "userMessageId": user_message_id,
        "rootNodeId": root_node_id(&run_id),
        "activeAttemptId": attempt_id(&run_id),
        "status": status,
        "requestedAt": instant(requested_at),
        "startedAt": instant(requested_at),
        "completedAt": completed_at,
        "checkpointId": Value::Null,
        "contextHandoffId": Value::Null
    })
}

fn activity_item_status(activity: &Activity) -> &'static str {
    match activity.status {
        ActivityStatus::InProgress => "running",
        ActivityStatus::Completed => "completed",
        ActivityStatus::Failed => "failed",
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
    let steered = steered_run_id(conversation, transcript);
    let selections = turn_model_selections(transcript);
    let mut shown_selection: Option<&Value> = None;

    for (index, message) in transcript.messages.iter().enumerate() {
        let pending = is_pending(message);
        let run = pending.then(|| pending_run(message, steered.as_deref()));
        // A queued message is the queue's row, not the timeline's, until a
        // turn takes it.
        let queued = matches!(run, Some(None));
        let turn = match run {
            Some(Some(run_id)) => json!(run_id),
            Some(None) => json!(queued_run_id(&conversation.task_id, &message.uuid)),
            None => turn_reference(conversation, message),
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
        let notice = notification(message, &text);
        let mut item = match (&notice, role.as_str()) {
            (Some(notice), _) => layered(
                layered(base, json!({ "type": "notification" })),
                notice.clone(),
            ),
            (None, "user") => layered(
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
            (None, "assistant") => layered(
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
        if item["type"] == "user_message" {
            let selection = message.turn_number.and_then(|turn| selections.get(&turn));
            if let (Some(from), Some(to)) = (shown_selection, selection) {
                if from != to {
                    item["modelSwitch"] = json!({ "from": from, "to": to });
                }
            }
            shown_selection = selection.or(shown_selection);
        }
        if !queued {
            timeline.push(((at.clone(), 1, index), item));
        }
        let mut conversation_message = json!({
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
        });
        if let Some(notice) = notice {
            conversation_message["notification"] = notice;
        }
        messages.push(conversation_message);
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
            ActivityKind::ToolCall => {
                let Some(item) = tool_item(conversation, activity, base) else {
                    continue;
                };
                item
            }
            ActivityKind::PlanUpdated => {
                let Some(item) = todo_list_item(activity, base) else {
                    continue;
                };
                item
            }
            ActivityKind::Compaction => layered(base, compaction_fields(conversation, activity)),
            ActivityKind::ArtifactShown => {
                let Some(item) = html_render_item(activity, base) else {
                    continue;
                };
                item
            }
            ActivityKind::AgentSwitched => {
                let turn = handoff_turn(conversation, transcript, activity);
                // A switch before the first run hands no history over.
                if turn.is_some_and(|turn| turn <= 1) {
                    continue;
                }
                let mut item = layered(base, handoff_fields(conversation, activity));
                if let Some(turn) = turn {
                    item["runId"] = json!(turn_id(&conversation.task_id, turn));
                }
                item
            }
            ActivityKind::SecretRequested => {
                let Some(item) = secret_request_item(activity, transcript, base) else {
                    continue;
                };
                item
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

    let outcomes = turn_outcomes(conversation, transcript);
    let endings = run_ending_items(
        conversation,
        &run_turns(conversation, transcript),
        &outcomes,
    );
    for (index, (at, item)) in endings.into_iter().enumerate() {
        timeline.push(((at, 3, index), item));
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
    // A call that came back unanswered, e.g. cut off by a sandbox restart that
    // resumed the same turn, would otherwise stay pending beside the re-ask.
    if transcript.settled_requests.contains(request_id) {
        return RequestStatus::Expired;
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
/// `message` because an answer is delivered as a Message, which also reaches
/// a turn that has ended; it is what makes the client offer Dismiss.
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
                "responseCapability": { "type": "message" },
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
    delegated: &[Conversation],
) -> Value {
    let mut thread = app_thread(conversation, project_id, defaults, lineage);
    let mut runs = runs(conversation, transcript, defaults);
    let context_handoffs = context_handoffs(conversation, transcript, &runs);
    for handoff in &context_handoffs {
        let target = handoff.get("targetRunId").and_then(Value::as_str);
        if let Some(run) = runs
            .iter_mut()
            .find(|run| run.get("id").and_then(Value::as_str) == target)
        {
            run["contextHandoffId"] = handoff["id"].clone();
        }
    }
    let updated_at = instant(
        conversation
            .updated_at
            .unwrap_or_else(|| created_at(conversation)),
    );
    thread["activeProviderThreadId"] = json!(provider_thread_id(&conversation.task_id));
    let model = model_selection(conversation, defaults)["model"].clone();
    let (mut turn_items, messages) = turn_items_and_messages(conversation, transcript, origins);
    let subagents = subagents(conversation, delegated, &runs, defaults);
    let carded = link_delegating_commands(&mut turn_items, &subagents);
    let uncarded: Vec<Conversation> = delegated
        .iter()
        .filter(|child| !carded.contains(&child.task_id))
        .cloned()
        .collect();
    let mut turn_items = with_thread_created(
        turn_items,
        thread_created_items(conversation, &uncarded, &runs, defaults),
    );
    if let Some(fork_item) = fork_turn_item(conversation, lineage) {
        for item in &mut turn_items {
            if let Some(ordinal) = item.get("ordinal").and_then(Value::as_i64) {
                item["ordinal"] = json!(ordinal + 1);
            }
        }
        turn_items.insert(0, fork_item);
    }
    let visible_turn_items = visible_turn_items(&conversation.task_id, &turn_items);
    let plans = plans(conversation, transcript);
    let mut nodes = root_nodes(&runs);
    nodes.extend(plan_nodes(&plans, &runs));
    let mut provider_thread = provider_thread(conversation, &runs, &updated_at);
    provider_thread["contextUsage"] = context_usage(transcript);
    provider_thread["handoffIds"] = context_handoffs
        .iter()
        .map(|handoff| handoff["id"].clone())
        .collect();
    json!({
        "thread": thread,
        "runs": runs,
        "attempts": attempts(&runs),
        "nodes": nodes,
        "subagents": subagents,
        "providerSessions": [provider_session(conversation, &model, &updated_at)],
        "providerThreads": [provider_thread],
        "providerTurns": provider_turns(&runs, &token_usage_by_run(conversation, transcript)),
        "runtimeRequests": runtime_requests(conversation, transcript),
        "messages": messages,
        "plans": plans,
        "turnItems": turn_items,
        "checkpointScopes": [],
        "checkpoints": checkpoints(conversation, &runs),
        "contextHandoffs": context_handoffs,
        "contextTransfers": [],
        "visibleTurnItems": visible_turn_items,
        "updatedAt": updated_at
    })
}

#[cfg(test)]
mod tests;
