//! Projection of Moatless rows onto the shapes T3's shell contract expects.
//!
//! Everything T3 calls a field name lives here or in the transport above it.
//! Nothing below this module knows the vocabulary: the input is a `Workspace`
//! row, the output is `serde_json::Value` on the wire.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use tracing::warn;

use super::AgentModelDefaults;
use super::dao::{
    Conversation, ConversationMatch, ConversationPullRequest, Transcript, TranscriptMessage,
};
use super::ports::{is_draft, t3_pr_state};
use crate::activity::T3ActivityViewExt;
use crate::agent::catalog;
use crate::agent::catalog::UiRpcAgentKind;
use crate::providers;
use crate::workspace::{UiRpcListedWorkspace, WorkspaceView as Workspace};
use moatless_core::change_feed::conversation::{ConversationChange, EVENTS_PER_CHANGE};
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityView;
use moatless_types::message::MessageType;
use moatless_types::running::{RunningInputs, Working, derive_running};
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::{TaskRelationType, TurnStatus};

const TOOL_RESPONSE_USER_TYPE: &str = "tool_response";

/// The fields a script is read out of, narrowed from the workspace's stored
/// script config. Every field the canonical `ScriptConfig` defaults is optional
/// here too — one required field that is merely usually present costs the whole
/// workspace its scripts.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiRpcScriptConfig {
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    command: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    run_on_start: bool,
}

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

/// The icons the contract accepts, and the one anything else becomes.
///
/// The contract's icon is a closed set and an unknown value fails the whole
/// listing to decode. A workspace is free to store any string, so the value is
/// checked here rather than trusted: an unrecognized icon is reported as the
/// default instead of refusing the script, because an icon is not worth losing
/// a script over.
const SCRIPT_ICONS: [&str; 6] = ["play", "test", "lint", "configure", "build", "debug"];
const DEFAULT_SCRIPT_ICON: &str = "play";

/// Whether the contract would accept this icon.
///
/// The read path tolerates anything and reports the default; the write path
/// ([`scripts::replace`](crate::scripts::replace)) has to decide whether to
/// *store* what it was handed, and stores nothing it could not read back.
#[must_use]
pub fn is_script_icon(icon: &str) -> bool {
    SCRIPT_ICONS.contains(&icon)
}

/// Prefix of the nominal worktree path reported for each conversation.
///
/// A conversation's checkout lives in its own Sandbox, at the sandbox image's
/// `WORKSPACE_DIR` — the same string for every conversation, because it is a
/// property of the image rather than of the Task.
///
/// This path also doubles as an identity. The client sends a thread's `worktreePath`
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
    workspaces: &[UiRpcListedWorkspace],
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

/// One conversation's listing row, asked for by id rather than arrived at
/// through a listing.
///
/// The row is the listing's, to the field: what a client holds for a
/// conversation must not depend on which read delivered it, or the same
/// conversation would render two ways depending on how it was reached.
pub(super) fn thread_shell(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
) -> Value {
    thread(conversation, project_id, defaults)
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
    // Null rather than absent, because the client reads the key to clear its
    // previous answer. Which rows carry a name is the listing's decision; see
    // `Conversation::owner_name`.
    row.insert(
        "ownerName".into(),
        conversation
            .owner_name
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
    );
    // A `moat cmd` command outlives the turn that started it, and the agent
    // reports idle the moment that turn ends — so without this the sidebar row
    // for a Task running a build is the row for a finished one. `working`
    // rather than `monitoring`: the contract reserves that for watch loops with
    // no other live work, and a build is work.
    row.insert(
        "backgroundLiveness".into(),
        if conversation.running_commands > 0 {
            Value::String("working".into())
        } else {
            Value::Null
        },
    );
    Value::Object(row)
}

/// What the projection needs to say where a Message came from, beyond the
/// Message row itself: the deployment's own address, for a link back into
/// Moatless, and the names of the Tasks other Messages were authored by.
pub(super) struct OriginContext<'a> {
    pub frontend_url: &'a str,
    pub source_task_names: &'a HashMap<String, String>,
}

/// The one adapter whose URL is derived rather than joined: the GitHub PR
/// adapter is GitHub-only and its subject key carries the coordinates.
const GITHUB_PR_ADAPTER: &str = "github_pr";

/// What `ChatImageAttachment` and `ChatFileAttachment` accept, field for field
/// (`contracts/src/orchestration.ts:186-205`). An attachment is projected as
/// whichever member it fits, and one that fits neither is left out rather than
/// sent for the client to refuse — see [`message_attachments`].
const ATTACHMENT_ID_MAX_CHARS: usize = 128;
const ATTACHMENT_NAME_MAX_CHARS: usize = 255;
const ATTACHMENT_MIME_MAX_CHARS: usize = 100;
/// The same ceilings the mint signs for and the resolver measures a referenced
/// upload against, so what this surface accepts and what it projects back
/// cannot drift apart.
use crate::attachments::MAX_FILE_BYTES as ATTACHMENT_MAX_FILE_BYTES;
use crate::attachments::MAX_IMAGE_BYTES as ATTACHMENT_MAX_IMAGE_BYTES;
use crate::attachments::SUPPORTED_IMAGE_MIME_TYPES;

/// Where one Message came from, or `None` when it came from inside T3.
///
/// The first arm that matches wins: a Message authored by another Task is a
/// task origin even where it also carries adapter columns.
fn message_origin(message: &TranscriptMessage, origins: &OriginContext<'_>) -> Option<Value> {
    if let Some(source_task_id) = message.source_task_id.as_deref() {
        let name = origins.source_task_names.get(source_task_id);
        return Some(json!({
            "kind": "task",
            "label": name,
            // A Task the reader cannot see yields no name and no link: a link
            // they could not follow would be worse than none.
            "url": name.map(|_| format!(
                "{}/tasks/{source_task_id}",
                origins.frontend_url.trim_end_matches('/')
            )),
            "user": Value::Null
        }));
    }
    let adapter = message.external_adapter.as_deref()?;
    // The handle the external system knows the writer by, or — on a row written
    // before that was captured — the Moatless user the send resolved to.
    let author = message
        .external_author
        .as_deref()
        .or(message.sent_by_name.as_deref());
    if adapter == GITHUB_PR_ADAPTER {
        let label = message
            .external_subject_key
            .as_deref()
            .or(message.external_source_name.as_deref());
        return Some(json!({
            "kind": adapter,
            "label": label,
            "url": github_pr_url(
                message.external_subject_key.as_deref(),
                message.external_event_id.as_deref()
            ),
            "user": author
        }));
    }
    // The Binding's own name for the subject first: `external_source_name` is
    // whatever matched the subscription, which for Linear is a team id.
    Some(json!({
        "kind": adapter,
        "label": message
            .binding_label
            .as_deref()
            .or(message.external_source_name.as_deref()),
        "url": message.binding_url.as_deref(),
        "user": author
    }))
}

/// The GitHub URL a pull-request comment opens at, from `owner/repo#N`.
///
/// The subject key carries the repository and PR number; the event id says
/// which kind of comment, which is the fragment. A key that is not `owner/repo#N`
/// yields no URL rather than a wrong one, and an event id this does not
/// recognise — or none — leaves the reader at the pull request itself.
fn github_pr_url(subject_key: Option<&str>, external_event_id: Option<&str>) -> Option<String> {
    let (repo, number) = subject_key?.split_once('#')?;
    if repo.split('/').count() != 2 || repo.split('/').any(str::is_empty) {
        return None;
    }
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let base = format!("https://github.com/{repo}/pull/{number}");
    let fragment = external_event_id.and_then(|id| {
        id.strip_prefix("github:ic:")
            .map(|comment| format!("#issuecomment-{comment}"))
            .or_else(|| {
                id.strip_prefix("github:rc:")
                    .map(|comment| format!("#discussion_r{comment}"))
            })
    });
    Some(match fragment {
        Some(fragment) => format!("{base}{fragment}"),
        None => base,
    })
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
    origins: &OriginContext<'_>,
) -> Value {
    let mut row = conversation_row(conversation, project_id, defaults);
    let rendered = rendered_transcript(conversation, transcript, origins);
    // The contract requires this one to be present, and a conversation that no
    // longer exists is not projected at all — the read that would have found it
    // comes back empty and the caller answers "not found" — so null is the only
    // value this can carry. Deleting is not archiving: `archivedAt` above is
    // the close time, and this stays empty.
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

/// One child Task, in the shape `Subtask` has on the wire
/// (`packages/contracts/src/subtasks.ts`).
///
/// Deliberately not a thread row. A subtask is rendered as a fleet member
/// beside the subagents of the Task it belongs to, so it carries what that row
/// shows and a `threadId` to open it by, and none of the model, worktree or
/// session fields a thread carries — the client that follows the link gets
/// those from the thread itself.
pub(super) fn subtask(conversation: &Conversation, relation: TaskRelationType) -> Value {
    json!({
        "threadId": conversation.task_id,
        "title": conversation
            .name
            .as_deref()
            .and_then(non_empty)
            .unwrap_or_else(|| "New thread".to_string()),
        "relation": match relation {
            TaskRelationType::ForkedFrom => "forkedFrom",
            // `related_to` is not a parent edge and never reaches this
            // projection; the listing filters it out before projecting.
            _ => "createdBy",
        },
        "status": subtask_status(conversation),
        // Both are the agent parked on a person's answer, and a subtask's row
        // says so rather than reporting the run as still working.
        "awaitingInput": conversation.pending_question || conversation.pending_plan,
        "agentType": conversation.agent_type.as_str(),
        "branch": conversation
            .branch
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
        // The newest turn's number is how many turns have run, which is what a
        // fleet row shows as the child's depth of work.
        "turnCount": conversation.turn_number.unwrap_or(0).max(0),
        "createdAt": instant(created_at(conversation)),
        "updatedAt": instant(conversation.updated_at.unwrap_or_else(|| created_at(conversation))),
        "archivedAt": nullable_instant(conversation.closed_at),
    })
}

/// A child Task's state in the agents panel's own status vocabulary, so a
/// subtask row and a subagent row read the same.
///
/// Closing is Moatless's way of saying the work is done, so it outranks the
/// newest turn: a closed Task whose last turn errored is finished, not failing.
/// A Task that has never run is `pending`, and one whose last turn completed
/// while the Task is still open is `idle` — it can be resumed, which is exactly
/// what the panel's idle row means.
fn subtask_status(conversation: &Conversation) -> &'static str {
    if conversation.closed_at.is_some() {
        return "completed";
    }
    let Some(status) = conversation.turn_status.as_deref() else {
        return "pending";
    };
    let Some(parsed) = TurnStatus::parse(status) else {
        warn!(
            task_id = %conversation.task_id,
            status,
            "Turn status has no T3 counterpart; reporting the subtask as pending"
        );
        return "pending";
    };
    match parsed {
        TurnStatus::Active | TurnStatus::Interrupted => "running",
        TurnStatus::Completed => "idle",
        TurnStatus::Error => "failed",
        TurnStatus::Stopped => "interrupted",
    }
}

/// The plans the agent proposed, answered or not.
///
/// A Moatless proposed plan is an `ExitPlanMode` call: the run parks on it, and
/// an answer releases it. The list is read off the transcript rather than
/// stored — one entry per call, identified by the call's own activity id, which
/// is stable across re-reads because the block that wrote it is.
///
/// An answered call keeps its entry and carries `implementedAt`, which is what
/// the client reads to stop offering the plan. A rejected call carries it too:
/// the field says the plan was answered, not that it was approved.
///
/// The card is a rendering of the plan. What answers it is the question joined
/// to the same call, which reaches the person as the answer panel.
fn proposed_plans(conversation: &Conversation, transcript: &Transcript) -> Vec<Value> {
    transcript
        .activities
        .iter()
        .filter(|activity| {
            activity.kind == ActivityKind::ToolCall
                && activity.summary == moatless_types::activity::PLAN_TOOL
        })
        .filter_map(|activity| {
            let plan = activity
                .tool_input()
                .get("plan")
                .and_then(Value::as_str)
                .filter(|plan| !plan.trim().is_empty())?;
            let (turn, at) = activity_anchor(conversation, transcript, activity);
            let implemented_at = answered_at(transcript, activity);
            Some(json!({
                "id": activity.activity_id,
                "turnId": turn,
                "planMarkdown": plan,
                "implementedAt": implemented_at,
                "implementationThreadId": Value::Null,
                "createdAt": at,
                "updatedAt": implemented_at.as_deref().unwrap_or(at.as_str())
            }))
        })
        .collect()
}

/// When the answer to a parked interactive call landed, or `None` while it is
/// still parked.
///
/// The join is `requestId`: the resolved row leaves its own `tool_use_id` column
/// empty and carries the call's id in its payload instead
/// (`mapping::user_input_resolved`). A settled call whose resolved row the
/// window does not reach reports the call's own instant, so that no settled
/// call reads as still parked.
fn answered_at(transcript: &Transcript, call: &Activity) -> Option<String> {
    if !call.is_settled() {
        return None;
    }
    // A row with no id to join on matches nothing: comparing two `None`s here
    // would take the first resolved row that carries no `requestId` either.
    let resolved = call.tool_use_id.as_deref().and_then(|tool_use_id| {
        transcript.activities.iter().find(|activity| {
            activity.kind == ActivityKind::UserInputResolved
                && activity.payload.get("requestId").and_then(Value::as_str) == Some(tool_use_id)
        })
    });
    Some(instant(resolved.unwrap_or(call).created_at))
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
/// A change to one Message emits its bubble (`thread.message-sent`) and its
/// work-log rows (`thread.activity-appended`), both upserts keyed on their own
/// ids. A change naming a uuid with no Message row — a turn that only called
/// tools — emits the activities alone. A change naming an activity rather than a
/// Message emits that row alone and no `session_set`.
pub(super) fn change_events(
    conversation: &Conversation,
    change: &ConversationChange,
    window: &Transcript,
    origins: &OriginContext<'_>,
) -> Vec<Value> {
    if let Some(activity_id) = change.activity_id.as_deref() {
        return activity_change_events(conversation, change, window, activity_id);
    }
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
        if let Some(bubble) = message_bubble(message, &turn, &at, origins) {
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
            activity_row(activity, &turn, &activity_at, window),
        );
        let slot = usize::try_from(activity_slot(activity)).unwrap_or(usize::MAX);
        payloads.push((slot.saturating_add(1), kind, payload));
    }

    // By slot, because a reader delivers an item only when its sequence is past
    // the last it delivered: a derived row takes a slot above the blocks, so it
    // is emitted between two of them and would cost the reader the one after it.
    payloads.sort_by_key(|(slot, _, _)| *slot);

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

/// The one work-log row a change names by activity id, as its event.
///
/// A Sandbox lifecycle change and a context-window reading happen on no Message,
/// so the change names the activity itself and this is its whole projection: no
/// bubble, and no `session_set` — nothing about the conversation's own session
/// moved. The row is numbered `ordinal + 1` for the reason a Message's
/// activities are, so a client sees the same sequence whether the row arrived in
/// a snapshot (via [`activity_sequence`]) or here. A change naming a row the
/// window does not hold says nothing, which is a complete answer.
fn activity_change_events(
    conversation: &Conversation,
    change: &ConversationChange,
    window: &Transcript,
    activity_id: &str,
) -> Vec<Value> {
    let Some(activity) = window
        .activities
        .iter()
        .find(|activity| activity.activity_id == activity_id)
    else {
        return Vec::new();
    };
    let (turn, at) = activity_anchor(conversation, window, activity);
    let (kind, payload) = activity_appended_payload(
        &conversation.task_id,
        activity_row(activity, &turn, &at, window),
    );
    let slot = usize::try_from(activity_slot(activity)).unwrap_or(usize::MAX);
    change
        .sequence(slot.saturating_add(1))
        .map(|sequence| vec![event(conversation, sequence, &at, kind, payload)])
        .unwrap_or_default()
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
    let mut payload = json!({
        "threadId": task_id,
        "messageId": bubble["id"],
        "role": bubble["role"],
        "text": bubble["text"],
        "turnId": bubble["turnId"],
        "streaming": bubble["streaming"],
        "createdAt": bubble["createdAt"],
        "updatedAt": bubble["updatedAt"]
    });
    // Copied the same way as the seven above, so the delta and the snapshot are
    // one shape: a bubble with no origin produces a payload with none.
    if let Some(origin) = bubble.get("origin") {
        if let Some(object) = payload.as_object_mut() {
            object.insert("origin".into(), origin.clone());
        }
    }
    payload
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
/// `interactionMode` is deliberately absent rather than null: it has no
/// separate Moatless state. `archivedAt` is the Task's close time, because
/// archiving a conversation is closing it. Settlement and snoozing are follower
/// state and are emitted here.
fn conversation_row(
    conversation: &Conversation,
    project_id: &str,
    defaults: &AgentModelDefaults,
) -> Map<String, Value> {
    let created = created_at(conversation);
    let pull_requests = thread_pull_requests(conversation);
    let Value::Object(row) = json!({
        "id": conversation.task_id,
        "projectId": project_id,
        // `tasks.name` is nullable and has no non-empty constraint, and the
        // contract refuses a blank title, so an unnamed conversation shows the
        // contract's "New thread" placeholder until it is auto-titled.
        "title": conversation
            .name
            .as_deref()
            .and_then(non_empty)
            .unwrap_or_else(|| "New thread".to_string()),
        "modelSelection": model_selection(conversation, defaults),
        "runtimeMode": RUNTIME_MODE,
        "branch": conversation
            .branch
            .as_deref()
            .and_then(non_empty)
            .map_or(Value::Null, Value::String),
        "worktreePath": worktree_path(conversation),
        // `linkedPullRequest` is the primary of `pullRequests`. Both are
        // emitted: a client from before `threadPullRequests` reads the single
        // reference, and the capability is what tells a newer one to prefer the
        // list.
        "linkedPullRequest": legacy_linked_pull_request(pull_requests.first(), project_id),
        "pullRequests": Value::Array(pull_requests),
        "latestTurn": latest_turn(conversation),
        "createdAt": instant(created),
        "updatedAt": instant(conversation.updated_at.unwrap_or(created)),
        "archivedAt": nullable_instant(conversation.closed_at),
        "session": session(conversation),
        "settledOverride": conversation
            .settled_override
            .as_deref()
            .map_or(Value::Null, |value| Value::String(value.to_string())),
        "settledAt": nullable_instant(conversation.settled_at),
        "snoozedUntil": nullable_instant(conversation.snoozed_until),
        "snoozedAt": nullable_instant(conversation.snoozed_at),
        "visibility": conversation.visibility.as_str()
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
fn rendered_transcript(
    conversation: &Conversation,
    transcript: &Transcript,
    origins: &OriginContext<'_>,
) -> RenderedTranscript {
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
            .extend(message_bubble(message, &turn, &at, origins));
    }
    let surviving = surviving_context_windows(transcript);
    for activity in &transcript.activities {
        if activity.kind == ActivityKind::ContextWindowUpdated
            && !surviving.contains(activity.activity_id.as_str())
        {
            continue;
        }
        let (turn, at) = activity_anchor(conversation, transcript, activity);
        rendered
            .activities
            .push(activity_row(activity, &turn, &at, transcript));
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
fn message_bubble(
    message: &TranscriptMessage,
    turn: &Value,
    at: &str,
    origins: &OriginContext<'_>,
) -> Option<Value> {
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
            // Only a `user` bubble can carry an origin: an external message is
            // someone else speaking, and the agent's own replies are not.
            (!text.is_empty()).then(|| {
                let mut row = message_row(
                    message,
                    "user",
                    text.to_string(),
                    turn,
                    at,
                    message_origin(message, origins),
                );
                // Only a `user` bubble can carry attachments: the uploads on a
                // Message are what the person attached to it.
                let attachments = message_attachments(message);
                if !attachments.is_empty() {
                    if let Some(object) = row.as_object_mut() {
                        object.insert("attachments".into(), Value::Array(attachments));
                    }
                }
                row
            })
        }
        MessageType::Assistant => (!text.is_empty())
            .then(|| message_row(message, "assistant", text.to_string(), turn, at, None)),
        // A run's own closing line, from before those stopped being written as
        // Messages. Neither the person nor the agent said it, and `system` is
        // the contract's role for exactly that.
        MessageType::Result | MessageType::Error => (!text.is_empty())
            .then(|| message_row(message, "system", text.to_string(), turn, at, None)),
    }
}

/// The uploads on a Message, as the contract's `ChatAttachment` union.
///
/// Which member an upload becomes is decided by which one it *fits*, not by
/// what its mime type looks like. A `ChatImageAttachment` is one of the four
/// types the image member names — gif, jpeg, png, webp — at up to 10 MiB, and
/// may be zero bytes; a `ChatFileAttachment` takes any type at one byte to
/// 50 MiB (`contracts/src/orchestration.ts:186-205`). So an upload the image
/// member cannot hold, because it is an SVG or because it is 20 MiB, is a file
/// rather than nothing: the mint accepts both under `type: "file"`, and the
/// file member is exactly the shape the contract has for them.
///
/// An upload past *both* members is left out rather than sent, because the
/// client decodes the *whole* thread snapshot against that schema: a single
/// field that fails its check costs the reader the conversation, not the
/// attachment. Leaving it out costs them one row in one bubble. Reading the
/// member off the mime type alone got this wrong in both directions at once —
/// dropping a large image the agent had already read, and sending an image
/// member for a type the client has no shape for.
///
/// The agent is unaffected either way — it reads the file off disk from the
/// path the prompt names, which is decided by the upload and not by this.
fn message_attachments(message: &TranscriptMessage) -> Vec<Value> {
    let Some(contexts) = message.file_contexts.as_ref().and_then(Value::as_array) else {
        return Vec::new();
    };

    contexts
        .iter()
        .filter_map(|context| {
            let id = context.get("referenceId").and_then(Value::as_str)?;
            if id.is_empty() || id.len() > ATTACHMENT_ID_MAX_CHARS || !is_attachment_id(id) {
                return None;
            }

            let data = context.get("data")?;
            let mime_type = data.get("mimeType").and_then(Value::as_str)?;
            if mime_type.is_empty() || mime_type.len() > ATTACHMENT_MIME_MAX_CHARS {
                return None;
            }
            let name = truncate_on_char_boundary(
                data.get("filename").and_then(Value::as_str)?.trim(),
                ATTACHMENT_NAME_MAX_CHARS,
            );
            if name.is_empty() {
                return None;
            }

            let size = data.get("size").and_then(Value::as_i64)?;
            // The member is whichever one the attachment fits, not the one its
            // mime type suggests, and fitting the image member takes more than
            // an `image/` prefix: the contract names four types, so an SVG or
            // an AVIF is a file however image-like it reads. The file member
            // carries any type up to 50 MiB, which is what both of those cases
            // land in — a 20 MiB PNG sent as a file, and a 1 KiB SVG.
            //
            // Getting this wrong in either direction costs the whole
            // conversation rather than the attachment: too strict and an upload
            // the mint accepted goes missing after the agent has read it, too
            // loose and the client is sent an image member it has no shape for.
            let lowered = mime_type.to_ascii_lowercase();
            let kind = if SUPPORTED_IMAGE_MIME_TYPES.contains(&lowered.as_str())
                && (0..=ATTACHMENT_MAX_IMAGE_BYTES).contains(&size)
            {
                "image"
            } else if (1..=ATTACHMENT_MAX_FILE_BYTES).contains(&size) {
                "file"
            } else {
                return None;
            };

            Some(json!({
                "type": kind,
                "id": id,
                "name": name,
                "mimeType": mime_type,
                "sizeBytes": size
            }))
        })
        .collect()
}

/// `ChatAttachmentId`'s alphabet (`contracts/src/orchestration.ts:152-155`). An
/// upload id is a uuid and always passes; this is what keeps a hand-written or
/// migrated id from reaching a client that would refuse the snapshot holding it.
fn is_attachment_id(id: &str) -> bool {
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Cut `text` to at most `max` bytes without splitting a character.
///
/// Bytes rather than characters, and deliberately: the contract's limit counts
/// UTF-16 units, which no multi-byte character can exceed the byte count of, so
/// a byte cut is never the looser reading of the same rule.
fn truncate_on_char_boundary(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
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
    origin: Option<Value>,
) -> Value {
    let mut row = json!({
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
    });
    // Present only on a Message that came from outside T3. An ordinary bubble
    // carries no `origin` key at all, byte-identical to before this field.
    if let Some(origin) = origin {
        if let Some(object) = row.as_object_mut() {
            object.insert("origin".into(), origin);
        }
    }
    row
}

/// One stored activity as the work-log row the contract carries.
///
/// The stored payload owns the title, status and complete call data. The
/// canonical activity view projects that into this client's compact shape. The
/// preview is stored too, in its own column, cut once when the result arrived
/// rather than on every send. Full `input`, `result` and `typedResult` stay in
/// storage for the Moatless frontend, which asks for one activity at a time;
/// this contract re-sends its whole snapshot whenever the conversation moves.
///
/// `sequence` is the number this row's own event carries — see
/// [`activity_sequence`]. An activity delivered on its own, long after the
/// snapshot it has to sort into, carries the number it was written with.
fn activity_row(activity: &Activity, turn: &Value, at: &str, window: &Transcript) -> Value {
    let view = ActivityView::new(activity);
    let mut payload = view.t3_payload();
    stamp_subagent_usage(&mut payload, window);

    json!({
        "id": view.activity_id(),
        "tone": view.t3_tone(),
        "kind": panel_kind(activity, &window.settled_requests).unwrap_or_else(|| view.t3_kind()),
        "summary": view.t3_summary(),
        "payload": Value::Object(payload),
        "turnId": turn,
        "sequence": activity_sequence(activity),
        "createdAt": at
    })
}

/// What the subagent this row spawned has spent, on the row that names it.
///
/// Stamped here rather than in the payload projection because the numbers are
/// not on the row: they are summed across Messages the transcript excludes, and
/// reach this point on the transcript that read them. Keyed by the `taskId` the
/// payload projection already stamped, so a row that names no subagent — every
/// row but one kind — is left alone.
fn stamp_subagent_usage(payload: &mut Map<String, Value>, window: &Transcript) {
    let Some(usage) = payload
        .get("taskId")
        .and_then(Value::as_str)
        .and_then(|task_id| window.subagent_usage.get(task_id))
    else {
        return;
    };
    payload.insert(
        "typedUsage".into(),
        json!({
            "totalTokens": usage.input_tokens + usage.output_tokens,
            "inputTokens": usage.input_tokens,
            "outputTokens": usage.output_tokens
        }),
    );
}

/// The requests whose call has come back — [`Transcript::settled_requests`].
type SettledCalls = HashSet<String>;

/// What a question's panel row is, once its call has come back.
///
/// The row a panel is derived from is written beside the call rather than being
/// it, so no result ever lands on it and its own status never moves. Left at
/// `requested` it asks forever: the client keeps the panel open until it sees a
/// row naming the same request resolved, and an answer to a settled call is
/// refused, so the question cannot be got rid of from there at all.
///
/// The call's status is what settles it, however it was settled — answered here,
/// answered on another surface, or dropped by a resuming session. This reads it
/// the way a tool row already reads its own: open, it is a request; come back, it
/// is a resolution.
fn panel_kind(activity: &Activity, settled: &SettledCalls) -> Option<&'static str> {
    if activity.kind != ActivityKind::UserInputRequested {
        return None;
    }
    let request_id = activity.payload.get("requestId").and_then(Value::as_str)?;
    settled
        .contains(request_id)
        .then_some(crate::activity::USER_INPUT_RESOLVED_KIND)
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
    activity.position * EVENTS_PER_CHANGE + activity_slot(activity) + 1
}

/// The slot a row owns inside its change, above the bubble's slot 0.
///
/// A row a panel is derived from is written beside the call at the same Message
/// and the same block, so the block alone numbers the two identically — and a
/// reader that delivers each sequence once keeps whichever arrived first, always
/// the call, because it sorts ahead of the row derived from it. The panel row is
/// then dropped on the way out, which is every appearance and every closure of
/// that panel.
///
/// So derived rows take a band above every block a Message can hold. Read off
/// the row rather than stored on it, so rows already written are numbered by the
/// same rule, and a snapshot and a live change go on agreeing.
fn activity_slot(activity: &Activity) -> i64 {
    let ordinal = i64::from(activity.ordinal);
    match activity.kind {
        ActivityKind::UserInputRequested | ActivityKind::PlanUpdated => DERIVED_SLOT_BASE + ordinal,
        _ => ordinal,
    }
}

/// Where the derived rows' slots begin. Above any block count a Message reaches,
/// and low enough that the band itself stays inside [`EVENTS_PER_CHANGE`].
const DERIVED_SLOT_BASE: i64 = 512;

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
        .is_some_and(TurnStatus::is_open)
}

/// The signals the running rule reads, as this projection can see them.
///
/// A listing reads rows and never the runtime, so `agent` is `None` — the
/// heartbeat reaches these rows through the reaper rather than through a
/// per-conversation call this poll could not afford. `sandbox_up` is read from
/// `sandbox_started_at` for the same reason: no observed pod state is at hand.
fn running_inputs(
    conversation: &Conversation,
    desired_state: SandboxDesiredState,
) -> RunningInputs {
    RunningInputs {
        desired_state: Some(desired_state),
        stop_reason: conversation.sandbox_stop_reason(),
        sandbox_up: Some(conversation.sandbox_started_at.is_some()),
        turn_status: conversation
            .turn_status
            .as_deref()
            .and_then(TurnStatus::parse),
        turn_ended_at: conversation.turn_ended_at,
        latest_user_message_at: conversation.latest_user_message_at,
        agent: None,
    }
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
    let Some(desired_state) = conversation.sandbox_desired_state() else {
        return Value::Null;
    };
    json!({
        "threadId": conversation.task_id,
        "status": session_status(conversation, desired_state),
        // A *driver kind*, not the catalog's display label. The client decodes
        // this field as one (`ProviderSessionDirectory.ts:27`) and locks the
        // composer to whatever it names; a label happens to satisfy
        // `ProviderDriverKind`'s pattern — it is an open branded slug, not an
        // enum — so `"Codex"` decoded, matched no published instance, and left
        // the composer disabled telling the person to enable a provider that
        // was already enabled.
        //
        // The same string as `providerInstanceId` below, and that is correct
        // rather than redundant here: this backend publishes exactly one
        // instance per driver (`providers.rs`), so on this deployment an
        // instance id *is* a driver kind. A deployment with two Codex
        // instances would need them to diverge; this one does not have that.
        "providerName": provider_instance(conversation),
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

/// What the agent is doing, in T3's session vocabulary.
///
/// The rule itself is [`derive_running`], shared with the REST live-status
/// route so the two surfaces cannot disagree. This function is only the
/// translation: T3 has no separate word for a run parked on a person, and
/// spends `ready` on a Sandbox that is up with nothing in flight.
fn session_status(conversation: &Conversation, desired_state: SandboxDesiredState) -> &'static str {
    match derive_running(&running_inputs(conversation, desired_state)) {
        Working::Running | Working::Waiting => "running",
        Working::Starting => "starting",
        Working::Idle => "ready",
        Working::Stopped => "stopped",
        Working::Error => "error",
    }
}

/// The working copy's location, or null when there is none.
///
/// A removed Sandbox has had its working copy reclaimed, so it reports none —
/// and null is also what tells the client there are no files to ask for, since
/// it falls back to the project's root and gets an answer saying so.
fn worktree_path(conversation: &Conversation) -> Value {
    match conversation.sandbox_desired_state() {
        Some(SandboxDesiredState::Running | SandboxDesiredState::Stopped) => {
            json!(nominal_worktree(&conversation.task_id))
        }
        Some(SandboxDesiredState::Removed) | None => Value::Null,
    }
}

/// Every pull request a Task is bound to, as T3's `ThreadPullRequestLink`,
/// primary first — see [`Conversation::pull_requests`] for that order.
///
/// Empty where the Task has no reportable GitHub PR binding. The client reads
/// this whenever the descriptor carries `threadPullRequests`, and derives its
/// own current pull request from it; `linkedPullRequest` beside it is the legacy
/// single reference, and stays the first element of this list.
fn thread_pull_requests(conversation: &Conversation) -> Vec<Value> {
    let Some(bound) = conversation.pull_requests.as_deref() else {
        return Vec::new();
    };
    bound
        .iter()
        .filter_map(|pr| {
            let repository = non_empty(&pr.repository)?;
            let url = non_empty(&pr.url)?;
            if pr.number <= 0 {
                return None;
            }
            Some(json!({
                "host": pull_request_host(&url),
                "repository": repository,
                "number": pr.number,
                "url": url,
                "source": thread_pull_request_source(&pr.origin),
                "linkedAt": instant(pr.linked_at),
                "snapshot": pull_request_snapshot(pr, &repository),
                // Moatless reads no stack from GitHub, so a link never carries
                // one and the client chains by base and head branch instead.
                "stack": Value::Null
            }))
        })
        .collect()
}

/// The legacy single reference for the primary of [`thread_pull_requests`].
///
/// A T3 project is a Moatless Workspace, so the reference is placed against the
/// same `projectId` the row carries. `ThreadPullRequestKey` is host-level and
/// carries no project, which is why this is built here rather than sliced off
/// the link.
fn legacy_linked_pull_request(link: Option<&Value>, project_id: &str) -> Value {
    let Some(link) = link else {
        return Value::Null;
    };
    json!({
        "projectId": project_id,
        "repository": link["repository"],
        "number": link["number"],
        "url": link["url"]
    })
}

/// The git host a pull request URL names, or `github.com` where it names none.
///
/// T3 keys a link by host so the same pull request reached through two projects
/// compares equal. Every binding this projection reports comes from the GitHub
/// adapter, so the default is the host that adapter talks to.
fn pull_request_host(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split('/')
        .next()
        .and_then(non_empty)
        .unwrap_or_else(|| "github.com".to_string())
}

/// Who linked a pull request, as T3 names it.
///
/// Moatless records the direction a Task and a subject met, not who acted, so
/// only the CLI-filed binding maps onto one of T3's authors. Everything else
/// reports `manual` — the client shows it as "Linked by you", and uses the
/// value itself only to hide a `stack-dismissed` tombstone, which no Moatless
/// binding is.
fn thread_pull_request_source(origin: &str) -> &'static str {
    match origin {
        // `moatless_adapter_model::binding::TASK_OWNED_BINDING_ORIGIN`, spelled
        // out: that crate sits behind a band this one reaches through a seam.
        "task_owned" => "created",
        _ => "manual",
    }
}

/// The host state a fetched binding carries, or `Null` while its blob is still
/// NULL: the client paints a title and a state off a link that has one, and
/// leaves the badge pending for one that does not.
///
/// Gate and stand-ins are `bound_pull_request_from_binding`'s
/// (`crates/server/src/t3code/backend.rs`), which answers `pullRequests.summary`
/// off the same blob, so the two readers must not disagree about what it holds.
fn pull_request_snapshot(pr: &ConversationPullRequest, repository: &str) -> Value {
    // Only a refresh writes the branches, which is what makes them the gate.
    let Some(head_branch) = pr.head_ref.as_deref().and_then(non_empty) else {
        return Value::Null;
    };
    let Some(base_branch) = pr.base_ref.as_deref().and_then(non_empty) else {
        return Value::Null;
    };
    // Trimmed, not merely non-empty: the contract types `title` as a trimmed
    // non-empty string, and a blank one fails validation.
    let title = pr
        .title
        .as_deref()
        .and_then(non_empty)
        .unwrap_or_else(|| format!("{repository}#{}", pr.number));
    let state_kind = pr.state_kind.as_deref().unwrap_or("open");
    let mut snapshot = Map::from_iter([
        ("state".to_string(), json!(t3_pr_state(state_kind))),
        ("title".to_string(), json!(title)),
        ("headBranch".to_string(), json!(head_branch)),
        ("baseBranch".to_string(), json!(base_branch)),
        ("isDraft".to_string(), json!(is_draft(state_kind))),
        // When this deployment read the pull request, which is not when GitHub
        // last changed it: the status blob carries no such stamp, and reporting
        // the read as the change would have every surface claim the pull
        // request moved on every refresh.
        ("updatedAt".to_string(), Value::Null),
        ("syncedAt".to_string(), json!(instant(pr.status_read_at))),
    ]);
    if let Some(decision) = pr.review_decision.as_deref().and_then(t3_review_decision) {
        snapshot.insert("reviewDecision".to_string(), json!(decision));
    }
    if let Some(state) = pr.mergeable_state.as_deref() {
        snapshot.insert("mergeability".to_string(), json!(t3_mergeability(state)));
    }
    // No `checksState`: nothing records one, and the contract reads an absent
    // key as "this server does not say" rather than "the checks are pending".
    Value::Object(snapshot)
}

/// GitHub's review verdict as T3's `PullRequestReviewDecision`, or `None` for a
/// verdict T3 has no word for. `dismissed` and `commented` are the two: neither
/// approves nor blocks a merge, so both read as no decision at all.
fn t3_review_decision(decision: &str) -> Option<&'static str> {
    match decision {
        "approved" => Some("approved"),
        "changes_requested" => Some("changes-requested"),
        "review_required" => Some("review-required"),
        _ => None,
    }
}

/// GitHub's mergeable state as T3's `PullRequestMergeability`. Only `dirty`
/// means merge conflicts; `blocked`, `behind` and `unstable` describe a
/// mergeable branch waiting on something else, and T3 has no word for that.
fn t3_mergeability(state: &str) -> &'static str {
    match state {
        "dirty" => "conflicting",
        "clean" => "mergeable",
        _ => "unknown",
    }
}

/// A distinct, stable, nominal worktree path for one conversation.
pub(crate) fn nominal_worktree(task_id: &str) -> String {
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
    workspace_id_from_project_root(cwd).is_some()
}

/// The workspace a project-root `cwd` names, if it names one.
///
/// The inverse of [`nominal_root`], and the reason both live here: the client
/// only ever sends back a root path this module gave it, so listing a not-yet-
/// created thread's branches reads the workspace id straight back out of it.
pub(super) fn workspace_id_from_project_root(cwd: &str) -> Option<&str> {
    cwd.strip_prefix(NOMINAL_ROOT_PREFIX)?
        .strip_prefix('/')
        .filter(|id| !id.is_empty() && !id.contains('/'))
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
fn agent_kind(conversation: &Conversation) -> UiRpcAgentKind {
    catalog::find(conversation.agent_type.as_str())
        .map_or(UiRpcAgentKind::ClaudeCode, |entry| entry.kind)
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
        UiRpcAgentKind::ClaudeCode => "claudeAgent",
        UiRpcAgentKind::Codex => "codex",
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
/// read them. The widening is load-bearing — a conversation whose project is
/// absent is dropped by the client in silence — and what justifies it is the
/// *names*.
///
/// A start command is not a name: it carries paths, flags and whatever else is
/// in a server config, so the widened half is projected down to what the
/// contract needs to render a group and no more.
/// [`UiRpcListedWorkspace`](crate::workspace::UiRpcListedWorkspace) carries the
/// difference rather than having it re-derived.
///
/// That minimum still discloses the id, the name, the root synthesized *from*
/// the id, and the two timestamps. The timestamps stay because the contract
/// requires them and supplies no empty value for either.
fn project(listed: &UiRpcListedWorkspace) -> Value {
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
        // The Workspace's own defaults, which the client uses to seed a new
        // thread. Only the widened half is withheld: neither is a name, but
        // both are configuration the viewer may not read.
        "projectIcon": if listed.readable {
            project_icon(workspace.icon.as_ref())
        } else {
            Value::Null
        },
        "defaultModelSelection": if listed.readable {
            default_model_selection(workspace)
        } else {
            Value::Null
        },
        // Required by the contract, so the narrowed shape is an empty list
        // rather than an absent field.
        "scripts": if listed.readable { scripts(workspace) } else { Vec::new() },
        // Whether this project's scripts can be edited from the client at all.
        //
        // Reported rather than left for the write to refuse, because the answer
        // is a property of the Workspace and does not change with what somebody
        // types: a git-synced run config is reconciled from
        // `.moatless/workspaces.json`, so an edit here would be silently undone
        // by the next sync. The client hides the controls instead of offering
        // an action that can only fail. The widened half of the union is not
        // editable either — it is projected without its scripts — and neither
        // is a deleted one, which is listed only because open work still names
        // it.
        "scriptsEditable": listed.readable
            && !is_git_synced(workspace)
            && workspace.deleted != Some(true),
        "createdAt": timestamp(workspace.created_at),
        "updatedAt": timestamp(workspace.updated_at)
    })
}

/// The Workspace's default model, in the `ModelSelection` shape T3 sends back.
///
/// `Null` for a Workspace that pins no model, or that pins one this catalog no
/// longer publishes: a selection naming a slug absent from the provider list is
/// dropped by the client anyway, and substituting a model nobody chose is the
/// failure `providers::publish_model` exists to avoid. The effort rides along as
/// an option under the instance's own descriptor id, which is where
/// `commands::selected_effort` reads it back.
///
/// The effort gets the same treatment: `default_effort` is an open column no
/// write path validates, and a level the selected model does not publish is a
/// choice that cannot run, so one outside `providers::publishes_effort` is
/// dropped and the model runs at its own default.
fn default_model_selection(workspace: &Workspace) -> Value {
    let Some(catalog_model) = workspace.default_model.as_deref() else {
        return Value::Null;
    };
    let Some((instance, slug)) = providers::publish_model(catalog_model) else {
        return Value::Null;
    };
    let mut selection = json!({ "instanceId": instance, "model": slug });
    if let Some(effort) = workspace
        .default_effort
        .as_deref()
        .map(str::trim)
        .filter(|level| providers::publishes_effort(instance, catalog_model, level))
    {
        selection["options"] = json!([{
            "id": providers::effort_option_id(instance),
            "value": effort,
        }]);
    }
    selection
}

/// The colours a `ProjectIconOverride` may name, from `ProjectIconColor` in the
/// client's `packages/contracts/src/orchestration.ts`, which is the authority.
///
/// A colour added there and not copied here draws the project's automatic glyph
/// until it is, which is the failure worth having: see [`project_icon`].
const PROJECT_ICON_COLORS: &[&str] = &[
    "gray", "red", "orange", "amber", "yellow", "lime", "green", "emerald", "teal", "cyan", "sky",
    "blue", "indigo", "violet", "purple", "fuchsia", "pink", "rose",
];

/// The Workspace's icon as a `ProjectIconOverride`, or `Null` when the stored
/// value is not one.
///
/// `workspaces.icon` is open jsonb any `PATCH` may write, and the client decodes
/// a project strictly — an icon its schema rejects fails the whole
/// `OrchestrationProject`, which takes the project and its conversations off the
/// client. So the shape is checked here rather than trusted, against the same
/// bounds the client's schema states.
fn project_icon(icon: Option<&Value>) -> Value {
    let icon = icon.unwrap_or(&Value::Null);
    let field = |name: &str| {
        icon.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
    };
    match icon.get("kind").and_then(Value::as_str) {
        Some("lucide") => {
            let (name, color) = (field("name"), field("color"));
            if is_lucide_icon_name(name) && PROJECT_ICON_COLORS.contains(&color) {
                json!({ "kind": "lucide", "name": name, "color": color })
            } else {
                Value::Null
            }
        }
        Some("emoji") => {
            let emoji = field("emoji");
            // `isMaxLength` counts what JS counts, so the bound is UTF-16 units.
            if emoji.is_empty() || emoji.encode_utf16().count() > 32 {
                Value::Null
            } else {
                json!({ "kind": "emoji", "emoji": emoji })
            }
        }
        _ => Value::Null,
    }
}

/// The client's `ProjectLucideIconName`: `^[a-z0-9]+(?:-[a-z0-9]+)*$`, at most
/// 64 characters.
fn is_lucide_icon_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Whether git owns this Workspace's run config.
///
/// The same string the REST surface's `ensure_workspace_mutable` tests, and the
/// same rule: a git-synced Workspace is read-only everywhere, because the file
/// it is reconciled from is the thing to edit.
fn is_git_synced(workspace: &Workspace) -> bool {
    workspace.source == "git"
}

/// A distinct, stable, nominal root path for one workspace.
fn nominal_root(workspace_id: &str) -> String {
    format!("{NOMINAL_ROOT_PREFIX}/{workspace_id}")
}

/// The workspace's declared Scripts.
///
/// Only Scripts. The workspace's *servers* were mapped here once, which put a
/// runnable-looking entry in the client's top bar for something the Sandbox
/// already runs as a container — pressing it would have started a second
/// process on a bound port. Setup commands are provisioning and are likewise
/// not scripts.
fn scripts(workspace: &Workspace) -> Vec<Value> {
    let Some(scripts) = workspace.scripts.as_ref() else {
        return Vec::new();
    };
    let scripts: Vec<UiRpcScriptConfig> = match serde_json::from_value(scripts.clone()) {
        Ok(scripts) => scripts,
        Err(error) => {
            warn!(
                %error,
                workspace_id = %workspace.id,
                "Workspace scripts config is unreadable; reporting no scripts"
            );
            return Vec::new();
        }
    };

    scripts
        .iter()
        .filter_map(|script| {
            // Two of a script's fields are non-empty strings, and one script
            // the client cannot read takes the entire listing down with it. A
            // script that cannot fill them is left out rather than emitted
            // malformed.
            let id = non_empty(&script.name)?;
            let command = non_empty(&script.command)?;
            let name = script
                .label
                .as_deref()
                .and_then(non_empty)
                .unwrap_or_else(|| id.clone());
            let icon = script
                .icon
                .as_deref()
                .filter(|icon| SCRIPT_ICONS.contains(icon))
                .unwrap_or(DEFAULT_SCRIPT_ICON);
            Some(json!({
                // The script's name: stable across reads, and unique within the
                // workspace. It is the identity rather than a generated id
                // because it has to survive a git reconcile.
                "id": id,
                "name": name,
                "command": command,
                "icon": icon,
                // The client's name for `run_on_start`. Moatless has no
                // worktrees — a Sandbox is the isolation — so the backend runs
                // the flagged Script itself when the Sandbox reports Ready
                // (`startup_scripts`), and the client only labels it "(setup)".
                "runOnWorktreeCreate": script.run_on_start,
                // A script that serves a port is worth auto-opening a preview
                // for; one that prints to a console is not.
                "autoOpenPreview": script.port.is_some(),
                // The port itself, so the client can show and edit it. This is
                // the fork's stand-in for upstream's `previewUrl`, which a
                // Moatless script cannot carry: the published address only
                // exists once a Task has a sandbox serving that port, and
                // `scripts.run` returns the real one. `previewUrl` is therefore
                // omitted rather than guessed.
                "port": script.port
            }))
        })
        .collect()
}

/// The trimmed value, or `None` when nothing is left.
///
/// Most strings on this wire are trimmed-non-empty, and an empty one is not a
/// missing field — it is an undecodable listing.
/// The conversations a search matched, in the shape `searchThreads` returns.
///
/// A match with no Workspace is dropped rather than reported: the contract's
/// match carries a project id and the client routes on it, so one without a
/// project is a row nothing could open. Those are the same older and
/// hand-written rows the shell listing already leaves out for the same reason.
pub(super) fn thread_matches(matches: &[ConversationMatch], query: &str) -> Vec<Value> {
    matches
        .iter()
        .filter_map(|matched| {
            let project_id = matched.workspace_id.as_deref()?;
            Some(json!({
                "threadId": matched.task_id,
                "projectId": project_id,
                "source": if matched.from_user { "user" } else { "assistant" },
                "snippet": snippet(&matched.matched_text, query),
                "messageCreatedAt": nullable_instant(matched.matched_at),
            }))
        })
        .collect()
}

/// The most one search snippet carries, and how much of it is text.
///
/// The contract caps the field at 240 (`orchestration.ts:1412`); the body is
/// four short of it to leave room for the two ellipses a windowed snippet
/// carries.
const SNIPPET_MAX: usize = 240;
const SNIPPET_BODY: usize = 236;

/// Enough of the matched text to recognise the conversation by.
///
/// Whitespace is collapsed first, because a Message's text is wrapped across
/// lines and a snippet is one line. Anything that still does not fit is
/// windowed around the match rather than cut from the start — the match is the
/// reason this row is on screen, and a snippet that does not contain it is a
/// row a person cannot place. The reference server builds it the same way
/// (`ProjectionSnapshotQuery.ts:180`).
///
/// Counted in characters throughout: a byte cut can land inside one, and the
/// contract's cap is on the decoded string.
fn snippet(text: &str, query: &str) -> String {
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let characters: Vec<char> = normalized.chars().collect();
    if characters.len() <= SNIPPET_MAX {
        return normalized;
    }

    let folded = normalized.to_lowercase();
    // Byte offset of the match, then how many characters precede it — the two
    // are not the same number the moment anything is non-ASCII.
    let matched_at = folded
        .find(&query.trim().to_lowercase())
        .map_or(0, |offset| folded[..offset].chars().count());
    let start = matched_at
        .saturating_sub(72)
        .min(characters.len() - SNIPPET_BODY);
    let end = (start + SNIPPET_BODY).min(characters.len());

    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.extend(&characters[start..end]);
    if end < characters.len() {
        snippet.push('…');
    }
    snippet
}

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
    use crate::activity_dao::SubagentUsage;
    use moatless_types::access_control::Scope;
    use moatless_types::activity::{ActivityKind, ActivityStatus};
    use moatless_types::agent::AgentType;
    use moatless_types::message::MessageRole;
    use moatless_types::sandbox::SandboxStopReason;
    use sqlx::types::Json;

    fn matched(workspace_id: Option<&str>, from_user: bool, text: &str) -> ConversationMatch {
        ConversationMatch {
            task_id: "task-a".into(),
            workspace_id: workspace_id.map(ToString::to_string),
            from_user,
            matched_text: text.into(),
            matched_at: None,
        }
    }

    /// The client routes on `projectId`. A match without one is a row that
    /// cannot be opened, so it is left out rather than shown and then failing.
    #[test]
    fn a_match_on_a_conversation_with_no_workspace_is_not_reported() {
        let reported = thread_matches(
            &[
                matched(None, true, "outbox"),
                matched(Some("workspace-a"), true, "outbox"),
            ],
            "outbox",
        );

        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0]["projectId"], "workspace-a");
        assert_eq!(reported[0]["source"], "user");
        assert_eq!(reported[0]["messageCreatedAt"], Value::Null);
    }

    #[test]
    fn the_agents_own_words_are_reported_as_the_agents() {
        let reported = thread_matches(&[matched(Some("workspace-a"), false, "outbox")], "outbox");
        assert_eq!(reported[0]["source"], "assistant");
    }

    /// A snippet is one line; the text it comes from is not.
    #[test]
    fn a_short_match_keeps_its_text_with_the_line_breaks_collapsed() {
        assert_eq!(
            snippet("the outbox\n  is  full\n", "outbox"),
            "the outbox is full"
        );
    }

    /// Cutting from the start would drop the match, and the match is the whole
    /// reason the row is on screen.
    #[test]
    fn a_long_match_is_windowed_around_the_match_and_fits_the_contracts_cap() {
        let text = format!("{} NEEDLE {}", "a ".repeat(400), "b ".repeat(400));
        let snippet = snippet(&text, "needle");

        assert!(snippet.contains("NEEDLE"), "the match survives the window");
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
        assert!(
            snippet.chars().count() <= SNIPPET_MAX,
            "{} characters is over the contract's cap",
            snippet.chars().count()
        );
    }

    /// The window is measured in characters because the cap is on the decoded
    /// string, and a byte cut lands inside a character the moment anything is
    /// not ASCII.
    #[test]
    fn a_window_never_cuts_a_character_in_half() {
        let text = format!("{} nål {}", "ä ".repeat(400), "ö ".repeat(400));
        let snippet = snippet(&text, "nål");

        assert!(snippet.contains("nål"));
        assert!(snippet.chars().count() <= SNIPPET_MAX);
    }

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
    // `scriptsEditable` is a fork addition to `OrchestrationProjectShell`: the
    // client hides the script edit controls when it is false. Optional so an
    // older web build ignores it.
    const PROJECT_OPTIONAL: &[&str] = &["repositoryIdentity", "projectIcon", "scriptsEditable"];

    const SCRIPT_REQUIRED: &[&str] = &["id", "name", "command", "icon", "runOnWorktreeCreate"];
    // `port` is a fork addition to `ProjectScript`: it stands in for upstream's
    // `previewUrl`, which a Moatless script cannot carry (see the projection).
    const SCRIPT_OPTIONAL: &[&str] = &["previewUrl", "autoOpenPreview", "port"];

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
    const THREAD_OPTIONAL: &[&str] = &[
        "interactionMode",
        "archivedAt",
        "backgroundLiveness",
        "linkedPullRequest",
        "pullRequests",
        "visibility",
        "ownerName",
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
            closed_at: None,
            visibility: moatless_types::task::Visibility::Private,
            owner_name: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state_raw: None,
            sandbox_stop_reason_raw: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            running_commands: 0,
            latest_user_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
            pull_requests: None,
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
            sandbox_desired_state_raw: Some(SandboxDesiredState::Running.as_str().to_string()),
            sandbox_started_at: Some(now),
            sandbox_updated_at: Some(now),
            latest_user_message_at: Some(now),
            assistant_model: Some("claude-sonnet-4-6".to_string()),
            owner_name: Some("Ada Lovelace".to_string()),
            running_commands: 2,
            ..conversation(task_id, workspace_id)
        }
    }

    fn threads(workspaces: &[UiRpcListedWorkspace], conversations: &[Conversation]) -> Vec<Value> {
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

    /// `archivedAt` is the Task's close time. An open conversation carries
    /// null, which is what puts it in the sidebar rather than the archive; a
    /// closed one carries the time, which is what the archived listing renders
    /// and what T3 reads to tell the two apart.
    #[test]
    fn a_closed_conversation_is_archived_at_the_time_it_was_closed() {
        let open = one_thread(&running_conversation("task-a", "ws-a"));
        assert_eq!(open["archivedAt"], Value::Null);

        let closed_at = DateTime::parse_from_rfc3339("2026-07-04T09:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut row = running_conversation("task-a", "ws-a");
        row.closed_at = Some(closed_at);
        assert_eq!(
            one_thread(&row)["archivedAt"],
            json!("2026-07-04T09:00:00.000Z")
        );
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
            &OriginContext {
                frontend_url: "http://localhost:3000",
                source_task_names: &HashMap::new(),
            },
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
    fn shell_and_detail_rows_carry_the_task_visibility() {
        let conversation = Conversation {
            visibility: moatless_types::task::Visibility::Public,
            ..running_conversation("task-a", "ws-a")
        };
        let detail = thread_detail(
            &conversation,
            "ws-a",
            &AgentModelDefaults::default(),
            &transcript(Vec::new(), Vec::new()),
            &OriginContext {
                frontend_url: "http://localhost:3000",
                source_task_names: &HashMap::new(),
            },
        );
        assert_eq!(one_thread(&conversation)["visibility"], json!("public"));
        assert_eq!(detail["visibility"], json!("public"));
        assert_eq!(
            one_thread(&running_conversation("task-b", "ws-a"))["visibility"],
            json!("private")
        );
    }

    #[test]
    fn a_row_names_its_owner_only_when_the_listing_read_one() {
        // The listing leaves the name out for a Task the viewer owns, so a row
        // that carries one is a row that belongs to somebody else. Null and not
        // absent: the client reads the key to clear the previous answer.
        assert_eq!(
            one_thread(&conversation("task-a", "ws-a"))["ownerName"],
            Value::Null
        );
        assert_eq!(
            one_thread(&running_conversation("task-b", "ws-a"))["ownerName"],
            json!("Ada Lovelace")
        );
    }

    #[test]
    fn a_running_moat_cmd_command_makes_the_row_read_as_working() {
        let thread = one_thread(&Conversation {
            running_commands: 1,
            ..conversation("task-a", "ws-a")
        });

        assert_eq!(
            thread["backgroundLiveness"],
            Value::String("working".into())
        );
    }

    #[test]
    fn a_row_with_no_running_command_reports_no_background_liveness() {
        // Null rather than absent: the sidebar reads the key to decide the row
        // is resting, and a Task that just finished its last command has to say
        // so rather than leave the client on its previous answer.
        let thread = one_thread(&conversation("task-a", "ws-a"));

        assert_eq!(thread["backgroundLiveness"], Value::Null);
    }

    #[test]
    fn background_liveness_reports_the_axis_not_the_count() {
        // The contract has two literals and neither is a number: a second
        // command must not change what the row says.
        let one = one_thread(&Conversation {
            running_commands: 1,
            ..conversation("task-a", "ws-a")
        });
        let many = one_thread(&Conversation {
            running_commands: 7,
            ..conversation("task-a", "ws-a")
        });

        assert_eq!(one["backgroundLiveness"], many["backgroundLiveness"]);
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
    fn a_blank_conversation_name_falls_back_to_new_thread() {
        let mut row = conversation("task-a", "ws-a");
        row.name = Some("   ".to_string());
        assert_eq!(one_thread(&row)["title"], "New thread");
        row.name = None;
        assert_eq!(one_thread(&row)["title"], "New thread");
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

    /// A turn opens from inside the Sandbox, so between a person sending and the
    /// agent starting there is a window — seconds warm, minutes when scheduling
    /// is slow — with work in flight and no turn to show for it. The session
    /// reads as running across it, not as a thread with nothing happening.
    #[test]
    fn a_sandbox_coming_up_for_a_message_is_a_session_working() {
        let mut row = running_conversation("task-a", "ws-a");
        // Sent, and nothing has run yet: the Sandbox is still being provisioned.
        row.turn_number = None;
        row.turn_status = None;
        row.turn_started_at = None;
        row.sandbox_started_at = None;
        assert_eq!(one_thread(&row)["session"]["status"], "running");
        assert_eq!(
            one_thread(&row)["session"]["activeTurnId"],
            Value::Null,
            "no turn has opened yet, and the session says so"
        );

        // Answered: the newest turn ended after the message, so nothing is owed.
        row.turn_number = Some(3);
        row.turn_status = Some("completed".to_string());
        row.turn_ended_at = Some(Utc::now());
        row.sandbox_started_at = Some(Utc::now());
        assert_eq!(one_thread(&row)["session"]["status"], "ready");

        // Asked again, on a Sandbox that has to come up first.
        row.latest_user_message_at = Some(Utc::now());
        assert_eq!(one_thread(&row)["session"]["status"], "running");
    }

    /// A turn that finished without recording when says nothing either way, and
    /// is read as settled rather than leaving the session running for good.
    #[test]
    fn a_turn_that_ended_at_no_recorded_time_owes_nothing() {
        let mut row = running_conversation("task-a", "ws-a");
        row.turn_status = Some("completed".to_string());
        row.turn_ended_at = None;
        assert_eq!(one_thread(&row)["session"]["status"], "ready");
    }

    #[test]
    fn sandbox_intent_becomes_the_sessions_status() {
        let mut row = running_conversation("task-a", "ws-a");
        row.turn_status = Some("completed".to_string());

        row.sandbox_started_at = None;
        assert_eq!(one_thread(&row)["session"]["status"], "starting");

        row.sandbox_started_at = Some(Utc::now());
        row.sandbox_desired_state_raw = Some(SandboxDesiredState::Stopped.as_str().to_string());
        assert_eq!(one_thread(&row)["session"]["status"], "stopped");

        row.sandbox_stop_reason_raw = Some(SandboxStopReason::Error.as_str().to_string());
        assert_eq!(one_thread(&row)["session"]["status"], "error");

        row.sandbox_desired_state_raw = Some(SandboxDesiredState::Removed.as_str().to_string());
        assert_eq!(
            one_thread(&row)["worktreePath"],
            Value::Null,
            "a removed sandbox has had its working copy reclaimed"
        );
    }

    /// A binding whose status blob has never been fetched, which is how one is
    /// written. `status_read_at` still resolves, off `last_event_at`.
    fn bound(repository: &str, number: i64) -> ConversationPullRequest {
        ConversationPullRequest {
            repository: repository.to_string(),
            number,
            url: format!("https://github.com/{repository}/pull/{number}"),
            title: None,
            base_ref: None,
            head_ref: None,
            state_kind: None,
            status_read_at: instant_at("2026-02-03T04:05:06Z"),
            linked_at: instant_at("2026-02-03T04:00:00Z"),
            origin: "connected_thread".to_string(),
            review_decision: None,
            mergeable_state: None,
        }
    }

    /// The same binding after a refresh, which fills every status field at once.
    fn fetched(pr: ConversationPullRequest, state_kind: &str) -> ConversationPullRequest {
        ConversationPullRequest {
            title: Some(format!("Ship {}", pr.number)),
            base_ref: Some("main".to_string()),
            head_ref: Some(format!("feature/{}", pr.number)),
            state_kind: Some(state_kind.to_string()),
            ..pr
        }
    }

    /// The first link a row reports.
    fn first_link(row: &Conversation) -> Value {
        one_thread(row)["pullRequests"][0].clone()
    }

    /// The link is the Task's binding, so it holds whatever branch the checkout
    /// is on — including none at all, which is the ordinary case for a Task
    /// created without picking one.
    #[test]
    fn a_thread_row_links_its_bound_pull_request_regardless_of_the_branch() {
        let mut row = conversation("task-a", "ws-a");
        row.branch = None;
        row.pull_requests = Some(Json(vec![bound("acme/web", 42)]));

        let link = first_link(&row);
        assert_eq!(link["host"], "github.com");
        assert_eq!(link["repository"], "acme/web");
        assert_eq!(link["number"], 42);
        assert_eq!(link["url"], "https://github.com/acme/web/pull/42");
        assert_eq!(link["linkedAt"], "2026-02-03T04:00:00.000Z");
        assert_eq!(link["stack"], Value::Null);
    }

    /// A client from before `threadPullRequests` reads the single reference, so
    /// the row keeps answering it with the primary of the list.
    #[test]
    fn a_thread_row_also_links_the_primary_as_the_legacy_reference() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![bound("acme/web", 42)]));

        let legacy = &one_thread(&row)["linkedPullRequest"];
        assert_eq!(legacy["projectId"], "ws-a");
        assert_eq!(legacy["repository"], "acme/web");
        assert_eq!(legacy["number"], 42);
        assert_eq!(legacy["url"], "https://github.com/acme/web/pull/42");
    }

    /// The query orders the list, so the row takes its first element as the
    /// primary rather than choosing again on a shape that has lost the states
    /// the choice was made from.
    #[test]
    fn a_thread_row_links_every_bound_pull_request_with_the_primary_first() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![bound("acme/web", 7), bound("acme/web", 42)]));

        let thread = one_thread(&row);
        let links = thread["pullRequests"]
            .as_array()
            .expect("pullRequests is an array");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0]["number"], 7);
        assert_eq!(links[1]["number"], 42);
        assert_eq!(thread["linkedPullRequest"]["number"], 7);
    }

    /// Moatless records the direction a Task and a subject met, so the CLI's own
    /// binding is the only one T3 can call `created`.
    #[test]
    fn a_thread_row_reports_a_task_owned_binding_as_created_from_the_thread() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![
            ConversationPullRequest {
                origin: "task_owned".to_string(),
                ..bound("acme/web", 42)
            },
            bound("acme/web", 7),
        ]));

        let thread = one_thread(&row);
        assert_eq!(thread["pullRequests"][0]["source"], "created");
        assert_eq!(thread["pullRequests"][1]["source"], "manual");
    }

    /// What the rows of the composer's pull-request menu and the sidebar's
    /// status chip read, so neither has to ask `pullRequests.summary` for a
    /// binding this deployment has already fetched.
    #[test]
    fn a_thread_row_carries_the_snapshot_a_refreshed_binding_recorded() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![fetched(bound("acme/web", 42), "open")]));

        let snapshot = first_link(&row)["snapshot"].clone();
        assert_eq!(snapshot["title"], "Ship 42");
        assert_eq!(snapshot["state"], "open");
        assert_eq!(snapshot["isDraft"], false);
        assert_eq!(snapshot["baseBranch"], "main");
        assert_eq!(snapshot["headBranch"], "feature/42");
        assert_eq!(snapshot["syncedAt"], "2026-02-03T04:05:06.000Z");
    }

    /// `syncedAt` is when this deployment read GitHub. The blob records no
    /// instant at which GitHub itself changed the pull request, so `updatedAt`
    /// stays null rather than repeating the read.
    #[test]
    fn a_snapshot_reports_no_update_instant_of_its_own() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![fetched(bound("acme/web", 42), "open")]));

        assert_eq!(first_link(&row)["snapshot"]["updatedAt"], Value::Null);
    }

    /// GitHub's spellings reach the wire as T3's, and a state neither side
    /// names is reported as `unknown` rather than dropped.
    #[test]
    fn a_snapshot_translates_the_review_and_merge_verdicts() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![ConversationPullRequest {
            review_decision: Some("changes_requested".to_string()),
            mergeable_state: Some("dirty".to_string()),
            ..fetched(bound("acme/web", 42), "open")
        }]));

        let snapshot = first_link(&row)["snapshot"].clone();
        assert_eq!(snapshot["reviewDecision"], "changes-requested");
        assert_eq!(snapshot["mergeability"], "conflicting");
    }

    /// A draft reports the same `state` as an open pull request, because the
    /// contract has no fourth value, so `isDraft` is what colours it grey.
    #[test]
    fn a_thread_row_reports_a_draft_binding_as_an_open_draft() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![fetched(bound("acme/web", 42), "draft")]));

        let snapshot = first_link(&row)["snapshot"].clone();
        assert_eq!(snapshot["state"], "open");
        assert_eq!(snapshot["isDraft"], true);
    }

    /// A blob holding the branches but neither a title nor a state is one
    /// `persisted_status_is_complete` treats as fetched, so no refresh ever
    /// repairs it and the fallback would answer with these two stand-ins
    /// forever. The row carries them instead of buying them with a round trip.
    #[test]
    fn a_thread_row_stands_in_for_a_title_and_a_state_the_blob_never_recorded() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![ConversationPullRequest {
            title: None,
            state_kind: None,
            ..fetched(bound("acme/web", 42), "open")
        }]));

        let snapshot = first_link(&row)["snapshot"].clone();
        assert_eq!(snapshot["title"], "acme/web#42");
        assert_eq!(snapshot["state"], "open");
        assert_eq!(snapshot["isDraft"], false);
    }

    /// A binding is written with its blob NULL and filled in by a refresh no
    /// listing can trigger, so the link travels without a snapshot rather than
    /// with a state nobody read off GitHub. `pullRequests.summary` is what the
    /// client falls back to, and what refreshes it.
    #[test]
    fn a_thread_row_carries_no_snapshot_for_a_binding_never_fetched() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![bound("acme/web", 42)]));

        let link = first_link(&row);
        assert_eq!(link["number"], 42);
        assert_eq!(link["snapshot"], Value::Null);
    }

    #[test]
    fn a_thread_row_with_no_binding_links_no_pull_request() {
        let thread = one_thread(&conversation("task-a", "ws-a"));
        assert_eq!(thread["linkedPullRequest"], Value::Null);
        assert_eq!(thread["pullRequests"], json!([]));
    }

    /// A reference is written whole, so a half-filled one is a bug upstream of
    /// here rather than something to put on the wire.
    #[test]
    fn a_thread_row_drops_a_pull_request_it_cannot_name() {
        let mut row = conversation("task-a", "ws-a");
        row.pull_requests = Some(Json(vec![
            ConversationPullRequest {
                repository: String::new(),
                ..bound("acme/web", 42)
            },
            bound("acme/web", 7),
        ]));

        let thread = one_thread(&row);
        assert_eq!(thread["pullRequests"].as_array().map(Vec::len), Some(1));
        assert_eq!(thread["linkedPullRequest"]["number"], 7);
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
        ws.workspace.scripts = Some(json!([{
            "name": "dev",
            "command": "bun run dev",
            "icon": "play",
            "port": 3000
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
    fn a_script_that_cannot_fill_the_required_fields_is_left_out() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.scripts = Some(json!([
            { "name": "  ", "command": "bun run dev" },
            { "name": "blank-command", "command": "  " },
            { "name": "no-command" }
        ]));
        assert!(project(&ws)["scripts"].as_array().unwrap().is_empty());
    }

    /// A workspace this viewer may read. The widened half of the union — a
    /// workspace listed only because a conversation references it — is
    /// [`a_referenced_workspace`].
    fn workspace(id: &str, name: &str) -> UiRpcListedWorkspace {
        UiRpcListedWorkspace {
            readable: true,
            workspace: Workspace {
                id: id.to_string(),
                name: name.to_string(),
                description: None,
                icon: None,
                default_model: None,
                default_effort: None,
                kind: "blank".to_string(),
                docker_image: None,
                docker_access: None,
                sandbox_service_account: None,
                system_prompt: None,
                setup_commands: None,
                env_vars: None,
                servers: None,
                scripts: None,
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
    fn a_referenced_workspace(id: &str, name: &str) -> UiRpcListedWorkspace {
        UiRpcListedWorkspace {
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
    fn the_workspace_defaults_reach_the_client_as_an_icon_and_a_model_selection() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.icon = Some(json!({ "kind": "lucide", "name": "rocket", "color": "blue" }));
        ws.workspace.default_model = Some("opus".to_string());
        ws.workspace.default_effort = Some("max".to_string());

        let project = project(&ws);

        assert_eq!(project["projectIcon"]["name"], "rocket");
        assert_eq!(
            project["defaultModelSelection"],
            json!({
                "instanceId": "claudeAgent",
                "model": "claude-opus-5",
                "options": [{ "id": "effort", "value": "max" }],
            }),
            "the catalog id is published as T3's own slug, under the instance's effort descriptor"
        );
    }

    #[test]
    fn a_model_this_catalog_does_not_publish_selects_nothing() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.default_model = Some("retired-model".to_string());
        assert_eq!(project(&ws)["defaultModelSelection"], Value::Null);

        // The `default` sentinel names no model, so it is not a selection either.
        ws.workspace.default_model = Some(UNRESOLVED_MODEL_CHOICE.to_string());
        assert_eq!(project(&ws)["defaultModelSelection"], Value::Null);
    }

    #[test]
    fn the_widened_half_of_the_listing_discloses_neither_default() {
        let mut ws = a_referenced_workspace("ws-a", "Alpha");
        ws.workspace.icon = Some(json!({ "kind": "emoji", "emoji": "🚀" }));
        ws.workspace.default_model = Some("opus".to_string());

        let project = project(&ws);

        assert_eq!(project["projectIcon"], Value::Null);
        assert_eq!(project["defaultModelSelection"], Value::Null);
    }

    #[test]
    fn an_effort_the_selected_model_does_not_offer_selects_nothing() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.default_model = Some("gpt-5.6-luna".to_string());
        ws.workspace.default_effort = Some("ultra".to_string());

        // `ultra` is a level its sibling models take and this one rejects, and
        // Codex fails the turn on one it does not know.
        assert_eq!(
            project(&ws)["defaultModelSelection"]["options"],
            Value::Null
        );

        ws.workspace.default_model = Some("gpt-5.6-sol".to_string());
        assert_eq!(
            project(&ws)["defaultModelSelection"]["options"],
            json!([{ "id": "reasoningEffort", "value": "ultra" }])
        );

        // Whitespace is not a level either.
        ws.workspace.default_effort = Some("  ".to_string());
        assert_eq!(
            project(&ws)["defaultModelSelection"]["options"],
            Value::Null
        );
    }

    #[test]
    fn an_icon_the_clients_schema_rejects_is_not_published() {
        let mut ws = workspace("ws-a", "Alpha");
        for icon in [
            json!({ "kind": "lucide", "name": "rocket", "color": "puce" }),
            json!({ "kind": "lucide", "name": "Rocket Ship", "color": "blue" }),
            json!({ "kind": "lucide", "name": "rocket" }),
            json!({ "kind": "emoji", "emoji": "" }),
            json!({ "kind": "sticker", "sticker": "rocket" }),
            json!("rocket"),
        ] {
            ws.workspace.icon = Some(icon.clone());
            assert_eq!(
                project(&ws)["projectIcon"],
                Value::Null,
                "the client drops a project whose icon it cannot decode: {icon}"
            );
        }

        ws.workspace.icon = Some(json!({ "kind": "emoji", "emoji": " 🚀 " }));
        assert_eq!(
            project(&ws)["projectIcon"],
            json!({ "kind": "emoji", "emoji": "🚀" }),
            "the schema takes a trimmed string, so the stored one is trimmed"
        );
    }

    #[test]
    fn declared_scripts_become_scripts_and_servers_do_not() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.setup_commands = Some(json!(["bun install"]));
        // Servers must contribute no script at all: pressing one is a bug.
        ws.workspace.servers = Some(json!([
            { "name": "dev", "port": 3000, "startCommands": ["bun run dev"], "default": true }
        ]));
        ws.workspace.scripts = Some(json!([
            {
                "name": "test",
                "label": "Run tests",
                "command": "bun test",
                "icon": "test"
            },
            {
                "name": "storybook",
                "command": "bun run storybook",
                "port": 6006
            },
            { "name": "no-command" }
        ]));

        let scripts = project(&ws)["scripts"].as_array().unwrap().clone();

        assert_eq!(
            scripts.len(),
            2,
            "a script with no command is left out, and servers contribute none"
        );
        assert_eq!(scripts[0]["id"], "test");
        assert_eq!(scripts[0]["name"], "Run tests");
        assert_eq!(scripts[0]["command"], "bun test");
        assert_eq!(scripts[0]["icon"], "test");
        assert_eq!(
            scripts[0]["autoOpenPreview"], false,
            "a script with no port does not auto-open a preview"
        );
        assert_eq!(scripts[1]["id"], "storybook");
        assert_eq!(scripts[1]["name"], "storybook", "name falls back to the id");
        assert_eq!(
            scripts[1]["autoOpenPreview"], true,
            "a script with a port auto-opens its preview"
        );
        assert!(
            scripts[1].get("previewUrl").is_none(),
            "a bare port is not a URL, so the field is left out"
        );
    }

    #[test]
    fn an_unknown_icon_falls_back_rather_than_failing_the_listing() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.scripts = Some(json!([
            { "name": "weird", "command": "echo hi", "icon": "rocket" }
        ]));
        let scripts = project(&ws)["scripts"].as_array().unwrap().clone();
        assert_eq!(scripts[0]["icon"], "play");
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
        let scripts = json!([{
            "name": "dev",
            "command": "bun run dev --config /home/them/secret.toml"
        }]);
        let mut readable = workspace("ws-a", "Alpha");
        readable.workspace.scripts = Some(scripts.clone());
        let mut referenced = a_referenced_workspace("ws-a", "Alpha");
        referenced.workspace.scripts = Some(scripts);

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
    fn an_unreadable_scripts_config_reports_no_scripts() {
        let mut ws = workspace("ws-a", "Alpha");
        ws.workspace.scripts = Some(json!({ "not": "a list" }));
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
    const DETAIL_OPTIONAL: &[&str] = &[
        "interactionMode",
        "archivedAt",
        "proposedPlans",
        "linkedPullRequest",
        "pullRequests",
        "visibility",
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
    const MESSAGE_OPTIONAL: &[&str] = &["attachments", "origin"];
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
            external_subject_key: None,
            external_author: None,
            source_task_id: None,
            binding_url: None,
            binding_label: None,
            sent_by_name: None,
            file_contexts: None,
            external_event_id: None,
            external_adapter: None,
            external_source_name: None,
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
            "itemType": moatless_types::activity::item_type(name),
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

    /// The rows, plus the requests the database would report settled for them —
    /// which for rows that carry their own call is exactly what it reads off
    /// them. A test whose call sits outside the rows sets the field directly.
    fn transcript(messages: Vec<TranscriptMessage>, activities: Vec<Activity>) -> Transcript {
        let settled_requests = activities
            .iter()
            .filter(|activity| {
                activity.kind == ActivityKind::ToolCall && ActivityView::new(activity).is_settled()
            })
            .filter_map(|activity| activity.tool_use_id.clone())
            .collect();
        Transcript {
            messages,
            activities,
            omitted: 0,
            settled_requests,
            subagent_usage: HashMap::new(),
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
            &OriginContext {
                frontend_url: "http://localhost:3000",
                source_task_names: &HashMap::new(),
            },
        )
    }

    /// One `file` context row, as the join reads it.
    fn a_file_context(reference_id: &str, filename: &str, mime_type: &str, size: i64) -> Value {
        json!({
            "referenceId": reference_id,
            "data": { "filename": filename, "mimeType": mime_type, "size": size }
        })
    }

    fn with_files(mut message: TranscriptMessage, contexts: Vec<Value>) -> TranscriptMessage {
        message.file_contexts = Some(Value::Array(contexts));
        message
    }

    /// `ChatImageAttachment` field for field: the id the upload is fetched
    /// back by, and the three things the bubble shows beside it.
    #[test]
    fn an_uploaded_image_becomes_a_chat_attachment() {
        let message = with_files(
            message(1, MessageType::User, "look at this"),
            vec![a_file_context(
                "6b1f7b2e-0000-4000-8000-000000000001",
                "screenshot.png",
                "image/png",
                4096,
            )],
        );

        assert_eq!(
            message_attachments(&message),
            vec![json!({
                "type": "image",
                "id": "6b1f7b2e-0000-4000-8000-000000000001",
                "name": "screenshot.png",
                "mimeType": "image/png",
                "sizeBytes": 4096
            })]
        );
    }

    /// `ChatFileAttachment` field for field. An upload that is not an image is
    /// not dropped: it is the other member of the union, and the bubble shows
    /// it as a file the person can download.
    #[test]
    fn an_uploaded_file_becomes_a_file_attachment() {
        let message = with_files(
            message(1, MessageType::User, "read this"),
            vec![a_file_context(
                "6b1f7b2e-0000-4000-8000-000000000002",
                "report.pdf",
                "application/pdf",
                40 * 1024 * 1024,
            )],
        );

        assert_eq!(
            message_attachments(&message),
            vec![json!({
                "type": "file",
                "id": "6b1f7b2e-0000-4000-8000-000000000002",
                "name": "report.pdf",
                "mimeType": "application/pdf",
                "sizeBytes": 40 * 1024 * 1024
            })]
        );
    }

    /// The image member is four mime types, not a prefix. An SVG is a valid
    /// file attachment — the mint takes any type under that member — and
    /// sending it back as a `ChatImageAttachment` would hand the client an
    /// image it has no shape for. It decodes the snapshot in one piece, so that
    /// costs the reader the conversation rather than the attachment.
    ///
    /// The inline data-URL path accepts any `image/*` too, so this is not only
    /// about what the file member lets through.
    #[test]
    fn an_image_type_the_member_does_not_name_comes_back_as_a_file() {
        for mime in ["image/svg+xml", "image/avif", "image/tiff"] {
            let message = with_files(
                message(1, MessageType::User, "look"),
                vec![a_file_context("id-a", "drawing", mime, 1024)],
            );

            let attachments = message_attachments(&message);
            assert_eq!(attachments.len(), 1, "{mime}: got {attachments:?}");
            assert_eq!(attachments[0]["type"], "file", "{mime}");
            assert_eq!(attachments[0]["mimeType"], mime);
        }

        // The four the contract does name still come back as images.
        for mime in ["image/gif", "image/jpeg", "image/png", "image/webp"] {
            let message = with_files(
                message(1, MessageType::User, "look"),
                vec![a_file_context("id-a", "shot", mime, 1024)],
            );
            assert_eq!(message_attachments(&message)[0]["type"], "image", "{mime}");
        }
    }

    /// An image type is not by itself the image member. The mint lets a `file`
    /// carry any type up to 50 MiB, so a large PNG sent as a file is something
    /// the contract allows — and reading the member off the mime type alone
    /// dropped it, after it had been stored and read by the agent. It comes back
    /// out as the member it fits.
    #[test]
    fn a_large_image_type_comes_back_as_the_file_member() {
        let message = with_files(
            message(1, MessageType::User, "look at this"),
            vec![a_file_context(
                "id-b",
                "huge.png",
                "image/png",
                ATTACHMENT_MAX_IMAGE_BYTES + 1,
            )],
        );

        let attachments = message_attachments(&message);
        assert_eq!(attachments.len(), 1, "got {attachments:?}");
        assert_eq!(attachments[0]["type"], "file");
        assert_eq!(attachments[0]["mimeType"], "image/png");
        assert_eq!(attachments[0]["sizeBytes"], ATTACHMENT_MAX_IMAGE_BYTES + 1);
    }

    /// The boundary between the two members, for one mime type that could be
    /// either. At the image ceiling it is still an image; one byte past it is a
    /// file rather than nothing.
    #[test]
    fn an_image_becomes_a_file_at_the_byte_it_outgrows_the_image_member() {
        let projected = |size| {
            let message = with_files(
                message(1, MessageType::User, "look"),
                vec![a_file_context("id-b", "shot.png", "image/png", size)],
            );
            message_attachments(&message)
                .first()
                .map(|attachment| attachment["type"].as_str().unwrap_or_default().to_string())
        };

        assert_eq!(
            projected(ATTACHMENT_MAX_IMAGE_BYTES).as_deref(),
            Some("image")
        );
        assert_eq!(
            projected(ATTACHMENT_MAX_IMAGE_BYTES + 1).as_deref(),
            Some("file")
        );
        assert_eq!(
            projected(ATTACHMENT_MAX_FILE_BYTES).as_deref(),
            Some("file")
        );
        // Past both members, so there is no shape left to send it in.
        assert_eq!(projected(ATTACHMENT_MAX_FILE_BYTES + 1), None);
    }

    /// The client decodes the whole snapshot against its schema, so an upload
    /// it would refuse costs the reader the conversation rather than the
    /// attachment. Each of these is therefore left out, and the ones beside it
    /// are not — an upload past both members has no shape left to send it in.
    #[test]
    fn an_upload_the_contract_has_no_shape_for_is_left_out() {
        let message = with_files(
            message(1, MessageType::User, "look at this"),
            vec![
                // Past `PROVIDER_SEND_TURN_MAX_FILE_BYTES`.
                a_file_context(
                    "id-f",
                    "huge.zip",
                    "application/zip",
                    ATTACHMENT_MAX_FILE_BYTES + 1,
                ),
                // `ChatFileAttachment` requires at least one byte; the image
                // member does not, which is why this is the file member's test.
                a_file_context("id-g", "empty.pdf", "application/pdf", 0),
                // Outside `ChatAttachmentId`'s alphabet.
                a_file_context("id c/../d", "ok.png", "image/png", 10),
                // `TrimmedNonEmptyString`.
                a_file_context("id-d", "   ", "image/png", 10),
                a_file_context("id-e", "kept.png", "image/png", 10),
            ],
        );

        let attachments = message_attachments(&message);
        assert_eq!(attachments.len(), 1, "got {attachments:?}");
        assert_eq!(attachments[0]["name"], "kept.png");
    }

    /// An image of no bytes is still a `ChatImageAttachment`: only the file
    /// member sets a minimum, and reading one member's rule onto the other
    /// would drop something the client would have rendered.
    #[test]
    fn an_empty_image_is_still_an_image_attachment() {
        let message = with_files(
            message(1, MessageType::User, "look at this"),
            vec![a_file_context("id-a", "blank.png", "image/png", 0)],
        );

        assert_eq!(message_attachments(&message).len(), 1);
    }

    /// A name past the contract's limit is cut rather than dropped: the person
    /// still sees their image, under a name that is shorter than they gave it.
    #[test]
    fn an_overlong_name_is_cut_to_what_the_contract_takes() {
        let long = format!("{}.png", "ä".repeat(400));
        let message = with_files(
            message(1, MessageType::User, "look at this"),
            vec![a_file_context("id-a", &long, "image/png", 10)],
        );

        let attachments = message_attachments(&message);
        let name = attachments[0]["name"].as_str().unwrap();
        assert!(
            name.len() <= ATTACHMENT_NAME_MAX_CHARS,
            "{} bytes",
            name.len()
        );
        assert!(long.starts_with(name));
    }

    /// A Message with no uploads is byte-identical to one projected before
    /// this surface read them — `attachments` is an optional key, and an empty
    /// array is not the same as its absence.
    #[test]
    fn a_message_without_uploads_carries_no_attachments_key() {
        let thread = detail(&transcript(
            vec![message(1, MessageType::User, "no files here")],
            Vec::new(),
        ));
        let bubble = &thread["messages"][0];

        assert_eq!(bubble["role"], "user");
        assert!(bubble.get("attachments").is_none());
    }

    /// Only the person attaches things. An assistant row carrying uploads
    /// would be the agent claiming the person sent them.
    #[test]
    fn only_a_user_bubble_carries_attachments() {
        let thread = detail(&transcript(
            vec![
                with_files(
                    message(1, MessageType::User, "look at this"),
                    vec![a_file_context("id-a", "a.png", "image/png", 10)],
                ),
                with_files(
                    message(2, MessageType::Assistant, "I see it"),
                    vec![a_file_context("id-b", "b.png", "image/png", 10)],
                ),
            ],
            Vec::new(),
        ));
        let messages = thread["messages"].as_array().unwrap();

        assert_eq!(messages[0]["attachments"].as_array().unwrap().len(), 1);
        assert!(messages[1].get("attachments").is_none());
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
                json!("Command run"),
                json!("File change"),
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
        assert!(ran["payload"]["data"].get("command").is_none());
        assert_eq!(ran["payload"]["detail"], "Bash: cargo build");
        assert_eq!(ran["payload"]["data"]["toolCallId"], "call-1");

        let failed = &work[2];
        assert_eq!(failed["tone"], "error");
        assert_eq!(failed["payload"]["status"], "failed");
        assert_eq!(failed["payload"]["itemType"], "file_change");
        assert!(failed["payload"]["data"].get("files").is_none());
        assert_eq!(
            failed["payload"]["detail"],
            r#"Edit: {"file_path":"src/main.rs"}"#
        );
    }

    /// A subagent reaches the client as one task it watches run.
    ///
    /// The subagent's own work is not in the Task's stream, so this row is the
    /// whole of what is seen of it: it is reported as a running task while the
    /// call is open, and the completion lands on the same row — same id, so the
    /// client replaces it rather than adding a second row.
    #[test]
    fn a_subagent_reaches_the_client_as_one_task_it_watches_run() {
        let mut conversation = a_conversation();
        let mut spawn = call(
            "msg-2",
            2,
            3,
            "call-3",
            "Task",
            json!({ "subagent_type": "code-reviewer", "description": "Audit the SQL" }),
        );
        spawn.status = ActivityStatus::InProgress;
        spawn.payload["status"] = json!("inProgress");
        conversation.activities.push(spawn);

        let projected = detail(&conversation);
        let running = projected["activities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["payload"]["data"]["toolCallId"] == "call-3")
            .expect("the spawning call is a row of the thread");
        assert_eq!(running["kind"], "task.progress");
        assert_eq!(running["tone"], "info");
        assert_eq!(running["payload"]["itemType"], "collab_agent_tool_call");
        // Headed by the word for what it is, with the brief beside it.
        assert_eq!(running["payload"]["summary"], "Subagent task");
        assert_eq!(running["payload"]["detail"], "code-reviewer: Audit the SQL");

        // The call comes back. The row is the same row.
        let running_id = running["id"].clone();
        let spawn = conversation
            .activities
            .iter_mut()
            .find(|activity| activity.tool_use_id.as_deref() == Some("call-3"))
            .unwrap();
        spawn.status = ActivityStatus::Completed;
        spawn.payload["status"] = json!("completed");

        let settled = detail(&conversation);
        let completed = settled["activities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["payload"]["data"]["toolCallId"] == "call-3")
            .expect("the settled call is still a row of the thread");
        assert_eq!(completed["kind"], "task.completed");
        assert_eq!(
            completed["id"], running_id,
            "the completion lands on the row the progress was reported on"
        );
    }

    /// What a subagent has spent is not on the row that names it: the numbers
    /// are summed across Messages the transcript excludes and reach the row on
    /// the transcript that read them, keyed by the call's own id.
    #[test]
    fn a_subagent_row_carries_what_its_agent_has_spent() {
        let mut conversation = a_conversation();
        let mut spawn = call(
            "msg-2",
            2,
            3,
            "call-3",
            "Task",
            json!({ "subagent_type": "code-reviewer", "description": "Audit the SQL" }),
        );
        spawn.status = ActivityStatus::InProgress;
        spawn.payload["status"] = json!("inProgress");
        conversation.activities.push(spawn);
        conversation.subagent_usage.insert(
            "call-3".to_string(),
            SubagentUsage {
                parent_uuid: "call-3".to_string(),
                input_tokens: 900,
                output_tokens: 100,
            },
        );

        let projected = detail(&conversation);
        let rows = projected["activities"].as_array().unwrap().clone();
        let spent = rows
            .iter()
            .find(|row| row["payload"]["taskId"] == "call-3")
            .expect("the spawning call is a row of the thread");
        assert_eq!(spent["payload"]["typedUsage"]["totalTokens"], 1000);
        assert_eq!(spent["payload"]["typedUsage"]["inputTokens"], 900);
        assert_eq!(spent["payload"]["typedUsage"]["outputTokens"], 100);

        // A row that names no subagent is left as it was.
        let ordinary = rows
            .iter()
            .find(|row| row["payload"]["data"]["toolCallId"] == "call-1")
            .expect("the conversation has an ordinary tool call");
        assert!(ordinary["payload"].get("typedUsage").is_none());
    }

    #[test]
    fn a_codex_shell_call_reaches_the_client_as_t3s_ran_command_row() {
        let mut conversation = a_conversation();
        let call = &mut conversation.activities[1];
        call.summary = "codex.shell".to_string();
        call.payload["title"] = json!("codex.shell");

        let ran = &detail(&conversation)["activities"][1];
        assert_eq!(ran["summary"], "Ran command");
        assert_eq!(ran["payload"]["itemType"], "command_execution");
        assert_eq!(ran["payload"]["detail"], "cargo build");
        assert_eq!(ran["payload"]["data"]["command"], "cargo build");
        assert!(ran["payload"].get("title").is_none());
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
        assert_eq!(work[1]["payload"]["detail"], "Bash: cargo build");
    }

    /// T3 presents what the tool was asked to do. A stored result preview is
    /// Moatless UI data and must not replace that provider-derived request.
    #[test]
    fn a_stored_result_preview_does_not_replace_the_tool_request() {
        let mut conversation = a_conversation();
        conversation.activities[1].payload["result"] = json!("x".repeat(10_000));
        conversation.activities[1].detail = Some("xxx\n… truncated".to_string());
        assert_eq!(
            detail(&conversation)["activities"][1]["payload"]["detail"],
            "Bash: cargo build"
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

    /// A message typed into the composer carries no origin key at all —
    /// byte-identical to a conversation before this field existed.
    #[test]
    fn a_message_typed_into_the_composer_carries_no_origin() {
        let origin = message_origin(
            &message(1, MessageType::User, "Hi"),
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        );
        assert!(origin.is_none());
    }

    /// A Message authored by another Task names and links that Task.
    #[test]
    fn a_message_from_another_task_names_and_links_that_task() {
        let mut msg = message(1, MessageType::User, "Handoff");
        msg.source_task_id = Some("task-xyz".into());
        let names = HashMap::from([("task-xyz".to_string(), "Upstream planning".to_string())]);
        let origin = message_origin(
            &msg,
            // Trailing slash on the base is trimmed, not doubled.
            &OriginContext {
                frontend_url: "https://moatless.example/",
                source_task_names: &names,
            },
        )
        .expect("a task-authored message has an origin");
        assert_eq!(origin["kind"], "task");
        assert_eq!(origin["label"], "Upstream planning");
        assert_eq!(origin["url"], "https://moatless.example/tasks/task-xyz");
        assert_eq!(origin["user"], Value::Null);
    }

    /// A Task the reader cannot see resolves to no name, and a link they could
    /// not follow is worse than none.
    #[test]
    fn a_task_origin_the_reader_cannot_see_has_neither_name_nor_link() {
        let mut msg = message(1, MessageType::User, "Handoff");
        msg.source_task_id = Some("task-hidden".into());
        let origin = message_origin(
            &msg,
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        )
        .expect("origin present");
        assert_eq!(origin["kind"], "task");
        assert_eq!(origin["label"], Value::Null);
        assert_eq!(origin["url"], Value::Null);
    }

    /// A task origin wins even where the row also carries adapter columns.
    #[test]
    fn a_task_origin_wins_over_adapter_columns() {
        let mut msg = message(1, MessageType::User, "Handoff");
        msg.source_task_id = Some("task-xyz".into());
        msg.external_adapter = Some("github_pr".into());
        msg.external_subject_key = Some("octo/repo#42".into());
        let names = HashMap::from([("task-xyz".to_string(), "Upstream".to_string())]);
        let origin = message_origin(
            &msg,
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &names,
            },
        )
        .expect("origin present");
        assert_eq!(origin["kind"], "task");
    }

    /// A GitHub PR message derives its URL from the subject key and event id and
    /// shows the handle the external system knows the writer by.
    #[test]
    fn a_github_pr_message_derives_its_url_and_shows_the_external_author() {
        let mut msg = message(1, MessageType::User, "Please rebase");
        msg.external_adapter = Some("github_pr".into());
        msg.external_subject_key = Some("octo/repo#42".into());
        msg.external_event_id = Some("github:ic:99887766".into());
        msg.external_source_name = Some("octo/repo".into());
        msg.external_author = Some("octocat".into());
        let origin = message_origin(
            &msg,
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        )
        .expect("origin present");
        assert_eq!(origin["kind"], "github_pr");
        assert_eq!(origin["label"], "octo/repo#42");
        assert_eq!(
            origin["url"],
            "https://github.com/octo/repo/pull/42#issuecomment-99887766"
        );
        assert_eq!(origin["user"], "octocat");
    }

    /// Any other adapter takes its label and URL from the Binding this Task
    /// holds, and falls back to the Moatless user when no external handle was
    /// captured.
    #[test]
    fn an_external_message_without_a_captured_handle_falls_back_to_the_moatless_user() {
        let mut msg = message(1, MessageType::User, "Filed from Linear");
        msg.external_adapter = Some("linear".into());
        // What a Linear subscription matches on is its team id; the issue's own
        // identifier is on the Binding.
        msg.external_source_name = Some("f1f0b3a2-team-uuid".into());
        msg.binding_label = Some("SP-2454".into());
        msg.binding_url = Some("https://linear.app/soap-labs/issue/SP-2454".into());
        msg.sent_by_name = Some("Ada Lovelace".into());
        let origin = message_origin(
            &msg,
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        )
        .expect("origin present");
        assert_eq!(origin["kind"], "linear");
        assert_eq!(origin["label"], "SP-2454");
        assert_eq!(origin["url"], "https://linear.app/soap-labs/issue/SP-2454");
        assert_eq!(origin["user"], "Ada Lovelace");
    }

    /// An adapter whose Binding names no subject keeps whatever matched the
    /// subscription rather than going unlabelled.
    #[test]
    fn an_external_message_whose_binding_names_nothing_falls_back_to_the_source_name() {
        let mut msg = message(1, MessageType::User, "From Slack");
        msg.external_adapter = Some("slack".into());
        msg.external_source_name = Some("#eng".into());
        let origin = message_origin(
            &msg,
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        )
        .expect("origin present");
        assert_eq!(origin["label"], "#eng");
        // Slack keeps channel coordinates on its Binding, not a link.
        assert_eq!(origin["url"], Value::Null);
    }

    /// The GitHub URL, and the fragment each kind of comment opens at.
    #[test]
    fn a_github_pr_url_is_built_from_the_subject_key_and_event_id() {
        assert_eq!(
            github_pr_url(Some("octo/repo#42"), Some("github:rc:12345")),
            Some("https://github.com/octo/repo/pull/42#discussion_r12345".to_string())
        );
        assert_eq!(
            github_pr_url(Some("octo/repo#42"), Some("github:ic:678")),
            Some("https://github.com/octo/repo/pull/42#issuecomment-678".to_string())
        );
        // An unrecognised event id — or none — leaves the reader at the PR.
        assert_eq!(
            github_pr_url(Some("octo/repo#42"), None),
            Some("https://github.com/octo/repo/pull/42".to_string())
        );
        assert_eq!(
            github_pr_url(Some("octo/repo#42"), Some("something-else")),
            Some("https://github.com/octo/repo/pull/42".to_string())
        );
    }

    /// A subject key that is not `owner/repo#N` yields no URL rather than a
    /// wrong one.
    #[test]
    fn a_subject_key_that_is_not_owner_repo_number_yields_no_url() {
        assert_eq!(github_pr_url(Some("not-a-key"), None), None);
        assert_eq!(github_pr_url(Some("owner/repo#"), None), None);
        assert_eq!(github_pr_url(Some("owner/repo#abc"), None), None);
        assert_eq!(github_pr_url(Some("owner#42"), None), None);
        assert_eq!(github_pr_url(Some("a/b/c#42"), None), None);
        assert_eq!(github_pr_url(None, None), None);
    }

    /// The `message-sent` event carries the same origin as the snapshot bubble,
    /// so a delta and a rebuild render one message the same way.
    #[test]
    fn the_message_sent_event_carries_the_same_origin_as_the_snapshot_bubble() {
        let mut msg = message(1, MessageType::User, "From Slack");
        msg.external_adapter = Some("slack".into());
        msg.external_source_name = Some("#eng".into());
        msg.binding_url = Some("https://acme.slack.com/archives/C1".into());
        msg.external_author = Some("ada".into());
        let bubble = message_bubble(
            &msg,
            &Value::Null,
            "2026-07-01T10:00:00.000Z",
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        )
        .expect("a user bubble");
        assert!(bubble.get("origin").is_some());
        let payload = message_sent_payload("task-a", &bubble);
        assert_eq!(payload["origin"], bubble["origin"]);
    }

    /// A composer bubble has no origin, so neither does its event.
    #[test]
    fn a_plain_message_sent_event_carries_no_origin() {
        let bubble = message_bubble(
            &message(1, MessageType::User, "Hi"),
            &Value::Null,
            "2026-07-01T10:00:00.000Z",
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        )
        .expect("a user bubble");
        assert!(bubble.get("origin").is_none());
        let payload = message_sent_payload("task-a", &bubble);
        assert!(payload.get("origin").is_none());
    }

    /// Only a person's bubble can carry an origin; the agent's own reply never
    /// does, even on a row that happens to hold adapter columns.
    #[test]
    fn an_assistant_bubble_never_carries_an_origin() {
        let mut msg = message(1, MessageType::Assistant, "On it");
        msg.external_adapter = Some("slack".into());
        let bubble = message_bubble(
            &msg,
            &Value::Null,
            "2026-07-01T10:00:00.000Z",
            &OriginContext {
                frontend_url: "https://moatless.example",
                source_task_names: &HashMap::new(),
            },
        )
        .expect("an assistant bubble");
        assert!(bubble.get("origin").is_none());
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
    /// `ExitPlanMode` call, and an unanswered one is a call still in progress.
    #[test]
    fn an_unanswered_plan_is_a_proposed_plan_carrying_no_answer() {
        let mut proposed = call(
            "msg-2",
            2,
            0,
            "call-1",
            moatless_types::activity::PLAN_TOOL,
            json!({ "plan": "1. Do the thing" }),
        );
        proposed.status = ActivityStatus::InProgress;

        let plans = detail(&transcript(Vec::new(), vec![proposed]))["proposedPlans"]
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
    }

    /// An answered plan keeps its card, and reports when the answer landed.
    #[test]
    fn an_answered_plan_is_still_a_proposed_plan_and_reports_when_it_was_answered() {
        let mut answered = call(
            "msg-2",
            2,
            0,
            "call-1",
            moatless_types::activity::PLAN_TOOL,
            json!({ "plan": "1. Do the thing" }),
        );
        answered.payload["result"] = json!({ "approved": true });
        let mut resolved = activity("msg-3", 3, 0, ActivityKind::UserInputResolved, "Answered");
        resolved.payload = json!({ "requestId": "call-1", "answers": [{ "answer": "Approve" }] });

        let plans = detail(&transcript(Vec::new(), vec![answered, resolved]))["proposedPlans"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0]["id"], "msg-2:0");
        assert_eq!(plans[0]["planMarkdown"], "1. Do the thing");
        assert_eq!(plans[0]["implementedAt"], instant(at(103)));
        assert_eq!(plans[0]["updatedAt"], instant(at(103)));
    }

    /// The window holds a fixed span, so it can reach the call without the row
    /// that answered it. A settled call reports an answer time either way.
    #[test]
    fn an_answered_plan_whose_answer_is_out_of_window_falls_back_to_its_own_instant() {
        let mut answered = call(
            "msg-2",
            2,
            0,
            "call-1",
            moatless_types::activity::PLAN_TOOL,
            json!({ "plan": "1. Do the thing" }),
        );
        answered.payload["result"] = json!({ "approved": true });

        let plans = detail(&transcript(Vec::new(), vec![answered]))["proposedPlans"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0]["implementedAt"], instant(at(102)));
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
                moatless_types::activity::PLAN_TOOL,
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

        let work =
            detail(&transcript(Vec::new(), vec![asked, plan, meter, resolved]))["activities"]
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

    /// A question the run has stopped waiting on reaches the client resolved,
    /// with no resolution row of its own.
    ///
    /// The client keeps a panel open until it sees a row naming the same request
    /// resolved, and refuses to answer a call that has come back — so a panel
    /// left `requested` here is one nobody can get rid of. The call's own status
    /// is the fact, and it is settled whoever settled it: this covers a question
    /// answered from another surface and one a resuming session dropped, neither
    /// of which writes a resolution.
    #[test]
    fn a_question_whose_call_came_back_is_projected_resolved() {
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

        let mut call = activity("msg-2", 2, 1, ActivityKind::ToolCall, "AskUserQuestion");
        call.tool_use_id = Some("call-1".to_string());
        call.status = ActivityStatus::InProgress;

        let kind_of = |rows: Vec<Activity>| {
            detail(&transcript(Vec::new(), rows))["activities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["payload"]["requestId"] == "call-1")
                .expect("the panel row")["kind"]
                .clone()
        };

        // Still waiting on a person: the panel opens.
        assert_eq!(
            kind_of(vec![asked.clone(), call.clone()]),
            "user-input.requested"
        );

        // The call came back. The panel row is the same row, and says so.
        call.status = ActivityStatus::Completed;
        assert_eq!(kind_of(vec![asked, call]), "user-input.resolved");
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
        assert_eq!(work[1]["kind"], "runtime.error");
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
            activity_id: None,
        }
    }

    /// The change that names activity `activity_id`, at `position`.
    fn activity_change(position: i64, activity_id: &str) -> ConversationChange {
        ConversationChange {
            position,
            message_uuid: None,
            activity_id: Some(activity_id.to_string()),
        }
    }

    fn events(change: &ConversationChange, window: &Transcript) -> Vec<Value> {
        change_events(
            &running_conversation("task-a", "ws-a"),
            change,
            window,
            &OriginContext {
                frontend_url: "http://localhost:3000",
                source_task_names: &HashMap::new(),
            },
        )
        .into_iter()
        .map(|item| {
            assert_eq!(item["kind"], "event", "every item is a stream event");
            item["event"].clone()
        })
        .collect()
    }

    /// A call and the panel row derived from it are two rows in one change, and
    /// each needs a sequence of its own.
    ///
    /// The feed delivers an item only when its sequence is past the last it
    /// delivered, so two rows sharing one costs the second row entirely — and
    /// the panel row is always the second, because it sorts after the call it is
    /// derived from. That is the panel's every appearance and every closure.
    #[test]
    fn a_panel_row_and_its_call_do_not_share_one_sequence() {
        let mut call = activity("msg-2", 2, 0, ActivityKind::ToolCall, "AskUserQuestion");
        call.tool_use_id = Some("call-1".to_string());
        call.status = ActivityStatus::Completed;

        let mut panel = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
        panel.activity_id = "msg-2:0:user-input".to_string();
        panel.payload = json!({
            "requestId": "call-1",
            "questions": [{
                "id": "Which store?",
                "header": "Storage",
                "question": "Which store?",
                "multiSelect": false,
                "options": [{ "label": "Postgres", "description": "Already deployed." }]
            }]
        });

        let window = transcript(Vec::new(), vec![call, panel]);
        let emitted = events(&change(2, Some("msg-2")), &window);

        let sequences: Vec<i64> = emitted
            .iter()
            .map(|event| event["sequence"].as_i64().expect("a sequence"))
            .collect();
        let unique: HashSet<i64> = sequences.iter().copied().collect();
        assert_eq!(
            unique.len(),
            sequences.len(),
            "each row needs its own sequence, got {sequences:?}"
        );
    }

    /// A panel row reaches the client resolved even when the call it names is
    /// not among the rows being projected.
    ///
    /// A resuming session re-presents the call under a new Message. The call
    /// itself is stored once — `(task_id, tool_use_id)` is unique, so the second
    /// arrival is dropped — but the panel row beside it carries no
    /// `tool_use_id`, so it is written again, on the new Message. From then on
    /// the two live on different Messages, and every change naming the panel's
    /// one carries the panel without the call.
    #[test]
    fn a_panel_row_is_resolved_by_a_call_outside_the_rows_being_read() {
        let mut panel = activity("msg-9", 9, 0, ActivityKind::UserInputRequested, "Question");
        panel.activity_id = "msg-9:0:user-input".to_string();
        panel.payload = json!({
            "requestId": "call-1",
            "questions": [{
                "id": "Which store?",
                "header": "Storage",
                "question": "Which store?",
                "multiSelect": false,
                "options": [{ "label": "Postgres", "description": "Already deployed." }]
            }]
        });

        // The call came back long ago, on the Message it was first presented
        // on, which this change does not name.
        let mut window = transcript(Vec::new(), vec![panel]);
        window.settled_requests = HashSet::from(["call-1".to_string()]);
        let emitted = events(&change(9, Some("msg-9")), &window);

        let panel_event = emitted
            .iter()
            .find(|event| event["payload"]["activity"]["payload"]["requestId"] == "call-1")
            .expect("the panel row");
        assert_eq!(
            panel_event["payload"]["activity"]["kind"], "user-input.resolved",
            "a settled call resolves its panel wherever the call is stored"
        );
    }

    /// A change's items climb, so a reader that delivers each sequence once
    /// keeps all of them.
    ///
    /// A derived row's slot sits above the blocks, so in row order it lands
    /// between two of them — and a reader comparing against the last sequence it
    /// delivered would drop every block after it, which is the rest of the turn.
    #[test]
    fn a_changes_items_climb_so_a_later_block_survives_a_derived_row() {
        let mut call = activity("msg-2", 2, 0, ActivityKind::ToolCall, "AskUserQuestion");
        call.tool_use_id = Some("call-1".to_string());

        let mut panel = activity("msg-2", 2, 0, ActivityKind::UserInputRequested, "Question");
        panel.activity_id = "msg-2:0:user-input".to_string();
        panel.payload = json!({
            "requestId": "call-1",
            "questions": [{
                "id": "Which store?",
                "header": "Storage",
                "question": "Which store?",
                "multiSelect": false,
                "options": [{ "label": "Postgres", "description": "Already deployed." }]
            }]
        });

        // The block after the one the question was asked on.
        let mut later = activity("msg-2", 2, 1, ActivityKind::ToolCall, "Bash");
        later.tool_use_id = Some("call-2".to_string());

        let window = transcript(Vec::new(), vec![call, panel, later]);
        let emitted = events(&change(2, Some("msg-2")), &window);

        let sequences: Vec<i64> = emitted
            .iter()
            .map(|event| event["sequence"].as_i64().expect("a sequence"))
            .collect();
        let mut climbing = sequences.clone();
        climbing.sort_unstable();
        assert_eq!(
            sequences, climbing,
            "items must be emitted in sequence order"
        );
        assert_eq!(sequences.len(), 3, "every row reaches the reader");
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
        assert_eq!(summaries, vec!["Thinking", "Command run", "File change"]);
        assert_eq!(sent[0]["payload"]["threadId"], "task-a");
    }

    /// A change that names an activity rather than a Message — a Sandbox
    /// lifecycle row, a context-window reading — carries that one work-log row
    /// and nothing else: no bubble, and no `thread.session-set`, because nothing
    /// about the conversation's own session moved. The row is numbered
    /// `ordinal + 1`, the same value its snapshot row carries.
    #[test]
    fn a_change_naming_an_activity_carries_that_row_and_no_session() {
        let mut ready = activity("", 6, 0, ActivityKind::SandboxStatus, "Ready");
        ready.activity_id = "evt-ready".to_string();
        ready.message_uuid = None;
        let window = transcript(Vec::new(), vec![ready]);

        let sent = events(&activity_change(6, "evt-ready"), &window);
        assert_eq!(sent.len(), 1, "one work-log row and no session: {sent:#?}");
        assert_eq!(sent[0]["type"], "thread.activity-appended");
        assert_eq!(sent[0]["payload"]["activity"]["id"], "evt-ready");
        assert_eq!(sent[0]["payload"]["activity"]["summary"], "Ready");
        assert_eq!(sent[0]["sequence"], 6 * EVENTS_PER_CHANGE + 1);
    }

    /// A change naming an activity the window does not hold says nothing — a
    /// complete answer, the same one a change naming an absent Message gives.
    #[test]
    fn a_change_naming_an_activity_not_in_the_window_says_nothing() {
        assert!(events(&activity_change(6, "evt-gone"), &a_conversation()).is_empty());
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
        assert_eq!(
            resolved,
            vec![("Command run", "completed"), ("File change", "failed")]
        );
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
            sandbox_desired_state_raw: None,
            ..running_conversation("task-a", "ws-a")
        };
        assert!(
            change_events(
                &without,
                &change(4, None),
                &transcript(Vec::new(), Vec::new()),
                &OriginContext {
                    frontend_url: "http://localhost:3000",
                    source_task_names: &HashMap::new(),
                },
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

    /// The project root the shell reports and the workspace id read back out of
    /// it are each other's inverse, so a draft thread's branch request resolves
    /// to the workspace it names. A worktree path is a different namespace and
    /// is not read as a project root.
    #[test]
    fn a_project_root_round_trips_to_its_workspace_id() {
        assert_eq!(
            workspace_id_from_project_root(&nominal_root("ws-1")),
            Some("ws-1")
        );
        assert!(is_project_root(&nominal_root("ws-1")));
        assert_eq!(
            workspace_id_from_project_root(&nominal_worktree("task-1")),
            None
        );
        assert_eq!(
            workspace_id_from_project_root("/moatless/workspaces/ws/1"),
            None
        );
        assert_eq!(
            workspace_id_from_project_root("/moatless/workspaces/"),
            None
        );
    }

    /// Field for field against `Subtask` in
    /// `packages/contracts/src/subtasks.ts`. Every field there is required, and
    /// the three nullable ones are written as null rather than skipped: a
    /// client decoding a struct does not get to treat an absent key as null.
    #[test]
    fn a_subtask_carries_every_field_the_contract_requires() {
        let mut child = running_conversation("task-child", "ws-1");
        child.name = Some("Migrate the callers".to_string());
        child.created_at = "2026-07-01T09:05:00.000Z".to_string();
        child.created_at_db = Some(instant_at("2026-07-01T09:05:00.000Z"));
        child.updated_at = Some(instant_at("2026-07-01T09:11:30.000Z"));

        assert_eq!(
            subtask(&child, TaskRelationType::CreatedBy),
            json!({
                "threadId": "task-child",
                "title": "Migrate the callers",
                "relation": "createdBy",
                "status": "running",
                "awaitingInput": false,
                "agentType": "claude-code",
                "branch": "feat/checkout",
                "turnCount": 3,
                "createdAt": "2026-07-01T09:05:00.000Z",
                "updatedAt": "2026-07-01T09:11:30.000Z",
                "archivedAt": Value::Null,
            })
        );
    }

    /// A fork is a child too, and the row says which kind it is because the two
    /// mean different things to a reader.
    #[test]
    fn a_forked_child_is_labelled_as_one() {
        assert_eq!(
            subtask(
                &conversation("task-fork", "ws-1"),
                TaskRelationType::ForkedFrom
            )["relation"],
            "forkedFrom"
        );
    }

    /// Closing outranks the newest turn. A child whose last turn errored and
    /// which was then closed is finished, and reporting it as failing would
    /// keep a settled fleet looking broken.
    #[test]
    fn subtask_status_reads_closing_before_the_newest_turn() {
        let mut child = running_conversation("task-child", "ws-1");
        child.turn_status = Some("error".to_string());
        assert_eq!(
            subtask(&child, TaskRelationType::CreatedBy)["status"],
            "failed"
        );

        child.closed_at = Some(Utc::now());
        assert_eq!(
            subtask(&child, TaskRelationType::CreatedBy)["status"],
            "completed"
        );
    }

    /// The four turn states a child can settle in, and the one it sits at
    /// before it has ever run.
    #[test]
    fn every_turn_state_maps_onto_the_panels_vocabulary() {
        let mut child = running_conversation("task-child", "ws-1");
        for (turn_status, expected) in [
            ("active", "running"),
            // The Sandbox went away mid-turn; the run resumes, so it is working.
            ("interrupted", "running"),
            ("completed", "idle"),
            ("error", "failed"),
            ("stopped", "interrupted"),
            // A status this build does not know is not reported as a state it
            // is not in.
            ("something-newer", "pending"),
        ] {
            child.turn_status = Some(turn_status.to_string());
            assert_eq!(
                subtask(&child, TaskRelationType::CreatedBy)["status"],
                expected,
                "turn status {turn_status}"
            );
        }

        child.turn_status = None;
        assert_eq!(
            subtask(&child, TaskRelationType::CreatedBy)["status"],
            "pending"
        );
    }

    /// Waiting on a person is not a state of the run, so it rides beside the
    /// status rather than replacing it.
    #[test]
    fn awaiting_input_is_reported_beside_a_running_status() {
        let mut child = running_conversation("task-child", "ws-1");
        child.pending_question = true;
        let row = subtask(&child, TaskRelationType::CreatedBy);
        assert_eq!(row["awaitingInput"], true);
        assert_eq!(row["status"], "running");

        child.pending_question = false;
        child.pending_plan = true;
        assert_eq!(
            subtask(&child, TaskRelationType::CreatedBy)["awaitingInput"],
            true
        );
    }

    /// An unnamed child gets the contract's placeholder: `title` refuses a
    /// blank string, and `tasks.name` is nullable.
    #[test]
    fn an_unnamed_subtask_falls_back_to_the_contracts_placeholder() {
        let mut child = conversation("task-child", "ws-1");
        child.name = Some("   ".to_string());
        child.branch = None;
        child.turn_number = None;
        let row = subtask(&child, TaskRelationType::CreatedBy);
        assert_eq!(row["title"], "New thread");
        assert_eq!(row["branch"], Value::Null);
        assert_eq!(row["turnCount"], 0);
    }

    fn instant_at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("a well-formed instant")
            .with_timezone(&Utc)
    }
}
