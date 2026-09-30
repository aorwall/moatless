//! Outbound Linear activity: what the running Agent tells the session panel.
//!
//! Every Linear agent session has a panel in the issue view, and this module is
//! the only thing that writes to it after provisioning, via the four `notify_*`
//! entry points the event pipeline calls.
//!
//! Imports nothing above it in the Adapter except [`auth`](super::auth),
//! [`subject`](super::subject) and [`agent_api`](super::agent_api) — never
//! `ingress`, `webhook`, `delivery`, or a workflow module — since the event
//! pipeline depends on this module and any other edge would pull them into
//! every compile of the pipeline.
//!
//! Everything here is best-effort: a failure to tell Linear (no binding,
//! missing connection, network error, GraphQL rejection) is logged and
//! swallowed, since it runs downstream of work that already succeeded and no
//! caller could act on it.

use std::sync::Arc;

use tracing::{debug, warn};

use moatless_adapter_model::apps::AdapterAppReader;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::identity::AppCredentials;

use moatless_adapter_model::connections::ConnectionLookup;
use moatless_ports::task::TranscriptAppend;
use moatless_types::http::HttpTimeouts;

use super::agent_api::LinearAgentApi;

pub use super::agent_api::ActivityKind;

/// Resolve every active Linear `(api, session_id)` pair bound to `task_id`.
///
/// Returns an empty vec on any DB error (logged) so callers can skip emission
/// silently without bubbling errors into the event-handler hot path.
async fn resolve_linear_sessions_for_task(
    timeouts: HttpTimeouts,
    app_tokens: &Arc<dyn AppCredentials>,
    task_bindings: &Bindings,
    adapter_connections: &dyn ConnectionLookup,
    apps: &dyn AdapterAppReader,
    task_id: &str,
) -> Vec<(LinearAgentApi, String)> {
    let rows = match super::subject::bindings_in_scope(
        task_bindings,
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

        let Some(api) = super::auth::api_for_binding(
            timeouts,
            adapter_connections,
            app_tokens.clone(),
            apps,
            &binding,
        )
        .await
        else {
            continue;
        };

        sessions.push((api, session_id));
    }
    sessions
}

/// Emit a Linear activity on every active Linear session bound to `task_id`.
///
/// Fire-and-forget: any failure is logged and swallowed so callers in the
/// event-consumer hot path are unaffected.
#[expect(
    clippy::too_many_arguments,
    reason = "four owned downward ports (app_tokens, task_bindings, adapter_connections, adapter apps) \
              inverted out of the AppState this fire-and-forget emitter used to \
              take by value, plus its four activity fields"
)]
pub async fn notify_linear_status(
    timeouts: HttpTimeouts,
    app_tokens: Arc<dyn AppCredentials>,
    task_bindings: Arc<Bindings>,
    adapter_connections: Arc<dyn ConnectionLookup>,
    apps: Arc<dyn AdapterAppReader>,
    task_id: String,
    kind: ActivityKind,
    body: String,
    ephemeral: bool,
) {
    for (api, session_id) in resolve_linear_sessions_for_task(
        timeouts,
        &app_tokens,
        task_bindings.as_ref(),
        adapter_connections.as_ref(),
        apps.as_ref(),
        &task_id,
    )
    .await
    {
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
/// optional `result`) instead of a status.
///
/// Same best-effort semantics: failures are logged and swallowed.
#[expect(
    clippy::too_many_arguments,
    reason = "four owned downward ports (app_tokens, task_bindings, adapter_connections, adapter apps) \
              inverted out of the AppState this fire-and-forget emitter used to \
              take by value, plus its four activity fields"
)]
pub async fn notify_linear_action(
    timeouts: HttpTimeouts,
    app_tokens: Arc<dyn AppCredentials>,
    task_bindings: Arc<Bindings>,
    adapter_connections: Arc<dyn ConnectionLookup>,
    apps: Arc<dyn AdapterAppReader>,
    task_id: String,
    action: String,
    parameter: String,
    result: Option<String>,
    ephemeral: bool,
) {
    for (api, session_id) in resolve_linear_sessions_for_task(
        timeouts,
        &app_tokens,
        task_bindings.as_ref(),
        adapter_connections.as_ref(),
        apps.as_ref(),
        &task_id,
    )
    .await
    {
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

/// One Linear activity an assembled Message owes, with the key that makes
/// posting it idempotent: the Message's uuid or the tool-use id, both
/// deterministic across a re-projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePost {
    /// Assistant `text` → a persistent `thought`. `text` is the Message's whole
    /// folded text, which grows as later blocks of the response land; the
    /// subscriber posts only what its ledger has not.
    Thought { key: String, text: String },
    /// A `tool_use` → an ephemeral pending `action`.
    Call {
        key: String,
        tool_name: String,
        parameter: String,
    },
    /// A settled call → a completed `action`, read back from its Activity.
    Result { key: String, tool_use_id: String },
}

impl MessagePost {
    pub fn key(&self) -> &str {
        match self {
            Self::Thought { key, .. } | Self::Call { key, .. } | Self::Result { key, .. } => key,
        }
    }
}

/// Text shorter than this, trimmed, is not posted (Claude connector tokens).
pub const MIN_THOUGHT_CHARS: usize = 4;

/// The activities a `message.upsert` payload shows, in order: each text and
/// tool call of a top-level assistant Message, then each call it settled.
pub fn message_posts(message: &serde_json::Value) -> Vec<MessagePost> {
    let str_of = |v: &serde_json::Value, key: &str| {
        v.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    if str_of(message, "messageType").as_deref() != Some("assistant")
        || message.get("parentUuid").is_some_and(|p| !p.is_null())
    {
        return Vec::new();
    }
    let Some(uuid) = str_of(message, "uuid") else {
        return Vec::new();
    };
    let empty = Vec::new();
    let blocks = message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);
    let mut posts = Vec::new();
    for block in blocks {
        match str_of(block, "type").as_deref() {
            Some("text") => {
                let text = str_of(block, "text").unwrap_or_default();
                if !text.trim().is_empty() {
                    posts.push(MessagePost::Thought {
                        key: format!("text:{uuid}"),
                        text,
                    });
                }
            }
            Some("tool_use") => {
                let (Some(id), Some(tool_name)) =
                    (str_of(block, "toolUseId"), str_of(block, "toolName"))
                else {
                    continue;
                };
                let empty = serde_json::Value::Null;
                let input = block.get("toolInput").unwrap_or(&empty);
                posts.push(MessagePost::Call {
                    key: format!("call:{id}"),
                    parameter: summarize_tool_input(&tool_name, input),
                    tool_name,
                });
            }
            _ => {}
        }
    }
    let results = message
        .get("toolResults")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);
    for result in results {
        if let Some(id) = str_of(result, "toolUseId") {
            posts.push(MessagePost::Result {
                key: format!("result:{id}"),
                tool_use_id: id,
            });
        }
    }
    posts
}

/// Post `posts` to every Linear session bound to the task.
///
/// A [`MessagePost::Result`] reads the full call back from its Activity, since
/// the upsert carries only previews; one not yet readable posts nothing.
#[expect(
    clippy::too_many_arguments,
    reason = "the client bounds, five owned downward ports, the task and its posts"
)]
pub async fn notify_linear_message_posts(
    timeouts: HttpTimeouts,
    app_tokens: Arc<dyn AppCredentials>,
    task_bindings: Arc<Bindings>,
    adapter_connections: Arc<dyn ConnectionLookup>,
    apps: Arc<dyn AdapterAppReader>,
    messages: Arc<dyn TranscriptAppend>,
    task_id: String,
    posts: Vec<MessagePost>,
) {
    if posts.is_empty() {
        return;
    }
    let sessions = resolve_linear_sessions_for_task(
        timeouts,
        &app_tokens,
        task_bindings.as_ref(),
        adapter_connections.as_ref(),
        apps.as_ref(),
        &task_id,
    )
    .await;
    if sessions.is_empty() {
        return;
    }

    for post in posts {
        match post {
            MessagePost::Thought { text, .. } => {
                for (api, session_id) in &sessions {
                    if let Err(e) = api
                        .agent_activity_create(session_id, ActivityKind::Thought, &text, false)
                        .await
                    {
                        warn!(task_id, session_id, error = %e, "notify_linear_message_posts: thought failed");
                    }
                }
            }
            MessagePost::Call {
                tool_name,
                parameter,
                ..
            } => {
                for (api, session_id) in &sessions {
                    if let Err(e) = api
                        .agent_activity_create_action(
                            session_id, &tool_name, &parameter, None, true,
                        )
                        .await
                    {
                        warn!(task_id, session_id, tool_name, error = %e, "notify_linear_message_posts: action failed");
                    }
                }
            }
            MessagePost::Result { tool_use_id, .. } => {
                let row = match messages.tool_call(&task_id, &tool_use_id).await {
                    Ok(Some(row)) => row,
                    Ok(None) => {
                        debug!(
                            task_id,
                            tool_use_id, "notify_linear_message_posts: no tool_use row"
                        );
                        continue;
                    }
                    Err(e) => {
                        debug!(task_id, tool_use_id, error = %e, "notify_linear_message_posts: tool_use lookup failed");
                        continue;
                    }
                };
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
                            "notify_linear_message_posts: result failed"
                        );
                    }
                }
            }
        }
    }
}

/// Maximum length of the `parameter` field in a Linear `action` activity.
/// Linear's UI renders activities in a narrow row, so longer strings get
/// truncated visually anyway.
const PARAMETER_MAX_LEN: usize = 200;

/// Maximum length of the `result` field in a Linear `action` activity. Tool
/// results (file contents, command output) are often multi-KB; cap to keep
/// Linear's UI legible.
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

/// Build the `result` field for a completed Linear `action` activity from a raw
/// tool result string.
///
/// Truncates to [`RESULT_MAX_LEN`] Unicode scalars and prefixes `"❌ "` when the
/// tool reported an error so the failure is obvious at a glance.
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
