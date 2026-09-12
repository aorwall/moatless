//! The contract, transcribed.
//!
//! `OrchestrationThreadShell` and the types it nests, copied out of
//! `packages/contracts/src/orchestration.ts` on `moatless-adapter`. "Optional"
//! means the contract can fill it — `Schema.optional` or a decoding default.
//! Everything else has to be on the wire, `null` included, or the whole
//! snapshot fails to decode and the sidebar goes blank.
//!
//! The unit tests assert this against hand-built rows; here it is asserted
//! against rows a real database produced, where the nullable columns are
//! genuinely null rather than null because a factory said so.

use serde_json::Value;

// =========================================================================
// The contract, transcribed
// =========================================================================
//
// `OrchestrationThreadShell` and the types it nests, copied out of
// `packages/contracts/src/orchestration.ts` on `moatless-adapter`. "Optional"
// means the contract can fill it — `Schema.optional` or a decoding default.
// Everything else has to be on the wire, `null` included, or the whole
// snapshot fails to decode and the sidebar goes blank.
//
// The unit tests assert this against hand-built rows; here it is asserted
// against rows a real database produced, where the nullable columns are
// genuinely null rather than null because a factory said so.

/// `OrchestrationThreadShell` (`orchestration.ts:401-427`).
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
    "backgroundLiveness",
    "linkedPullRequest",
    "linkedPullRequests",
    "interactionMode",
    "archivedAt",
    "settledOverride",
    "settledAt",
    "snoozedUntil",
    "snoozedAt",
    "ownerName",
];

const THREAD_NON_EMPTY: &[&str] = &["id", "projectId", "title"];

/// `ModelSelection` (`:64-68`), after the wire transform.
const MODEL_SELECTION_REQUIRED: &[&str] = &["instanceId", "model"];

const MODEL_SELECTION_OPTIONAL: &[&str] = &["options"];

/// `OrchestrationLatestTurn` (`:334-342`).
const TURN_REQUIRED: &[&str] = &[
    "turnId",
    "state",
    "requestedAt",
    "startedAt",
    "completedAt",
    "assistantMessageId",
];

const TURN_OPTIONAL: &[&str] = &["sourceProposedPlan"];

/// `OrchestrationSession` (`:272-281`).
const SESSION_REQUIRED: &[&str] = &[
    "threadId",
    "status",
    "providerName",
    "activeTurnId",
    "lastError",
    "updatedAt",
];

const SESSION_OPTIONAL: &[&str] = &["providerInstanceId", "runtimeMode"];

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
    assert_keys(thread, "thread", THREAD_REQUIRED, THREAD_OPTIONAL);
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
    if !thread["latestTurn"].is_null() {
        assert_keys(
            &thread["latestTurn"],
            "latestTurn",
            TURN_REQUIRED,
            TURN_OPTIONAL,
        );
    }
    if !thread["session"].is_null() {
        assert_keys(
            &thread["session"],
            "session",
            SESSION_REQUIRED,
            SESSION_OPTIONAL,
        );
    }
}

// =========================================================================
// Delivery: a watching client sees a turn change
// =========================================================================
//
// The snapshot is a listing read at a position; everything that happens after
// it reaches the client as stream events off the change feed. These tests drive
// the real writers — the turn-number allocator and the gRPC ingress — rather
// than inserting change records by hand, because a record written by a test
// helper proves only that the helper works. What is asserted is what a client
// watching the feed is handed.

/// The `thread-upserted` member of `OrchestrationShellStreamEvent`
/// (`orchestration.ts:449-453`). Three keys, all required, none optional — an
/// undeclared one fails the union's decode and the client applies nothing.
const THREAD_UPSERTED_REQUIRED: &[&str] = &["kind", "sequence", "thread"];

/// One delivered event, checked against the contract the way the whole row is:
/// keys first, then the nested thread, then `NonNegativeInt` on the position.
pub(super) fn assert_decodable_upsert(event: &Value) {
    assert_keys(event, "thread-upserted", THREAD_UPSERTED_REQUIRED, &[]);
    assert_eq!(event["kind"], "thread-upserted");
    let sequence = event["sequence"]
        .as_i64()
        .expect("thread-upserted.sequence must be an integer");
    assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
    assert_decodable_thread(&event["thread"]);
}

// =========================================================================
// Reconnecting
// =========================================================================
//
// A client that comes back holds a position and asks for what it missed. What
// it may be told is one of three things, and the whole of this ticket is that
// they are three rather than two: *here is what changed*, *nothing changed*,
// and *more changed than I kept, here is the list again*. The last two are the
// same empty result unless they are made different, and a client that cannot
// tell them apart renders a stale sidebar forever while reporting itself live.

/// The `snapshot` member of `OrchestrationShellStreamItem`
/// (`orchestration.ts:466-469`). Two keys, both required — the member is what
/// carries "rebuild" in the contract's own vocabulary, so an undeclared key
/// here is a client that decodes nothing and rebuilds never.
const SNAPSHOT_ITEM_REQUIRED: &[&str] = &["kind", "snapshot"];

/// `OrchestrationShellSnapshot` (`orchestration.ts:430-435`).
const SHELL_SNAPSHOT_REQUIRED: &[&str] = &["snapshotSequence", "projects", "threads", "updatedAt"];

/// One rebuild frame, checked against the contract the way a delivered row is.
pub(super) fn assert_decodable_snapshot_item(item: &Value) {
    assert_keys(item, "snapshot item", SNAPSHOT_ITEM_REQUIRED, &[]);
    assert_eq!(item["kind"], "snapshot");
    assert_keys(
        &item["snapshot"],
        "OrchestrationShellSnapshot",
        SHELL_SNAPSHOT_REQUIRED,
        &[],
    );
    let sequence = item["snapshot"]["snapshotSequence"]
        .as_i64()
        .expect("snapshotSequence must be an integer");
    assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
}

/// `ExecutionEnvironmentDescriptor` (`environment.ts:57-63`). Every key
/// required; the client decodes this before it has a connection, so an
/// undeclared one is an environment nothing can reach.
pub(super) const DESCRIPTOR_REQUIRED: &[&str] = &[
    "environmentId",
    "label",
    "platform",
    "serverVersion",
    "capabilities",
];

// =========================================================================
// The rest of the change kinds, delivered
// =========================================================================
//
// Ticket 04 delivered a turn moving. What follows is everything else a sidebar
// shows moving, and the two directions ticket 04 could not express: a row
// *entering* a listing, and a row *leaving* one.

/// The other three members of `OrchestrationShellStreamEvent`
/// (`orchestration.ts:438-459`), and the two non-event members of
/// `OrchestrationShellStreamItem` (`:462-471`).
pub(super) const THREAD_REMOVED_REQUIRED: &[&str] = &["kind", "sequence", "threadId"];

const PROJECT_UPSERTED_REQUIRED: &[&str] = &["kind", "sequence", "project"];

const PROJECT_REMOVED_REQUIRED: &[&str] = &["kind", "sequence", "projectId"];

/// `OrchestrationShellSnapshot` (`:430-435`) and `OrchestrationProjectShell`
/// (`:389-398`).
const SNAPSHOT_REQUIRED: &[&str] = &["snapshotSequence", "projects", "threads", "updatedAt"];

const PROJECT_REQUIRED: &[&str] = &[
    "id",
    "title",
    "workspaceRoot",
    "defaultModelSelection",
    "scripts",
    "createdAt",
    "updatedAt",
];

const PROJECT_OPTIONAL: &[&str] = &["repositoryIdentity", "scriptsEditable"];

const PROJECT_NON_EMPTY: &[&str] = &["id", "title", "workspaceRoot"];

/// One item off the feed, checked against the whole union rather than the one
/// member a test happens to expect. An item the client cannot decode is not a
/// missing update — the stream's decode fails and the subscription stops.
pub(super) fn assert_decodable_item(item: &Value) {
    let sequence = || {
        let sequence = item["sequence"]
            .as_i64()
            .unwrap_or_else(|| panic!("{} .sequence must be an integer", item["kind"]));
        assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
    };
    match item["kind"].as_str() {
        Some("thread-upserted") => assert_decodable_upsert(item),
        Some("thread-removed") => {
            assert_keys(item, "thread-removed", THREAD_REMOVED_REQUIRED, &[]);
            sequence();
            assert_non_empty(item, "thread-removed", &["threadId"]);
        }
        Some("project-upserted") => {
            assert_keys(item, "project-upserted", PROJECT_UPSERTED_REQUIRED, &[]);
            sequence();
            assert_keys(
                &item["project"],
                "project",
                PROJECT_REQUIRED,
                PROJECT_OPTIONAL,
            );
            assert_non_empty(&item["project"], "project", PROJECT_NON_EMPTY);
        }
        Some("project-removed") => {
            assert_keys(item, "project-removed", PROJECT_REMOVED_REQUIRED, &[]);
            sequence();
            assert_non_empty(item, "project-removed", &["projectId"]);
        }
        Some("snapshot") => {
            assert_keys(item, "snapshot", SNAPSHOT_ITEM_REQUIRED, &[]);
            assert_keys(&item["snapshot"], "shell snapshot", SNAPSHOT_REQUIRED, &[]);
            for thread in item["snapshot"]["threads"].as_array().unwrap() {
                assert_decodable_thread(thread);
            }
            for project in item["snapshot"]["projects"].as_array().unwrap() {
                assert_keys(project, "project", PROJECT_REQUIRED, PROJECT_OPTIONAL);
            }
        }
        other => panic!("`{other:?}` is not a member of `OrchestrationShellStreamItem`"),
    }
}
