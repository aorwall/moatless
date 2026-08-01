//! Projection of Moatless rows onto the shapes T3's shell contract expects.
//!
//! Everything T3 calls a field name lives here or in the transport above it.
//! Nothing below this module knows the vocabulary: the input is a `Workspace`
//! row, the output is `serde_json::Value` on the wire.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use tracing::warn;

use super::AgentModelDefaults;
use super::dao::{Conversation, Transcript, TranscriptMessage};
use crate::agent::catalog::{self, AgentKind};
use crate::message::models::{MessageType, TOOL_RESPONSE_USER_TYPE};
use crate::sandbox::models::ServerConfig;
use crate::task_activities::dao::Activity;
use crate::task_activities::mapping::ActivityKind;
use crate::task_activities::view::ActivityView;
use crate::task_activities::{ConversationChange, EVENTS_PER_CHANGE};
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
pub(super) const UNRESOLVED_MODEL_CHOICE: &str = "default";

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
/// `checkpoints` is empty here and stays empty: they are T3's per-turn worktree
/// snapshots and Moatless captures none — the diff a person sees comes from the
/// Sandbox's own git state, not from a checkpoint ref this backend holds.
/// Filling it with something derived would put a revert button in front of a
/// person that nothing behind it can honour.
///
/// `proposedPlans` is read back out of the transcript — see [`proposed_plans`].
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
    row.insert(
        "proposedPlans".into(),
        Value::Array(proposed_plans(conversation, transcript)),
    );
    row.insert("checkpoints".into(), Value::Array(Vec::new()));
    Value::Object(row)
}

/// The plans the agent proposed and nobody has answered.
///
/// A Moatless proposed plan is an `ExitPlanMode` call still in progress: the run
/// is parked on it, and it leaves that state the moment the call settles. So the
/// list is read off the transcript rather than stored — one entry per unanswered
/// call, identified by the call's own activity id, which is stable across
/// re-reads because the block that wrote it is.
///
/// The card is a rendering of the plan. What answers it is the question joined
/// to the same call, which reaches the person as the answer panel.
fn proposed_plans(conversation: &Conversation, transcript: &Transcript) -> Vec<Value> {
    transcript
        .activities
        .iter()
        .filter(|activity| {
            activity.kind == ActivityKind::ToolCall
                && activity.summary == crate::task_activities::mapping::PLAN_TOOL
                && !activity.is_settled()
        })
        .filter_map(|activity| {
            let plan = activity
                .tool_input()
                .get("plan")
                .and_then(Value::as_str)
                .filter(|plan| !plan.trim().is_empty())?;
            let (turn, at) = activity_anchor(conversation, transcript, activity);
            Some(json!({
                "id": activity.activity_id,
                "turnId": turn,
                "planMarkdown": plan,
                "implementedAt": Value::Null,
                "implementationThreadId": Value::Null,
                "createdAt": at,
                "updatedAt": at
            }))
        })
        .collect()
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
/// Two things, both about the rows the change names:
///
/// 1. The Message's bubble, as `thread.message-sent`. The client upserts on the
///    message id, so re-sending an unchanged bubble is free, and the appends
///    that assemble one Message's text from its blocks are exactly why it is
///    re-sent.
/// 2. Its work-log rows, as `thread.activity-appended`. Also an upsert, keyed
///    on the activity id — which is what makes a tool result cheap here: the
///    result settles the row the call already wrote, and re-sending that row
///    under the same id flips the card from running to finished. There is no
///    third case for resolving rows the change answers, and no run of earlier
///    Messages to read in order to find them.
///
/// A change can name a uuid with no Message row at all: a turn that only called
/// tools said nothing. Then only the second half fires.
pub(super) fn change_events(
    conversation: &Conversation,
    change: &ConversationChange,
    window: &Transcript,
) -> Vec<Value> {
    let Some(uuid) = change.message_uuid.as_deref() else {
        return session_set(conversation, change).into_iter().collect();
    };
    let message = window.messages.iter().find(|message| message.uuid == uuid);
    let activities: Vec<&Activity> = window
        .activities
        .iter()
        .filter(|activity| activity.message_uuid.as_deref() == Some(uuid))
        .collect();
    if message.is_none() && activities.is_empty() {
        return Vec::new();
    }

    // The envelope's instant is the changed row's own. A change naming no
    // Message is dated by the first activity under it.
    let at = message.map_or_else(
        || {
            activities.first().map_or_else(
                || instant(created_at(conversation)),
                |activity| instant(activity.created_at),
            )
        },
        |message| instant(message_instant(conversation, message)),
    );

    // Each item numbered by the slot it owns rather than by where it lands in
    // this list: the bubble holds slot 0 whether or not it is emitted, and each
    // activity holds its own `ordinal + 1`. See [`activity_sequence`].
    let mut payloads: Vec<(usize, &str, Value)> = Vec::new();
    if let Some(message) = message {
        let turn = turn_reference(conversation, message);
        if let Some(bubble) = message_bubble(message, &turn, &at) {
            payloads.push((
                0,
                MESSAGE_SENT,
                message_sent_payload(&conversation.task_id, &bubble),
            ));
        }
    }
    for activity in activities {
        let (turn, activity_at) = activity_anchor(conversation, window, activity);
        let (kind, payload) = activity_appended_payload(
            &conversation.task_id,
            activity_row(activity, &turn, &activity_at),
        );
        let slot = usize::try_from(activity.ordinal).unwrap_or(usize::MAX);
        payloads.push((slot.saturating_add(1), kind, payload));
    }

    // `filter_map`, not `map_while`: the items are no longer numbered in list
    // order, so one past the ceiling says nothing about the next.
    payloads
        .into_iter()
        .filter_map(|(index, kind, payload)| {
            let sequence = change.sequence(index)?;
            Some(event(conversation, sequence, &at, kind, payload))
        })
        .collect()
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
    let at = instant(
        conversation
            .updated_at
            .unwrap_or_else(|| created_at(conversation)),
    );
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
/// Two fields are deliberately absent rather than null. `interactionMode` has
/// no separate Moatless state, and `archivedAt` would need a close timestamp
/// this backend does not record. Settlement and snoozing are follower state and
/// are emitted here.
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
        "session": session(conversation),
        "settledOverride": conversation
            .settled_override
            .as_deref()
            .map_or(Value::Null, |value| Value::String(value.to_string())),
        "settledAt": nullable_instant(conversation.settled_at),
        "snoozedUntil": nullable_instant(conversation.snoozed_until),
        "snoozedAt": nullable_instant(conversation.snoozed_at)
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

/// Split a Task's stored rows into the messages and activities T3 renders.
///
/// Two walks rather than one, because the two collections no longer nest: an
/// activity belongs to a Message by provenance, not by containment, and a turn
/// that only called tools has activities and no Message at all. Ordering is
/// carried explicitly on each side — Message order for the bubbles, the
/// conversation's own `position` for the work log.
fn rendered_transcript(conversation: &Conversation, transcript: &Transcript) -> RenderedTranscript {
    let mut rendered = RenderedTranscript {
        messages: Vec::new(),
        activities: Vec::new(),
    };
    if transcript.omitted > 0 {
        rendered
            .messages
            .push(elision_notice(conversation, transcript));
    }

    for message in &transcript.messages {
        let at = instant(message_instant(conversation, message));
        let turn = turn_reference(conversation, message);
        rendered
            .messages
            .extend(message_bubble(message, &turn, &at));
    }
    let surviving = surviving_context_windows(transcript);
    for activity in &transcript.activities {
        if activity.kind == ActivityKind::ContextWindowUpdated
            && !surviving.contains(activity.activity_id.as_str())
        {
            continue;
        }
        let (turn, at) = activity_anchor(conversation, transcript, activity);
        rendered.activities.push(activity_row(activity, &turn, &at));
    }

    rendered
}

/// Which context-window readings a snapshot carries: the newest per turn.
///
/// The meter shows one number, and the client reads it off the newest reading it
/// holds — every earlier one in the same turn is a row nothing renders. A
/// harness reporting per model request can produce several a minute, and this
/// projection re-sends its whole log every time the conversation moves, so the
/// superseded ones are dropped on the way out rather than stored differently.
///
/// Per turn rather than per conversation, because a Turn is the unit a person
/// scrolls back to: the reading a finished turn ended on stays legible beside it.
fn surviving_context_windows(transcript: &Transcript) -> HashSet<&str> {
    let mut newest: std::collections::HashMap<Value, &Activity> = std::collections::HashMap::new();
    for activity in &transcript.activities {
        if activity.kind != ActivityKind::ContextWindowUpdated {
            continue;
        }
        let turn = transcript
            .messages
            .iter()
            .rev()
            .find(|message| {
                message
                    .created_at
                    .is_some_and(|created| created <= activity.created_at)
            })
            .and_then(|message| message.turn_number)
            .map_or(Value::Null, |number| json!(number));
        newest
            .entry(turn)
            .and_modify(|held| {
                if (held.position, held.ordinal) <= (activity.position, activity.ordinal) {
                    *held = activity;
                }
            })
            .or_insert(activity);
    }
    newest
        .into_values()
        .map(|activity| activity.activity_id.as_str())
        .collect()
}

/// The bubble one Message contributes to the conversation, or `None` when it
/// contributes none.
///
/// Three of the four ways to contribute nothing are about *which* Message it is
/// rather than what it holds, which is why this is a match on the type rather
/// than a filter on the text.
fn message_bubble(message: &TranscriptMessage, turn: &Value, at: &str) -> Option<Value> {
    let text = message.message_text.as_deref().unwrap_or_default();
    match message.message_type {
        // A leftover carrier row for tool results. Nothing writes these any
        // more, and what they carried is on the activity the call wrote.
        MessageType::System => None,
        MessageType::User => {
            // A `user` row of this kind is the person answering a tool the agent
            // used to ask them something — the answer already reaches the client
            // as that call's outcome, and `message/transform.rs` makes the same
            // exclusion for the same reason.
            if message.user_type == TOOL_RESPONSE_USER_TYPE {
                return None;
            }
            (!text.is_empty()).then(|| message_row(message, "user", text.to_string(), turn, at))
        }
        MessageType::Assistant => (!text.is_empty())
            .then(|| message_row(message, "assistant", text.to_string(), turn, at)),
        // A run's own closing line, from before those stopped being written as
        // Messages. Neither the person nor the agent said it, and `system` is
        // the contract's role for exactly that.
        MessageType::Result | MessageType::Error => {
            (!text.is_empty()).then(|| message_row(message, "system", text.to_string(), turn, at))
        }
    }
}

/// The Turn an activity is shown under, and when it happened.
///
/// An activity carries no turn of its own — a turn is a property of the
/// conversation, and the row records what happened, not when the bookkeeping
/// around it changed. So the Turn is borrowed, in three steps:
///
/// 1. From the Message the activity was written with, when that Message exists.
/// 2. From the newest Message before it, when it does not — a turn that only
///    called tools wrote no Message, and its calls belong with the turn already
///    under way.
/// 3. From the conversation's newest Turn, when the window holds no Message at
///    all. This is the live path: a change is read moments after it was
///    recorded, so the newest Turn is the one it happened in.
fn activity_anchor(
    conversation: &Conversation,
    window: &Transcript,
    activity: &Activity,
) -> (Value, String) {
    if let Some(message) = activity
        .message_uuid
        .as_deref()
        .and_then(|uuid| window.messages.iter().find(|message| message.uuid == uuid))
    {
        return (
            turn_reference(conversation, message),
            instant(message_instant(conversation, message)),
        );
    }
    let preceding = window.messages.iter().rev().find(|message| {
        message
            .created_at
            .is_some_and(|created| created <= activity.created_at)
    });
    let turn = match preceding {
        Some(message) => turn_reference(conversation, message),
        None => conversation.turn_number.map_or(Value::Null, |number| {
            Value::String(turn_id(&conversation.task_id, number))
        }),
    };
    (turn, instant(activity.created_at))
}

/// The head of a transcript that did not fit, said out loud.
///
/// A capped read that simply started in the middle would look like a
/// conversation that began there. This is the one message in a transcript that
/// no row in the database backs, which is why it says so.
fn elision_notice(conversation: &Conversation, transcript: &Transcript) -> Value {
    let at = instant(transcript.messages.first().map_or_else(
        || created_at(conversation),
        |first| message_instant(conversation, first),
    ));
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

/// One stored activity as the work-log row the contract carries.
///
/// The stored `payload` is already this contract's shape — the writer builds it
/// there, in [`crate::task_activities::mapping`], so the two cannot drift. The
/// preview it carries is stored too, in its own column, cut once when the result
/// arrived rather than on every send. `input`, `result` and `typedResult` are
/// stripped: they are kept whole for the Moatless frontend, which asks for one
/// activity at a time, and this contract re-sends its whole snapshot every time
/// the conversation moves.
///
/// `sequence` is the number this row's own event carries — see
/// [`activity_sequence`]. An activity delivered on its own, long after the
/// snapshot it has to sort into, carries the number it was written with.
fn activity_row(activity: &Activity, turn: &Value, at: &str) -> Value {
    let view = ActivityView::new(activity);

    json!({
        "id": view.activity_id(),
        "tone": view.t3_tone(),
        "kind": view.t3_kind(),
        "summary": view.summary(),
        "payload": Value::Object(view.t3_payload()),
        "turnId": turn,
        "sequence": activity_sequence(activity),
        "createdAt": at
    })
}

/// Where one activity sits in the conversation's event numbering.
///
/// `position * EVENTS_PER_CHANGE + ordinal + 1` — the number
/// [`ConversationChange::sequence`] computes for the same row, so a client sees
/// the same value whether it arrived in a snapshot or as an event. That is what
/// the invariant at `task_activities::models` asks for: "a snapshot's sequence
/// and an event's sequence have to be the same kind of number for that to mean
/// anything".
///
/// **Slot 0 is the Message's bubble, whether or not one is emitted.** A change
/// enumerates a list that begins with the bubble only when the Message produced
/// one, so an index counted while walking would shift by one depending on
/// whether the caller's slice held that Message at all. Reserving the slot
/// removes the dependency; a change with nothing to say leaves a gap, and a
/// sequence is a cursor rather than a count.
///
/// The `+1` lives here rather than in the stored column: folding it in would
/// force either a special case for the rows that name no Message or the rule
/// written twice.
fn activity_sequence(activity: &Activity) -> i64 {
    activity.position * EVENTS_PER_CHANGE + i64::from(activity.ordinal) + 1
}

/// The Turn this Message belongs to, in T3's identity for one.
fn turn_reference(conversation: &Conversation, message: &TranscriptMessage) -> Value {
    message.turn_number.map_or(Value::Null, |number| {
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
    use std::collections::HashMap;

    use super::*;
    use crate::message::models::MessageRole;
    use crate::shared_models::AgentType;
    use crate::task_activities::mapping::{ActivityKind, ActivityStatus};
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
        "settledOverride",
        "settledAt",
        "snoozedUntil",
        "snoozedAt",
    ];
    const THREAD_OPTIONAL: &[&str] = &["interactionMode", "archivedAt"];
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
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
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
    fn settlement_and_snooze_state_are_null_when_unset() {
        let thread = one_thread(&running_conversation("task-a", "ws-a"));
        for key in ["settledOverride", "settledAt", "snoozedUntil", "snoozedAt"] {
            assert_eq!(thread[key], Value::Null, "{key}");
        }
    }

    #[test]
    fn shell_and_detail_rows_carry_the_same_settlement_and_snooze_state() {
        let settled_at = DateTime::parse_from_rfc3339("2026-07-02T09:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let snoozed_until = DateTime::parse_from_rfc3339("2026-07-03T09:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let snoozed_at = DateTime::parse_from_rfc3339("2026-07-02T10:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let conversation = Conversation {
            settled_override: Some("settled".to_string()),
            settled_at: Some(settled_at),
            snoozed_until: Some(snoozed_until),
            snoozed_at: Some(snoozed_at),
            ..running_conversation("task-a", "ws-a")
        };

        let shell = one_thread(&conversation);
        let detail = thread_detail(
            &conversation,
            "ws-a",
            &AgentModelDefaults::default(),
            &transcript(Vec::new(), Vec::new()),
        );
        for key in ["settledOverride", "settledAt", "snoozedUntil", "snoozedAt"] {
            assert_eq!(shell[key], detail[key], "{key}");
        }
        assert_eq!(shell["settledOverride"], json!("settled"));
        assert_eq!(shell["settledAt"], json!("2026-07-02T09:00:00.000Z"));
        assert_eq!(shell["snoozedUntil"], json!("2026-07-03T09:00:00.000Z"));
        assert_eq!(shell["snoozedAt"], json!("2026-07-02T10:00:00.000Z"));
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
        for key in [
            "hasPendingApprovals",
            "hasPendingUserInput",
            "hasActionableProposedPlan",
        ] {
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
        assert_eq!(
            model_name(&conversation("task-b", "ws-a"), &defaults),
            "claude-opus-4-5"
        );
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
            &[conversation("task-a", "ws-a"), orphan, elsewhere],
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
        assert!(
            snapshot["snapshotSequence"]
                .as_i64()
                .is_some_and(|n| n >= 0)
        );

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
        "settledOverride",
        "settledAt",
        "snoozedUntil",
        "snoozedAt",
    ];
    const DETAIL_OPTIONAL: &[&str] = &["interactionMode", "archivedAt", "proposedPlans"];

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

    /// A fixed instant the fixture's rows are dated from, so the ordering
    /// between a Message and an activity is stated rather than raced.
    fn at(offset: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_780_000_000 + offset, 0).expect("valid instant")
    }

    fn message(id: i32, message_type: MessageType, text: &str) -> TranscriptMessage {
        TranscriptMessage {
            id,
            uuid: format!("msg-{id}"),
            message_type,
            message_role: match message_type {
                MessageType::User => MessageRole::User,
                MessageType::Assistant => MessageRole::Assistant,
                _ => MessageRole::System,
            },
            message_text: Some(text.to_string()),
            user_type: "human".to_string(),
            turn_number: Some(3),
            // Minute `id` past the hour, so the fixture's own order is legible
            // in an assertion failure.
            timestamp: format!("2026-07-01T10:{id:02}:00.000Z"),
            created_at: Some(at(i64::from(id) * 100)),
        }
    }

    /// One activity of `kind`, written by block `ordinal` of the Message at
    /// `position`. Every activity of one Message shares that Message's single
    /// change position, which is what the writer stores.
    fn activity(
        message_uuid: &str,
        position: i64,
        ordinal: i32,
        kind: ActivityKind,
        summary: &str,
    ) -> Activity {
        Activity {
            activity_id: format!("{message_uuid}:{ordinal}"),
            task_id: "task-a".to_string(),
            position,
            ordinal,
            message_uuid: Some(message_uuid.to_string()),
            parent_uuid: None,
            kind,
            status: ActivityStatus::Completed,
            tool_use_id: None,
            summary: summary.to_string(),
            payload: json!({ "summary": summary }),
            detail: None,
            is_error: false,
            external_event_id: None,
            external_adapter: None,
            external_source_name: None,
            created_at: at(100 + position),
        }
    }

    /// One tool call, as the writer stores it once its result has landed.
    fn call(
        message_uuid: &str,
        position: i64,
        ordinal: i32,
        id: &str,
        name: &str,
        input: Value,
    ) -> Activity {
        let mut activity = activity(
            message_uuid,
            position,
            ordinal,
            ActivityKind::ToolCall,
            name,
        );
        activity.tool_use_id = Some(id.to_string());
        let mut payload = json!({
            "title": name,
            "itemType": crate::task_activities::mapping::item_type(name),
            "status": "completed",
            "data": { "toolCallId": id },
            "input": input,
        });
        if let Some(command) = input.get("command") {
            payload["command"] = command.clone();
        }
        activity.payload = payload;
        activity
    }

    fn transcript(messages: Vec<TranscriptMessage>, activities: Vec<Activity>) -> Transcript {
        Transcript {
            messages,
            activities,
            omitted: 0,
        }
    }

    /// The fixture transcript: a person asks; the agent thinks, runs a command
    /// that works and one that fails — saying nothing at all while it does, so
    /// that turn has activities and no Message row — and then answers.
    fn a_conversation() -> Transcript {
        let mut bash = call(
            "msg-2",
            2,
            1,
            "call-1",
            "Bash",
            json!({ "command": "cargo build" }),
        );
        bash.payload["result"] = json!("Compiling");
        bash.detail = Some("Compiling".to_string());
        let mut edit = call(
            "msg-2",
            2,
            2,
            "call-2",
            "Edit",
            json!({ "file_path": "src/main.rs" }),
        );
        edit.payload["status"] = json!("failed");
        edit.payload["result"] = json!("no such file");
        edit.detail = Some("no such file".to_string());
        edit.status = ActivityStatus::Failed;
        edit.is_error = true;

        transcript(
            vec![
                message(1, MessageType::User, "Fix the build"),
                message(4, MessageType::Assistant, "Done."),
            ],
            vec![
                activity("msg-2", 2, 0, ActivityKind::Thinking, "Thinking"),
                bash,
                edit,
                // The answering Message thought once before it spoke: a bubble
                // and an activity in the same, later change.
                activity("msg-4", 4, 0, ActivityKind::Thinking, "Thinking"),
            ],
        )
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
            "the turn that only called tools is not a bubble of its own"
        );

        let work = thread["activities"].as_array().unwrap();
        assert_eq!(
            work.iter()
                .map(|a| a["summary"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!("Thinking"),
                json!("Bash"),
                json!("Edit"),
                json!("Thinking")
            ]
        );
    }

    /// The client sorts a work log by `sequence` and falls back to comparing
    /// `createdAt` as strings. Every activity of one Message shares that
    /// Message's single timestamp, so without a sequence a turn's tool calls
    /// would arrive in an order the transcript never had.
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
    }

    /// An activity delivered on its own has to sort into a work log built
    /// without it, which is only true while the number is the one the row was
    /// written with rather than a count taken while walking the transcript.
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

        // The same activities read with none of the conversation around them,
        // as a change re-reading one turn would see them.
        let alone = transcript(Vec::new(), a_conversation().activities);
        for activity in detail(&alone)["activities"].as_array().unwrap() {
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
        waiting.activities[1].status = ActivityStatus::InProgress;
        waiting.activities[1].payload["status"] = json!("inProgress");
        waiting.activities[1]
            .payload
            .as_object_mut()
            .unwrap()
            .remove("result");
        waiting.activities[1].detail = None;

        let work = detail(&waiting)["activities"].as_array().unwrap().clone();
        assert_eq!(work[1]["kind"], "tool.updated");
        assert_eq!(work[1]["payload"]["status"], "inProgress");
        assert!(work[1]["payload"].get("detail").is_none());
    }

    /// The preview is carried through as stored, not re-derived. Cutting it here
    /// would mean cutting it again on every send, and a subscription re-sends
    /// the whole transcript every time the conversation moves.
    #[test]
    fn the_stored_preview_is_what_reaches_the_client() {
        let mut conversation = a_conversation();
        conversation.activities[1].payload["result"] = json!("x".repeat(10_000));
        conversation.activities[1].detail = Some("xxx\n… truncated".to_string());
        assert_eq!(
            detail(&conversation)["activities"][1]["payload"]["detail"],
            "xxx\n… truncated"
        );
    }

    /// The result is stored whole beside its preview, so the raw values never
    /// reach this contract — which re-sends its whole snapshot every time the
    /// conversation moves.
    #[test]
    fn an_activity_carries_no_raw_call_or_result_payload() {
        let payload = detail(&a_conversation())["activities"][1]["payload"].clone();
        for key in ["input", "result", "typedResult"] {
            assert!(
                payload.get(key).is_none(),
                "the T3 payload must not carry `{key}`"
            );
        }
    }

    /// A capped read that simply began in the middle would look like a
    /// conversation that began there.
    #[test]
    fn a_capped_transcript_says_what_it_left_out() {
        let mut conversation = a_conversation();
        conversation.omitted = 12;
        let messages = detail(&conversation)["messages"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages[0]["role"], "system");
        assert!(
            messages[0]["text"]
                .as_str()
                .unwrap()
                .starts_with("12 earlier messages")
        );
        assert_eq!(messages[0]["turnId"], Value::Null);
        assert_keys(&messages[0], "message", MESSAGE_REQUIRED, MESSAGE_OPTIONAL);
    }

    /// The run's own closing line is neither the person's nor the agent's.
    #[test]
    fn a_result_row_is_spoken_by_the_system() {
        let conversation = transcript(
            vec![message(1, MessageType::Result, "Stopped after an error")],
            Vec::new(),
        );
        let messages = detail(&conversation)["messages"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["text"], "Stopped after an error");
    }

    // ── the four panels ─────────────────────────────────────────────

    /// The card is read off the transcript: a Moatless proposed plan is an
    /// `ExitPlanMode` call still in progress, and it leaves that state the
    /// moment the call settles.
    #[test]
    fn an_unanswered_plan_is_a_proposed_plan_and_a_settled_one_is_not() {
        let mut proposed = call(
            "msg-2",
            2,
            0,
            "call-1",
            crate::task_activities::mapping::PLAN_TOOL,
            json!({ "plan": "1. Do the thing" }),
        );
        proposed.status = ActivityStatus::InProgress;

        let plans = detail(&transcript(Vec::new(), vec![proposed.clone()]))["proposedPlans"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0]["id"], "msg-2:0");
        assert_eq!(plans[0]["planMarkdown"], "1. Do the thing");
        assert_eq!(plans[0]["turnId"], "task-a:3");
        // Nothing behind these, so neither is claimed.
        assert_eq!(plans[0]["implementedAt"], Value::Null);
        assert_eq!(plans[0]["implementationThreadId"], Value::Null);

        let mut answered = proposed;
        answered.status = ActivityStatus::Completed;
        answered.payload["result"] = json!({ "approved": true });
        assert_eq!(
            detail(&transcript(Vec::new(), vec![answered]))["proposedPlans"],
            json!([])
        );
    }

    /// A card with no plan in it is a card rendering nothing.
    #[test]
    fn a_plan_call_carrying_no_plan_proposes_none() {
        for input in [json!({}), json!({ "plan": "   " })] {
            let mut proposed = call(
                "msg-2",
                2,
                0,
                "call-1",
                crate::task_activities::mapping::PLAN_TOOL,
                input,
            );
            proposed.status = ActivityStatus::InProgress;
            assert_eq!(
                detail(&transcript(Vec::new(), vec![proposed]))["proposedPlans"],
                json!([])
            );
        }
    }

    /// The meter reads the newest reading it holds and every earlier one in the
    /// same turn renders nothing, so a snapshot carries one per turn rather than
    /// the thousands a long thread accumulates.
    #[test]
    fn a_snapshot_carries_one_context_window_reading_per_turn() {
        let mut readings: Vec<Activity> = (0..3)
            .map(|ordinal| {
                let mut reading = activity(
                    "msg-2",
                    2,
                    ordinal,
                    ActivityKind::ContextWindowUpdated,
                    "Context window updated",
                );
                reading.message_uuid = None;
                reading.payload = json!({ "usedTokens": 1_000 * (i64::from(ordinal) + 1) });
                reading
            })
            .collect();
        readings.push(activity("msg-2", 2, 9, ActivityKind::Thinking, "Thinking"));

        let work = detail(&transcript(Vec::new(), readings))["activities"]
            .as_array()
            .unwrap()
            .clone();
        let meters: Vec<&Value> = work
            .iter()
            .filter(|row| row["kind"] == "context-window.updated")
            .collect();
        assert_eq!(meters.len(), 1, "only the newest survives: {work:#?}");
        assert_eq!(meters[0]["payload"]["usedTokens"], 3_000);
        assert_eq!(work.len(), 2, "everything else is untouched");
    }

    /// One case per kind the client derives a panel from, pinned against what
    /// that client's own parser requires. A payload it cannot read is not a
    /// degraded panel — it is no panel at all, silently.
    #[test]
    fn each_panel_reaches_the_client_in_the_shape_its_parser_requires() {
        let mut asked = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
        asked.payload = json!({
            "requestId": "call-1",
            "questions": [{
                "id": "Which store?",
                "header": "Storage",
                "question": "Which store?",
                "multiSelect": false,
                "options": [{ "label": "Postgres", "description": "Already deployed." }]
            }]
        });

        let mut resolved = activity("msg-3", 3, 0, ActivityKind::UserInputResolved, "Answered");
        resolved.payload = json!({
            "requestId": "call-1",
            "answers": { "Which store?": "Postgres" }
        });

        let mut plan = activity("msg-2", 2, 1, ActivityKind::PlanUpdated, "Plan updated");
        plan.payload = json!({
            "plan": [{ "step": "first", "status": "inProgress" }]
        });

        let mut meter = activity(
            "msg-2",
            2,
            2,
            ActivityKind::ContextWindowUpdated,
            "Context window updated",
        );
        meter.message_uuid = None;
        meter.payload = json!({ "usedTokens": 1_000, "maxTokens": 200_000 });

        let work = detail(&transcript(Vec::new(), vec![asked, plan, meter, resolved]))
            ["activities"]
            .as_array()
            .unwrap()
            .clone();
        let by_kind = |kind: &str| {
            work.iter()
                .find(|row| row["kind"] == kind)
                .unwrap_or_else(|| panic!("no {kind} row in {work:#?}"))
                .clone()
        };

        // The panel opens on a question whose id, header, question and options
        // are all present — the parser drops one missing any of them, and every
        // option needs a description of its own.
        let asked = by_kind("user-input.requested");
        assert_eq!(asked["tone"], "info");
        assert_eq!(asked["payload"]["requestId"], "call-1");
        let question = &asked["payload"]["questions"][0];
        for key in ["id", "header", "question"] {
            assert!(question[key].is_string(), "questions[0].{key}");
        }
        assert!(question["options"][0]["label"].is_string());
        assert!(question["options"][0]["description"].is_string());

        // And closes on a row naming the same request.
        let resolved = by_kind("user-input.resolved");
        assert_eq!(resolved["payload"]["requestId"], "call-1");

        // The plan panel reads `payload.plan`, each entry a step and its state.
        let plan = by_kind("turn.plan.updated");
        assert_eq!(plan["payload"]["plan"][0]["step"], "first");
        assert_eq!(plan["payload"]["plan"][0]["status"], "inProgress");

        // The meter reads a used count and, where one is known, a maximum.
        let meter = by_kind("context-window.updated");
        assert_eq!(meter["payload"]["usedTokens"], 1_000);
        assert_eq!(meter["payload"]["maxTokens"], 200_000);
    }

    /// A Sandbox activity belongs to no Message and no turn, and its kind ends
    /// in neither `.started` nor `.completed`, so it claims no position in the
    /// client's own lifecycle sort.
    #[test]
    fn a_sandbox_activity_claims_no_lifecycle_position() {
        let mut stopped = activity("", 9, 0, ActivityKind::SandboxStatus, "stopped");
        stopped.message_uuid = None;
        let mut failed = activity("", 10, 0, ActivityKind::SandboxError, "error");
        failed.message_uuid = None;
        failed.is_error = true;

        let work = detail(&transcript(Vec::new(), vec![stopped, failed]))["activities"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(work[0]["kind"], "task.progress");
        assert_eq!(work[0]["tone"], "info");
        assert_eq!(work[1]["kind"], "task.progress");
        assert_eq!(work[1]["tone"], "error");
    }

    /// The transcript is ordered by `timestamp` and the client compares these
    /// as strings, so a row in another format would sort away from its
    /// neighbours. A row whose timestamp will not parse still needs one.
    #[test]
    fn every_message_reports_a_comparable_instant() {
        let mut conversation = a_conversation();
        conversation.messages[0].timestamp = "not a timestamp".to_string();
        let messages = detail(&conversation)["messages"]
            .as_array()
            .unwrap()
            .clone();
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
    /// An activity whose turn wrote no Message borrows the one before it.
    #[test]
    fn a_message_outside_any_turn_reports_no_turn() {
        let mut conversation = a_conversation();
        conversation.messages[1].turn_number = None;
        let detail = detail(&conversation);
        assert_eq!(detail["messages"][0]["turnId"], "task-a:3");
        assert_eq!(detail["messages"][1]["turnId"], Value::Null);
        assert_eq!(detail["activities"][0]["turnId"], "task-a:3");
    }

    /// An assistant turn that only called tools contributes activities and no
    /// bubble — an empty one would render as a blank reply, and the writer
    /// stores no Message row for it at all.
    #[test]
    fn a_turn_that_only_called_tools_says_nothing() {
        let conversation = transcript(
            Vec::new(),
            vec![call("msg-2", 1, 0, "call-1", "Grep", json!({}))],
        );
        let detail = detail(&conversation);
        assert!(detail["messages"].as_array().unwrap().is_empty());
        assert_eq!(detail["activities"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_conversation_still_decodes() {
        let detail = detail(&transcript(Vec::new(), Vec::new()));
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
        let sent = &events(&change(1, Some("msg-1")), &window)[0];
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
        let sent = events(&change(1, Some("msg-1")), &window);
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
        let sent = events(&change(2, Some("msg-2")), &window);
        assert!(
            sent.iter()
                .all(|event| event["type"] == "thread.activity-appended"),
            "no bubble for a turn that said nothing: {sent:#?}"
        );
        let summaries: Vec<&str> = sent
            .iter()
            .map(|event| event["payload"]["activity"]["summary"].as_str().unwrap())
            .collect();
        assert_eq!(summaries, vec!["Thinking", "Bash", "Edit"]);
        assert_eq!(sent[0]["payload"]["threadId"], "task-a");
    }

    /// A tool result settles the row its call already wrote, and the change it
    /// records names the *calling* Message. So the outcome reaches the client
    /// as that same row re-sent under the same id, which the client upserts —
    /// no separate result row, and no run of earlier Messages to search.
    #[test]
    fn a_settled_call_is_re_sent_under_the_id_the_snapshot_gave_it() {
        let window = a_conversation();
        let sent = events(&change(2, Some("msg-2")), &window);
        let resolved: Vec<(&str, &str)> = sent
            .iter()
            .filter(|event| event["payload"]["activity"]["kind"] != "task.progress")
            .map(|event| {
                (
                    event["payload"]["activity"]["summary"].as_str().unwrap(),
                    event["payload"]["activity"]["payload"]["status"]
                        .as_str()
                        .unwrap(),
                )
            })
            .collect();
        assert_eq!(resolved, vec![("Bash", "completed"), ("Edit", "failed")]);
        assert_eq!(sent[1]["payload"]["activity"]["id"], "msg-2:1");
    }

    /// The client drops an event whose sequence it has passed, so two may never
    /// share one — including across the boundary between two changes.
    #[test]
    fn every_event_of_a_change_has_its_own_ascending_sequence() {
        let window = a_conversation();
        let first: Vec<i64> = events(&change(2, Some("msg-2")), &window)
            .iter()
            .map(|event| event["sequence"].as_i64().unwrap())
            .collect();
        let second: Vec<i64> = events(&change(4, Some("msg-4")), &window)
            .iter()
            .map(|event| event["sequence"].as_i64().unwrap())
            .collect();
        assert!(first.windows(2).all(|pair| pair[0] < pair[1]), "{first:?}");
        assert!(
            first.last() < second.first(),
            "a later change is numbered above an earlier one: {first:?} then {second:?}"
        );
    }

    /// The invariant the whole numbering exists for: a row a client received as
    /// an event and the same row read out of a snapshot have to carry one
    /// number, or sorting one against the other means nothing.
    #[test]
    fn an_activitys_event_is_numbered_the_same_as_its_snapshot_row() {
        let window = a_conversation();
        let snapshot: HashMap<String, i64> = detail(&window)["activities"]
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

        for change in [change(2, Some("msg-2")), change(4, Some("msg-4"))] {
            for event in events(&change, &window) {
                let Some(activity) = event["payload"].get("activity") else {
                    continue;
                };
                let id = activity["id"].as_str().unwrap();
                assert_eq!(
                    event["sequence"].as_i64().unwrap(),
                    snapshot[id],
                    "activity {id} is numbered the same either way"
                );
            }
        }
    }

    /// Slot 0 belongs to the bubble whether or not the Message wrote one, so a
    /// turn's first activity is numbered the same in a change that carries a
    /// bubble and one that does not.
    #[test]
    fn the_bubbles_slot_is_reserved_even_when_no_bubble_is_sent() {
        let window = a_conversation();
        let silent = events(&change(2, Some("msg-2")), &window);
        assert_eq!(silent[0]["payload"]["activity"]["summary"], "Thinking");
        assert_eq!(silent[0]["sequence"], 2 * EVENTS_PER_CHANGE + 1);

        let spoken = events(&change(4, Some("msg-4")), &window);
        assert_eq!(spoken[0]["type"], "thread.message-sent");
        assert_eq!(spoken[0]["sequence"], 4 * EVENTS_PER_CHANGE);
        assert_eq!(spoken[1]["payload"]["activity"]["summary"], "Thinking");
        assert_eq!(spoken[1]["sequence"], 4 * EVENTS_PER_CHANGE + 1);
    }

    /// A turn of ten or more blocks: the client falls back to comparing ids as
    /// strings, where `…:10` sorts before `…:2`. The number is what keeps the
    /// tenth block after the second.
    #[test]
    fn a_turn_of_ten_blocks_is_numbered_in_block_order() {
        let activities: Vec<Activity> = (0..12)
            .map(|ordinal| activity("msg-2", 2, ordinal, ActivityKind::Thinking, "Thinking"))
            .collect();
        let ids: Vec<String> = activities
            .iter()
            .map(|activity| activity.activity_id.clone())
            .collect();
        let mut sorted_by_id = ids.clone();
        sorted_by_id.sort();
        assert_ne!(
            ids, sorted_by_id,
            "the fixture has to be one the id order gets wrong"
        );

        let sequences: Vec<i64> = detail(&transcript(Vec::new(), activities))["activities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|activity| activity["sequence"].as_i64().unwrap())
            .collect();
        assert!(
            sequences.windows(2).all(|pair| pair[0] < pair[1]),
            "{sequences:?}"
        );
    }

    /// A change naming a uuid the window holds nothing for — no Message and no
    /// activity, so deleted, or one of the subagent rows no transcript reads —
    /// is a complete answer of nothing.
    #[test]
    fn a_change_naming_a_row_that_is_not_there_says_nothing() {
        assert!(events(&change(4, Some("msg-404")), &a_conversation()).is_empty());
    }

    /// A change naming no Message is the conversation's own, and the client
    /// derives the turn it renders from the session it is sent.
    #[test]
    fn a_change_to_the_conversation_itself_carries_its_session() {
        let sent = events(&change(4, None), &transcript(Vec::new(), Vec::new()));
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
            change_events(
                &without,
                &change(4, None),
                &transcript(Vec::new(), Vec::new())
            )
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
        let sent = events(&change(1, Some("msg-1")), &window);
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
        let sent = events(&change(2, Some("msg-2")), &window);
        for (event, activity) in sent.iter().zip(listed.iter()) {
            assert_eq!(event["payload"]["activity"], *activity);
        }
    }
}
