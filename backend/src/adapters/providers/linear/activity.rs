//! Outbound Linear activity: what the running Agent tells the session panel.
//!
//! Every Linear agent session has a panel in the issue view, and this module is
//! the only thing that writes to it after provisioning. The four `notify_*`
//! entry points are called from the event pipeline
//! (`event_handler/consumer.rs`, `event_handler/agent_handlers.rs`) as an Agent
//! produces status, actions, assistant blocks and tool results.
//!
//! # This module imports nothing above it
//!
//! It may reach [`auth`](super::auth), [`subject`](super::subject) and
//! [`agent_api`](super::agent_api), and nothing else in the Adapter — not
//! `ingress`, not `webhook`, not `delivery`, and not one of the five workflow
//! modules. That direction is deliberate and is spec Decision 6: the event
//! pipeline depends on this module, so every compile of the event pipeline
//! would otherwise pull in session provisioning, the issue workflows and the
//! close path. Keeping the dependency one-directional is what makes this the
//! one module of the Linear Adapter the pipeline gets to see.
//!
//! [`ActivityKind`] is re-exported for the same reason: a caller that has to
//! name `agent_api` to say which kind of activity it wants is a second import,
//! and the pipeline is meant to have exactly one.
//!
//! # Everything here is best-effort
//!
//! These run on the event-consumer hot path, downstream of work that has
//! already happened. A failure to tell Linear about it — no binding, a missing
//! connection, a network error, a GraphQL rejection — is logged and swallowed:
//! there is no caller that could act on it, and an error bubbling out of here
//! would stall a pipeline whose real work already succeeded.

use tracing::{debug, warn};

use crate::state::AppState;

use super::agent_api::LinearAgentApi;

pub(crate) use super::agent_api::ActivityKind;

/// Resolve every active Linear `(api, session_id)` pair bound to `task_id`.
/// Internal helper used by every fire-and-forget activity emitter — keeps the
/// task-binding + workspace-OAuth lookup in one place.
///
/// Returns an empty vec on any DB error (logged) so callers can skip emission
/// silently without bubbling errors into the event-handler hot path.
async fn resolve_linear_sessions_for_task(
    state: &AppState,
    task_id: &str,
) -> Vec<(LinearAgentApi, String)> {
    let rows = match super::subject::bindings_in_scope(
        state,
        super::subject::BindingScope::SessionsOfTask(task_id),
    )
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(task_id, error = %e, "linear sessions: binding lookup failed");
            return Vec::new();
        }
    };

    let mut sessions = Vec::with_capacity(rows.len());
    for binding in rows {
        let Some(session_id) = binding
            .destination_config
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        let Some(api) = super::auth::api_for_binding(state, &binding).await else {
            continue;
        };

        sessions.push((api, session_id));
    }
    sessions
}

/// Best-effort emit a Linear activity on every active Linear session bound to
/// `task_id`. Fire-and-forget: any failure (no binding, missing connection,
/// network error, GraphQL rejection) is logged and swallowed — callers in the
/// event-consumer hot path must not be affected.
pub(crate) async fn notify_linear_status(
    state: AppState,
    task_id: String,
    kind: ActivityKind,
    body: String,
    ephemeral: bool,
) {
    for (api, session_id) in resolve_linear_sessions_for_task(&state, &task_id).await {
        if let Err(e) = api
            .agent_activity_create(&session_id, kind, &body, ephemeral)
            .await
        {
            warn!(
                task_id,
                session_id,
                error = %e,
                "notify_linear_status: agent_activity_create failed"
            );
        }
    }
}

/// Sibling of [`notify_linear_status`] that emits an `action` activity (with
/// optional `result`) on every active Linear session bound to `task_id`.
/// Same best-effort semantics: failures are logged and swallowed so callers
/// in the event-consumer hot path are unaffected.
pub(crate) async fn notify_linear_action(
    state: AppState,
    task_id: String,
    action: String,
    parameter: String,
    result: Option<String>,
    ephemeral: bool,
) {
    for (api, session_id) in resolve_linear_sessions_for_task(&state, &task_id).await {
        if let Err(e) = api
            .agent_activity_create_action(
                &session_id,
                &action,
                &parameter,
                result.as_deref(),
                ephemeral,
            )
            .await
        {
            warn!(
                task_id,
                session_id,
                error = %e,
                "notify_linear_action: agent_activity_create_action failed"
            );
        }
    }
}

/// Best-effort: walk an assistant message's `content` block array and emit one
/// Linear activity per block — `text` blocks become ephemeral `thought`s, and
/// `tool_use` blocks become ephemeral `action`s with a one-line parameter
/// summary derived from `tool_input`.
///
/// Caller is responsible for guarding on `payload.type == "assistant"`; this
/// function will silently no-op on non-assistant content shapes.
///
/// Skips:
/// - `tool_result` blocks (system/user role messages).
/// - `text` blocks shorter than 4 chars after trim (Claude connector tokens).
pub(crate) async fn notify_linear_message_activities(
    state: AppState,
    task_id: String,
    message: serde_json::Value,
) {
    let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) else {
        return;
    };
    if blocks.is_empty() {
        return;
    }

    let sessions = resolve_linear_sessions_for_task(&state, &task_id).await;
    if sessions.is_empty() {
        return;
    }

    for block in blocks {
        let block_type = block
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        match block_type {
            "text" => {
                let text = block
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim();
                if text.chars().count() < 4 {
                    continue;
                }
                for (api, session_id) in &sessions {
                    if let Err(e) = api
                        .agent_activity_create(session_id, ActivityKind::Thought, text, false)
                        .await
                    {
                        warn!(
                            task_id,
                            session_id,
                            error = %e,
                            "notify_linear_message_activities: thought failed"
                        );
                    }
                }
            }
            "tool_use" => {
                let Some(tool_name) = block.get("name").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let empty = serde_json::Value::Null;
                let input = block.get("input").unwrap_or(&empty);
                let parameter = summarize_tool_input(tool_name, input);
                for (api, session_id) in &sessions {
                    if let Err(e) = api
                        .agent_activity_create_action(session_id, tool_name, &parameter, None, true)
                        .await
                    {
                        warn!(
                            task_id,
                            session_id,
                            tool_name,
                            error = %e,
                            "notify_linear_message_activities: action failed"
                        );
                    }
                }
            }
            _ => {
                // tool_result blocks are handled by notify_linear_tool_results
                // on the user-role agent.message sibling; any other block kinds
                // are intentionally ignored.
            }
        }
    }
}

/// Walk a user-role agent message and, for every `tool_result` block, emit a
/// non-ephemeral `action` activity carrying the tool's result. Pairs with the
/// ephemeral pending `action` emitted at `tool_use` time so a Linear viewer
/// sees both the call and its outcome.
///
/// Fire-and-forget: any DB or GraphQL failure is logged and swallowed.
pub(crate) async fn notify_linear_tool_results(
    state: AppState,
    task_id: String,
    message: serde_json::Value,
) {
    let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) else {
        return;
    };
    if blocks.is_empty() {
        return;
    }

    // Collect tool_use_ids up-front so we can skip the sessions lookup when the
    // user message carries no tool_result blocks (e.g. plain text follow-ups).
    let tool_use_ids: Vec<String> = blocks
        .iter()
        .filter_map(|b| {
            let kind = b.get("type").and_then(serde_json::Value::as_str)?;
            if kind != "tool_result" {
                return None;
            }
            b.get("tool_use_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect();
    if tool_use_ids.is_empty() {
        return;
    }

    let sessions = resolve_linear_sessions_for_task(&state, &task_id).await;
    if sessions.is_empty() {
        return;
    }

    for tool_use_id in tool_use_ids {
        let row = match state.messages.find_tool_call(&task_id, &tool_use_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                debug!(
                    task_id,
                    tool_use_id, "notify_linear_tool_results: tool_use lookup returned no row"
                );
                continue;
            }
            Err(e) => {
                debug!(
                    task_id,
                    tool_use_id,
                    error = %e,
                    "notify_linear_tool_results: tool_use lookup failed"
                );
                continue;
            }
        };

        // Race guard: pending ephemeral stays in place until the result row
        // exists. Without truncated result content there is nothing to surface.
        let Some(result_text) = row.result_content.as_deref() else {
            continue;
        };

        let parameter = summarize_tool_input(&row.tool_name, &row.tool_input);
        let result = truncate_result_for_linear(result_text, row.is_error.unwrap_or(false));

        for (api, session_id) in &sessions {
            if let Err(e) = api
                .agent_activity_create_action(
                    session_id,
                    &row.tool_name,
                    &parameter,
                    Some(&result),
                    false,
                )
                .await
            {
                warn!(
                    task_id,
                    session_id,
                    tool_name = %row.tool_name,
                    error = %e,
                    "notify_linear_tool_results: action failed"
                );
            }
        }
    }
}

/// Maximum length of the `parameter` field in a Linear `action` activity.
/// Linear's UI renders activities in a narrow row, so longer strings get
/// truncated visually anyway.
const PARAMETER_MAX_LEN: usize = 200;

/// Maximum length of the `result` field in a Linear `action` activity.
/// Tool results (file contents, command output) are often multi-KB; cap to
/// keep Linear's UI legible.
const RESULT_MAX_LEN: usize = 500;

/// One-line summary of a Claude tool-use input suitable for Linear's
/// `action.parameter` slot. Falls through to a JSON dump for unknown tools.
fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    fn s(v: &serde_json::Value, key: &str) -> Option<String> {
        v.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }

    let raw = match tool_name {
        "Bash" => s(input, "command").unwrap_or_default(),
        "Edit" | "Write" | "Read" | "NotebookEdit" => s(input, "file_path").unwrap_or_default(),
        "Grep" => {
            let pattern = s(input, "pattern").unwrap_or_default();
            match s(input, "path") {
                Some(p) if !p.is_empty() => format!("{pattern} in {p}"),
                _ => pattern,
            }
        }
        "Glob" => s(input, "pattern").unwrap_or_default(),
        "WebFetch" | "WebSearch" => s(input, "url")
            .or_else(|| s(input, "query"))
            .unwrap_or_default(),
        "Task" | "Agent" => s(input, "description").unwrap_or_default(),
        name if name.starts_with("mcp__") => {
            let suffix = name.rsplit("__").next().unwrap_or(name);
            let detail = s(input, "query").or_else(|| s(input, "name"));
            match detail {
                Some(d) if !d.is_empty() => format!("{suffix}: {d}"),
                _ => suffix.to_string(),
            }
        }
        _ => serde_json::to_string(input).unwrap_or_default(),
    };

    truncate_chars(&raw, PARAMETER_MAX_LEN)
}

/// Truncate `s` to at most `max_chars` Unicode scalar values, appending an
/// ellipsis (`…`) when truncation occurs.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Build the `result` field for a completed Linear `action` activity from a
/// raw tool result string. Truncates to [`RESULT_MAX_LEN`] Unicode scalars
/// and prefixes `"❌ "` when the tool reported an error so the failure is
/// obvious at a glance in Linear's UI.
fn truncate_result_for_linear(text: &str, is_error: bool) -> String {
    let trimmed = text.trim();
    let body = truncate_chars(trimmed, RESULT_MAX_LEN);
    if is_error {
        format!("❌ {body}")
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_tool_input_bash_command() {
        let input = serde_json::json!({ "command": "cargo check" });
        assert_eq!(summarize_tool_input("Bash", &input), "cargo check");
    }

    #[test]
    fn summarize_tool_input_edit_path() {
        let input = serde_json::json!({
            "file_path": "/repo/src/main.rs",
            "old_string": "x",
            "new_string": "y"
        });
        assert_eq!(summarize_tool_input("Edit", &input), "/repo/src/main.rs");
        assert_eq!(summarize_tool_input("Read", &input), "/repo/src/main.rs");
        assert_eq!(summarize_tool_input("Write", &input), "/repo/src/main.rs");
    }

    #[test]
    fn summarize_tool_input_grep_with_path() {
        let input = serde_json::json!({ "pattern": "TODO", "path": "backend/src" });
        assert_eq!(summarize_tool_input("Grep", &input), "TODO in backend/src");
    }

    #[test]
    fn summarize_tool_input_grep_without_path() {
        let input = serde_json::json!({ "pattern": "TODO" });
        assert_eq!(summarize_tool_input("Grep", &input), "TODO");
    }

    #[test]
    fn summarize_tool_input_mcp_tool() {
        let input = serde_json::json!({ "query": "rust async" });
        assert_eq!(
            summarize_tool_input("mcp__context7__search", &input),
            "search: rust async"
        );
    }

    #[test]
    fn summarize_tool_input_unknown_falls_back_to_json() {
        let input = serde_json::json!({ "foo": "bar" });
        let out = summarize_tool_input("Mystery", &input);
        assert!(out.contains("\"foo\""));
        assert!(out.contains("\"bar\""));
    }

    #[test]
    fn summarize_tool_input_truncates_oversized() {
        let long = "x".repeat(500);
        let input = serde_json::json!({ "command": long });
        let out = summarize_tool_input("Bash", &input);
        assert_eq!(out.chars().count(), PARAMETER_MAX_LEN);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_result_for_linear_short_ok() {
        let out = truncate_result_for_linear("hello world", false);
        assert_eq!(out, "hello world");
    }

    #[test]
    fn truncate_result_for_linear_trims_whitespace() {
        let out = truncate_result_for_linear("  hello  \n", false);
        assert_eq!(out, "hello");
    }

    #[test]
    fn truncate_result_for_linear_prefixes_error() {
        let out = truncate_result_for_linear("file not found", true);
        assert_eq!(out, "❌ file not found");
    }

    #[test]
    fn truncate_result_for_linear_caps_length() {
        let long = "x".repeat(RESULT_MAX_LEN + 50);
        let out = truncate_result_for_linear(&long, false);
        assert_eq!(out.chars().count(), RESULT_MAX_LEN);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_result_for_linear_caps_then_prefixes_error() {
        let long = "x".repeat(RESULT_MAX_LEN + 50);
        let out = truncate_result_for_linear(&long, true);
        // "❌ " prefix added after truncation, so the visible body stays at the cap.
        assert!(out.starts_with("❌ "));
        assert!(out.ends_with('…'));
        // 2 chars ("❌ ") + RESULT_MAX_LEN body — but "❌" is a single char, plus space = 2 chars total prefix.
        assert_eq!(out.chars().count(), RESULT_MAX_LEN + 2);
    }
}
