//! The shell: the listing T3 opens with, and one conversation's row in it.

use serde_json::{Map, Value, json};

use super::issue::thread_issues;
use super::model::model_selection;
use super::pull_request::legacy_linked_pull_request;
use super::pull_request::thread_pull_requests;
use super::scalar::created_at;
use super::scalar::instant;
use super::scalar::non_empty;
use super::scalar::nullable_instant;
use super::session::RUNTIME_MODE;
use super::session::session;
use super::session::worktree_path;
use super::turn::latest_turn;
use crate::AgentModelDefaults;
use crate::dao::Conversation;

/// The fields both shapes of a conversation carry, with the same values.
///
/// `interactionMode` is deliberately absent rather than null, having no
/// separate Moatless state; `archivedAt` is the Task's close time, since
/// archiving a conversation is closing it.
pub(super) fn conversation_row(
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
        "issues": thread_issues(conversation, project_id),
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
        "pinnedAt": nullable_instant(conversation.pinned_at),
        "pinOrderKey": nullable_order_key(conversation.pin_order_key.as_deref()),
        "activeOrderKey": nullable_order_key(conversation.active_order_key.as_deref()),
        "autoSettleDisabledAt": nullable_instant(conversation.auto_settle_disabled_at),
        "visibility": conversation.visibility.as_str(),
        // The Task's own tags, so `hiddenThreadTags` matches what `moat
        // set-tags` wrote. Read-only: `thread.tag.set` is refused.
        "tags": conversation.tags
    }) else {
        unreachable!("a json! object literal is an object")
    };
    row
}

/// An order key as the contract's trimmed non-empty string, or null. The
/// client mints the keys, so a blank one is a stored value the contract would
/// refuse to decode.
fn nullable_order_key(key: Option<&str>) -> Value {
    key.and_then(non_empty).map_or(Value::Null, Value::String)
}
