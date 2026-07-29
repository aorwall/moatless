//! Projection of Moatless rows onto the shapes T3's shell contract expects.
//!
//! Everything T3 calls a field name lives here or in the transport above it.
//! Nothing below this module knows the vocabulary: the input is a `Workspace`
//! row, the output is `serde_json::Value` on the wire.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use tracing::warn;

use super::dao::Conversation;
use super::AgentModelDefaults;
use crate::agent::catalog::{self, AgentKind};
use crate::sandbox::models::ServerConfig;
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

/// Where a Sandbox checks a conversation's repositories out.
///
/// The same string for every conversation, because it is the sandbox image's
/// `WORKSPACE_DIR` rather than anything per-Task. Unlike a project's root this
/// is a label and not an identity — the client only ever displays it — so
/// reporting one shared value collapses nothing.
const SANDBOX_WORKSPACE_DIR: &str = "/opt/moatless/workspace";

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
/// activities, no checkpoints, no proposed plans, and no `deletedAt`.
///
/// Six fields are deliberately absent rather than null. `interactionMode`,
/// `archivedAt`, `settledOverride`, `settledAt`, `snoozedUntil` and `snoozedAt`
/// all carry a decoding default or are optional, and Moatless has nothing
/// honest to put in them: closing a conversation records no timestamp of its
/// own, and settlement and snoozing do not exist here at all. Leaving them out
/// lets the contract fill them.
fn thread(conversation: &Conversation, project_id: &str, defaults: &AgentModelDefaults) -> Value {
    let created = created_at(conversation);
    let latest_turn = latest_turn(conversation);
    json!({
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
        "latestTurn": latest_turn,
        "createdAt": instant(created),
        "updatedAt": instant(conversation.updated_at.unwrap_or(created)),
        "session": session(conversation),
        // T3 counts only messages a person wrote, which is what keeps the
        // agent's own output out of the sidebar's ordering entirely.
        "latestUserMessageAt": nullable_instant(conversation.latest_user_message_at),
        // The contract requires a definite answer and supplies no empty value.
        // Moatless has no approval queue, so the answer is always no.
        "hasPendingApprovals": false,
        "hasPendingUserInput": conversation.pending_question,
        "hasActionableProposedPlan": conversation.pending_plan
    })
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
/// A removed Sandbox has had its working copy reclaimed, so it reports none.
fn worktree_path(conversation: &Conversation) -> Value {
    match conversation.sandbox_desired_state {
        Some(SandboxDesiredState::Running | SandboxDesiredState::Stopped) => {
            json!(SANDBOX_WORKSPACE_DIR)
        }
        Some(SandboxDesiredState::Removed) | None => Value::Null,
    }
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
            codex: Some("gpt-5.5".to_string()),
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
}
