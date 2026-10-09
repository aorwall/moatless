//! T3 Code projection accessors for shared activity rows.

use std::borrow::Cow;

use serde_json::{Map, Value};

use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityView;

const MAX_PROJECTED_FILES: usize = 12;
const MAX_PROJECTED_FILE_DEPTH: usize = 4;

/// The kind that closes an open question panel on the client, once its call has
/// returned — see `projection::panel_kind`.
pub const USER_INPUT_RESOLVED_KIND: &str = "user-input.resolved";

pub trait T3ActivityViewExt {
    fn t3_summary(&self) -> Cow<'_, str>;
    fn t3_tone(&self) -> &'static str;
    fn t3_kind(&self) -> &'static str;
    fn t3_payload(&self) -> Map<String, Value>;
}

impl T3ActivityViewExt for ActivityView<'_> {
    fn t3_summary(&self) -> Cow<'_, str> {
        if self.kind() != ActivityKind::ToolCall {
            return Cow::Borrowed(self.summary());
        }
        Cow::Owned(tool_presentation(self.summary(), self.input()).summary)
    }

    fn t3_tone(&self) -> &'static str {
        match self.kind() {
            _ if self.is_error() => "error",
            // The tone T3 gives its own `task.*` rows.
            ActivityKind::ToolCall if is_subagent_call(self) => "info",
            ActivityKind::ToolCall => "tool",
            // `SandboxWarning` falls through to `info`, which is what upstream
            // assigns `runtime.warning`; its amber comes from the kind, not the
            // tone.
            ActivityKind::Thinking
            | ActivityKind::SandboxStatus
            | ActivityKind::SandboxWarning
            | ActivityKind::SandboxError
            | ActivityKind::UserInputRequested
            | ActivityKind::UserInputResolved
            | ActivityKind::PlanUpdated
            | ActivityKind::ContextWindowUpdated
            | ActivityKind::AgentSwitched
            | ActivityKind::ArtifactShown
            | ActivityKind::Compaction => "info",
        }
    }

    fn t3_kind(&self) -> &'static str {
        match self.kind() {
            // A subagent is a task the client watches run, not a call it
            // watches return: the row keeps its id, so the progress it reports
            // and the completion that follows land on the one row rather than
            // adding another.
            ActivityKind::ToolCall if is_subagent_call(self) => {
                if self.is_settled() {
                    "task.completed"
                } else {
                    "task.progress"
                }
            }
            ActivityKind::ToolCall if self.is_settled() => "tool.completed",
            ActivityKind::ToolCall => "tool.updated",
            // Severity in T3's work log is carried by the kind: `runtime.error`
            // colours a heading red, `runtime.warning` amber, and everything
            // else on `task.progress` is a plain running-agent row.
            ActivityKind::SandboxError => "runtime.error",
            // Amber, like a recovered stall: the run goes on, on another agent.
            // A person's own pick is no warning.
            ActivityKind::AgentSwitched if self.payload()["chosen"] == true => "task.progress",
            ActivityKind::SandboxWarning | ActivityKind::AgentSwitched => "runtime.warning",
            ActivityKind::Thinking
            | ActivityKind::SandboxStatus
            | ActivityKind::Compaction
            | ActivityKind::ArtifactShown => "task.progress",
            // The four the client derives a panel from rather than a work-log
            // row. Each spelling is the one that client's own deriver matches
            // on, which is why they are dotted where the column is not.
            ActivityKind::UserInputRequested => "user-input.requested",
            ActivityKind::UserInputResolved => USER_INPUT_RESOLVED_KIND,
            ActivityKind::PlanUpdated => "turn.plan.updated",
            ActivityKind::ContextWindowUpdated => "context-window.updated",
        }
    }

    fn t3_payload(&self) -> Map<String, Value> {
        let mut payload = match self.payload() {
            Value::Object(fields) => fields.clone(),
            _ => Map::new(),
        };

        if self.kind() == ActivityKind::ToolCall {
            let presentation = tool_presentation(self.summary(), self.input());
            let data = project_tool_data(&payload, self.summary(), presentation.item_type);
            if !data.is_empty() {
                payload.insert("data".into(), Value::Object(data));
            }
            payload.insert(
                "itemType".into(),
                Value::String(presentation.item_type.into()),
            );
            match presentation.detail {
                Some(detail) => payload.insert("detail".into(), Value::String(detail)),
                None => payload.remove("detail"),
            };
            // T3 stores the provider title as the activity summary. Its
            // ingestion payload intentionally does not duplicate it.
            payload.remove("title");
            // A `task.*` row is headed by its payload summary and takes the
            // brief as the line beside it. Without one the brief would be the
            // heading and the row would lose the word for what it is.
            if is_subagent_call(self) {
                payload.insert("summary".into(), Value::String(presentation.summary));
                stamp_agent_identity(&mut payload, self);
            }
        } else if let Some(detail) = self.detail() {
            payload.insert("detail".into(), Value::String(detail.to_string()));
        }
        // The thought itself renders only as a V2 reasoning item; this
        // snapshot is re-sent whole on every move.
        if self.kind() == ActivityKind::Thinking {
            payload.remove("text");
            payload.remove("truncated");
        }
        payload.remove("input");
        payload.remove("result");
        payload.remove("typedResult");
        payload.remove("command");
        payload
    }
}

/// The identity the client's Agents surface folds a subagent card from.
///
/// It keys its roster by `taskId`, admits only rows stamped `agentKind:
/// "agent"`, and uses `tool_use_id` as the wire key the subagent's own rows
/// carry as `parent_uuid`.
fn stamp_agent_identity(payload: &mut Map<String, Value>, view: &ActivityView<'_>) {
    let Some(task_id) = view.tool_use_id() else {
        return;
    };
    payload.insert("taskId".into(), Value::String(task_id.to_string()));
    payload.insert("agentKind".into(), Value::String("agent".into()));
    let input = view.input();
    if let Some(role) = string_field(input, "subagent_type") {
        payload.insert("role".into(), Value::String(role));
    }
    if let Some(title) = string_field(input, "description")
        .or_else(|| string_field(input, "prompt").map(|prompt| truncate_chars(&prompt, 200)))
    {
        payload.insert("title".into(), Value::String(title));
    }
}

/// Whether this row is the call that spawned a subagent.
///
/// The subagent's own work never enters the Task's stream, so this row is
/// reported as a task rather than a tool call.
pub fn is_subagent_call(view: &ActivityView<'_>) -> bool {
    view.kind() == ActivityKind::ToolCall
        && t3_item_type(view.summary()) == "collab_agent_tool_call"
}

struct ToolPresentation {
    summary: String,
    item_type: &'static str,
    detail: Option<String>,
}

/// Provider-specific lifecycle presentation, matching the point where T3's
/// Claude and Codex adapters turn raw agent items into runtime events.
fn tool_presentation(tool_name: &str, input: &Value) -> ToolPresentation {
    let item_type = t3_item_type(tool_name);
    if tool_name.starts_with("codex.") {
        return codex_tool_presentation(tool_name, input, item_type);
    }

    ToolPresentation {
        summary: claude_tool_title(item_type).to_string(),
        item_type,
        detail: Some(claude_tool_detail(tool_name, input)),
    }
}

/// The provider normalization T3 applies before its canonical activity reaches
/// persistence. Moatless keeps the provider-original call in storage and uses
/// this classifier only while projecting the T3 wire representation.
pub(crate) fn t3_item_type(tool_name: &str) -> &'static str {
    match tool_name {
        "codex.shell" => return "command_execution",
        "codex.edit" => return "file_change",
        "codex.mcp_tool_call" => return "mcp_tool_call",
        "codex.collab_tool_call" => return "collab_agent_tool_call",
        "codex.web_search" => return "web_search",
        _ => {}
    }

    let normalized = tool_name.to_lowercase();
    if normalized.contains("agent") || normalized == "task" {
        return "collab_agent_tool_call";
    }
    if ["bash", "command", "shell", "terminal"]
        .iter()
        .any(|word| normalized.contains(word))
    {
        return "command_execution";
    }
    if [
        "edit", "write", "file", "patch", "replace", "create", "delete",
    ]
    .iter()
    .any(|word| normalized.contains(word))
    {
        return "file_change";
    }
    if normalized.contains("mcp") {
        return "mcp_tool_call";
    }
    if normalized.contains("websearch") || normalized.contains("web search") {
        return "web_search";
    }
    if normalized.contains("image") {
        return "image_view";
    }
    "dynamic_tool_call"
}

fn codex_tool_presentation(
    tool_name: &str,
    input: &Value,
    item_type: &'static str,
) -> ToolPresentation {
    let summary = match tool_name {
        "codex.shell" => "Ran command".to_string(),
        "codex.edit" => "File change".to_string(),
        "codex.mcp_tool_call" => codex_mcp_title(input),
        "codex.web_search" => "Web search".to_string(),
        // T3's Codex adapter assigns no title to collab lifecycle items, so
        // its ingestion layer uses this fallback.
        "codex.collab_tool_call" => "Tool".to_string(),
        _ => "Tool call".to_string(),
    };
    let detail = match tool_name {
        "codex.shell" => string_field(input, "command"),
        "codex.web_search" => codex_web_search_detail(input),
        "codex.collab_tool_call" => string_field(input, "prompt"),
        _ => None,
    };
    ToolPresentation {
        summary,
        item_type,
        detail,
    }
}

fn codex_mcp_title(input: &Value) -> String {
    match (
        input.get("server").and_then(Value::as_str),
        input.get("tool").and_then(Value::as_str),
    ) {
        (Some(server), Some(tool)) if !server.trim().is_empty() && !tool.trim().is_empty() => {
            format!("{} · {}", server.trim(), tool.trim())
        }
        _ => "MCP tool call".to_string(),
    }
}

fn codex_web_search_detail(input: &Value) -> Option<String> {
    string_field(input, "query").or_else(|| {
        let action = input.get("action")?;
        for key in ["query", "pattern", "url"] {
            if let Some(value) = string_field(action, key) {
                return Some(value);
            }
        }
        action
            .get("queries")?
            .as_array()?
            .iter()
            .find_map(|query| {
                query
                    .as_str()
                    .map(str::trim)
                    .filter(|query| !query.is_empty())
            })
            .map(str::to_string)
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn claude_tool_title(item_type: &str) -> &'static str {
    match item_type {
        "command_execution" => "Command run",
        "file_change" => "File change",
        "mcp_tool_call" => "MCP tool call",
        "collab_agent_tool_call" => "Subagent task",
        "web_search" => "Web search",
        "image_view" => "Image view",
        "dynamic_tool_call" => "Tool call",
        _ => "Item",
    }
}

fn claude_tool_detail(tool_name: &str, input: &Value) -> String {
    if let Some(command) = string_field(input, "command").or_else(|| string_field(input, "cmd")) {
        return format!("{tool_name}: {}", truncate_chars(&command, 400));
    }

    if t3_item_type(tool_name) == "collab_agent_tool_call" {
        let description = string_field(input, "description");
        let prompt = string_field(input, "prompt").map(|value| truncate_chars(&value, 200));
        if let Some(label) = description.or(prompt) {
            return string_field(input, "subagent_type")
                .map_or(label.clone(), |kind| format!("{kind}: {label}"));
        }
    }

    let serialized =
        serde_json::to_string(input).unwrap_or_else(|_| "[unserializable input]".into());
    if serialized.chars().count() <= 400 {
        return format!("{tool_name}: {serialized}");
    }
    format!("{tool_name}: {}...", truncate_chars(&serialized, 397))
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// Keeps the small pieces T3's work-log renderer reads; the full input and
/// result stay in storage, mirroring the Codex/Claude/MCP-specific projections
/// T3 itself applies.
fn project_tool_data(
    payload: &Map<String, Value>,
    tool_name: &str,
    item_type: &str,
) -> Map<String, Value> {
    let stored_data = payload.get("data").and_then(Value::as_object);
    let mut projected = Map::new();

    for key in ["toolCallId", "kind"] {
        if let Some(value) = stored_data.and_then(|data| data.get(key)) {
            projected.insert(key.into(), value.clone());
        }
    }

    if item_type == "mcp_tool_call" {
        project_mcp_data(&mut projected, payload, tool_name);
        return projected;
    }

    if let Some(command) = stored_data
        .and_then(|data| data.get("command"))
        .or_else(|| {
            tool_name
                .starts_with("codex.")
                .then(|| payload.get("input").and_then(|input| input.get("command")))
                .flatten()
        })
    {
        projected.insert("command".into(), command.clone());
    }

    let mut files = Vec::new();
    for value in [
        payload.get("input"),
        payload.get("result"),
        payload.get("typedResult"),
    ]
    .into_iter()
    .flatten()
    {
        collect_changed_files(value, &mut files, 0);
    }
    if !files.is_empty() {
        projected.insert(
            "files".into(),
            Value::Array(
                files
                    .into_iter()
                    .map(|path| {
                        Value::Object(Map::from_iter([("path".into(), Value::String(path))]))
                    })
                    .collect(),
            ),
        );
    }

    projected
}

fn project_mcp_data(
    projected: &mut Map<String, Value>,
    payload: &Map<String, Value>,
    tool_name: &str,
) {
    let input = payload.get("input").cloned().unwrap_or(Value::Null);
    let result = payload
        .get("typedResult")
        .or_else(|| payload.get("result"))
        .cloned();
    if tool_name == "codex.mcp_tool_call" {
        let mut item = input.as_object().cloned().unwrap_or_default();
        item.insert("type".into(), Value::String("mcpToolCall".into()));
        if let Some(result) = result {
            item.insert("result".into(), result);
        }
        projected.insert("item".into(), Value::Object(item));
        return;
    }

    projected.insert("toolName".into(), Value::String(tool_name.to_string()));
    projected.insert("input".into(), input);
    if let Some(result) = result {
        projected.insert("result".into(), result);
    }
}

fn collect_changed_files(value: &Value, files: &mut Vec<String>, depth: usize) {
    if depth > MAX_PROJECTED_FILE_DEPTH || files.len() >= MAX_PROJECTED_FILES {
        return;
    }
    if let Some(values) = value.as_array() {
        for value in values {
            collect_changed_files(value, files, depth + 1);
            if files.len() >= MAX_PROJECTED_FILES {
                return;
            }
        }
        return;
    }
    let Some(fields) = value.as_object() else {
        return;
    };

    for key in [
        "path",
        "filePath",
        "relativePath",
        "filename",
        "newPath",
        "oldPath",
    ] {
        let Some(path) = fields.get(key).and_then(Value::as_str) else {
            continue;
        };
        let path = path.trim();
        if !path.is_empty() && !files.iter().any(|held| held == path) {
            files.push(path.to_string());
            if files.len() >= MAX_PROJECTED_FILES {
                return;
            }
        }
    }

    for key in [
        "item",
        "result",
        "input",
        "data",
        "changes",
        "files",
        "edits",
        "patch",
        "patches",
        "operations",
    ] {
        if let Some(nested) = fields.get(key) {
            collect_changed_files(nested, files, depth + 1);
            if files.len() >= MAX_PROJECTED_FILES {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use moatless_types::activity::Activity;
    use moatless_types::activity::ActivityStatus;
    use serde_json::json;

    fn stored(kind: ActivityKind, payload: Value) -> Activity {
        Activity {
            activity_id: "msg-1:0".to_string(),
            task_id: "task-1".to_string(),
            position: 1,
            ordinal: 0,
            message_uuid: Some("msg-1".to_string()),
            parent_uuid: None,
            kind,
            status: ActivityStatus::Completed,
            tool_use_id: None,
            summary: "summary".to_string(),
            payload,
            detail: None,
            is_error: false,
            created_at: Utc::now(),
        }
    }

    /// Each spelling is the one the client's own deriver matches on. A kind
    /// that reaches it under any other name renders as nothing at all.
    #[test]
    fn every_kind_reaches_the_client_under_the_name_it_matches_on() {
        for (kind, expected) in [
            (ActivityKind::Thinking, "task.progress"),
            (ActivityKind::SandboxStatus, "task.progress"),
            (ActivityKind::SandboxWarning, "runtime.warning"),
            (ActivityKind::SandboxError, "runtime.error"),
            (ActivityKind::UserInputRequested, "user-input.requested"),
            (ActivityKind::UserInputResolved, "user-input.resolved"),
            (ActivityKind::PlanUpdated, "turn.plan.updated"),
            (ActivityKind::ContextWindowUpdated, "context-window.updated"),
            (ActivityKind::Compaction, "task.progress"),
            (ActivityKind::AgentSwitched, "runtime.warning"),
            (ActivityKind::ArtifactShown, "task.progress"),
        ] {
            let row = stored(kind, json!({}));
            assert_eq!(ActivityView::new(&row).t3_kind(), expected);
        }

        let mut call = stored(ActivityKind::ToolCall, json!({}));
        call.status = ActivityStatus::InProgress;
        assert_eq!(ActivityView::new(&call).t3_kind(), "tool.updated");
        call.status = ActivityStatus::Completed;
        assert_eq!(ActivityView::new(&call).t3_kind(), "tool.completed");
    }

    /// A fallback's switch is amber; a person's own pick is a plain row.
    #[test]
    fn a_chosen_agent_switch_is_no_warning() {
        let chosen = stored(ActivityKind::AgentSwitched, json!({ "chosen": true }));
        assert_eq!(ActivityView::new(&chosen).t3_kind(), "task.progress");
        let fallback = stored(ActivityKind::AgentSwitched, json!({ "chosen": false }));
        assert_eq!(ActivityView::new(&fallback).t3_kind(), "runtime.warning");
    }

    /// A tool call's raw input/result never reach this contract since it
    /// re-sends its whole snapshot each move; a panel's own payload carries
    /// what a tool call strips, which is why the questions survive.
    #[test]
    fn a_panels_payload_survives_the_strip_a_calls_input_does_not() {
        let call = stored(
            ActivityKind::ToolCall,
            json!({ "title": "AskUserQuestion", "input": { "questions": [] }, "result": "ok" }),
        );
        let stripped = ActivityView::new(&call).t3_payload();
        assert!(stripped.get("input").is_none());
        assert!(stripped.get("result").is_none());

        let asked = stored(
            ActivityKind::UserInputRequested,
            json!({
                "requestId": "tu-1",
                "questions": [{ "id": "Which?", "question": "Which?" }]
            }),
        );
        let payload = ActivityView::new(&asked).t3_payload();
        assert_eq!(payload["requestId"], "tu-1");
        assert_eq!(payload["questions"][0]["id"], "Which?");
    }

    #[test]
    fn a_tool_call_projects_the_command_and_changed_files_the_t3_client_reads() {
        let mut call = stored(
            ActivityKind::ToolCall,
            json!({
                "title": "codex.shell",
                "itemType": "command_execution",
                "data": { "toolCallId": "tu-1", "ignored": "bulk" },
                "command": "cargo test",
                "input": {
                    "command": "cargo test",
                    "file_path": "src/main.rs",
                    "changes": [
                        { "oldPath": "src/old.rs", "newPath": "src/new.rs" },
                        { "path": "src/main.rs" }
                    ],
                    "content": "the complete replacement file stays in storage"
                },
                "result": { "changes": [{ "path": "src/result.rs", "diff": "large diff" }] },
                "typedResult": { "files": [{ "filePath": "src/typed.rs" }] }
            }),
        );
        call.summary = "codex.shell".to_string();

        let projected = ActivityView::new(&call).t3_payload();
        assert_eq!(projected["data"]["command"], "cargo test");
        assert_eq!(projected["data"]["toolCallId"], "tu-1");
        assert_eq!(
            projected["data"]["files"],
            json!([
                { "path": "src/new.rs" },
                { "path": "src/old.rs" },
                { "path": "src/main.rs" },
                { "path": "src/result.rs" },
                { "path": "src/typed.rs" }
            ])
        );
        assert!(projected.get("command").is_none());
        assert!(projected["data"].get("ignored").is_none());
        for key in ["input", "result", "typedResult"] {
            assert!(projected.get(key).is_none());
            assert!(
                call.payload.get(key).is_some(),
                "stored `{key}` is untouched"
            );
        }
    }

    #[test]
    fn claude_bash_uses_the_same_presentation_as_t3s_claude_adapter() {
        let mut call = stored(
            ActivityKind::ToolCall,
            json!({
                "title": "Bash",
                "itemType": "dynamic_tool_call",
                "data": { "toolCallId": "tu-1" },
                "input": { "command": "cargo test" }
            }),
        );
        call.summary = "Bash".to_string();
        call.detail = Some("test output must not replace the request detail".to_string());

        let view = ActivityView::new(&call);
        let payload = view.t3_payload();
        assert_eq!(view.t3_summary(), "Command run");
        assert_eq!(payload["itemType"], "command_execution");
        assert_eq!(payload["detail"], "Bash: cargo test");
        assert!(payload["data"].get("command").is_none());
        assert!(payload.get("title").is_none());
        assert_eq!(call.summary, "Bash");
        assert_eq!(call.payload["title"], "Bash");
        assert_eq!(call.payload["itemType"], "dynamic_tool_call");
        assert_eq!(call.payload["input"]["command"], "cargo test");
        assert_eq!(
            call.detail.as_deref(),
            Some("test output must not replace the request detail")
        );
    }

    /// The spawning call itself, as T3's Claude adapter presents it: the tools
    /// it classifies as agent work all reach the client under one item type and
    /// one title, and the detail is the subagent's kind and its brief.
    #[test]
    fn a_subagent_call_uses_the_same_presentation_as_t3s_claude_adapter() {
        for name in ["Task", "Agent", "general-purpose-agent"] {
            let mut call = stored(
                ActivityKind::ToolCall,
                json!({
                    "title": name,
                    "itemType": "dynamic_tool_call",
                    "data": { "toolCallId": "tu-1" },
                    "input": {
                        "subagent_type": "code-reviewer",
                        "description": "Audit the SQL changes",
                        "prompt": "Read every migration and report"
                    }
                }),
            );
            call.summary = name.to_string();

            let view = ActivityView::new(&call);
            let payload = view.t3_payload();
            assert_eq!(payload["itemType"], "collab_agent_tool_call", "{name}");
            assert_eq!(view.t3_summary(), "Subagent task", "{name}");
            assert_eq!(payload["detail"], "code-reviewer: Audit the SQL changes");
        }
    }

    /// The client's Agents surface admits a row only when both halves of the
    /// stamp are on it: the roster is keyed by `taskId`, and a row without
    /// `agentKind: "agent"` is background work that stays in the work log.
    #[test]
    fn a_subagent_call_carries_the_identity_the_agents_surface_folds_on() {
        let mut call = stored(
            ActivityKind::ToolCall,
            json!({
                "input": {
                    "subagent_type": "code-reviewer",
                    "description": "Audit the SQL changes",
                    "prompt": "Read every migration and report"
                }
            }),
        );
        call.summary = "Task".to_string();
        call.tool_use_id = Some("toolu_a1".to_string());

        let payload = ActivityView::new(&call).t3_payload();
        assert_eq!(payload["taskId"], "toolu_a1");
        assert_eq!(payload["agentKind"], "agent");
        assert_eq!(payload["role"], "code-reviewer");
        assert_eq!(payload["title"], "Audit the SQL changes");
    }

    /// The key is the call's own id, so a call stored without one is a row the
    /// roster cannot be built from and is left as it was.
    #[test]
    fn a_subagent_call_without_a_tool_use_id_is_not_stamped() {
        let mut call = stored(ActivityKind::ToolCall, json!({ "input": {} }));
        call.summary = "Task".to_string();

        let payload = ActivityView::new(&call).t3_payload();
        assert!(!payload.contains_key("taskId"));
        assert!(!payload.contains_key("agentKind"));
    }

    /// Every other call keeps the shape it had. A stamp on a `Bash` row would
    /// move it out of the work log and into the roster.
    #[test]
    fn an_ordinary_tool_call_carries_no_agent_identity() {
        let mut bash = stored(ActivityKind::ToolCall, json!({ "input": {} }));
        bash.summary = "Bash".to_string();
        bash.tool_use_id = Some("toolu_b1".to_string());

        let payload = ActivityView::new(&bash).t3_payload();
        assert!(!payload.contains_key("taskId"));
        assert!(!payload.contains_key("agentKind"));
        assert!(!payload.contains_key("role"));
    }

    /// A subagent is reported as a task the client watches run.
    ///
    /// This row is all that is seen of it: open it is progress, settled it is a
    /// completion, and a failed one is toned like any other error.
    #[test]
    fn a_subagent_call_is_reported_as_a_task_rather_than_a_tool() {
        let mut call = stored(ActivityKind::ToolCall, json!({ "input": {} }));
        call.summary = "Task".to_string();

        call.status = ActivityStatus::InProgress;
        assert_eq!(ActivityView::new(&call).t3_kind(), "task.progress");
        assert_eq!(ActivityView::new(&call).t3_tone(), "info");

        call.status = ActivityStatus::Completed;
        assert_eq!(ActivityView::new(&call).t3_kind(), "task.completed");
        assert_eq!(ActivityView::new(&call).t3_tone(), "info");

        call.status = ActivityStatus::Failed;
        call.is_error = true;
        assert_eq!(ActivityView::new(&call).t3_kind(), "task.completed");
        assert_eq!(ActivityView::new(&call).t3_tone(), "error");

        // Every other call stays a tool call.
        let mut bash = stored(ActivityKind::ToolCall, json!({ "input": {} }));
        bash.summary = "Bash".to_string();
        bash.status = ActivityStatus::InProgress;
        assert_eq!(ActivityView::new(&bash).t3_kind(), "tool.updated");
        assert_eq!(ActivityView::new(&bash).t3_tone(), "tool");
    }

    /// Ordered as T3's classifier is: `Task` alone is a subagent call, but
    /// `TaskCreate` reaches the `create` arm first and is a file change. Both
    /// clients agree, so the order is kept rather than "corrected".
    #[test]
    fn a_name_that_only_looks_like_agent_work_is_classified_as_t3_classifies_it() {
        assert_eq!(t3_item_type("Task"), "collab_agent_tool_call");
        assert_eq!(t3_item_type("TaskCreate"), "file_change");
        assert_eq!(t3_item_type("TaskList"), "dynamic_tool_call");
    }

    /// Without a `subagent_type` the detail is the brief alone, and the
    /// description is preferred over the prompt.
    #[test]
    fn a_subagent_call_without_a_kind_details_the_brief_alone() {
        let mut call = stored(
            ActivityKind::ToolCall,
            json!({ "input": { "prompt": "Read every migration" } }),
        );
        call.summary = "Task".to_string();

        assert_eq!(
            ActivityView::new(&call).t3_payload()["detail"],
            "Read every migration"
        );
    }

    #[test]
    fn codex_tools_use_the_same_presentations_as_t3s_codex_adapter() {
        for (name, input, summary, item_type, detail) in [
            (
                "codex.shell",
                json!({ "command": "cargo test" }),
                "Ran command",
                "command_execution",
                Some("cargo test"),
            ),
            (
                "codex.edit",
                json!({ "changes": [{ "path": "src/main.rs" }] }),
                "File change",
                "file_change",
                None,
            ),
            (
                "codex.web_search",
                json!({ "query": "Rust serde" }),
                "Web search",
                "web_search",
                Some("Rust serde"),
            ),
            (
                "codex.collab_tool_call",
                json!({ "prompt": "Inspect the parser" }),
                "Tool",
                "collab_agent_tool_call",
                Some("Inspect the parser"),
            ),
        ] {
            let mut call = stored(
                ActivityKind::ToolCall,
                json!({
                    "title": name,
                    "itemType": "dynamic_tool_call",
                    "data": { "toolCallId": "tu-1" },
                    "input": input
                }),
            );
            call.summary = name.to_string();

            let view = ActivityView::new(&call);
            let payload = view.t3_payload();
            assert_eq!(view.t3_summary(), summary, "{name}");
            assert_eq!(payload["itemType"], item_type, "{name}");
            assert_eq!(
                payload.get("detail").and_then(Value::as_str),
                detail,
                "{name}"
            );
            assert!(payload.get("title").is_none(), "{name}");
        }
    }

    #[test]
    fn codex_mcp_uses_the_server_and_tool_title_t3_builds() {
        let mut call = stored(
            ActivityKind::ToolCall,
            json!({
                "title": "codex.mcp_tool_call",
                "itemType": "dynamic_tool_call",
                "data": { "toolCallId": "tu-1" },
                "input": {
                    "server": "github",
                    "tool": "get_issue",
                    "arguments": { "number": 234 }
                },
                "typedResult": { "status": "completed", "content": ["issue"] }
            }),
        );
        call.summary = "codex.mcp_tool_call".to_string();

        let view = ActivityView::new(&call);
        let payload = view.t3_payload();
        assert_eq!(view.t3_summary(), "github · get_issue");
        assert_eq!(payload["itemType"], "mcp_tool_call");
        assert_eq!(payload["data"]["item"]["type"], "mcpToolCall");
        assert_eq!(payload["data"]["item"]["arguments"]["number"], 234);
        assert_eq!(payload["data"]["item"]["result"]["status"], "completed");
    }

    /// The three Sandbox severities reach T3 as the kinds its work log colours:
    /// a plain progress row, an amber warning, a red error. Severity rides the
    /// kind, so a warning keeps tone `info` and takes its amber from the kind.
    #[test]
    fn the_three_sandbox_severities_map_to_the_kinds_t3_colours() {
        let status = stored(ActivityKind::SandboxStatus, json!({}));
        assert_eq!(ActivityView::new(&status).t3_kind(), "task.progress");
        assert_eq!(ActivityView::new(&status).t3_tone(), "info");

        let warning = stored(ActivityKind::SandboxWarning, json!({}));
        assert_eq!(ActivityView::new(&warning).t3_kind(), "runtime.warning");
        assert_eq!(ActivityView::new(&warning).t3_tone(), "info");

        let mut error = stored(ActivityKind::SandboxError, json!({}));
        error.is_error = true;
        assert_eq!(ActivityView::new(&error).t3_kind(), "runtime.error");
        assert_eq!(ActivityView::new(&error).t3_tone(), "error");
    }

    /// The four are things that happened, not things that failed.
    #[test]
    fn a_panel_row_is_toned_as_information() {
        for kind in [
            ActivityKind::UserInputRequested,
            ActivityKind::UserInputResolved,
            ActivityKind::PlanUpdated,
            ActivityKind::ContextWindowUpdated,
        ] {
            let row = stored(kind, json!({}));
            assert_eq!(ActivityView::new(&row).t3_tone(), "info");
        }
    }
}
