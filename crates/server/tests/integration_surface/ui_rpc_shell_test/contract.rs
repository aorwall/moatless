//! The contract, transcribed.
//!
//! See the comment below for what `OrchestrationV2ThreadShell` copies from
//! `packages/contracts/src/orchestrationV2.ts` and what "Optional" means here.

use serde_json::Value;

// `OrchestrationV2ThreadShell` and its nested types, copied from
// `packages/contracts/src/orchestrationV2.ts`. "Optional" means the contract
// can fill it (`Schema.optional` or a decoding default); everything else
// must be on the wire, `null` included, or the snapshot fails to decode.
// Asserted here against real database rows, not hand-built ones.

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

/// Keys the row emits that the V2 schema does not declare; the decoder drops
/// them. Listed so a new undeclared key still fails here.
const UNDECLARED_CARRYOVER: &[&str] = &["latestTurn", "session"];

const THREAD_NON_EMPTY: &[&str] = &["id", "projectId", "title"];

/// `ModelSelection` (`modelSelection.ts`), after the wire transform.
const MODEL_SELECTION_REQUIRED: &[&str] = &["instanceId", "model"];

const MODEL_SELECTION_OPTIONAL: &[&str] = &["options"];

/// `OrchestrationV2AppThreadLineage`.
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

fn assert_non_empty(value: &Value, what: &str, keys: &[&str]) {
    for key in keys {
        let text = value[*key]
            .as_str()
            .unwrap_or_else(|| panic!("{what}.{key} must be a string"));
        assert_eq!(text, text.trim(), "{what}.{key} must be trimmed");
        assert!(!text.is_empty(), "{what}.{key} must not be empty");
    }
}

/// Every key the client reads off one conversation row, asserted against the
/// contract. Nested objects are checked too — an undeclared key one level down
/// fails the decode just as surely as one at the top.
pub(super) fn assert_decodable_thread(thread: &Value) {
    let optional: Vec<&str> = THREAD_OPTIONAL
        .iter()
        .chain(UNDECLARED_CARRYOVER)
        .copied()
        .collect();
    assert_keys(thread, "thread", THREAD_REQUIRED, &optional);
    assert_non_empty(thread, "thread", THREAD_NON_EMPTY);
    assert_keys(
        &thread["modelSelection"],
        "modelSelection",
        MODEL_SELECTION_REQUIRED,
        MODEL_SELECTION_OPTIONAL,
    );
    assert_non_empty(
        &thread["modelSelection"],
        "modelSelection",
        MODEL_SELECTION_REQUIRED,
    );
    assert_keys(
        &thread["lineage"],
        "lineage",
        LINEAGE_REQUIRED,
        LINEAGE_OPTIONAL,
    );
    let status = thread["status"]
        .as_str()
        .expect("thread.status must be a string");
    assert!(
        SHELL_STATUSES.contains(&status),
        "`{status}` is not an `OrchestrationV2ShellThreadStatus`"
    );
}

// Delivery: a watching client sees a turn change.
//
// The snapshot is a listing read at a position; everything after reaches
// the client as stream events off the change feed. These tests drive the
// real writers (allocator, gRPC ingress), not hand-inserted records,
// since that would only prove the helper works.

/// The `thread.updated` member of `OrchestrationV2ShellStreamEvent`. An
/// undeclared key fails the union's decode and the client applies nothing.
const THREAD_UPDATED_REQUIRED: &[&str] = &["kind", "sequence", "location", "thread"];

/// `OrchestrationV2ShellThreadLocation`.
const LOCATIONS: &[&str] = &["active", "archive"];

fn assert_position(item: &Value) {
    let sequence = item["sequence"]
        .as_i64()
        .unwrap_or_else(|| panic!("{} .sequence must be an integer", item["kind"]));
    assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
}

fn assert_location(item: &Value) {
    let location = item["location"].as_str().unwrap_or_default();
    assert!(
        LOCATIONS.contains(&location),
        "`{location}` is not an `OrchestrationV2ShellThreadLocation`"
    );
}

/// One delivered row, checked against the contract the way the whole row is:
/// keys first, then the nested thread, then `NonNegativeInt` on the position.
pub(super) fn assert_decodable_update(event: &Value) {
    assert_keys(event, "thread.updated", THREAD_UPDATED_REQUIRED, &[]);
    assert_eq!(event["kind"], "thread.updated");
    assert_position(event);
    assert_location(event);
    assert_decodable_thread(&event["thread"]);
}

// Reconnecting: a client holds a position and asks what it missed, and
// gets one of three answers — here is what changed, nothing changed, or
// more changed than I kept (here is the list again). The last two are
// the same empty result unless made different; a client unable to tell
// them apart renders a stale sidebar forever while reporting itself live.

/// The `snapshot` member of `OrchestrationV2ShellStreamItem`, both keys
/// required. An undeclared key here is a client that never rebuilds.
const SNAPSHOT_ITEM_REQUIRED: &[&str] = &["kind", "snapshot"];

/// `OrchestrationV2ShellSnapshot`.
const SHELL_SNAPSHOT_REQUIRED: &[&str] = &[
    "schemaVersion",
    "snapshotSequence",
    "projects",
    "threads",
    "archivedThreads",
];

const SHELL_SNAPSHOT_OPTIONAL: &[&str] = &["olderSettled"];

/// One rebuild frame, checked against the contract the way a delivered row is.
pub(super) fn assert_decodable_snapshot_item(item: &Value) {
    assert_keys(item, "snapshot item", SNAPSHOT_ITEM_REQUIRED, &[]);
    assert_eq!(item["kind"], "snapshot");
    let snapshot = &item["snapshot"];
    assert_keys(
        snapshot,
        "OrchestrationV2ShellSnapshot",
        SHELL_SNAPSHOT_REQUIRED,
        SHELL_SNAPSHOT_OPTIONAL,
    );
    let sequence = snapshot["snapshotSequence"]
        .as_i64()
        .expect("snapshotSequence must be an integer");
    assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
    for key in ["threads", "archivedThreads"] {
        for thread in snapshot[key].as_array().unwrap() {
            assert_decodable_thread(thread);
        }
    }
    for project in snapshot["projects"].as_array().unwrap() {
        assert_keys(project, "project", PROJECT_REQUIRED, PROJECT_OPTIONAL);
        assert_non_empty(project, "project", PROJECT_NON_EMPTY);
    }
}

/// `ExecutionEnvironmentDescriptor` (`environment.ts`). The client decodes
/// this before it has a connection, so an undeclared key is an environment
/// nothing can reach.
pub(super) const DESCRIPTOR_REQUIRED: &[&str] = &[
    "environmentId",
    "label",
    "platform",
    "serverVersion",
    "capabilities",
];

/// Absent reads as protocol 1, which the client refuses to connect to.
pub(super) const DESCRIPTOR_OPTIONAL: &[&str] = &["orchestrationProtocolVersion"];

// The rest of the change kinds a sidebar shows moving: a row entering a
// listing, and a row leaving one.

/// The other members of `OrchestrationV2ShellStreamEvent`.
pub(super) const THREAD_REMOVED_REQUIRED: &[&str] = &["kind", "sequence", "location", "threadId"];

const PROJECT_UPDATED_REQUIRED: &[&str] = &["kind", "sequence", "project"];

/// `OrchestrationProjectShell` (`orchestration.ts`).
const PROJECT_REQUIRED: &[&str] = &[
    "id",
    "title",
    "workspaceRoot",
    "defaultModelSelection",
    "scripts",
    "createdAt",
    "updatedAt",
];

const PROJECT_OPTIONAL: &[&str] = &["repositoryIdentity", "projectIcon", "scriptsEditable"];

const PROJECT_NON_EMPTY: &[&str] = &["id", "title", "workspaceRoot"];

/// One item off the feed, checked against the whole union rather than the one
/// member a test happens to expect. An item the client cannot decode is not a
/// missing update — the stream's decode fails and the subscription stops.
pub(super) fn assert_decodable_item(item: &Value) {
    match item["kind"].as_str() {
        Some("thread.updated") => assert_decodable_update(item),
        Some("thread.removed") => {
            assert_keys(item, "thread.removed", THREAD_REMOVED_REQUIRED, &[]);
            assert_position(item);
            assert_location(item);
            assert_non_empty(item, "thread.removed", &["threadId"]);
        }
        Some("project.updated") => {
            assert_keys(item, "project.updated", PROJECT_UPDATED_REQUIRED, &[]);
            assert_position(item);
            assert_keys(
                &item["project"],
                "project",
                PROJECT_REQUIRED,
                PROJECT_OPTIONAL,
            );
            assert_non_empty(&item["project"], "project", PROJECT_NON_EMPTY);
        }
        Some("snapshot") => assert_decodable_snapshot_item(item),
        other => panic!("`{other:?}` is not a member of `OrchestrationV2ShellStreamItem`"),
    }
}
