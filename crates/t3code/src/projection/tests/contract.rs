//! The client's own schema, transcribed from `orchestration.ts`, and the rows
//! checked against it field for field.

use super::fixtures::*;
use super::*;

// -----------------------------------------------------------------------
// The contract, transcribed
// -----------------------------------------------------------------------
//
// Copied from `packages/contracts/src/orchestration.ts` in
// `soaplabs/t3code@moatless-adapter` — `OrchestrationShellSnapshot`,
// `OrchestrationProjectShell` and `ProjectScript` — and deliberately *not*
// derived from the projection below: a test written from the projection
// cannot see a required field the projection never thought to emit, and one
// missing field makes the entire listing undecodable.

const SNAPSHOT_REQUIRED: &[&str] = &["snapshotSequence", "projects", "threads", "updatedAt"];
const SNAPSHOT_OPTIONAL: &[&str] = &[];

pub(super) const PROJECT_REQUIRED: &[&str] = &[
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
pub(super) const PROJECT_OPTIONAL: &[&str] =
    &["repositoryIdentity", "projectIcon", "scriptsEditable"];

const SCRIPT_REQUIRED: &[&str] = &["id", "name", "command", "icon", "runOnWorktreeCreate"];
// `port` is a fork addition to `ProjectScript`: it stands in for upstream's
// `previewUrl`, which a Moatless script cannot carry (see the projection).
const SCRIPT_OPTIONAL: &[&str] = &["previewUrl", "autoOpenPreview", "port"];

/// Trimmed-non-empty strings in the transcribed types above. An empty value is
/// refused by the contract exactly as a missing key would be.
pub(super) const PROJECT_NON_EMPTY: &[&str] = &["id", "title", "workspaceRoot"];
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
    "pinnedAt",
    "pinOrderKey",
    "activeOrderKey",
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

pub(super) fn assert_keys(value: &Value, what: &str, required: &[&str], optional: &[&str]) {
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

pub(super) fn assert_non_empty(value: &Value, what: &str, keys: &[&str]) {
    for key in keys {
        let text = value[*key]
            .as_str()
            .unwrap_or_else(|| panic!("{what}.{key} must be a string"));
        assert_eq!(text, text.trim(), "{what}.{key} must be trimmed");
        assert!(!text.is_empty(), "{what}.{key} must not be empty");
    }
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

// -----------------------------------------------------------------------
// The conversation itself
// -----------------------------------------------------------------------
//
// `OrchestrationThread` (`orchestration.ts:342-378`) — the shell row's
// fuller twin. It drops the four the shell adds for its own sorting and
// badges, and adds the conversation: `deletedAt`, `messages`,
// `proposedPlans`, `activities`, `checkpoints`.

pub(super) const DETAIL_REQUIRED: &[&str] = &[
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
pub(super) const DETAIL_OPTIONAL: &[&str] = &[
    "interactionMode",
    "archivedAt",
    "proposedPlans",
    "linkedPullRequest",
    "pullRequests",
    "visibility",
    "pinnedAt",
    "pinOrderKey",
    "activeOrderKey",
];

// `OrchestrationMessage` (`:227-236`).
pub(super) const MESSAGE_REQUIRED: &[&str] = &[
    "id",
    "role",
    "text",
    "turnId",
    "streaming",
    "createdAt",
    "updatedAt",
];
pub(super) const MESSAGE_OPTIONAL: &[&str] = &["attachments", "origin"];
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

/// `OrchestrationCheckpointSummary` (`:597-605`).
pub(super) const CHECKPOINT_REQUIRED: &[&str] = &[
    "turnId",
    "checkpointTurnCount",
    "checkpointRef",
    "status",
    "files",
    "assistantMessageId",
    "completedAt",
];
pub(super) const CHECKPOINT_OPTIONAL: &[&str] = &[];
/// `OrchestrationThreadActivityTone` (`:304-309`).
const ACTIVITY_TONES: &[&str] = &["info", "tool", "approval", "error"];

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

/// The shell's four extra fields are the shell's. Emitting one here is not a
/// harmless extra: `OrchestrationThread` is a closed struct, so an undeclared
/// key fails the decode and the conversation never renders.
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

// ── Events ──────────────────────────────────────────────────────────

pub(super) const EVENT_REQUIRED: &[&str] = &[
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
