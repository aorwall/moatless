//! The tool calls and plans in a transcript window, as V2 turn items and plan
//! artifacts.
//!
//! A tool call renders as the item its kind names (`t3_item_type`, the same
//! classifier the V1 work log uses). The calls another row already stands for
//! render nothing here: a question is its `UserInputRequested` row, and a todo
//! list its `PlanUpdated` row.

use serde_json::{Map, Value, json};

use super::model::provider_instance;
use super::scalar::instant;
use super::transcript::activity_anchor;
use super::v2_provider::has_started;
use super::v2_provider::root_node_id;
use crate::activity::t3_item_type;
use crate::dao::Conversation;
use crate::transcript_dao::Transcript;
use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityStatus;
use moatless_types::activity::PLAN_TOOL;
use moatless_types::activity::QUESTION_TOOL;
use moatless_types::activity::TODO_TOOL;
use moatless_types::activity::result_text;
use moatless_types::codex::CODEX_TOOL_TODO_LIST;

/// One tool call as its turn item, layered over `base`, or `None` for a call
/// another row renders.
pub(super) fn tool_item(
    conversation: &Conversation,
    activity: &Activity,
    base: Value,
) -> Option<Value> {
    let tool = activity.summary.as_str();
    if [QUESTION_TOOL, TODO_TOOL, CODEX_TOOL_TODO_LIST].contains(&tool) {
        return None;
    }
    let input = activity.tool_input();
    let output = activity.result().and_then(result_text).map(str::to_string);
    let fields = if tool == PLAN_TOOL {
        let markdown = plan_markdown(activity)?;
        json!({
            "type": "proposed_plan",
            "planId": activity.activity_id,
            "markdown": markdown,
            "streaming": false
        })
    } else {
        match t3_item_type(tool) {
            "command_execution" => json!({
                "type": "command_execution",
                "input": text(input, &["command", "cmd"])
                    .map_or_else(|| input.to_string(), str::to_string),
                "output": output,
                "outputIndicatesFailure": activity.is_error,
                "exitCode": command_exit_code(activity)
            }),
            "file_change" => file_change(input).unwrap_or_else(|| dynamic_tool(activity)),
            "collab_agent_tool_call" => json!({
                "type": "subagent",
                "title": text(input, &["description"]),
                "subagentId": format!("{}:subagent", activity.activity_id),
                "origin": "provider_native",
                "driver": provider_instance(conversation),
                "providerInstanceId": provider_instance(conversation),
                "childThreadId": Value::Null,
                "prompt": text(input, &["prompt", "description"]).unwrap_or_default(),
                "result": output
            }),
            "web_search" => json!({
                "type": "web_search",
                "patterns": text(input, &["query", "pattern"]).into_iter().collect::<Vec<_>>()
            }),
            _ => dynamic_tool(activity),
        }
    };
    Some(layered(base, fields))
}

/// A `PlanUpdated` row as the todo list the composer shows, or `None` when it
/// carries no step to show.
pub(super) fn todo_list_item(activity: &Activity, base: Value) -> Option<Value> {
    let steps = plan_steps(activity);
    if steps.is_empty() {
        return None;
    }
    Some(layered(
        base,
        json!({ "type": "todo_list", "planId": activity.activity_id, "steps": steps }),
    ))
}

/// The plan artifacts a window holds: each proposed plan, active until it is
/// answered, and each todo list, of which only the newest is active.
pub(super) fn plans(conversation: &Conversation, transcript: &Transcript) -> Vec<Value> {
    let newest_todo = transcript
        .activities
        .iter()
        .rev()
        .find(|activity| activity.kind == ActivityKind::PlanUpdated)
        .map(|activity| activity.activity_id.as_str());
    transcript
        .activities
        .iter()
        .filter_map(|activity| {
            let (run_id, _) = activity_anchor(conversation, transcript, activity);
            let base = json!({
                "id": activity.activity_id,
                "threadId": conversation.task_id,
                "runId": run_id,
                "nodeId": format!("{}:plan", activity.activity_id),
                "updatedAt": instant(activity.created_at)
            });
            let fields = match activity.kind {
                ActivityKind::ToolCall if activity.summary == PLAN_TOOL => json!({
                    "kind": "proposed_plan",
                    "markdown": plan_markdown(activity)?,
                    "status": if activity.status == ActivityStatus::InProgress { "active" } else { "completed" }
                }),
                ActivityKind::PlanUpdated => {
                    let steps = plan_steps(activity);
                    if steps.is_empty() {
                        return None;
                    }
                    let active = newest_todo == Some(activity.activity_id.as_str());
                    json!({
                        "kind": "todo_list",
                        "steps": steps,
                        "status": if active { "active" } else { "superseded" }
                    })
                }
                _ => return None,
            };
            Some(layered(base, fields))
        })
        .collect()
}

/// The node each plan artifact names, under its run's `root_turn` node when
/// `runs` has one.
pub(super) fn plan_nodes(plans: &[Value], runs: &[Value]) -> Vec<Value> {
    plans
        .iter()
        .filter_map(|plan| {
            let node_id = plan.get("nodeId")?.as_str()?;
            let root = plan
                .get("runId")
                .and_then(Value::as_str)
                .filter(|run_id| {
                    runs.iter().any(|run| {
                        run.get("id").and_then(Value::as_str) == Some(run_id) && has_started(run)
                    })
                })
                .map(root_node_id);
            let proposed = plan.get("kind").and_then(Value::as_str) == Some("proposed_plan");
            let status = match plan.get("status").and_then(Value::as_str) {
                Some("active") if proposed => "waiting",
                Some("active") => "running",
                _ => "completed",
            };
            Some(json!({
                "id": node_id,
                "threadId": plan.get("threadId"),
                "runId": plan.get("runId"),
                "parentNodeId": root,
                "rootNodeId": root.as_deref().unwrap_or(node_id),
                "kind": if proposed { "plan" } else { "todo_list" },
                "status": status,
                "countsForRun": false,
                "providerThreadId": Value::Null,
                "providerTurnId": Value::Null,
                "nativeItemRef": Value::Null,
                "runtimeRequestId": Value::Null,
                "checkpointScopeId": Value::Null,
                "startedAt": plan.get("updatedAt"),
                "completedAt": if status == "completed" { plan.get("updatedAt") } else { None }
            }))
        })
        .collect()
}

fn plan_markdown(activity: &Activity) -> Option<&str> {
    activity
        .tool_input()
        .get("plan")
        .and_then(Value::as_str)
        .filter(|plan| !plan.trim().is_empty())
}

/// A plan row's steps as `OrchestrationV2PlanStep`s, numbered in order.
fn plan_steps(activity: &Activity) -> Vec<Value> {
    activity
        .payload
        .get("plan")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let step = entry
                .get("step")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|step| !step.is_empty())?;
            let status = match entry.get("status").and_then(Value::as_str) {
                Some("completed") => "completed",
                Some("inProgress") => "running",
                _ => "pending",
            };
            Some((step, status))
        })
        .enumerate()
        .map(|(index, (step, status))| json!({ "id": (index + 1).to_string(), "text": step, "status": status }))
        .collect()
}

/// A file edit, from Claude's `file_path` inputs or Codex's `changes` list;
/// `None` when the call names no file.
fn file_change(input: &Value) -> Option<Value> {
    if let Some(changes) = input.get("changes").and_then(Value::as_array) {
        let details: Vec<Value> = changes
            .iter()
            .filter_map(|change| {
                let path = text(change, &["path"])?;
                let operation = change
                    .get("kind")
                    .and_then(|kind| kind.as_str().or_else(|| kind.get("type")?.as_str()))
                    .unwrap_or("update");
                Some(json!({ "operation": operation, "path": path }))
            })
            .collect();
        let file_name = details.first()?.get("path")?.clone();
        let mut fields =
            json!({ "type": "file_change", "fileName": file_name, "changes": details });
        if let [change] = changes.as_slice() {
            if let Some(diff) = text(change, &["diff"]) {
                fields["diffStr"] = json!(diff);
            }
        }
        return Some(fields);
    }
    let file_name = text(input, &["file_path", "notebook_path", "path"])?;
    let mut fields = Map::new();
    fields.insert("type".into(), json!("file_change"));
    fields.insert("fileName".into(), json!(file_name));
    if let Some(old) = input.get("old_string").and_then(Value::as_str) {
        fields.insert("oldStr".into(), json!(old));
    }
    if let Some(new) = input
        .get("new_string")
        .or_else(|| input.get("content"))
        .or_else(|| input.get("new_source"))
        .and_then(Value::as_str)
    {
        fields.insert("newStr".into(), json!(new));
    }
    Some(Value::Object(fields))
}

fn dynamic_tool(activity: &Activity) -> Value {
    json!({
        "type": "dynamic_tool",
        "toolName": Some(activity.summary.trim()).filter(|name| !name.is_empty()),
        "input": activity.tool_input(),
        "output": activity.result()
    })
}

/// A shell command's exit code, derived rather than stored: Moatless records
/// whether a call settled and whether it errored, not the process's status.
/// `null` while the call is still running.
pub(super) fn command_exit_code(activity: &Activity) -> Value {
    match activity.status {
        ActivityStatus::InProgress => Value::Null,
        ActivityStatus::Completed => json!(0),
        ActivityStatus::Failed => json!(1),
    }
}

/// The first of `keys` holding non-blank text.
fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
    })
}

/// Lay `extra`'s fields over `base`'s. A non-object `extra` adds nothing.
pub(super) fn layered(base: Value, extra: Value) -> Value {
    match (base, extra) {
        (Value::Object(mut base), Value::Object(extra)) => {
            base.extend(extra);
            Value::Object(base)
        }
        (base, _) => base,
    }
}
