//! The wire shapes this directory builds, asserted field for field.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::AgentModelDefaults;
use crate::agent::catalog;
use crate::dao::{
    Conversation, ConversationMatch, ConversationPullRequest, Transcript, TranscriptMessage,
};
use crate::workspace::{UiRpcListedWorkspace, WorkspaceView as Workspace};
use moatless_core::change_feed::conversation::{ConversationChange, EVENTS_PER_CHANGE};
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityView;
use moatless_types::message::MessageType;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskRelationType;

use super::detail::*;
use super::events::*;
use super::model::*;
use super::origin::*;
use super::project::*;
use super::scalar::*;
use super::search::*;
use super::shell::*;
use super::transcript::*;
use super::turn::*;

use crate::activity_dao::SubagentUsage;
use moatless_types::access_control::Scope;
use moatless_types::activity::ActivityStatus;
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
// derived from the projection below: a test written from the projection
// cannot see a required field the projection never thought to emit, and one
// missing field makes the entire listing undecodable.

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
    ws.workspace.default_model = Some(catalog::UNRESOLVED_MODEL_CHOICE.to_string());
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

/// `OrchestrationCheckpointSummary` (`:597-605`).
const CHECKPOINT_REQUIRED: &[&str] = &[
    "turnId",
    "checkpointTurnCount",
    "checkpointRef",
    "status",
    "files",
    "assistantMessageId",
    "completedAt",
];
const CHECKPOINT_OPTIONAL: &[&str] = &[];
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
    let mut turns: Vec<i32> = messages.iter().filter_map(|m| m.turn_number).collect();
    turns.sort_unstable();
    turns.dedup();
    Transcript {
        messages,
        activities,
        omitted: 0,
        // What `task_turns` holds for these rows. A test about a turn with
        // no Message of its own sets this itself.
        turns,
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
    detail_of(running_conversation("task-a", "ws-a"), transcript)
}

/// The same, with the fixture's turn finished — the only state that offers
/// a checkpoint, since a rewind is refused while a turn is in flight.
fn quiet_detail(transcript: &Transcript) -> Value {
    let mut conversation = running_conversation("task-a", "ws-a");
    conversation.turn_status = Some("completed".to_string());
    detail_of(conversation, transcript)
}

fn detail_of(conversation: Conversation, transcript: &Transcript) -> Value {
    thread_detail(
        &conversation,
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

    let work = detail(&transcript(Vec::new(), vec![asked, plan, meter, resolved]))["activities"]
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
/// The call's own status is the fact, and it is settled whoever settled it:
/// this covers a question answered from another surface and one a resuming
/// session dropped, neither of which writes a resolution.
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

// ── Checkpoints ─────────────────────────────────────────────────────

#[test]
fn a_checkpoint_per_turn_labelled_by_its_closing_bubble() {
    let mut messages = vec![
        message(1, MessageType::User, "do a thing"),
        message(2, MessageType::Assistant, "did it"),
        message(3, MessageType::User, "and another"),
        message(4, MessageType::Assistant, "did that too"),
    ];
    messages[0].turn_number = Some(1);
    messages[1].turn_number = Some(1);

    let detail = quiet_detail(&transcript(messages, Vec::new()));
    let checkpoints = detail["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 2);
    assert_keys(
        &checkpoints[0],
        "checkpoint",
        CHECKPOINT_REQUIRED,
        CHECKPOINT_OPTIONAL,
    );
    assert_eq!(checkpoints[0]["turnId"], "task-a:1");
    assert_eq!(checkpoints[0]["checkpointRef"], "task-a:1");
    assert_eq!(checkpoints[0]["checkpointTurnCount"], json!(1));
    assert_eq!(checkpoints[0]["assistantMessageId"], "msg-2");
    assert_eq!(checkpoints[0]["completedAt"], "2026-07-01T10:02:00.000Z");
    assert_eq!(checkpoints[1]["checkpointTurnCount"], json!(3));
    assert_eq!(checkpoints[1]["assistantMessageId"], "msg-4");
}

/// `rewind` refuses every cut while a turn is open — including one below it,
/// unlike a fork — so offering any checkpoint would be offering a control
/// that refuses when pressed.
#[test]
fn no_checkpoint_while_a_turn_is_in_flight() {
    let mut message = message(1, MessageType::Assistant, "did it");
    message.turn_number = Some(1);
    let detail = detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"], json!([]));
}

/// `ready` with no files, never `missing`: the client defers on `missing`
/// and would re-examine the turn for a diff for as long as the thread is
/// open, and this backend has no worktree snapshot to hand it either way.
#[test]
fn a_checkpoint_is_ready_and_carries_no_files() {
    let mut message = message(1, MessageType::Assistant, "did it");
    message.turn_number = Some(1);
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"][0]["status"], "ready");
    assert_eq!(detail["checkpoints"][0]["files"], json!([]));
}

/// A turn whose agent only called tools wrote no bubble to name.
#[test]
fn a_checkpoint_on_a_silent_turn_names_no_assistant_message() {
    let mut message = message(1, MessageType::User, "do a thing");
    message.turn_number = Some(1);
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"][0]["assistantMessageId"], Value::Null);
}

/// The turns are `task_turns`, not the Messages: a turn past
/// `TRANSCRIPT_LIMIT` is one a rewind still takes, and withholding it would
/// make a long thread's older turns silently unrewindable.
#[test]
fn a_turn_with_no_message_in_the_window_is_still_a_checkpoint() {
    let mut rows = transcript(Vec::new(), Vec::new());
    rows.turns = vec![7];
    let mut conversation = running_conversation("task-a", "ws-a");
    conversation.turn_status = Some("completed".to_string());
    conversation.created_at_db = Some(instant_at("2026-07-01T09:00:00Z"));
    let detail = detail_of(conversation, &rows);
    let checkpoints = detail["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0]["checkpointTurnCount"], json!(7));
    assert_eq!(checkpoints[0]["assistantMessageId"], Value::Null);
    assert_eq!(checkpoints[0]["completedAt"], "2026-07-01T09:00:00.000Z");
}

/// A queued Message has no Turn, and a checkpoint the client cannot revert
/// to would be a control that refuses when pressed.
#[test]
fn a_queued_message_is_no_checkpoint() {
    let mut message = message(1, MessageType::User, "not picked up yet");
    message.turn_number = None;
    let detail = quiet_detail(&transcript(vec![message], Vec::new()));
    assert_eq!(detail["checkpoints"], json!([]));
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
        reverted_to_turn: None,
    }
}

/// The change that names activity `activity_id`, at `position`.
fn activity_change(position: i64, activity_id: &str) -> ConversationChange {
    ConversationChange {
        position,
        message_uuid: None,
        activity_id: Some(activity_id.to_string()),
        reverted_to_turn: None,
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

/// The rows a rewind dropped are gone, so the change that carries it names
/// none of them and this is read off the change itself.
#[test]
fn a_rewind_emits_the_cut_it_made() {
    let cut = ConversationChange {
        position: 7,
        message_uuid: None,
        activity_id: None,
        reverted_to_turn: Some(2),
    };
    let emitted = events(&cut, &transcript(Vec::new(), Vec::new()));
    assert_eq!(emitted.len(), 1);
    assert_keys(&emitted[0], "event", EVENT_REQUIRED, &[]);
    assert_eq!(emitted[0]["type"], "thread.reverted");
    assert_eq!(emitted[0]["payload"]["threadId"], "task-a");
    assert_eq!(emitted[0]["payload"]["turnCount"], json!(2));
}

/// A rewind that kept nothing is `0`, not an absence: the client empties the
/// thread rather than leaving what it holds in place.
#[test]
fn a_rewind_to_nothing_says_zero() {
    let cut = ConversationChange {
        position: 7,
        message_uuid: None,
        activity_id: None,
        reverted_to_turn: Some(0),
    };
    let emitted = events(&cut, &transcript(Vec::new(), Vec::new()));
    assert_eq!(emitted[0]["type"], "thread.reverted");
    assert_eq!(emitted[0]["payload"]["turnCount"], json!(0));
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
/// A resuming session re-presents the call under a new Message. The call is
/// stored once — `(task_id, tool_use_id)` is unique — but the panel row beside
/// it carries no `tool_use_id` and is written again, so from then on the two
/// live on different Messages.
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
