//! Hooks invoked after agent messages are persisted.
//!
//! Message-domain work only: walk the persisted `tool_result` blocks, resolve
//! the tool call each result belongs to, unwrap the agent harness's shell
//! wrapper, normalize line continuations — and return the resulting command
//! lines and their output as targets. Dispatching those targets to the GitHub
//! adapter's shell reconciler (which owns every GitHub-specific behaviour — the
//! GitHub PR status/reaction design) is the server-band caller's job: that
//! `tasks -> adapter` step lives at the call site, not in this hook.
//!
//! Also hosts the derived, UI-only `message.upsert` / `subagentProgress`
//! assembly (server-owned assembly and skeleton-message model).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use moatless_events::{EventType, MessageSubType};
use serde_json::json;
use tracing::{debug, warn};

use super::models::MessageType;
use super::schemas::{SubagentStatus, UiMessage};
use moatless_core::events::EventPublisher;
use crate::file_storage::dao::FileDao;
use crate::message::dao::MessageDao;

/// One shell tool call resolved from a persisted message, ready to hand to the
/// GitHub reconciler: the command line (wrapper stripped, continuations joined)
/// and its result text, keyed by the `tool_use_id` the gh claim is recorded
/// against.
pub struct ShellReconcileTarget {
    pub tool_use_id: String,
    pub cmd: String,
    pub result_text: String,
}

/// Resolve every shell tool call in the just-persisted message to the command
/// line and result the GitHub reconciler needs. This is the message-domain half
/// and stays in the tasks band: walk the `tool_result` blocks, resolve each to
/// its `tool_use` call, unwrap the agent harness's shell wrapper, normalize line
/// continuations. Dispatching the targets to the adapter is the (server-band)
/// caller's job — it owns the `tasks -> adapter` direction, so the parse and the
/// dispatch live on opposite sides of that band edge.
///
/// `message_type` gates the walk to system-role messages (SDK "user" role),
/// where tool results land. Lookups that fail are logged and skipped — the
/// event-handler loop is fire-and-forget.
pub async fn extract_shell_reconcile_targets(
    messages: &Arc<dyn MessageDao>,
    task_id: &str,
    message_type: MessageType,
    content: &serde_json::Value,
) -> Vec<ShellReconcileTarget> {
    let mut targets = Vec::new();

    // Tool results land on system-role messages (SDK "user" role).
    if message_type != MessageType::System {
        return targets;
    }

    let Some(blocks) = content.as_array() else {
        return targets;
    };

    for block in blocks {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        let Some(tool_use_id) = block.get("tool_use_id").and_then(|v| v.as_str()) else {
            continue;
        };

        let tool_call = match messages.find_tool_call(task_id, tool_use_id).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                debug!(task_id, tool_use_id, "tool_use not found for tool_result");
                continue;
            }
            Err(e) => {
                warn!(task_id, tool_use_id, error = %e, "find_tool_call failed");
                continue;
            }
        };

        let raw_cmd = tool_call
            .tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if raw_cmd.is_empty() {
            continue;
        }

        // Claude Code's `Bash` and Codex's `codex.shell` use the same
        // tool_input.command field, but Codex always wraps the command as
        // `/bin/bash -lc '<inner>'` — unwrap it so the reconciler's command
        // parsers can match.
        let cmd: &str = match tool_call.tool_name.as_str() {
            "Bash" => raw_cmd,
            "codex.shell" => unwrap_shell_wrapper(raw_cmd),
            _ => continue,
        };
        // Backslash-newline continuations (`cd x && \⏎gh pr create …`) defeat
        // the reconciler's whitespace tokenizers — join them before handing
        // the command over.
        let cmd = normalize_line_continuations(cmd);

        let result_text = block.get("content").and_then(|v| v.as_str()).unwrap_or("");

        // The claim is recorded on the call, not on the Message it was made
        // from: a turn that only ran the command said nothing and wrote no
        // Message row for the claim to land on.
        targets.push(ShellReconcileTarget {
            tool_use_id: tool_use_id.to_string(),
            cmd: cmd.into_owned(),
            result_text: result_text.to_string(),
        });
    }

    targets
}

// =============================================================================
// message.upsert — derived, UI-only assembled Message events (server-owned assembly and skeleton-message model)
// =============================================================================

/// Inject `subagentSummary` onto every `Agent` tool-use block in `messages`
/// from one batched `GROUP BY parent_uuid` aggregate. Shared by the read path
/// (`GET /messages`) and the live top-level emit so the collapsed `Agent` card
/// has a single source of truth. Fire-and-forget: an aggregate failure leaves
/// the summaries absent (the card falls back to whatever it last had).
pub(crate) async fn hydrate_subagent_summaries_from_dao(
    message_dao: &Arc<dyn MessageDao>,
    task_id: &str,
    messages: &mut [UiMessage],
) {
    let ids = super::transform::agent_tool_use_ids(messages);
    if ids.is_empty() {
        return;
    }
    let aggregates = match message_dao.get_subagent_aggregates(task_id, &ids).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to load subagent aggregates");
            return;
        }
    };
    let map: HashMap<String, (u32, u32)> = aggregates
        .into_iter()
        .map(|a| {
            (
                a.parent_uuid,
                (
                    u32::try_from(a.message_count).unwrap_or(0),
                    u32::try_from(a.tokens).unwrap_or(0),
                ),
            )
        })
        .collect();
    super::transform::inject_subagent_summaries(messages, &map);
}

/// Re-assemble the top-level Message **affected** by a change to `message_uuid`
/// — resolving a folded `tool_result`/system row to its parent assistant
/// Message — and publish a derived `message.upsert` carrying the assembled
/// `UiMessage`. UI-only — the event-handler consumer gates `message.*` out of
/// `task_events`, and the SSE consumer relays it to clients. Fire-and-forget:
/// assembly failures are logged and the read path hydrates on next open.
///
/// Named over its ports (event publisher + message/file DAOs) rather than a full
/// `AppState` so the server-band caller passes exactly the ports it holds.
pub async fn emit_message_upsert_from_daos(
    events: Option<&Arc<dyn EventPublisher>>,
    message_dao: &Arc<dyn MessageDao>,
    file_dao: Option<&Arc<dyn FileDao>>,
    task_id: &str,
    message_uuid: &str,
) {
    let Some(events) = events else {
        return;
    };

    let loaded = match message_dao
        .get_message_for_assembly(task_id, message_uuid)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to load rows for message.upsert assembly");
            return;
        }
    };
    let Some(((messages, contexts, blocks), target_uuid)) = loaded else {
        // The changed row folds into a parent not yet resolvable, is a subagent
        // row (handled by `subagentProgress`), or is unknown. All are covered by
        // the read-path hydration.
        debug!(task_id = %task_id, uuid = %message_uuid, "no affected Message resolved; skipping upsert");
        return;
    };

    // Legacy file-context fallback, mirroring the read path.
    let legacy_file_ids: Vec<String> = messages
        .iter()
        .flat_map(super::models::TaskMessage::file_ids)
        .collect();
    let file_metadata = if legacy_file_ids.is_empty() {
        HashMap::new()
    } else if let Some(file_dao) = file_dao {
        file_dao
            .find_by_ids(&legacy_file_ids)
            .await
            .unwrap_or_else(|e| {
                warn!(task_id = %task_id, error = %e, "Failed to batch-load file metadata for message.upsert");
                HashMap::new()
            })
    } else {
        HashMap::new()
    };

    let mut assembled =
        super::transform::transform_to_ui_with_files(&messages, &contexts, &blocks, &file_metadata);
    // Inject subagent summaries onto `Agent` blocks via the shared helper.
    hydrate_subagent_summaries_from_dao(message_dao, task_id, &mut assembled).await;

    let Some(message) = assembled.into_iter().find(|m| m.uuid == target_uuid) else {
        debug!(task_id = %task_id, uuid = %target_uuid, "affected Message not found in assembly; skipping upsert");
        return;
    };

    // Slim shape matches the frontend read path (`summary: true`).
    let slim = crate::message::handlers::strip_heavy_fields(message);
    let payload = match serde_json::to_value(&slim) {
        Ok(v) => v,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to serialize UiMessage for message.upsert");
            return;
        }
    };

    events
        .publish_raw_event(
            task_id,
            EventType::Message.as_str(),
            MessageSubType::Upsert.as_str(),
            None,
            None,
            payload,
        )
        .await;
}

/// Publish a derived `message`/`subagentProgress` event carrying the **absolute**
/// summary for one subagent (`{ toolUseId, messageCount, tokens, status }`).
/// Emitted instead of a parent `message.upsert` when a subagent row is
/// persisted, so the collapsed `Agent` card's count/tokens stay live without
/// ever streaming subagent transcript content. Fire-and-forget.
pub async fn emit_subagent_progress(
    events: Option<&Arc<dyn EventPublisher>>,
    messages: &Arc<dyn MessageDao>,
    task_id: &str,
    tool_use_id: &str,
) {
    let Some(events) = events else {
        return;
    };

    let aggregates = match messages
        .get_subagent_aggregates(task_id, std::slice::from_ref(&tool_use_id.to_string()))
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id = %task_id, tool_use_id, error = %e, "Failed to load subagent aggregate for progress");
            return;
        }
    };
    let (message_count, tokens) = aggregates.first().map_or((0, 0), |a| {
        (
            u32::try_from(a.message_count).unwrap_or(0),
            u32::try_from(a.tokens).unwrap_or(0),
        )
    });

    // Status: the one shared rule keyed on the outer `Agent` tool_result, so the
    // live path can't drift from the read path's `inject_subagent_summaries`.
    let status = match messages.find_tool_call(task_id, tool_use_id).await {
        Ok(Some(call)) => {
            let present = call.result_content.is_some() || call.is_error.is_some();
            SubagentStatus::from_outer_result(present, call.is_error == Some(true))
        }
        _ => SubagentStatus::Running,
    };

    let payload = json!({
        "toolUseId": tool_use_id,
        "messageCount": message_count,
        "tokens": tokens,
        "status": status.as_str(),
    });

    events
        .publish_raw_event(
            task_id,
            EventType::Message.as_str(),
            MessageSubType::SubagentProgress.as_str(),
            None,
            None,
            payload,
        )
        .await;
}

/// Strip a `<shell> -<flags> '<inner>'` wrapper that Codex always wraps
/// shell commands in (production form: `/bin/bash -lc '<inner>'`). Falls
/// through unchanged if no recognized wrapper is present.
pub fn unwrap_shell_wrapper(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    const PREFIXES: &[&str] = &[
        "/bin/bash -lc ",
        "/bin/bash -c ",
        "/bin/sh -lc ",
        "/bin/sh -c ",
        "bash -lc ",
        "bash -c ",
        "sh -c ",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            if let Some(inner) = rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                return inner;
            }
            if let Some(inner) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                return inner;
            }
            return rest;
        }
    }
    cmd
}

/// Join shell line continuations (`\` followed by a newline) into a single
/// space so the whitespace tokenizers downstream see one logical command line.
pub fn normalize_line_continuations(cmd: &str) -> Cow<'_, str> {
    if cmd.contains("\\\n") || cmd.contains("\\\r\n") {
        Cow::Owned(cmd.replace("\\\r\n", " ").replace("\\\n", " "))
    } else {
        Cow::Borrowed(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_shell_wrapper_strips_bin_bash_lc() {
        assert_eq!(
            unwrap_shell_wrapper("/bin/bash -lc 'gh pr create --title foo'"),
            "gh pr create --title foo"
        );
    }

    #[test]
    fn unwrap_shell_wrapper_strips_double_quoted() {
        assert_eq!(
            unwrap_shell_wrapper("/bin/bash -c \"gh pr view 1\""),
            "gh pr view 1"
        );
    }

    #[test]
    fn unwrap_shell_wrapper_passthrough_when_no_wrapper() {
        assert_eq!(
            unwrap_shell_wrapper("gh pr create --title foo"),
            "gh pr create --title foo"
        );
    }
}
