//! The client's own schema, transcribed from `orchestrationV2.ts`, and the rows
//! checked against it field for field.

use super::fixtures::*;
use super::*;

// The contract, transcribed from `packages/contracts/src/orchestrationV2.ts`
// and `orchestration.ts` (`soaplabs/t3code@moatless-adapter`):
// `OrchestrationV2ShellSnapshot`, `OrchestrationProjectShell`, `ProjectScript`.
//
// Deliberately not derived from the projection below: a test derived from it
// can't see a field the projection forgot to emit.

const SNAPSHOT_REQUIRED: &[&str] = &[
    "schemaVersion",
    "snapshotSequence",
    "projects",
    "threads",
    "archivedThreads",
];
const SNAPSHOT_OPTIONAL: &[&str] = &["olderSettled"];

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

// `OrchestrationV2ThreadShell`: the shell's conversation row.
//
// "Optional" means `Schema.optional` or `withDecodingDefault`; everything
// else must be on the wire, `null` included, or the whole snapshot fails.
const THREAD_REQUIRED: &[&str] = &[
    "createdBy",
    "creationSource",
    "id",
    "projectId",
    "title",
    "providerInstanceId",
    "modelSelection",
    "runtimeMode",
    "interactionMode",
    "branch",
    "worktreePath",
    "lineage",
    "forkedFrom",
    "activeProviderThreadId",
    "latestRunId",
    "activeRunId",
    "status",
    "pendingRuntimeRequest",
    "latestVisibleMessage",
    "latestUserMessageAt",
    "hasActionableProposedPlan",
    "itemCount",
    "visibleItemCount",
    "createdAt",
    "updatedAt",
    "archivedAt",
    "settledOverride",
    "settledAt",
    "deletedAt",
];
const THREAD_OPTIONAL: &[&str] = &[
    "linkedPullRequest",
    "pullRequests",
    "branchPullRequest",
    "issues",
    "historyOrigin",
    "linkOrigin",
    "delegatedFrom",
    "workflowRun",
    "latestRunRequestedAt",
    "latestRunStartedAt",
    "latestRunCompletedAt",
    "latestRunStopRequested",
    "latestRunBackgroundWake",
    "activityRunStartedAt",
    "activityRunStatus",
    "blockedOnThreadWait",
    "lastError",
    "lastErrorClass",
    "lastErrorCode",
    "usageLimitResetAt",
    "pendingGuestMessageCount",
    "latestUserAuthoredMessageAt",
    "pendingBackgroundTasks",
    "providerInstanceHistory",
    "goal",
    "unsettledAt",
    "snoozedUntil",
    "snoozedAt",
    "limitRecovery",
    "pinnedAt",
    "autoSettleDisabledAt",
    "pinOrderKey",
    "activeOrderKey",
    "tags",
    "lastVisitedAt",
    "opens",
    "titleRegeneration",
    "lastTitleRegeneration",
    "visibility",
    "ownerName",
    "childCount",
];
const THREAD_NON_EMPTY: &[&str] = &["id", "projectId", "title"];

/// Keys the shell row and the projection's `thread` emit that neither V2
/// schema declares. The decoder drops an undeclared key rather than failing,
/// so these cost bytes, not the render; listed so a new one still fails here.
const UNDECLARED_CARRYOVER: &[&str] = &["latestTurn", "session"];

// `ModelSelection` (`modelSelection.ts`), after the wire transform. `provider`
// is the legacy shape the transform absorbs, not something to emit.
const MODEL_SELECTION_REQUIRED: &[&str] = &["instanceId", "model"];
const MODEL_SELECTION_OPTIONAL: &[&str] = &["options"];
const MODEL_SELECTION_NON_EMPTY: &[&str] = &["instanceId", "model"];

// `OrchestrationV2AppThreadLineage`.
const LINEAGE_REQUIRED: &[&str] = &["parentThreadId", "relationshipToParent", "rootThreadId"];
const LINEAGE_OPTIONAL: &[&str] = &["sideChat", "workflowStep"];

/// `OrchestrationV2ShellThreadStatus`: `idle` or an `OrchestrationV2RunStatus`.
const SHELL_STATUSES: &[&str] = &[
    "idle",
    "preparing",
    "queued",
    "starting",
    "running",
    "waiting",
    "completed",
    "interrupted",
    "failed",
    "cancelled",
    "rolled_back",
];
/// `RuntimeMode` (`providerPolicy.ts`).
const RUNTIME_MODES: &[&str] = &[
    "approval-required",
    "auto-accept-edits",
    "auto",
    "full-access",
];
/// `ProviderInteractionMode` (`providerPolicy.ts`).
const INTERACTION_MODES: &[&str] = &["default", "plan"];

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

    assert_keys(
        &thread,
        "thread",
        THREAD_REQUIRED,
        &[THREAD_OPTIONAL, UNDECLARED_CARRYOVER].concat(),
    );
    assert_non_empty(&thread, "thread", THREAD_NON_EMPTY);
    assert!(
        RUNTIME_MODES.contains(&thread["runtimeMode"].as_str().unwrap()),
        "thread.runtimeMode must be one of the contract's literals"
    );
    assert!(
        INTERACTION_MODES.contains(&thread["interactionMode"].as_str().unwrap()),
        "thread.interactionMode must be one of the contract's literals"
    );
    assert!(
        SHELL_STATUSES.contains(&thread["status"].as_str().unwrap()),
        "thread.status must be one of the contract's literals"
    );
    assert!(thread["hasActionableProposedPlan"].is_boolean());
    for key in ["itemCount", "visibleItemCount"] {
        assert!(
            thread[key].as_u64().is_some(),
            "thread.{key} must be a non-negative integer"
        );
    }

    let model = &thread["modelSelection"];
    assert_keys(
        model,
        "modelSelection",
        MODEL_SELECTION_REQUIRED,
        MODEL_SELECTION_OPTIONAL,
    );
    assert_non_empty(model, "modelSelection", MODEL_SELECTION_NON_EMPTY);

    assert_keys(
        &thread["lineage"],
        "lineage",
        LINEAGE_REQUIRED,
        LINEAGE_OPTIONAL,
    );
}

#[test]
fn the_wire_shape_matches_the_contract_field_for_field() {
    let mut ws = workspace("ws-a", "Alpha");
    ws.workspace.scripts = decoded(json!([{
        "name": "dev",
        "command": "bun run dev",
        "icon": "play",
        "port": 3000
    }]));

    let snapshot = shell_snapshot_v2(&[ws], &[], &[], &AgentModelDefaults::default(), 0);
    assert_keys(&snapshot, "snapshot", SNAPSHOT_REQUIRED, SNAPSHOT_OPTIONAL);
    assert!(
        snapshot["snapshotSequence"]
            .as_i64()
            .is_some_and(|n| n >= 0)
    );
    assert!(
        snapshot["schemaVersion"].as_i64().is_some_and(|n| n > 0),
        "snapshot.schemaVersion must be a positive integer"
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

// `OrchestrationV2ThreadProjection`: the whole conversation a thread
// subscription delivers.

pub(super) const PROJECTION_REQUIRED: &[&str] = &[
    "thread",
    "runs",
    "attempts",
    "nodes",
    "subagents",
    "providerSessions",
    "providerThreads",
    "providerTurns",
    "runtimeRequests",
    "messages",
    "plans",
    "turnItems",
    "checkpointScopes",
    "checkpoints",
    "contextHandoffs",
    "contextTransfers",
    "visibleTurnItems",
    "updatedAt",
];

// `OrchestrationV2AppThread`: the projection's `thread`.
pub(super) const DETAIL_REQUIRED: &[&str] = &[
    "createdBy",
    "creationSource",
    "id",
    "projectId",
    "title",
    "providerInstanceId",
    "modelSelection",
    "runtimeMode",
    "interactionMode",
    "branch",
    "worktreePath",
    "activeProviderThreadId",
    "lineage",
    "forkedFrom",
    "createdAt",
    "updatedAt",
    "archivedAt",
    "deletedAt",
];
pub(super) const DETAIL_OPTIONAL: &[&str] = &[
    "linkedPullRequest",
    "pullRequests",
    "branchPullRequest",
    "issues",
    "historyOrigin",
    "linkOrigin",
    "delegatedFrom",
    "workflowRun",
    "settledOverride",
    "settledAt",
    "unsettledAt",
    "snoozedUntil",
    "snoozedAt",
    "lastSnoozeWakeAt",
    "limitRecovery",
    "guestShare",
    "pinnedAt",
    "autoSettleDisabledAt",
    "pinOrderKey",
    "activeOrderKey",
    "tags",
    "lastVisitedAt",
    "opens",
    "titleRegeneration",
    "lastTitleRegeneration",
    "titleRenamed",
    "rollbackRequestId",
    "rollbackFailure",
    "rollbackUnrestored",
    "visibility",
];

// `OrchestrationV2ConversationMessage`.
pub(super) const MESSAGE_REQUIRED: &[&str] = &[
    "createdBy",
    "creationSource",
    "id",
    "threadId",
    "runId",
    "nodeId",
    "role",
    "text",
    "attachments",
    "streaming",
    "createdAt",
    "updatedAt",
];
pub(super) const MESSAGE_OPTIONAL: &[&str] = &[
    "notification",
    "scheduledTaskId",
    "followsThreadSelection",
    "senderThreadId",
    "linkedSender",
    "idleCompaction",
    "workflowScript",
    "context",
    "origin",
    "delegatedCompletion",
];
const MESSAGE_ROLES: &[&str] = &["user", "assistant", "system"];

/// `OrchestrationV2Checkpoint`.
pub(super) const CHECKPOINT_REQUIRED: &[&str] = &[
    "id",
    "threadId",
    "scopeId",
    "runId",
    "nodeId",
    "parentCheckpointId",
    "ordinalWithinScope",
    "appRunOrdinal",
    "ref",
    "status",
    "files",
    "capturedAt",
];
pub(super) const CHECKPOINT_OPTIONAL: &[&str] = &["filesTotal"];
pub(super) const CHECKPOINT_STATUSES: &[&str] = &["ready", "missing", "error", "stale"];

#[test]
fn the_detail_wire_shape_matches_the_contract_field_for_field() {
    let projection = detail(&a_conversation());
    assert_keys(&projection, "projection", PROJECTION_REQUIRED, &[]);

    let thread = &projection["thread"];
    assert_keys(
        thread,
        "thread",
        DETAIL_REQUIRED,
        &[DETAIL_OPTIONAL, UNDECLARED_CARRYOVER].concat(),
    );
    assert_non_empty(thread, "thread", THREAD_NON_EMPTY);
    assert_keys(
        &thread["lineage"],
        "lineage",
        LINEAGE_REQUIRED,
        LINEAGE_OPTIONAL,
    );

    let messages = projection["messages"].as_array().unwrap();
    assert!(!messages.is_empty(), "the fixture's messages must project");
    for message in messages {
        assert_keys(message, "message", MESSAGE_REQUIRED, MESSAGE_OPTIONAL);
        assert!(
            MESSAGE_ROLES.contains(&message["role"].as_str().unwrap()),
            "message.role must be one of the contract's three literals"
        );
        assert!(message["streaming"].is_boolean());
        assert!(message["attachments"].is_array());
        assert_eq!(message["threadId"], thread["id"]);
        assert!(
            !message["id"].as_str().unwrap().is_empty(),
            "message.id must not be empty"
        );
    }

    for checkpoint in projection["checkpoints"].as_array().unwrap() {
        assert_keys(
            checkpoint,
            "checkpoint",
            CHECKPOINT_REQUIRED,
            CHECKPOINT_OPTIONAL,
        );
        assert!(
            CHECKPOINT_STATUSES.contains(&checkpoint["status"].as_str().unwrap()),
            "checkpoint.status must be one of the contract's literals"
        );
    }
}

/// The shell row's run and badge fields are the shell's; the projection carries
/// the runs and requests they summarise.
#[test]
fn the_detail_row_carries_none_of_the_shells_own_fields() {
    let thread = detail(&a_conversation())["thread"].clone();
    for key in [
        "latestUserMessageAt",
        "hasActionableProposedPlan",
        "pendingRuntimeRequest",
        "latestRunId",
        "activeRunId",
        "status",
    ] {
        assert!(
            thread.get(key).is_none(),
            "the detail row must not carry the shell's `{key}`"
        );
    }
}
