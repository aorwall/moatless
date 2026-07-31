//! Projection of Moatless rows onto the shapes T3's shell contract expects.
//!
//! Everything T3 calls a field name lives here or in the transport above it.
//! Nothing below this module knows the vocabulary: the input is a `Workspace`
//! row, the output is `serde_json::Value` on the wire.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Map, Value};
use tracing::warn;

use super::dao::{Conversation, Transcript, TranscriptBlock, TranscriptMessage};
use super::AgentModelDefaults;
use crate::agent::catalog::{self, AgentKind};
use crate::message::models::MessageType;
use crate::sandbox::models::ServerConfig;
use crate::task_conversation::ConversationChange;
use crate::task_turn::models::TurnStatus;
use crate::workspace::dao::ListedWorkspace;
use crate::workspace::models::Workspace;
use moatless_api_schemas::enums::{SandboxDesiredState, SandboxStopReason};

/// Prefix of the nominal root path reported for each workspace.
///
/// T3 treats a project's root path as an identity: it must be unique across
/// active projects, it is the basis of the client's own grouping key, and a
/// duplicate is refused outright on the write path. Moatless has no
/// per-workspace path — repositories are checked out inside a per-task sandbox
/// under a directory that is the same string for every workspace — so
/// reporting the real directory would collapse every project into one sidebar
/// group. A distinct path is synthesized per workspace instead. It is nominal
/// either way: the real checkout only exists once a task has a sandbox, and
/// this prefix deliberately matches the equally nominal `cwd` the environment
/// already reports.
const NOMINAL_ROOT_PREFIX: &str = "/moatless/workspaces";

/// The only icon a Moatless server can honestly claim.
///
/// The contract's icon is a closed set — play, test, lint, configure, build,
/// debug — and every Moatless server is a long-running process bound to a port,
/// not a task runner. Reading `test` or `build` out of a server's name would be
/// inferring intent from a free-text label that names the server rather than
/// says what it does.
const SCRIPT_ICON: &str = "play";

/// Prefix of the nominal worktree path reported for each conversation.
///
/// A conversation's checkout lives in its own Sandbox, at the sandbox image's
/// `WORKSPACE_DIR` — the same string for every conversation, because it is a
/// property of the image rather than of the Task. That was reported verbatim
/// while this path was only ever displayed.
///
/// It is now an identity as well. The client sends a thread's `worktreePath`
/// back as the `cwd` of every workspace-file request it makes for that thread
/// (`ChatView.tsx`: `activeThread.worktreePath ?? activeProject.workspaceRoot`),
/// so one shared string would name every conversation's files at once and
/// resolve to none of them. A distinct path is synthesized per Task instead,
/// and [`files`](super::files) reverses it. It sits in its own namespace rather
/// than under [`NOMINAL_ROOT_PREFIX`]: a Sandbox checkout is not a directory
/// inside its Workspace, and nesting one nominal path inside another would
/// claim it is.
const NOMINAL_WORKTREE_PREFIX: &str = "/moatless/tasks";

/// The runtime mode every conversation reports.
///
/// The contract requires one of four named modes and supplies no empty value.
/// Moatless has no equivalent, and the permissive mode is both the contract's
/// own default and an honest description of a Sandbox: the agent runs as a
/// process in a pod with no approval gate in front of it.
const RUNTIME_MODE: &str = "full-access";

/// A catalog model choice that names no model.
///
/// It means *whatever this agent picks*, is resolved inside the Sandbox at run
/// time, and is never written back. Reporting it as a model identity would put
/// a word in the row that names nothing, so the chain falls through it.
const UNRESOLVED_MODEL_CHOICE: &str = "default";

/// The last resort of the model chain, and the only invented value in the row.
///
/// Reached only by a conversation that has never reported a model *and* runs a
/// kind of agent the deployment configures no default for. Deliberately not a
/// plausible model name: a sidebar showing a real-looking model nobody chose is
/// the failure this chain exists to avoid. Every arrival here is logged.
const UNKNOWN_MODEL: &str = "unknown";

/// The shell listing: the viewer's projects, and the conversations in them.
///
/// A conversation whose project is absent from `workspaces` is left out here.
/// The client groups conversations by project and silently skips any whose
/// project it cannot find — a bare `continue`, with no error and nothing logged
/// — so a row emitted against a missing project vanishes with no way to
/// diagnose it. Dropping it here costs the same row and leaves a warning behind.
///
/// `sequence` is the listing position the snapshot was read at. The client
/// stores it and discards every later event at or below it, so it has to be
/// read *before* the rows: a position taken afterwards would sit above a change
/// already reflected in those rows, and the event carrying that change would be
/// discarded as stale — leaving one row frozen with no sign that anything was
/// dropped. Taken beforehand the worst case is a change delivered twice, and a
/// re-read of the whole row twice is the same row.
pub(super) fn shell_snapshot(
    workspaces: &[ListedWorkspace],
    conversations: &[Conversation],
    defaults: &AgentModelDefaults,
    sequence: i64,
) -> Value {
    let projects: HashSet<&str> = workspaces
        .iter()
        .map(|listed| listed.workspace.id.as_str())
        .collect();
    let threads: Vec<Value> = conversations
        .iter()
        .filter_map(|conversation| {
            let project_id = conversation
                .workspace_id
                .as_deref()
                .filter(|id| projects.contains(id));
            let Some(project_id) = project_id else {
                warn!(
                    task_id = %conversation.task_id,
                    workspace_id = ?conversation.workspace_id,
                    "Conversation has no listable workspace; leaving it out of the T3 shell listing"
                );
                return None;
            };
            Some(thread(conversation, project_id, defaults))
        })
        .collect();

    json!({
        "snapshotSequence": sequence,
        "projects": workspaces.iter().map(project).collect::<Vec<_>>(),
        "threads": threads,
        "updatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

/// One conversation's row, re-read and sent whole, at the position that carried
/// it.
///
/// Never a per-change difference: the client applies this as an upsert keyed on
/// the conversation's id and moves its position to `sequence`, so what it needs
/// is the current row rather than an account of what moved. That is also what
/// makes coalescing safe — a burst collapses into one of these, and the burst's
/// last position is the one it carries.
pub(super) fn thread_upserted(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    sequence: i64,
) -> Value {
    json!({
        "kind": "thread-upserted",
        "sequence": sequence,
        "thread": thread(conversation, project_id, defaults)
    })
}

/// One conversation leaving a viewer's listing.
///
/// The client filters its held conversations by id, so an id it does not hold is
/// a no-op *for the client*. It is not a no-op for this server: the id is the
/// whole payload, and a deployment that emitted one whenever a re-read came back
/// empty would tell every connected client the id of every conversation that
/// changed anywhere. So this is only ever built for a conversation the viewer
/// was previously sent — see `ShellFeed`.
pub(super) fn thread_removed(task_id: &str, sequence: i64) -> Value {
    json!({
        "kind": "thread-removed",
        "sequence": sequence,
        "threadId": task_id
    })
}

/// One project, re-read and sent whole, at the position that carried it.
///
/// The project half of the sidebar changes far less than the conversation half,
/// but it changes visibly: a renamed project's group heading, a new project's
/// empty group, a deleted project taking its conversations off the screen with
/// it.
pub(super) fn project_upserted(workspace: &ListedWorkspace, sequence: i64) -> Value {
    json!({
        "kind": "project-upserted",
        "sequence": sequence,
        "project": project(workspace)
    })
}

/// One project leaving a viewer's listing. Same disclosure rule as
/// [`thread_removed`]: only for a project the viewer was previously sent.
pub(super) fn project_removed(workspace_id: &str, sequence: i64) -> Value {
    json!({
        "kind": "project-removed",
        "sequence": sequence,
        "projectId": workspace_id
    })
}

/// One conversation, in the shape the shell listing carries.
///
/// This is the contract's *shell* thread, not its full one: no messages, no
/// activities, no checkpoints, no proposed plans, and no `deletedAt`. See
/// [`thread_detail`] for the other member of the pair.
fn thread(conversation: &Conversation, project_id: &str, defaults: &AgentModelDefaults) -> Value {
    let mut row = conversation_row(conversation, project_id, defaults);
    // T3 counts only messages a person wrote, which is what keeps the agent's
    // own output out of the sidebar's ordering entirely.
    row.insert(
        "latestUserMessageAt".into(),
        nullable_instant(conversation.latest_user_message_at),
    );
    // The contract requires a definite answer and supplies no empty value.
    // Moatless has no approval queue, so the answer is always no.
    row.insert("hasPendingApprovals".into(), Value::Bool(false));
    row.insert(
        "hasPendingUserInput".into(),
        Value::Bool(conversation.pending_question),
    );
    row.insert(
        "hasActionableProposedPlan".into(),
        Value::Bool(conversation.pending_plan),
    );
    Value::Object(row)
}

/// One conversation, in the shape the thread view carries: the shared row plus
/// the transcript.
///
/// Three of the contract's collections are empty here and stay empty.
/// `checkpoints` are T3's per-turn worktree snapshots and Moatless captures
/// none — the diff a person sees comes from the Sandbox's own git state, not
/// from a checkpoint ref this backend holds. `proposedPlans` are a durable
/// entity T3 stores and reopens; Moatless has an unanswered `ExitPlanMode` tool
/// call, which is a fact about the transcript rather than a row, and the shell
/// already reports it as `hasActionableProposedPlan`. Filling either with
/// something derived would put a revert button and a plan card in front of a
/// person that nothing behind them can honour.
///
/// The four shell-only fields are absent rather than false: they are not part
/// of the contract's full thread, and the client reads a conversation's pending
/// state from its activities here.
pub(super) fn thread_detail(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
    transcript: &Transcript,
) -> Value {
    let mut row = conversation_row(conversation, project_id, defaults);
    let rendered = rendered_transcript(conversation, transcript);
    // Closing a conversation in Moatless records no timestamp of its own, and
    // the contract requires this one to be present. A conversation that no
    // longer exists is not projected at all — the read that would have found it
    // comes back empty and the caller answers "not found" — so null is the only
    // value this can carry.
    row.insert("deletedAt".into(), Value::Null);
    row.insert("messages".into(), Value::Array(rendered.messages));
    row.insert("activities".into(), Value::Array(rendered.activities));
    row.insert("proposedPlans".into(), Value::Array(Vec::new()));
    row.insert("checkpoints".into(), Value::Array(Vec::new()));
    Value::Object(row)
}

/// Everything one recorded change tells a client that already holds this
/// conversation, as stream items, in the order the client applies them.
///
/// The contract numbers events, so each item's sequence comes from the change's
/// position and its own index within the change — see
/// [`ConversationChange::sequence`]. That is also the ceiling on how much one
/// change may say: past [`EVENTS_PER_CHANGE`] the next position begins, and
/// this stops rather than colliding with it.
///
/// `window` is the changed Message and the short run before it
/// (`ConversationDao::changed_messages`); a change naming a Message the window
/// does not hold — deleted, or never rendered — says nothing, which is a
/// complete answer rather than a failure.
///
/// # What a change to one Message tells the client
///
/// Three things, and only the first is about the changed row itself:
///
/// 1. Its bubble, as `thread.message-sent`. The client upserts on the message
///    id, so re-sending an unchanged bubble is free, and the merges that
///    assemble one Message from its blocks are exactly why it is re-sent.
/// 2. Its work-log rows, as `thread.activity-appended`. Also an upsert, keyed
///    on the activity id.
/// 3. **The rows it answers.** A tool call's card is on the assistant Message
///    that made the call, and what flips it from running to finished is a
///    *later* Message carrying the result. Nothing rewrites the calling row, so
///    a change that carries tool results re-sends the calls it resolves or the
///    work log stalls with every card running.
pub(super) fn change_events(
    conversation: &Conversation,
    change: &ConversationChange,
    window: &Transcript,
) -> Vec<Value> {
    let Some(uuid) = change.message_uuid.as_deref() else {
        return session_set(conversation, change).into_iter().collect();
    };
    let Some(message) = window
        .messages
        .iter()
        .find(|message| message.uuid == uuid)
    else {
        return Vec::new();
    };

    let outcomes = tool_outcomes(&window.messages);
    let displayed = displayed_blocks(&window.blocks);
    let at = instant(message_instant(conversation, message));
    let turn = turn_reference(conversation, message);

    // The calls this row answers first: they are about work the client has
    // already been shown, and the row itself is what happened next.
    let mut payloads: Vec<(&str, Value)> = resolved_calls(conversation, message, window, &outcomes)
        .into_iter()
        .map(|activity| activity_appended_payload(&conversation.task_id, activity))
        .collect();
    if let Some(bubble) = message_bubble(message, &displayed, &turn, &at) {
        payloads.push((
            MESSAGE_SENT,
            message_sent_payload(&conversation.task_id, &bubble),
        ));
    }
    for activity in message_activities(message, &outcomes, &turn, &at) {
        payloads.push(activity_appended_payload(&conversation.task_id, activity));
    }

    payloads
        .into_iter()
        .enumerate()
        .map_while(|(index, (kind, payload))| {
            let sequence = change.sequence(index)?;
            Some(event(conversation, sequence, &at, kind, payload))
        })
        .collect()
}

/// The work-log rows a Message's tool results resolve, re-projected with the
/// outcome attached.
///
/// The window is what bounds this: a result whose call is older than the run of
/// Messages read alongside the change finds nothing here, and the card stays
/// running until the client next takes a snapshot. See `CHANGE_LOOKBACK`.
fn resolved_calls(
    conversation: &Conversation,
    answered_by: &TranscriptMessage,
    window: &Transcript,
    outcomes: &HashMap<&str, ToolOutcome<'_>>,
) -> Vec<Value> {
    let answered: HashSet<&str> = content_blocks(&answered_by.message_content)
        .iter()
        .filter(|block| block_type(block) == Some("tool_result"))
        .filter_map(|block| block.get("tool_use_id").and_then(Value::as_str))
        .collect();
    if answered.is_empty() {
        return Vec::new();
    }
    let mut resolved = Vec::new();
    for message in &window.messages {
        if message.uuid == answered_by.uuid {
            continue;
        }
        let at = instant(message_instant(conversation, message));
        let turn = turn_reference(conversation, message);
        for (index, block) in content_blocks(&message.message_content).iter().enumerate() {
            if block_type(block) != Some("tool_use") {
                continue;
            }
            let calls = block.get("id").and_then(Value::as_str);
            if calls.is_some_and(|id| answered.contains(id)) {
                resolved.push(tool_activity(message, index, block, outcomes, &turn, &at));
            }
        }
    }
    resolved
}

/// The conversation's session as an event, or nothing when it has none.
///
/// A change that names no Message is a change to the conversation itself — its
/// Turn or its Sandbox — and the contract carries both inside the session: the
/// client derives `latestTurn` from the session it is sent. A conversation with
/// no Sandbox row has no session at all, and the contract's payload has no
/// empty value for one, so there is nothing to say until it does.
fn session_set(conversation: &Conversation, change: &ConversationChange) -> Option<Value> {
    let session = session(conversation);
    if session.is_null() {
        return None;
    }
    let at = instant(conversation.updated_at.unwrap_or_else(|| created_at(conversation)));
    let sequence = change.sequence(0)?;
    Some(event(
        conversation,
        sequence,
        &at,
        "thread.session-set",
        json!({ "threadId": conversation.task_id, "session": session }),
    ))
}

const MESSAGE_SENT: &str = "thread.message-sent";

/// One work-log row as the event carrying it. The activity is the contract's
/// own shape already, so this only names the conversation it belongs to.
fn activity_appended_payload(task_id: &str, activity: Value) -> (&'static str, Value) {
    (
        "thread.activity-appended",
        json!({ "threadId": task_id, "activity": activity }),
    )
}

/// A message bubble as the event carrying it.
///
/// The contract spells one message two ways — a member of a snapshot's
/// `messages`, and this payload — with the same values under different keys.
/// Rewriting rather than re-deriving keeps the two from drifting: whatever a
/// snapshot would have shown is what an event says.
fn message_sent_payload(task_id: &str, bubble: &Value) -> Value {
    json!({
        "threadId": task_id,
        "messageId": bubble["id"],
        "role": bubble["role"],
        "text": bubble["text"],
        "turnId": bubble["turnId"],
        "streaming": bubble["streaming"],
        "createdAt": bubble["createdAt"],
        "updatedAt": bubble["updatedAt"]
    })
}

/// One event, in the envelope every event on this contract carries.
///
/// Five of the envelope's fields are null or empty and stay that way. Moatless
/// has no command bus: nothing here was caused by a numbered command, so
/// `commandId` and `correlationId` have nothing to name, and no event is
/// derived from another so `causationEventId` names nothing either. `metadata`
/// is provider bookkeeping this projection does not read.
///
/// `eventId` is the conversation and the sequence, because that pair is what
/// identifies an event here: the same change re-read produces the same ids, and
/// a client that sees one twice sees the same event rather than a new one.
fn event(
    conversation: &Conversation,
    sequence: i64,
    occurred_at: &str,
    event_type: &str,
    payload: Value,
) -> Value {
    json!({
        "kind": "event",
        "event": {
            "sequence": sequence,
            "eventId": format!("{}:{sequence}", conversation.task_id),
            "aggregateKind": "thread",
            "aggregateId": conversation.task_id,
            "occurredAt": occurred_at,
            "commandId": Value::Null,
            "causationEventId": Value::Null,
            "correlationId": Value::Null,
            "metadata": json!({}),
            "type": event_type,
            "payload": payload
        }
    })
}

/// The fields both shapes of a conversation carry, with the same values.
///
/// Six fields are deliberately absent rather than null. `interactionMode`,
/// `archivedAt`, `settledOverride`, `settledAt`, `snoozedUntil` and `snoozedAt`
/// all carry a decoding default or are optional, and Moatless has nothing
/// honest to put in them: closing a conversation records no timestamp of its
/// own, and settlement and snoozing do not exist here at all. Leaving them out
/// lets the contract fill them.
fn conversation_row(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
) -> Map<String, Value> {
    let created = created_at(conversation);
    let Value::Object(row) = json!({
        "id": conversation.task_id,
        "projectId": project_id,
        // `tasks.name` is nullable and has no non-empty constraint, and the
        // contract refuses a blank title, so an unnamed conversation falls back
        // to its id rather than taking the whole listing down.
        "title": conversation
            .name
            .as_deref()
            .and_then(non_empty)
            .unwrap_or_else(|| conversation.task_id.clone()),
        "modelSelection": model_selection(conversation, defaults),
        "runtimeMode": RUNTIME_MODE,
        "branch": conversation
            .branch
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
        "worktreePath": worktree_path(conversation),
        "latestTurn": latest_turn(conversation),
        "createdAt": instant(created),
        "updatedAt": instant(conversation.updated_at.unwrap_or(created)),
        "session": session(conversation)
    }) else {
        unreachable!("a json! object literal is an object")
    };
    row
}

/// The two collections a conversation's transcript becomes.
///
/// T3 splits a conversation the way a reader does and Moatless does not: what
/// somebody *said* is a message, and what the agent *did* on the way to saying
/// it is an activity the client renders as a collapsible work log. One Moatless
/// Message can therefore produce a message and several activities, or only
/// activities — an assistant turn that did nothing but call tools has no text
/// and contributes no bubble.
struct RenderedTranscript {
    messages: Vec<Value>,
    activities: Vec<Value>,
}

/// What came back from one tool call.
struct ToolOutcome<'a> {
    text: Option<&'a str>,
    is_error: bool,
}

/// The numbering a work log is sorted by.
///
/// The client sorts activities by `sequence` and falls back to comparing
/// `createdAt` **as strings** when it is absent — and every block of one Message
/// shares that Message's single timestamp, so without a sequence a turn's tool
/// calls would sort by id.
///
/// Derived from the row and the block rather than counted while walking the
/// transcript, and that is the load-bearing part: an activity delivered on its
/// own, long after the snapshot it has to sort into, has no walk to be counted
/// by. `task_messages.id` ascends with the transcript and the block index
/// ascends within a Message, so the pair reproduces the order a full read would
/// have produced, from one row in isolation.
///
/// [`BLOCKS_PER_MESSAGE`] is the stride between two Messages. A Message with
/// more blocks than that would sort into its successor's range — the client
/// then breaks the tie on `createdAt` and `id`, so the cost is a mis-ordered
/// work log rather than a lost row.
fn activity_sequence(message: &TranscriptMessage, block_index: usize) -> i64 {
    i64::from(message.id) * BLOCKS_PER_MESSAGE
        + i64::try_from(block_index).unwrap_or(BLOCKS_PER_MESSAGE - 1)
}

const BLOCKS_PER_MESSAGE: i64 = 1_024;

/// Split a Task's Messages into the messages and activities T3 renders.
///
/// Ordering is the transcript's own and is carried explicitly — see
/// [`activity_sequence`].
fn rendered_transcript(
    conversation: &Conversation,
    transcript: &Transcript,
) -> RenderedTranscript {
    let outcomes = tool_outcomes(&transcript.messages);
    let displayed = displayed_blocks(&transcript.blocks);

    let mut rendered = RenderedTranscript {
        messages: Vec::new(),
        activities: Vec::new(),
    };
    if transcript.omitted > 0 {
        rendered.messages.push(elision_notice(conversation, transcript));
    }

    for message in &transcript.messages {
        let at = instant(message_instant(conversation, message));
        let turn = turn_reference(conversation, message);
        rendered
            .messages
            .extend(message_bubble(message, &displayed, &turn, &at));
        rendered
            .activities
            .extend(message_activities(message, &outcomes, &turn, &at));
    }

    rendered
}

/// The bubble one Message contributes to the conversation, or `None` when it
/// contributes none.
///
/// Three of the four ways to contribute nothing are about *which* Message it is
/// rather than what it holds, which is why this is a match on the type rather
/// than a filter on the text.
fn message_bubble(
    message: &TranscriptMessage,
    displayed: &HashMap<i32, Vec<&str>>,
    turn: &Value,
    at: &str,
) -> Option<Value> {
    match message.message_type {
        // The carrier rows for tool results. Their content is already attached
        // to the call it answers, and rendering them as their own bubbles would
        // put every tool's raw output in the conversation.
        MessageType::System => None,
        MessageType::User => {
            // A `user`-typed row holding tool results is the SDK streaming
            // results back, not a person writing. `message/transform.rs` makes
            // the same exclusion for the same reason.
            if carries_tool_results(&message.message_content) {
                return None;
            }
            let text = user_text(message, displayed);
            (!text.is_empty()).then(|| message_row(message, "user", text, turn, at))
        }
        MessageType::Assistant => {
            let spoken = spoken_text(message);
            (!spoken.is_empty()).then(|| message_row(message, "assistant", spoken, turn, at))
        }
        // The run's own closing line — "Stopped after an error", a result
        // summary. Neither the person nor the agent said it, and `system` is the
        // contract's role for exactly that.
        MessageType::Result | MessageType::Error => {
            let text = spoken_text(message);
            (!text.is_empty()).then(|| message_row(message, "system", text, turn, at))
        }
    }
}

/// The rows one Message contributes to the work log, in block order.
///
/// Only an assistant Message contributes any: thinking and tool calls are the
/// two things the agent does that are not speech, and no other kind of row
/// carries either.
fn message_activities(
    message: &TranscriptMessage,
    outcomes: &HashMap<&str, ToolOutcome<'_>>,
    turn: &Value,
    at: &str,
) -> Vec<Value> {
    if !matches!(message.message_type, MessageType::Assistant) {
        return Vec::new();
    }
    content_blocks(&message.message_content)
        .iter()
        .enumerate()
        .filter_map(|(index, block)| match block_type(block) {
            Some("thinking") => Some(thinking_activity(message, index, turn, at)),
            Some("tool_use") => Some(tool_activity(message, index, block, outcomes, turn, at)),
            // `text`, `image`, and anything a future provider adds. Silent
            // rather than warned: an unrenderable block is a gap in this
            // projection, not a broken row, and one per image in every
            // transcript is log noise.
            _ => None,
        })
        .collect()
}

/// The text blocks of a Message, joined as markdown paragraphs.
fn spoken_text(message: &TranscriptMessage) -> String {
    let mut text = String::new();
    for block in content_blocks(&message.message_content) {
        if block_type(block) == Some("text") {
            push_paragraph(&mut text, block_text(block));
        }
    }
    text
}

/// The head of a transcript that did not fit, said out loud.
///
/// A capped read that simply started in the middle would look like a
/// conversation that began there. This is the one message in a transcript that
/// no row in the database backs, which is why it says so.
fn elision_notice(conversation: &Conversation, transcript: &Transcript) -> Value {
    let at = instant(
        transcript
            .messages
            .first()
            .map_or_else(|| created_at(conversation), |first| {
                message_instant(conversation, first)
            }),
    );
    json!({
        "id": format!("{}:elided", conversation.task_id),
        "role": "system",
        "text": format!(
            "{} earlier messages are not shown in this view.",
            transcript.omitted
        ),
        "turnId": Value::Null,
        "streaming": false,
        "createdAt": at,
        "updatedAt": at
    })
}

/// One message bubble.
fn message_row(
    message: &TranscriptMessage,
    role: &str,
    text: String,
    turn: &Value,
    at: &str,
) -> Value {
    json!({
        // `task_messages.uuid` is unique across the deployment and is the
        // identity every other Moatless surface already refers to a Message by.
        "id": message.uuid,
        "role": role,
        "text": text,
        "turnId": turn,
        // Moatless persists a Message once it is whole; there is no partial row
        // to report as still arriving.
        "streaming": false,
        "createdAt": at,
        // No column records a later edit, and none happens: the in-place
        // updates this table takes never touch content.
        "updatedAt": at
    })
}

/// One stretch of the agent thinking.
///
/// Reported as `task.progress`, which is the kind the client renders in the
/// thinking tone. The thought itself is not carried: `message_content` holds
/// only the block's presence for redacted thinking, and a summary line is what
/// the work log shows either way.
fn thinking_activity(
    message: &TranscriptMessage,
    index: usize,
    turn: &Value,
    at: &str,
) -> Value {
    json!({
        "id": format!("{}:{index}", message.uuid),
        "tone": "info",
        "kind": "task.progress",
        "summary": "Thinking",
        "payload": { "summary": "Thinking" },
        "turnId": turn,
        "sequence": activity_sequence(message, index),
        "createdAt": at
    })
}

/// One tool call, with whatever came back from it.
///
/// A call whose result has not arrived is `tool.updated` and in progress rather
/// than `tool.completed`: the client collapses an update into the completion
/// that follows it, so a call reported complete before it is would keep the
/// completed row and drop the real outcome.
fn tool_activity(
    message: &TranscriptMessage,
    index: usize,
    block: &Value,
    outcomes: &HashMap<&str, ToolOutcome<'_>>,
    turn: &Value,
    at: &str,
) -> Value {
    let name = block
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Tool call");
    let tool_use_id = block.get("id").and_then(Value::as_str);
    let outcome = tool_use_id.and_then(|id| outcomes.get(id));
    let Value::Object(mut fields) = json!({
        "title": name,
        "itemType": item_type(name),
        "status": match outcome {
            None => "inProgress",
            Some(outcome) if outcome.is_error => "failed",
            Some(_) => "completed"
        }
    }) else {
        unreachable!("a json! object literal is an object")
    };
    if let Some(tool_use_id) = tool_use_id {
        fields.insert("data".into(), json!({ "toolCallId": tool_use_id }));
    }
    // The client reads the command out of the payload to render a terminal row,
    // and falls back to the detail text for everything else.
    if let Some(command) = block
        .get("input")
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
    {
        fields.insert("command".into(), Value::String(command.to_string()));
    }
    if let Some(text) = outcome.and_then(|outcome| outcome.text) {
        fields.insert("detail".into(), Value::String(truncated(text)));
    }

    json!({
        "id": format!("{}:{index}", message.uuid),
        "tone": if outcome.is_some_and(|outcome| outcome.is_error) { "error" } else { "tool" },
        "kind": if outcome.is_some() { "tool.completed" } else { "tool.updated" },
        "summary": name,
        "payload": Value::Object(fields),
        "turnId": turn,
        "sequence": activity_sequence(message, index),
        "createdAt": at
    })
}

/// Which of the contract's closed set of tool shapes a Moatless tool call is.
///
/// The set names how the client *renders* a call — a terminal row, a diff, a
/// generic card — and only the two shapes with a dedicated renderer are worth
/// claiming. Everything else, including every MCP and skill tool a deployment
/// adds, is the generic one; guessing from a free-text tool name is how a
/// search tool ends up rendered as a file diff.
fn item_type(tool_name: &str) -> &'static str {
    match tool_name {
        "Bash" | "BashOutput" => "command_execution",
        "Edit" | "Write" | "NotebookEdit" => "file_change",
        _ => "dynamic_tool_call",
    }
}

/// Every tool result in the transcript, keyed by the call it answers.
///
/// Results arrive on later rows than the call — a `system` row, or a `user` row
/// the SDK streamed back — so this is built over the whole transcript before
/// any of it is walked. A call whose result was evicted by the transcript cap,
/// or has not come back yet, is simply absent.
fn tool_outcomes(messages: &[TranscriptMessage]) -> HashMap<&str, ToolOutcome<'_>> {
    let mut outcomes = HashMap::new();
    for message in messages {
        for block in content_blocks(&message.message_content) {
            if block_type(block) != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            outcomes.insert(
                tool_use_id,
                ToolOutcome {
                    text: tool_result_text(block),
                    is_error: block
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
            );
        }
    }
    outcomes
}

/// A tool result's visible output.
///
/// The block's `content` is a string for most tools and a list of content
/// blocks for the ones that return structured output; only the first text block
/// of the latter is shown, which is what the work log has room for.
fn tool_result_text(block: &Value) -> Option<&str> {
    let content = block.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text);
    }
    content
        .as_array()?
        .iter()
        .find(|part| block_type(part) == Some("text"))
        .and_then(block_text)
}

/// The blocks of a user Message a person actually wrote, by Message.
fn displayed_blocks(blocks: &[TranscriptBlock]) -> HashMap<i32, Vec<&str>> {
    let mut displayed: HashMap<i32, Vec<&str>> = HashMap::new();
    for block in blocks {
        if let Some(text) = block.text_content.as_deref() {
            displayed.entry(block.task_message_id).or_default().push(text);
        }
    }
    displayed
}

/// What a person wrote, without what the request was enriched with.
///
/// Enriched blocks are the truth when they exist. They do not exist for
/// Messages written before that write path, and for those the text blocks of
/// `message_content` are the same text — the enrichment that would make the two
/// differ is exactly what those rows predate.
fn user_text(message: &TranscriptMessage, displayed: &HashMap<i32, Vec<&str>>) -> String {
    let mut text = String::new();
    if let Some(blocks) = displayed.get(&message.id) {
        for block in blocks {
            push_paragraph(&mut text, Some(block));
        }
        return text;
    }
    // A Message sent as a bare string rather than a block list.
    if let Some(whole) = message.message_content.as_str() {
        return whole.to_string();
    }
    for block in content_blocks(&message.message_content) {
        if block_type(block) == Some("text") {
            push_paragraph(&mut text, block_text(block));
        }
    }
    text
}

/// The Turn this Message belongs to, in T3's identity for one.
fn turn_reference(conversation: &Conversation, message: &TranscriptMessage) -> Value {
    message
        .turn_number
        .map_or(Value::Null, |number| {
            Value::String(turn_id(&conversation.task_id, number))
        })
}

/// When a Message happened.
///
/// `timestamp` is what the writer said, and is what the transcript is ordered
/// by, so it is also what the transcript reports — normalized, because the
/// client compares these as strings and a row in another format would sort
/// away from its neighbours. A row whose `timestamp` will not parse falls back
/// to when the database recorded it, and then to the conversation's own start.
fn message_instant(conversation: &Conversation, message: &TranscriptMessage) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&message.timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .ok()
        .or(message.created_at)
        .unwrap_or_else(|| created_at(conversation))
}

/// Tool output long enough to be a file rather than a preview.
///
/// The work log shows a few lines and expands to the rest, but the rest still
/// crosses the socket; a single `Read` of a large file would otherwise put its
/// whole contents in the snapshot, and a subscription re-sends the snapshot
/// every time the conversation moves. Cut with a marker rather than silently.
const TOOL_DETAIL_LIMIT: usize = 2_000;

fn truncated(text: &str) -> String {
    if text.len() <= TOOL_DETAIL_LIMIT {
        return text.to_string();
    }
    // On a character boundary: `text` is a `str`, so slicing mid-codepoint
    // panics.
    let mut end = TOOL_DETAIL_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… truncated", &text[..end])
}

fn content_blocks(content: &Value) -> &[Value] {
    content.as_array().map_or(&[], Vec::as_slice)
}

fn block_type(block: &Value) -> Option<&str> {
    block.get("type").and_then(Value::as_str)
}

fn block_text(block: &Value) -> Option<&str> {
    block.get("text").and_then(Value::as_str)
}

fn carries_tool_results(content: &Value) -> bool {
    content_blocks(content)
        .iter()
        .any(|block| block_type(block) == Some("tool_result"))
}

/// Append a paragraph, keeping the blank line between blocks that markdown
/// needs to render them as separate ones.
fn push_paragraph(text: &mut String, addition: Option<&str>) {
    let Some(addition) = addition.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(addition);
}

/// The conversation's newest turn, or null when it has never run.
///
/// A Moatless turn comes into existence already running, so there is no
/// requested-but-not-started instant distinct from the start: both report the
/// turn's `started_at`.
///
/// `assistantMessageId` is null because the shell listing carries no messages.
/// Choosing the identity a message reports belongs with the projection that
/// emits messages, and inventing one here that a later phase then has to match
/// would be the more expensive mistake.
fn latest_turn(conversation: &Conversation) -> Value {
    let (Some(turn_number), Some(status)) = (
        conversation.turn_number,
        conversation.turn_status.as_deref(),
    ) else {
        return Value::Null;
    };
    let Some(state) = turn_state(status) else {
        warn!(
            task_id = %conversation.task_id,
            status,
            "Turn status has no T3 counterpart; reporting no latest turn"
        );
        return Value::Null;
    };
    json!({
        "turnId": turn_id(&conversation.task_id, turn_number),
        "state": state,
        "requestedAt": timestamp(conversation.turn_started_at),
        "startedAt": timestamp(conversation.turn_started_at),
        "completedAt": nullable_instant(conversation.turn_ended_at),
        "assistantMessageId": Value::Null
    })
}

/// Moatless turn state in T3's four-value vocabulary.
///
/// Five states map onto four. Two of the pairings are not the name match:
/// *stopped* is a person cutting a turn short, which is what T3 calls
/// *interrupted*; and Moatless's own *interrupted* means the Sandbox went away
/// mid-turn, which leaves the turn **open** — it resumes on reschedule, and the
/// reaper reconciles it into a terminal state if it never does. Reporting that
/// as T3's *interrupted* would show a turn that is about to carry on as a turn
/// that ended, and the sidebar would quietly stop showing live work.
///
/// `None` for a status this build does not know — a row written by a newer
/// backend, which has no honest counterpart.
fn turn_state(status: &str) -> Option<&'static str> {
    Some(match TurnStatus::parse(status)? {
        TurnStatus::Active | TurnStatus::Interrupted => "running",
        TurnStatus::Completed => "completed",
        TurnStatus::Error => "error",
        TurnStatus::Stopped => "interrupted",
    })
}

/// Whether the newest turn is still open. Both Moatless states that mean
/// *not finished* count, which is the same grouping the rest of this backend
/// uses when it asks which turns are still running.
fn turn_is_open(conversation: &Conversation) -> bool {
    conversation
        .turn_status
        .as_deref()
        .and_then(TurnStatus::parse)
        .is_some_and(|status| matches!(status, TurnStatus::Active | TurnStatus::Interrupted))
}

/// A turn's identity on the wire.
///
/// Moatless identifies a turn by (task, turn number) and has no single-column
/// id, so the pair is joined into one. Stable across reads and unique across
/// the deployment, which is what the contract's identity needs.
fn turn_id(task_id: &str, turn_number: i32) -> String {
    format!("{task_id}:{turn_number}")
}

/// The conversation's session, or null when no Sandbox was ever requested.
///
/// The contract's session carries what Moatless splits in two: the durable
/// Sandbox lifecycle intent and the agent session running inside it. Absence of
/// a `sandboxes` row is the lazy converged-Stopped default, and it is the one
/// case that means *no session*, not *a stopped session*.
fn session(conversation: &Conversation) -> Value {
    let Some(desired_state) = conversation.sandbox_desired_state else {
        return Value::Null;
    };
    json!({
        "threadId": conversation.task_id,
        "status": session_status(conversation, desired_state),
        "providerName": catalog::find(conversation.agent_type.as_str())
            .map(|entry| entry.label.to_string())
            .or_else(|| non_empty(conversation.agent_type.as_str()))
            .map_or(Value::Null, Value::String),
        "providerInstanceId": provider_instance(conversation),
        // `runtimeMode` is omitted: the session's copy carries a decoding
        // default, unlike the conversation's.
        "activeTurnId": match (turn_is_open(conversation), conversation.turn_number) {
            (true, Some(turn_number)) => json!(turn_id(&conversation.task_id, turn_number)),
            _ => Value::Null,
        },
        // Moatless records no durable last-error text for a Sandbox. The stop
        // reason it does record is already carried by the status below.
        "lastError": Value::Null,
        "updatedAt": timestamp(conversation.sandbox_updated_at)
    })
}

/// Sandbox lifecycle intent in T3's session vocabulary.
fn session_status(conversation: &Conversation, desired_state: SandboxDesiredState) -> &'static str {
    match desired_state {
        // A running Sandbox with an open turn is the agent working. Without
        // one it is a session waiting for input — unless it has not come up
        // yet, which the client renders as connecting rather than idle.
        SandboxDesiredState::Running => {
            if turn_is_open(conversation) {
                "running"
            } else if conversation.sandbox_started_at.is_none() {
                "starting"
            } else {
                "ready"
            }
        }
        // A Sandbox stood down after a failure is the one stop the client has
        // a distinct rendering for, and it is the one worth showing.
        SandboxDesiredState::Stopped | SandboxDesiredState::Removed => {
            match conversation.sandbox_stop_reason {
                Some(SandboxStopReason::Error) => "error",
                _ => "stopped",
            }
        }
    }
}

/// The working copy's location, or null when there is none.
///
/// A removed Sandbox has had its working copy reclaimed, so it reports none —
/// and null is also what tells the client there are no files to ask for, since
/// it falls back to the project's root and gets an answer saying so.
fn worktree_path(conversation: &Conversation) -> Value {
    match conversation.sandbox_desired_state {
        Some(SandboxDesiredState::Running | SandboxDesiredState::Stopped) => {
            json!(nominal_worktree(&conversation.task_id))
        }
        Some(SandboxDesiredState::Removed) | None => Value::Null,
    }
}

/// A distinct, stable, nominal worktree path for one conversation.
fn nominal_worktree(task_id: &str) -> String {
    format!("{NOMINAL_WORKTREE_PREFIX}/{task_id}")
}

/// The conversation whose files a `cwd` names, if it names one.
///
/// The exact inverse of [`nominal_worktree`], and the reason both live here:
/// the client only ever sends back a path this module gave it, so the two have
/// to be read together to stay each other's inverse. A `cwd` with a further
/// path segment is deliberately refused rather than truncated — the contract
/// has no notion of opening a subdirectory as the workspace root, so one is a
/// path this module never issued.
pub(super) fn task_id_from_cwd(cwd: &str) -> Option<&str> {
    cwd.strip_prefix(NOMINAL_WORKTREE_PREFIX)?
        .strip_prefix('/')
        .filter(|task_id| !task_id.is_empty() && !task_id.contains('/'))
}

/// Whether a `cwd` is a project's root rather than a conversation's worktree.
///
/// The client falls back to the project root for a thread reporting no
/// worktree, so this is what a request for a conversation with no Sandbox
/// arrives as. Worth telling apart from an unrecognized path: one is a state
/// the person can change, the other is a bug.
pub(super) fn is_project_root(cwd: &str) -> bool {
    cwd.strip_prefix(NOMINAL_ROOT_PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

/// The model the conversation is running, on the provider instance that runs it.
///
/// The contract requires both halves, refuses an empty one, and has no decoding
/// default for either, so the chain has to end in an answer.
fn model_selection(conversation: &Conversation, defaults: &AgentModelDefaults) -> Value {
    json!({
        "instanceId": provider_instance(conversation),
        "model": model_name(conversation, defaults)
    })
}

/// The model name, resolved through the four steps that always end in an answer.
fn model_name(conversation: &Conversation, defaults: &AgentModelDefaults) -> String {
    // 1. What the newest turn's agent replies said they were produced by. Only
    //    the agent's own reply carries the identity of the model that ran.
    if let Some(model) = conversation.assistant_model.as_deref().and_then(non_empty) {
        return model;
    }
    // 2. What was asked for when that turn started — skipping a catalog choice
    //    that names no model, which is what it actually resolves to.
    if let Some(model) = conversation
        .requested_model
        .as_deref()
        .and_then(non_empty)
        .filter(|model| model != UNRESOLVED_MODEL_CHOICE)
    {
        return model;
    }
    // 3. The deployment's default for this kind of agent. Optional
    //    configuration: a deployment that sets none falls through.
    if let Some(model) = defaults.for_kind(agent_kind(conversation)) {
        return model.to_string();
    }
    // 4. The invention, logged so it is visible rather than silent.
    warn!(
        task_id = %conversation.task_id,
        agent_type = %conversation.agent_type,
        "No model could be resolved for the conversation; reporting a placeholder"
    );
    UNKNOWN_MODEL.to_string()
}

/// Which driver family backs this conversation's agent.
fn agent_kind(conversation: &Conversation) -> AgentKind {
    catalog::find(conversation.agent_type.as_str())
        .map_or(AgentKind::ClaudeCode, |entry| entry.kind)
}

/// The provider instance a model is resolved against.
///
/// The instance is the routing key, not part of the model name, and Moatless
/// has no provider-instance concept. T3's default instance for a driver is
/// named after the driver itself, and a conversation's agent kind already names
/// the driver. An instance a build does not recognize is marked unavailable at
/// runtime rather than refusing to decode, so this is the safe half of the pair.
fn provider_instance(conversation: &Conversation) -> &'static str {
    match agent_kind(conversation) {
        AgentKind::ClaudeCode => "claudeAgent",
        AgentKind::Codex => "codex",
    }
}

/// One project, in the shape the shell listing carries.
///
/// This is the contract's *shell* project, which is not quite its full project:
/// the shell one has no deletion timestamp, and soft-deleted workspaces never
/// reach here anyway.
///
/// # Two halves of a union, projected differently
///
/// The listing's projects are a union: the Workspaces this viewer may read,
/// plus the Workspaces a listed conversation references whether or not they may
/// read them (Decision 5). The widening is load-bearing — a conversation whose
/// project is absent is dropped by the client in silence — and it is justified
/// by the *names*: "seeing the names gives a person the shape of the deployment
/// they work in".
///
/// A start command is not a name. It carries paths, flags and whatever else
/// someone put in a server config, and projecting both halves alike handed all
/// of it to anyone who followed a single conversation running in a private
/// Workspace. So the widened half is projected down to what the contract needs
/// in order to render a group and no more, and
/// [`ListedWorkspace`](crate::workspace::dao::ListedWorkspace) is what makes
/// the difference available here rather than re-derived.
///
/// What that minimum still discloses, stated rather than implied: the id, the
/// name, the root synthesized *from* the id, and the two timestamps. The
/// timestamps stay because the contract requires them and supplies no empty
/// value for either, and an invented one would be the sort of plausible
/// stand-in this module refuses everywhere else.
fn project(listed: &ListedWorkspace) -> Value {
    let workspace = &listed.workspace;
    json!({
        "id": workspace.id,
        // `workspaces.name` is NOT NULL but not non-empty, and the contract
        // refuses a blank title. An unnamed workspace would take the whole
        // listing down with it, so it falls back to its id.
        "title": non_empty(&workspace.name).unwrap_or_else(|| workspace.id.clone()),
        "workspaceRoot": nominal_root(&workspace.id),
        // The environment advertises no repository identity, so the client
        // never groups projects across environments by git remote.
        "repositoryIdentity": Value::Null,
        // Model choice belongs to a Turn in Moatless, not to a Workspace.
        "defaultModelSelection": Value::Null,
        // Required by the contract, so the narrowed shape is an empty list
        // rather than an absent field.
        "scripts": if listed.readable { scripts(workspace) } else { Vec::new() },
        "createdAt": timestamp(workspace.created_at),
        "updatedAt": timestamp(workspace.updated_at)
    })
}

/// A distinct, stable, nominal root path for one workspace.
fn nominal_root(workspace_id: &str) -> String {
    format!("{NOMINAL_ROOT_PREFIX}/{workspace_id}")
}

/// The workspace's servers, as the scripts a person can run in the project.
///
/// Setup commands are provisioning rather than scripts and are deliberately
/// not mapped here.
fn scripts(workspace: &Workspace) -> Vec<Value> {
    let Some(servers) = workspace.servers.as_ref() else {
        return Vec::new();
    };
    let servers: Vec<ServerConfig> = match serde_json::from_value(servers.clone()) {
        Ok(servers) => servers,
        Err(error) => {
            warn!(
                %error,
                workspace_id = %workspace.id,
                "Workspace servers config is unreadable; reporting no scripts"
            );
            return Vec::new();
        }
    };

    servers
        .iter()
        .filter_map(|server| {
            // Three of a script's fields are non-empty strings, and one script
            // the client cannot read takes the entire listing down with it. A
            // server that cannot fill them is left out rather than emitted
            // malformed. A server with no start commands is one of those: it
            // runs its image's default command, so there is no command to name.
            let id = non_empty(&server.name)?;
            let command = non_empty(&server.start_commands.as_ref()?.join(" && "))?;
            let name = server
                .label
                .as_deref()
                .and_then(non_empty)
                .unwrap_or_else(|| id.clone());
            Some(json!({
                // The server's name: stable across reads, and already unique
                // within the workspace because it names a container in the pod.
                "id": id,
                "name": name,
                // Joined the same way the sandbox joins them when it starts the
                // server.
                "command": command,
                "icon": SCRIPT_ICON,
                // An instruction to the client, not a description of Moatless:
                // it asks the client to run this command itself when it creates
                // a working copy. A Moatless server is started by the Sandbox,
                // which runs it as a container in the pod, so the client running
                // it again would put a second process on the same bound port.
                "runOnWorktreeCreate": false,
                "autoOpenPreview": server.default
                // `previewUrl` is omitted rather than guessed: Moatless stores a
                // port, the contract wants a URL, and the published address only
                // exists once a task has a sandbox serving that port.
            }))
        })
        .collect()
}

/// The trimmed value, or `None` when nothing is left.
///
/// Most strings on this wire are trimmed-non-empty, and an empty one is not a
/// missing field — it is an undecodable listing.
fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Both columns default to the insert time, so a missing value means a
/// hand-written row rather than an unknown instant.
fn timestamp(value: Option<DateTime<Utc>>) -> String {
    instant(value.unwrap_or_else(Utc::now))
}

/// The one timestamp format on this wire.
fn instant(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A nullable instant, for the contract's several `NullOr(IsoDateTime)` fields.
fn nullable_instant(value: Option<DateTime<Utc>>) -> Value {
    value.map_or(Value::Null, |value| json!(instant(value)))
}

/// When the conversation was created.
///
/// `tasks` records this twice: as text the caller supplied, and as a database
/// timestamp added later. The typed column wins; the text is the fallback for
/// rows written before it existed.
fn created_at(conversation: &Conversation) -> DateTime<Utc> {
    conversation
        .created_at_db
        .or_else(|| {
            DateTime::parse_from_rfc3339(&conversation.created_at)
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })
        .unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::models::MessageRole;
    use crate::shared_models::AgentType;
    use moatless_backend_core::scope::Scope;

    // -----------------------------------------------------------------------
    // The contract, transcribed
    // -----------------------------------------------------------------------
    //
    // Copied from `packages/contracts/src/orchestration.ts` in
    // `soaplabs/t3code@moatless-adapter` — `OrchestrationShellSnapshot`,
    // `OrchestrationProjectShell` and `ProjectScript` — and deliberately *not*
    // derived from the projection below. A test written from the projection
    // cannot see a required field the projection never thought to emit, and a
    // single missing one makes the entire listing undecodable, so the sidebar
    // goes blank rather than losing one button.
    //
    // Note that the shell listing carries `OrchestrationProjectShell`, which is
    // the near-twin of `OrchestrationProject` minus `deletedAt`.

    const SNAPSHOT_REQUIRED: &[&str] = &["snapshotSequence", "projects", "threads", "updatedAt"];
    const SNAPSHOT_OPTIONAL: &[&str] = &[];

    const PROJECT_REQUIRED: &[&str] = &[
        "id",
        "title",
        "workspaceRoot",
        "defaultModelSelection",
        "scripts",
        "createdAt",
        "updatedAt",
    ];
    const PROJECT_OPTIONAL: &[&str] = &["repositoryIdentity"];

    const SCRIPT_REQUIRED: &[&str] = &["id", "name", "command", "icon", "runOnWorktreeCreate"];
    const SCRIPT_OPTIONAL: &[&str] = &["previewUrl", "autoOpenPreview"];

    /// Trimmed-non-empty strings in the transcribed types above. An empty value
    /// is refused by the contract exactly as a missing key would be.
    const PROJECT_NON_EMPTY: &[&str] = &["id", "title", "workspaceRoot"];
    const SCRIPT_NON_EMPTY: &[&str] = &["id", "name", "command"];

    const SCRIPT_ICONS: &[&str] = &["play", "test", "lint", "configure", "build", "debug"];

    // `OrchestrationThreadShell` (`orchestration.ts:401-427`) — the shell's
    // conversation row, and the near-twin of `OrchestrationThread` without its
    // messages, activities, checkpoints, proposed plans and `deletedAt`.
    //
    // "Optional" here means the contract can fill it: either `Schema.optional`
    // or a `withDecodingDefault`. Everything else must be on the wire, `null`
    // included, or the whole snapshot fails to decode and the sidebar goes
    // blank rather than losing one row.
    const THREAD_REQUIRED: &[&str] = &[
        "id",
        "projectId",
        "title",
        "modelSelection",
        "runtimeMode",
        "branch",
        "worktreePath",
        "latestTurn",
        "createdAt",
        "updatedAt",
        "session",
        "latestUserMessageAt",
        "hasPendingApprovals",
        "hasPendingUserInput",
        "hasActionableProposedPlan",
    ];
    const THREAD_OPTIONAL: &[&str] = &[
        "interactionMode",
        "archivedAt",
        "settledOverride",
        "settledAt",
        "snoozedUntil",
        "snoozedAt",
    ];
    const THREAD_NON_EMPTY: &[&str] = &["id", "projectId", "title"];

    // `ModelSelection` (`:64-68`), after the wire transform. `provider` is the
    // legacy shape the transform absorbs, not something to emit.
    const MODEL_SELECTION_REQUIRED: &[&str] = &["instanceId", "model"];
    const MODEL_SELECTION_OPTIONAL: &[&str] = &["options"];
    const MODEL_SELECTION_NON_EMPTY: &[&str] = &["instanceId", "model"];

    // `OrchestrationLatestTurn` (`:334-342`).
    const TURN_REQUIRED: &[&str] = &[
        "turnId",
        "state",
        "requestedAt",
        "startedAt",
        "completedAt",
        "assistantMessageId",
    ];
    const TURN_OPTIONAL: &[&str] = &["sourceProposedPlan"];
    const TURN_NON_EMPTY: &[&str] = &["turnId", "state"];

    // `OrchestrationSession` (`:272-281`).
    const SESSION_REQUIRED: &[&str] = &[
        "threadId",
        "status",
        "providerName",
        "activeTurnId",
        "lastError",
        "updatedAt",
    ];
    const SESSION_OPTIONAL: &[&str] = &["providerInstanceId", "runtimeMode"];
    const SESSION_NON_EMPTY: &[&str] = &["threadId", "status"];

    /// `OrchestrationLatestTurnState` (`:326-331`) — four values, not five.
    const TURN_STATES: &[&str] = &["running", "interrupted", "completed", "error"];
    /// `OrchestrationSessionStatus` (`:261-269`).
    const SESSION_STATUSES: &[&str] = &[
        "idle",
        "starting",
        "running",
        "ready",
        "interrupted",
        "stopped",
        "error",
    ];
    /// `RuntimeMode` (`:117-122`).
    const RUNTIME_MODES: &[&str] = &[
        "approval-required",
        "auto-accept-edits",
        "auto",
        "full-access",
    ];

    fn assert_keys(value: &Value, what: &str, required: &[&str], optional: &[&str]) {
        let object = value
            .as_object()
            .unwrap_or_else(|| panic!("{what} must be an object"));
        for key in required {
            assert!(object.contains_key(*key), "{what} is missing `{key}`");
        }
        for key in object.keys() {
            assert!(
                required.contains(&key.as_str()) || optional.contains(&key.as_str()),
                "{what} emits `{key}`, which the contract does not declare"
            );
        }
    }

    fn assert_non_empty(value: &Value, what: &str, keys: &[&str]) {
        for key in keys {
            let text = value[*key]
                .as_str()
                .unwrap_or_else(|| panic!("{what}.{key} must be a string"));
            assert_eq!(text, text.trim(), "{what}.{key} must be trimmed");
            assert!(!text.is_empty(), "{what}.{key} must not be empty");
        }
    }

    fn conversation(task_id: &str, workspace_id: &str) -> Conversation {
        Conversation {
            task_id: task_id.to_string(),
            workspace_id: Some(workspace_id.to_string()),
            name: Some("Fix the checkout".to_string()),
            branch: Some("feat/checkout".to_string()),
            agent_type: AgentType(AgentType::CLAUDE_CODE.to_string()),
            created_at: "2026-07-01T09:00:00.000Z".to_string(),
            created_at_db: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state: None,
            sandbox_stop_reason: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            latest_user_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
        }
    }

    /// A conversation with every optional part of the row filled in, so the key
    /// assertions see the widest shape this module ever emits.
    fn running_conversation(task_id: &str, workspace_id: &str) -> Conversation {
        let now = Utc::now();
        Conversation {
            turn_number: Some(3),
            turn_status: Some("active".to_string()),
            turn_started_at: Some(now),
            sandbox_desired_state: Some(SandboxDesiredState::Running),
            sandbox_started_at: Some(now),
            sandbox_updated_at: Some(now),
            latest_user_message_at: Some(now),
            assistant_model: Some("claude-sonnet-4-6".to_string()),
            ..conversation(task_id, workspace_id)
        }
    }

    fn threads(workspaces: &[ListedWorkspace], conversations: &[Conversation]) -> Vec<Value> {
        shell_snapshot(workspaces, conversations, &AgentModelDefaults::default(), 0)["threads"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn one_thread(conversation: &Conversation) -> Value {
        let listed = threads(
            &[workspace("ws-a", "Alpha")],
            std::slice::from_ref(conversation),
        );
        assert_eq!(listed.len(), 1, "the conversation should have been listed");
        listed[0].clone()
    }

    #[test]
    fn the_conversation_wire_shape_matches_the_contract_field_for_field() {
        let thread = one_thread(&running_conversation("task-a", "ws-a"));

        assert_keys(&thread, "thread", THREAD_REQUIRED, THREAD_OPTIONAL);
        assert_non_empty(&thread, "thread", THREAD_NON_EMPTY);
        assert!(
            RUNTIME_MODES.contains(&thread["runtimeMode"].as_str().unwrap()),
            "thread.runtimeMode must be one of the contract's literals"
        );
        for key in ["hasPendingApprovals", "hasPendingUserInput", "hasActionableProposedPlan"] {
            assert!(thread[key].is_boolean(), "thread.{key} must be a boolean");
        }

        let model = &thread["modelSelection"];
        assert_keys(
            model,
            "modelSelection",
            MODEL_SELECTION_REQUIRED,
            MODEL_SELECTION_OPTIONAL,
        );
        assert_non_empty(model, "modelSelection", MODEL_SELECTION_NON_EMPTY);

        let turn = &thread["latestTurn"];
        assert_keys(turn, "latestTurn", TURN_REQUIRED, TURN_OPTIONAL);
        assert_non_empty(turn, "latestTurn", TURN_NON_EMPTY);
        assert!(
            TURN_STATES.contains(&turn["state"].as_str().unwrap()),
            "latestTurn.state must be one of the contract's four literals"
        );

        let session = &thread["session"];
        assert_keys(session, "session", SESSION_REQUIRED, SESSION_OPTIONAL);
        assert_non_empty(session, "session", SESSION_NON_EMPTY);
        assert!(
            SESSION_STATUSES.contains(&session["status"].as_str().unwrap()),
            "session.status must be one of the contract's literals"
        );
    }

    /// The instance id is a branded slug, and an id that does not match is
    /// refused at decode rather than downgraded.
    #[test]
    fn the_provider_instance_is_a_well_formed_slug() {
        for agent_type in ["claude-code", "codex", "claude-code-fable", "made-up"] {
            let mut row = running_conversation("task-a", "ws-a");
            row.agent_type = AgentType(agent_type.to_string());
            let instance = one_thread(&row)["modelSelection"]["instanceId"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(instance.len() <= 64, "{instance} is too long for the slug");
            let mut chars = instance.chars();
            assert!(chars.next().is_some_and(|c| c.is_ascii_alphabetic()));
            assert!(chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        }
    }

    #[test]
    fn a_conversation_that_never_ran_reports_no_turn_and_no_session() {
        let thread = one_thread(&conversation("task-a", "ws-a"));
        assert_eq!(thread["latestTurn"], Value::Null);
        assert_eq!(thread["session"], Value::Null);
        assert_eq!(thread["worktreePath"], Value::Null);
        assert_eq!(thread["latestUserMessageAt"], Value::Null);
    }

    #[test]
    fn a_blank_conversation_name_falls_back_to_the_id() {
        let mut row = conversation("task-a", "ws-a");
        row.name = Some("   ".to_string());
        assert_eq!(one_thread(&row)["title"], "task-a");
        row.name = None;
        assert_eq!(one_thread(&row)["title"], "task-a");
    }

    /// Two Moatless states mean *not finished* and both land on `running`; the
    /// shared word `interrupted` does not mean the same thing on the two sides.
    #[test]
    fn every_moatless_turn_state_has_a_counterpart() {
        assert_eq!(turn_state("active"), Some("running"));
        assert_eq!(turn_state("interrupted"), Some("running"));
        assert_eq!(turn_state("stopped"), Some("interrupted"));
        assert_eq!(turn_state("completed"), Some("completed"));
        assert_eq!(turn_state("error"), Some("error"));
        assert_eq!(turn_state("from-a-newer-backend"), None);
    }

    #[test]
    fn an_unmappable_turn_state_reports_no_turn_rather_than_a_guess() {
        let mut row = running_conversation("task-a", "ws-a");
        row.turn_status = Some("from-a-newer-backend".to_string());
        let thread = one_thread(&row);
        assert_eq!(thread["latestTurn"], Value::Null);
        assert_eq!(thread["session"]["activeTurnId"], Value::Null);
    }

    /// The session's turn and the row's turn are the same turn, so the two ids
    /// have to agree — the client uses one to look up the other.
    #[test]
    fn an_open_turn_is_the_sessions_active_turn() {
        let thread = one_thread(&running_conversation("task-a", "ws-a"));
        assert_eq!(thread["latestTurn"]["turnId"], "task-a:3");
        assert_eq!(thread["session"]["activeTurnId"], "task-a:3");
        assert_eq!(thread["session"]["status"], "running");
    }

    #[test]
    fn a_finished_turn_leaves_the_session_with_no_active_turn() {
        let mut row = running_conversation("task-a", "ws-a");
        row.turn_status = Some("completed".to_string());
        row.turn_ended_at = Some(Utc::now());
        let thread = one_thread(&row);
        assert_eq!(thread["latestTurn"]["state"], "completed");
        assert!(thread["latestTurn"]["completedAt"].is_string());
        assert_eq!(thread["session"]["activeTurnId"], Value::Null);
        assert_eq!(thread["session"]["status"], "ready");
    }

    #[test]
    fn sandbox_intent_becomes_the_sessions_status() {
        let mut row = running_conversation("task-a", "ws-a");
        row.turn_status = Some("completed".to_string());

        row.sandbox_started_at = None;
        assert_eq!(one_thread(&row)["session"]["status"], "starting");

        row.sandbox_started_at = Some(Utc::now());
        row.sandbox_desired_state = Some(SandboxDesiredState::Stopped);
        assert_eq!(one_thread(&row)["session"]["status"], "stopped");

        row.sandbox_stop_reason = Some(SandboxStopReason::Error);
        assert_eq!(one_thread(&row)["session"]["status"], "error");

        row.sandbox_desired_state = Some(SandboxDesiredState::Removed);
        assert_eq!(
            one_thread(&row)["worktreePath"],
            Value::Null,
            "a removed sandbox has had its working copy reclaimed"
        );
    }

    /// The chain has to end in an answer, because the contract refuses an empty
    /// one and supplies no default of its own.
    #[test]
    fn the_model_chain_ends_in_an_answer() {
        let defaults = AgentModelDefaults {
            claude_code: Some("claude-opus-4-5".to_string()),
            codex: Some("gpt-5.6-sol".to_string()),
        };
        let mut row = conversation("task-a", "ws-a");

        assert_eq!(model_name(&row, &defaults), "claude-opus-4-5");

        // A catalog choice that names no model is not an answer: it resolves
        // inside the sandbox and is never written back.
        row.requested_model = Some("default".to_string());
        assert_eq!(model_name(&row, &defaults), "claude-opus-4-5");

        row.requested_model = Some("opus".to_string());
        assert_eq!(model_name(&row, &defaults), "opus");

        // Only the agent's own reply carries the identity of the model that ran.
        row.assistant_model = Some("claude-opus-4-5-20260115".to_string());
        assert_eq!(model_name(&row, &defaults), "claude-opus-4-5-20260115");

        row.agent_type = AgentType("codex".to_string());
        assert_eq!(model_name(&conversation("task-b", "ws-a"), &defaults), "claude-opus-4-5");
        assert_eq!(model_name(&row, &defaults), "claude-opus-4-5-20260115");
    }

    #[test]
    fn a_deployment_that_configures_no_default_still_reports_a_model() {
        let row = conversation("task-a", "ws-a");
        assert_eq!(model_name(&row, &AgentModelDefaults::default()), "unknown");
    }

    #[test]
    fn the_provider_instance_follows_the_agent_kind() {
        let mut row = conversation("task-a", "ws-a");
        assert_eq!(provider_instance(&row), "claudeAgent");
        row.agent_type = AgentType("codex".to_string());
        assert_eq!(provider_instance(&row), "codex");
        // An agent this build does not know is not a reason to refuse a row.
        row.agent_type = AgentType("from-a-newer-backend".to_string());
        assert_eq!(provider_instance(&row), "claudeAgent");
    }

    /// The client groups by project and drops a conversation whose project it
    /// cannot find, in silence. Leaving it out here costs the same row and
    /// leaves something to diagnose it by.
    #[test]
    fn a_conversation_with_no_listable_project_is_left_out() {
        let mut orphan = conversation("task-orphan", "ws-a");
        orphan.workspace_id = None;
        let mut elsewhere = conversation("task-elsewhere", "ws-a");
        elsewhere.workspace_id = Some("ws-not-listed".to_string());

        let listed = threads(
            &[workspace("ws-a", "Alpha")],
            &[
                conversation("task-a", "ws-a"),
                orphan,
                elsewhere,
            ],
        );
        assert_eq!(
            listed.iter().map(|t| t["id"].clone()).collect::<Vec<_>>(),
            vec![json!("task-a")]
        );
    }

    #[test]
    fn the_wire_shape_matches_the_contract_field_for_field() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.servers = Some(json!([{
            "name": "dev",
            "port": 3000,
            "startCommands": ["bun run dev"],
            "default": true
        }]));

        let snapshot = shell_snapshot(&[ws], &[], &AgentModelDefaults::default(), 0);
        assert_keys(&snapshot, "snapshot", SNAPSHOT_REQUIRED, SNAPSHOT_OPTIONAL);
        assert!(snapshot["snapshotSequence"]
            .as_i64()
            .is_some_and(|n| n >= 0));

        let project = &snapshot["projects"][0];
        assert_keys(project, "project", PROJECT_REQUIRED, PROJECT_OPTIONAL);
        assert_non_empty(project, "project", PROJECT_NON_EMPTY);

        let script = &project["scripts"][0];
        assert_keys(script, "script", SCRIPT_REQUIRED, SCRIPT_OPTIONAL);
        assert_non_empty(script, "script", SCRIPT_NON_EMPTY);
        assert!(
            SCRIPT_ICONS.contains(&script["icon"].as_str().unwrap()),
            "script.icon must be one of the contract's literals"
        );
        assert!(script["runOnWorktreeCreate"].is_boolean());
    }

    #[test]
    fn a_blank_workspace_name_falls_back_to_the_id() {
        let project = project(&workspace("ws-a", "   "));
        assert_eq!(project["title"], "ws-a");
    }

    #[test]
    fn a_server_that_cannot_fill_the_required_fields_is_left_out() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.servers = Some(json!([
            { "name": "  ", "port": 3000, "startCommands": ["bun run dev"] },
            { "name": "blank-command", "port": 4000, "startCommands": ["  "] },
            { "name": "empty-commands", "port": 5000, "startCommands": [] }
        ]));
        assert!(project(&ws)["scripts"].as_array().unwrap().is_empty());
    }

    /// A workspace this viewer may read. The widened half of the union — a
    /// workspace listed only because a conversation references it — is
    /// [`a_referenced_workspace`].
    fn workspace(id: &str, name: &str) -> ListedWorkspace {
        ListedWorkspace {
            readable: true,
            workspace: Workspace {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            kind: "blank".to_string(),
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            resources: None,
            network_profile: None,
            source: "manual".to_string(),
            source_repository_id: None,
            source_key: None,
            source_config_path: None,
            scope: Scope::User,
            owner_user_id: Some("user-1".to_string()),
            created_by: Some("user-1".to_string()),
            deleted: Some(false),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            },
        }
    }

    /// The same row, listed only because something the viewer follows runs in
    /// it. The read rule refused it.
    fn a_referenced_workspace(id: &str, name: &str) -> ListedWorkspace {
        ListedWorkspace {
            readable: false,
            ..workspace(id, name)
        }
    }

    #[test]
    fn each_workspace_gets_a_distinct_root_so_groups_do_not_collapse() {
        let snapshot = shell_snapshot(
            &[workspace("ws-a", "Alpha"), workspace("ws-b", "Beta")],
            &[],
            &AgentModelDefaults::default(),
            0,
        );
        let projects = snapshot["projects"].as_array().unwrap();

        assert_eq!(projects[0]["id"], "ws-a");
        assert_eq!(projects[0]["title"], "Alpha");
        assert_eq!(projects[1]["id"], "ws-b");
        assert_ne!(projects[0]["workspaceRoot"], projects[1]["workspaceRoot"]);
    }

    #[test]
    fn servers_become_scripts_and_setup_commands_do_not() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.setup_commands = Some(json!(["bun install"]));
        ws.workspace.servers = Some(json!([
            {
                "name": "dev",
                "label": "Dev server",
                "port": 3000,
                "startCommands": ["bun install", "bun run dev"],
                "default": true
            },
            { "name": "no-command", "port": 4000 }
        ]));

        let scripts = project(&ws)["scripts"].as_array().unwrap().clone();

        assert_eq!(scripts.len(), 1, "a server with no command is not a script");
        assert_eq!(scripts[0]["id"], "dev");
        assert_eq!(scripts[0]["name"], "Dev server");
        assert_eq!(scripts[0]["command"], "bun install && bun run dev");
        assert_eq!(scripts[0]["autoOpenPreview"], true);
        assert!(
            scripts[0].get("previewUrl").is_none(),
            "a bare port is not a URL, so the field is left out"
        );
    }

    /// The widened half of the project union carries what renders a group and
    /// nothing else.
    ///
    /// Asserted against the *same row* projected both ways, so the difference
    /// cannot be a difference in the fixture: the only thing that changes is
    /// which half of Decision 5's union the row came from. A projection that
    /// treats the halves alike fails one of the two `scripts` assertions
    /// whichever way it treats them.
    #[test]
    fn a_project_the_viewer_may_not_read_carries_no_scripts() {
        let servers = json!([{
            "name": "dev",
            "port": 3000,
            "startCommands": ["bun run dev --config /home/them/secret.toml"]
        }]);
        let mut readable = workspace("ws-a", "Alpha");
        readable.workspace.servers = Some(servers.clone());
        let mut referenced = a_referenced_workspace("ws-a", "Alpha");
        referenced.workspace.servers = Some(servers);

        assert_eq!(
            project(&readable)["scripts"][0]["command"],
            "bun run dev --config /home/them/secret.toml"
        );
        assert_eq!(project(&referenced)["scripts"], json!([]));

        // Still decodable, and still enough to render the group the widening
        // exists for.
        let projected = project(&referenced);
        assert_keys(&projected, "project", PROJECT_REQUIRED, PROJECT_OPTIONAL);
        assert_non_empty(&projected, "project", PROJECT_NON_EMPTY);
        assert_eq!(projected["title"], "Alpha");
    }

    #[test]
    fn an_unreadable_servers_config_reports_no_scripts() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.servers = Some(json!({ "not": "a list" }));
        assert!(project(&ws)["scripts"].as_array().unwrap().is_empty());
    }

    // -----------------------------------------------------------------------
    // The conversation itself
    // -----------------------------------------------------------------------
    //
    // `OrchestrationThread` (`orchestration.ts:342-378`) — the shell row's
    // fuller twin. It drops the four the shell adds for its own sorting and
    // badges, and adds the conversation: `deletedAt`, `messages`,
    // `proposedPlans`, `activities`, `checkpoints`.

    const DETAIL_REQUIRED: &[&str] = &[
        "id",
        "projectId",
        "title",
        "modelSelection",
        "runtimeMode",
        "branch",
        "worktreePath",
        "latestTurn",
        "createdAt",
        "updatedAt",
        "session",
        "deletedAt",
        "messages",
        "activities",
        "checkpoints",
    ];
    const DETAIL_OPTIONAL: &[&str] = &[
        "interactionMode",
        "archivedAt",
        "settledOverride",
        "settledAt",
        "snoozedUntil",
        "snoozedAt",
        "proposedPlans",
    ];

    // `OrchestrationMessage` (`:227-236`).
    const MESSAGE_REQUIRED: &[&str] = &[
        "id",
        "role",
        "text",
        "turnId",
        "streaming",
        "createdAt",
        "updatedAt",
    ];
    const MESSAGE_OPTIONAL: &[&str] = &["attachments"];
    const MESSAGE_ROLES: &[&str] = &["user", "assistant", "system"];

    // `OrchestrationThreadActivity` (`:312-321`).
    const ACTIVITY_REQUIRED: &[&str] = &[
        "id",
        "tone",
        "kind",
        "summary",
        "payload",
        "turnId",
        "createdAt",
    ];
    const ACTIVITY_OPTIONAL: &[&str] = &["sequence"];
    const ACTIVITY_NON_EMPTY: &[&str] = &["id", "kind", "summary"];
    /// `OrchestrationThreadActivityTone` (`:304-309`).
    const ACTIVITY_TONES: &[&str] = &["info", "tool", "approval", "error"];

    fn message(id: i32, message_type: MessageType, content: Value) -> TranscriptMessage {
        TranscriptMessage {
            id,
            uuid: format!("msg-{id}"),
            message_type,
            message_role: match message_type {
                MessageType::User => MessageRole::User,
                MessageType::Assistant => MessageRole::Assistant,
                _ => MessageRole::System,
            },
            message_content: content,
            turn_number: Some(3),
            // Minute `id` past the hour, so the fixture's own order is legible
            // in an assertion failure.
            timestamp: format!("2026-07-01T10:{id:02}:00.000Z"),
            created_at: Some(Utc::now()),
        }
    }

    fn transcript(messages: Vec<TranscriptMessage>) -> Transcript {
        Transcript {
            messages,
            blocks: Vec::new(),
            omitted: 0,
        }
    }

    /// The fixture transcript: a person asks, the agent thinks, runs a command
    /// that works and one that fails, and answers.
    fn a_conversation() -> Transcript {
        transcript(vec![
            message(1, MessageType::User, json!([{ "type": "text", "text": "Fix the build" }])),
            message(
                2,
                MessageType::Assistant,
                json!([
                    { "type": "thinking", "thinking": "" },
                    { "type": "tool_use", "id": "call-1", "name": "Bash",
                      "input": { "command": "cargo build" } },
                    { "type": "tool_use", "id": "call-2", "name": "Edit",
                      "input": { "file_path": "src/main.rs" } }
                ]),
            ),
            message(
                3,
                MessageType::User,
                json!([
                    { "type": "tool_result", "tool_use_id": "call-1", "content": "Compiling" },
                    { "type": "tool_result", "tool_use_id": "call-2",
                      "content": "no such file", "is_error": true }
                ]),
            ),
            message(4, MessageType::Assistant, json!([{ "type": "text", "text": "Done." }])),
        ])
    }

    fn detail(transcript: &Transcript) -> Value {
        thread_detail(
            &running_conversation("task-a", "ws-a"),
            "ws-a",
            &AgentModelDefaults::default(),
            transcript,
        )
    }

    #[test]
    fn the_detail_wire_shape_matches_the_contract_field_for_field() {
        let thread = detail(&a_conversation());

        assert_keys(&thread, "thread", DETAIL_REQUIRED, DETAIL_OPTIONAL);
        assert_non_empty(&thread, "thread", THREAD_NON_EMPTY);

        for message in thread["messages"].as_array().unwrap() {
            assert_keys(message, "message", MESSAGE_REQUIRED, MESSAGE_OPTIONAL);
            assert!(
                MESSAGE_ROLES.contains(&message["role"].as_str().unwrap()),
                "message.role must be one of the contract's three literals"
            );
            assert!(message["streaming"].is_boolean());
            assert!(
                !message["id"].as_str().unwrap().is_empty(),
                "message.id must not be empty"
            );
        }

        for activity in thread["activities"].as_array().unwrap() {
            assert_keys(activity, "activity", ACTIVITY_REQUIRED, ACTIVITY_OPTIONAL);
            assert_non_empty(activity, "activity", ACTIVITY_NON_EMPTY);
            assert!(
                ACTIVITY_TONES.contains(&activity["tone"].as_str().unwrap()),
                "activity.tone must be one of the contract's four literals"
            );
        }
    }

    /// The shell's four extra fields are the shell's. Emitting one here is not
    /// a harmless extra: `OrchestrationThread` is a closed struct, so an
    /// undeclared key fails the decode and the conversation never renders.
    #[test]
    fn the_detail_row_carries_none_of_the_shells_own_fields() {
        let thread = detail(&a_conversation());
        for key in [
            "latestUserMessageAt",
            "hasPendingApprovals",
            "hasPendingUserInput",
            "hasActionableProposedPlan",
        ] {
            assert!(
                thread.get(key).is_none(),
                "the detail row must not carry the shell's `{key}`"
            );
        }
    }

    #[test]
    fn what_was_said_becomes_messages_and_what_was_done_becomes_activities() {
        let thread = detail(&a_conversation());

        let spoken = thread["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| (message["role"].clone(), message["text"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            spoken,
            vec![
                (json!("user"), json!("Fix the build")),
                (json!("assistant"), json!("Done."))
            ],
            "the tool-result carrier row is not a bubble of its own"
        );

        let work = thread["activities"].as_array().unwrap();
        assert_eq!(
            work.iter().map(|a| a["summary"].clone()).collect::<Vec<_>>(),
            vec![json!("Thinking"), json!("Bash"), json!("Edit")]
        );
    }

    /// The client sorts a work log by `sequence` and falls back to comparing
    /// `createdAt` as strings. Every activity of one Message shares that
    /// Message's single timestamp, so without a sequence a turn's tool calls
    /// would arrive in an order the transcript never had.
    ///
    /// The numbers themselves are the row and the block rather than a count, so
    /// what is asserted is the order — which is all the client reads — and that
    /// two activities of one Message stay inside their own Message's range.
    #[test]
    fn activities_are_numbered_in_transcript_order() {
        let thread = detail(&a_conversation());
        let sequences = thread["activities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|activity| activity["sequence"].as_i64().unwrap())
            .collect::<Vec<_>>();
        assert!(
            sequences.windows(2).all(|pair| pair[0] < pair[1]),
            "activities ascend in transcript order: {sequences:?}"
        );
        assert!(
            sequences[2] - sequences[1] < BLOCKS_PER_MESSAGE,
            "two calls of one assistant Message are one stride apart at most"
        );
    }

    /// An activity delivered on its own has to sort into a work log built
    /// without it, which is only true while the number is derived from the row
    /// rather than counted while walking the transcript.
    #[test]
    fn an_activitys_number_does_not_depend_on_the_transcript_it_was_read_with() {
        let whole = detail(&a_conversation());
        let numbered: HashMap<String, i64> = whole["activities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|activity| {
                (
                    activity["id"].as_str().unwrap().to_string(),
                    activity["sequence"].as_i64().unwrap(),
                )
            })
            .collect();

        // The same conversation read from its second Message onwards, as a
        // change re-reading one row would see it.
        let tail = transcript(a_conversation().messages[1..].to_vec());
        for activity in detail(&tail)["activities"].as_array().unwrap() {
            let id = activity["id"].as_str().unwrap();
            assert_eq!(
                activity["sequence"].as_i64().unwrap(),
                numbered[id],
                "activity {id} keeps its number"
            );
        }
    }

    #[test]
    fn a_tool_call_carries_its_outcome() {
        let thread = detail(&a_conversation());
        let work = thread["activities"].as_array().unwrap();

        let ran = &work[1];
        assert_eq!(ran["kind"], "tool.completed");
        assert_eq!(ran["tone"], "tool");
        assert_eq!(ran["payload"]["status"], "completed");
        assert_eq!(ran["payload"]["itemType"], "command_execution");
        assert_eq!(ran["payload"]["command"], "cargo build");
        assert_eq!(ran["payload"]["detail"], "Compiling");
        assert_eq!(ran["payload"]["data"]["toolCallId"], "call-1");

        let failed = &work[2];
        assert_eq!(failed["tone"], "error");
        assert_eq!(failed["payload"]["status"], "failed");
        assert_eq!(failed["payload"]["itemType"], "file_change");
        assert!(
            failed["payload"].get("command").is_none(),
            "a call with no command has no terminal row to render"
        );
    }

    /// A call still running is reported as an update, not a completion: the
    /// client collapses an update into the completion that follows it, so a
    /// call reported complete before it is keeps the wrong row and drops the
    /// real outcome when it arrives.
    #[test]
    fn a_tool_call_with_no_result_yet_is_still_in_progress() {
        let mut waiting = a_conversation();
        waiting.messages.truncate(2);
        let work = detail(&waiting)["activities"].as_array().unwrap().clone();
        assert_eq!(work[1]["kind"], "tool.updated");
        assert_eq!(work[1]["payload"]["status"], "inProgress");
        assert!(work[1]["payload"].get("detail").is_none());
    }

    /// The result of a call whose text was long enough to be a file is cut, and
    /// says so. A subscription re-sends the whole transcript every time the
    /// conversation moves, so one `Read` of a large file would otherwise be
    /// paid for again on every message.
    #[test]
    fn a_large_tool_result_is_cut_with_a_marker() {
        let long = "x".repeat(TOOL_DETAIL_LIMIT * 2);
        let mut conversation = a_conversation();
        conversation.messages[2].message_content = json!([
            { "type": "tool_result", "tool_use_id": "call-1", "content": long }
        ]);
        let detail = detail(&conversation)["activities"][1]["payload"]["detail"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(detail.ends_with("… truncated"));
        assert!(detail.len() < TOOL_DETAIL_LIMIT + 32);
    }

    /// A tool result comes back as a bare string for most tools and as a block
    /// list for the ones that return structured output.
    #[test]
    fn a_structured_tool_result_shows_its_text() {
        let mut conversation = a_conversation();
        conversation.messages[2].message_content = json!([
            { "type": "tool_result", "tool_use_id": "call-1", "content": [
                { "type": "image", "source": {} },
                { "type": "text", "text": "3 files changed" }
            ] }
        ]);
        assert_eq!(
            detail(&conversation)["activities"][1]["payload"]["detail"],
            "3 files changed"
        );
    }

    /// The enriched blocks are what the person wrote; `message_content` is what
    /// the provider was sent, which for an enriched request is that text plus
    /// whatever was appended to it.
    #[test]
    fn a_user_message_prefers_the_blocks_the_ui_displays() {
        let mut conversation = a_conversation();
        conversation.blocks = vec![TranscriptBlock {
            task_message_id: 1,
            text_content: Some("Fix the build".to_string()),
        }];
        conversation.messages[0].message_content = json!([
            { "type": "text", "text": "Fix the build\n\n<system-reminder>…</system-reminder>" }
        ]);
        assert_eq!(detail(&conversation)["messages"][0]["text"], "Fix the build");
    }

    /// A Message written as a bare string rather than a block list.
    #[test]
    fn a_user_message_stored_as_a_string_still_renders() {
        let mut conversation = a_conversation();
        conversation.messages[0].message_content = json!("Fix the build");
        assert_eq!(detail(&conversation)["messages"][0]["text"], "Fix the build");
    }

    /// A capped read that simply began in the middle would look like a
    /// conversation that began there.
    #[test]
    fn a_capped_transcript_says_what_it_left_out() {
        let mut conversation = a_conversation();
        conversation.omitted = 12;
        let messages = detail(&conversation)["messages"].as_array().unwrap().clone();
        assert_eq!(messages[0]["role"], "system");
        assert!(messages[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("12 earlier messages"));
        assert_eq!(messages[0]["turnId"], Value::Null);
        assert_keys(&messages[0], "message", MESSAGE_REQUIRED, MESSAGE_OPTIONAL);
    }

    /// The run's own closing line is neither the person's nor the agent's.
    #[test]
    fn a_result_row_is_spoken_by_the_system() {
        let conversation = transcript(vec![message(
            1,
            MessageType::Result,
            json!([{ "type": "text", "text": "Stopped after an error" }]),
        )]);
        let messages = detail(&conversation)["messages"].as_array().unwrap().clone();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["text"], "Stopped after an error");
    }

    /// Only two of the contract's shapes have a renderer of their own, and
    /// guessing from a free-text tool name is how a search tool ends up drawn
    /// as a file diff.
    #[test]
    fn only_a_tool_with_a_renderer_claims_one() {
        assert_eq!(item_type("Bash"), "command_execution");
        assert_eq!(item_type("BashOutput"), "command_execution");
        assert_eq!(item_type("Edit"), "file_change");
        assert_eq!(item_type("Write"), "file_change");
        assert_eq!(item_type("NotebookEdit"), "file_change");
        assert_eq!(item_type("Grep"), "dynamic_tool_call");
        assert_eq!(item_type("mcp__linear__create_issue"), "dynamic_tool_call");
    }

    /// The transcript is ordered by `timestamp` and the client compares these
    /// as strings, so a row in another format would sort away from its
    /// neighbours. A row whose timestamp will not parse still needs one.
    #[test]
    fn every_message_reports_a_comparable_instant() {
        let mut conversation = a_conversation();
        conversation.messages[0].timestamp = "not a timestamp".to_string();
        let messages = detail(&conversation)["messages"].as_array().unwrap().clone();
        for message in &messages {
            let at = message["createdAt"].as_str().unwrap();
            assert!(
                DateTime::parse_from_rfc3339(at).is_ok(),
                "{at} is not an instant the client can compare"
            );
            assert_eq!(message["createdAt"], message["updatedAt"]);
        }
    }

    /// Nothing here is answered from a Turn that is not this Message's own.
    #[test]
    fn a_message_outside_any_turn_reports_no_turn() {
        let mut conversation = a_conversation();
        conversation.messages[0].turn_number = None;
        let detail = detail(&conversation);
        assert_eq!(detail["messages"][0]["turnId"], Value::Null);
        assert_eq!(detail["messages"][1]["turnId"], "task-a:3");
        assert_eq!(detail["activities"][0]["turnId"], "task-a:3");
    }

    /// An assistant turn that only called tools contributes activities and no
    /// bubble — an empty one would render as a blank reply.
    #[test]
    fn a_turn_that_only_called_tools_says_nothing() {
        let conversation = transcript(vec![message(
            1,
            MessageType::Assistant,
            json!([{ "type": "tool_use", "id": "call-1", "name": "Grep", "input": {} }]),
        )]);
        let detail = detail(&conversation);
        assert!(detail["messages"].as_array().unwrap().is_empty());
        assert_eq!(detail["activities"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_conversation_still_decodes() {
        let detail = detail(&transcript(Vec::new()));
        assert_keys(&detail, "thread", DETAIL_REQUIRED, DETAIL_OPTIONAL);
        assert_eq!(detail["messages"], json!([]));
        assert_eq!(detail["activities"], json!([]));
        assert_eq!(detail["checkpoints"], json!([]));
        assert_eq!(detail["deletedAt"], Value::Null);
    }

    // ── Events ──────────────────────────────────────────────────────────

    const EVENT_REQUIRED: &[&str] = &[
        "sequence",
        "eventId",
        "aggregateKind",
        "aggregateId",
        "occurredAt",
        "commandId",
        "causationEventId",
        "correlationId",
        "metadata",
        "type",
        "payload",
    ];

    /// The change that names Message `uuid`, at `position`.
    fn change(position: i64, uuid: Option<&str>) -> ConversationChange {
        ConversationChange {
            position,
            message_uuid: uuid.map(str::to_string),
        }
    }

    fn events(change: &ConversationChange, window: &Transcript) -> Vec<Value> {
        change_events(&running_conversation("task-a", "ws-a"), change, window)
            .into_iter()
            .map(|item| {
                assert_eq!(item["kind"], "event", "every item is a stream event");
                item["event"].clone()
            })
            .collect()
    }

    #[test]
    fn the_event_envelope_matches_the_contract_field_for_field() {
        let window = a_conversation();
        let sent = &events(&change(4, Some("msg-1")), &window)[0];
        assert_keys(sent, "event", EVENT_REQUIRED, &[]);
        assert_eq!(sent["aggregateKind"], "thread");
        assert_eq!(sent["aggregateId"], "task-a");
        assert_eq!(sent["commandId"], Value::Null);
        assert_eq!(sent["causationEventId"], Value::Null);
        assert_eq!(sent["correlationId"], Value::Null);
        assert_eq!(sent["metadata"], json!({}));
        assert_eq!(sent["eventId"], format!("task-a:{}", sent["sequence"]));
    }

    /// A person's Message is one bubble and no work-log rows.
    #[test]
    fn a_change_to_a_user_message_carries_the_bubble_it_wrote() {
        let window = a_conversation();
        let sent = events(&change(4, Some("msg-1")), &window);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["type"], "thread.message-sent");
        assert_eq!(sent[0]["payload"]["threadId"], "task-a");
        assert_eq!(sent[0]["payload"]["messageId"], "msg-1");
        assert_eq!(sent[0]["payload"]["role"], "user");
        assert_eq!(sent[0]["payload"]["text"], "Fix the build");
        assert_eq!(sent[0]["payload"]["streaming"], false);
        assert_eq!(sent[0]["payload"]["turnId"], "task-a:3");
    }

    /// An assistant Message that only called tools has no bubble, so its change
    /// says only what the work log shows.
    #[test]
    fn a_change_to_a_tool_calling_message_carries_only_its_work_log_rows() {
        let window = a_conversation();
        let sent = events(&change(4, Some("msg-2")), &window);
        assert!(
            sent.iter().all(|event| event["type"] == "thread.activity-appended"),
            "no bubble for a turn that said nothing: {sent:#?}"
        );
        let summaries: Vec<&str> = sent
            .iter()
            .map(|event| event["payload"]["activity"]["summary"].as_str().unwrap())
            .collect();
        assert_eq!(summaries, vec!["Thinking", "Bash", "Edit"]);
        assert_eq!(sent[0]["payload"]["threadId"], "task-a");
    }

    /// The whole reason a change is read with the Messages before it: nothing
    /// rewrites the row a tool call is on, so the row carrying the *result* is
    /// what has to re-send the call.
    #[test]
    fn a_change_carrying_tool_results_re_sends_the_calls_it_answers() {
        let window = a_conversation();
        let sent = events(&change(4, Some("msg-3")), &window);
        let resolved: Vec<(&str, &str)> = sent
            .iter()
            .map(|event| {
                (
                    event["payload"]["activity"]["summary"].as_str().unwrap(),
                    event["payload"]["activity"]["payload"]["status"]
                        .as_str()
                        .unwrap(),
                )
            })
            .collect();
        assert_eq!(
            resolved,
            vec![("Bash", "completed"), ("Edit", "failed")],
            "both calls the result row answers are re-sent with their outcome"
        );
        assert_eq!(
            sent[0]["payload"]["activity"]["id"], "msg-2:1",
            "under the identity the snapshot gave them, so the client upserts"
        );
    }

    /// The client drops an event whose sequence it has passed, so two may never
    /// share one — including across the boundary between two changes.
    #[test]
    fn every_event_of_a_change_has_its_own_ascending_sequence() {
        let window = a_conversation();
        let first: Vec<i64> = events(&change(4, Some("msg-3")), &window)
            .iter()
            .map(|event| event["sequence"].as_i64().unwrap())
            .collect();
        let second: Vec<i64> = events(&change(5, Some("msg-4")), &window)
            .iter()
            .map(|event| event["sequence"].as_i64().unwrap())
            .collect();
        assert!(first.windows(2).all(|pair| pair[0] < pair[1]), "{first:?}");
        assert!(
            first.last() < second.first(),
            "a later change is numbered above an earlier one: {first:?} then {second:?}"
        );
    }

    /// A change naming a Message the window does not hold — deleted, or one of
    /// the subagent rows no transcript reads — is a complete answer of nothing.
    #[test]
    fn a_change_naming_a_message_that_is_not_there_says_nothing() {
        assert!(events(&change(4, Some("msg-404")), &a_conversation()).is_empty());
    }

    /// A change naming no Message is the conversation's own, and the client
    /// derives the turn it renders from the session it is sent.
    #[test]
    fn a_change_to_the_conversation_itself_carries_its_session() {
        let sent = events(&change(4, None), &transcript(Vec::new()));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["type"], "thread.session-set");
        assert_eq!(sent[0]["payload"]["threadId"], "task-a");
        assert_eq!(sent[0]["payload"]["session"]["status"], "running");
        assert_eq!(sent[0]["payload"]["session"]["activeTurnId"], "task-a:3");
    }

    /// The contract's session payload has no empty value, so a conversation
    /// that has never had a Sandbox has nothing to say rather than a null to
    /// send.
    #[test]
    fn a_conversation_with_no_sandbox_sends_no_session_event() {
        let without = Conversation {
            sandbox_desired_state: None,
            ..running_conversation("task-a", "ws-a")
        };
        assert!(
            change_events(&without, &change(4, None), &transcript(Vec::new()))
                .is_empty()
        );
    }

    /// A message event carries the same values the snapshot's own bubble does,
    /// under the contract's other set of names for them.
    #[test]
    fn a_message_event_and_a_snapshot_bubble_agree() {
        let window = a_conversation();
        let listed = detail(&window);
        let bubble = &listed["messages"].as_array().unwrap()[0];
        let sent = events(&change(4, Some("msg-1")), &window);
        let payload = &sent[0]["payload"];
        assert_eq!(payload["messageId"], bubble["id"]);
        assert_eq!(payload["role"], bubble["role"]);
        assert_eq!(payload["text"], bubble["text"]);
        assert_eq!(payload["turnId"], bubble["turnId"]);
        assert_eq!(payload["streaming"], bubble["streaming"]);
        assert_eq!(payload["createdAt"], bubble["createdAt"]);
        assert_eq!(payload["updatedAt"], bubble["updatedAt"]);
    }

    /// An activity event carries the activity a snapshot would have carried,
    /// unchanged — including the number it sorts by, which is what lets it
    /// arrive long after the snapshot it has to sort into.
    #[test]
    fn an_activity_event_and_a_snapshot_activity_agree() {
        let window = a_conversation();
        let listed = detail(&window)["activities"].as_array().unwrap().clone();
        let sent = events(&change(4, Some("msg-2")), &window);
        for (event, activity) in sent.iter().zip(listed.iter()) {
            assert_eq!(event["payload"]["activity"], *activity);
        }
    }
}
