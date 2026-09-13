//! The session a conversation's Sandbox presents to the client.

use serde_json::{Value, json};

use super::model::provider_instance;
use super::scalar::timestamp;
use super::turn::running_inputs;
use super::turn::turn_id;
use super::turn::turn_is_open;
use crate::dao::Conversation;
use crate::nominal_path;
use moatless_types::running::{Working, derive_running};
use moatless_types::sandbox::SandboxDesiredState;

/// The runtime mode every conversation reports.
///
/// The contract requires one of four named modes with no empty value; Moatless
/// has no equivalent, and the permissive mode is both the contract's default
/// and an honest description — the agent runs as a process in a pod with no
/// approval gate.
pub(super) const RUNTIME_MODE: &str = "full-access";

/// The conversation's session, or null when no Sandbox was ever requested.
///
/// The contract's session carries what Moatless splits in two: the durable
/// Sandbox lifecycle intent and the agent session running inside it. Absence of
/// a `sandboxes` row is the lazy converged-Stopped default — the one case
/// meaning *no session*, not *a stopped session*.
pub(super) fn session(conversation: &Conversation) -> Value {
    let Some(desired_state) = conversation.sandbox_desired_state() else {
        return Value::Null;
    };
    json!({
        "threadId": conversation.task_id,
        "status": session_status(conversation, desired_state),
        // A *driver kind*, not the catalog's display label. The client decodes
        // this field as one (`ProviderSessionDirectory.ts:27`) and locks the
        // composer to whatever it names, and `ProviderDriverKind` is an open
        // branded slug that a display label also satisfies — so a label here
        // matches no published instance and leaves the composer disabled.
        //
        // The same string as `providerInstanceId` below, because this backend
        // publishes exactly one instance per driver (`providers.rs`).
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
/// route so the two surfaces cannot disagree; T3 has no separate word for a run
/// parked on a person, and spends `ready` on a Sandbox that is up with nothing
/// in flight.
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
/// A removed Sandbox has had its working copy reclaimed; null also tells the
/// client there are no files to ask for, so it falls back to the project's root
/// and gets an answer saying so.
pub(super) fn worktree_path(conversation: &Conversation) -> Value {
    match conversation.sandbox_desired_state() {
        Some(SandboxDesiredState::Running | SandboxDesiredState::Stopped) => {
            json!(nominal_path::nominal_worktree(&conversation.task_id))
        }
        Some(SandboxDesiredState::Removed) | None => Value::Null,
    }
}
