//! The Tasks a thread delegated to (its `created_by` children), as V2
//! app-owned subagents, the `thread_created` rows that announce them, and the
//! `moat tasks create` calls that spawned them.
//!
//! A forked child is not a subagent: the fork is the child's own lineage, and
//! the parent's timeline does not announce it.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::model::model_selection;
use super::model::provider_instance;
use super::scalar::created_at;
use super::scalar::instant;
use super::scalar::non_empty;
use super::scalar::nullable_instant;
use super::v2::shell_status;
use super::v2_items::layered;
use super::v2_provider::root_node_id;
use crate::AgentModelDefaults;
use crate::dao::Conversation;

/// The command an agent delegates with. Its output names the created Task,
/// as `Task {id} created …` or in the `--json` body.
const DELEGATE_COMMAND: &str = "moat tasks create";

/// The `subagents[]` id of a delegated Task, and the `subagentId` of the card
/// that links to it: the client joins the two by this id.
fn subagent_id(child_task_id: &str) -> String {
    format!("subagent:task:{child_task_id}")
}

/// The run of `runs` a child created at `at` was spawned under: the newest one
/// started by then. `None` when no run had started.
fn spawning_run<'a>(runs: &'a [Value], at: &str) -> Option<&'a Value> {
    runs.iter()
        .filter(|run| {
            run["startedAt"]
                .as_str()
                .is_some_and(|started| started <= at)
        })
        .max_by_key(|run| run["ordinal"].as_i64())
}

/// A child's shell status in the subagent vocabulary, which has `waiting`
/// for a question and `pending` for a send no turn has taken yet.
fn subagent_status(child: &Conversation) -> &'static str {
    match shell_status(child) {
        "running" if child.pending_question => "waiting",
        "running" => "running",
        "starting" => "pending",
        "failed" => "failed",
        "interrupted" => "interrupted",
        "completed" => "completed",
        _ => "idle",
    }
}

fn title(child: &Conversation) -> Value {
    child
        .name
        .as_deref()
        .and_then(non_empty)
        .map_or(Value::Null, Value::String)
}

/// One `OrchestrationV2Subagent` per delegated Task, in creation order.
///
/// `parentNodeId` is the root node of the run that spawned the child, or the
/// subagent's own id when no run had started: the contract requires one.
pub(super) fn subagents(
    conversation: &Conversation,
    children: &[Conversation],
    runs: &[Value],
    defaults: &AgentModelDefaults,
) -> Vec<Value> {
    let mut children: Vec<&Conversation> = children.iter().collect();
    children.sort_by_key(|child| (created_at(child), child.task_id.clone()));
    children
        .into_iter()
        .map(|child| {
            let id = subagent_id(&child.task_id);
            let started = instant(created_at(child));
            let run_id = spawning_run(runs, &started).and_then(|run| run["id"].as_str());
            let status = subagent_status(child);
            let settled = matches!(status, "completed" | "failed" | "interrupted");
            json!({
                "id": id,
                "threadId": conversation.task_id,
                "runId": run_id,
                "parentNodeId": run_id.map_or_else(|| id.clone(), root_node_id),
                "origin": "app_owned",
                "createdBy": "agent",
                "driver": provider_instance(child),
                "providerInstanceId": provider_instance(child),
                "providerThreadId": Value::Null,
                "childThreadId": child.task_id,
                "nativeTaskRef": Value::Null,
                "prompt": "",
                "title": title(child),
                "model": model_selection(child, defaults)["model"],
                "status": status,
                "result": if status == "failed" { json!(child.turn_error) } else { Value::Null },
                "startedAt": started,
                "completedAt": if settled { nullable_instant(child.turn_ended_at) } else { Value::Null },
                "updatedAt": instant(child.updated_at.unwrap_or_else(|| created_at(child)))
            })
        })
        .collect()
}

/// One `thread_created` turn item per delegated Task, at the instant it was
/// created, unnumbered: [`with_thread_created`] places them in this order.
pub(super) fn thread_created_items(
    conversation: &Conversation,
    children: &[Conversation],
    runs: &[Value],
    defaults: &AgentModelDefaults,
) -> Vec<Value> {
    let mut children: Vec<&Conversation> = children.iter().collect();
    children.sort_by_key(|child| (created_at(child), child.task_id.clone()));
    children
        .into_iter()
        .map(|child| {
            let at = instant(created_at(child));
            let run_id = spawning_run(runs, &at).map_or(Value::Null, |run| run["id"].clone());
            json!({
                "id": format!("item:thread-created:{}", child.task_id),
                "threadId": conversation.task_id,
                "runId": run_id,
                "nodeId": Value::Null,
                "providerThreadId": Value::Null,
                "providerTurnId": Value::Null,
                "nativeItemRef": Value::Null,
                "parentItemId": Value::Null,
                "status": "completed",
                "title": title(child),
                "startedAt": at,
                "completedAt": at,
                "updatedAt": at,
                "type": "thread_created",
                "targetThreadId": child.task_id,
                "targetRunId": Value::Null,
                "targetProviderInstanceId": provider_instance(child),
                "targetModel": model_selection(child, defaults)["model"]
            })
        })
        .collect()
}

/// `turn_items` with each of `created` placed after the items that started no
/// later than it, then all of them numbered in that order.
pub(super) fn with_thread_created(turn_items: Vec<Value>, created: Vec<Value>) -> Vec<Value> {
    let mut items = turn_items;
    for item in created {
        let at = item["startedAt"].as_str().unwrap_or_default().to_string();
        let position = items
            .iter()
            .position(|existing| {
                existing["startedAt"]
                    .as_str()
                    .is_some_and(|started| started > at.as_str())
            })
            .unwrap_or(items.len());
        items.insert(position, item);
    }
    for (ordinal, item) in items.iter_mut().enumerate() {
        item["ordinal"] = json!(ordinal);
    }
    items
}

/// Turns each completed `moat tasks create` command whose output names one of
/// `subagents` into the subagent card for it, so the card opens the child.
/// Answers the child Task ids that got a card; such a child needs no
/// `thread_created` row, which would be a second entry opening it.
pub(super) fn link_delegating_commands(
    turn_items: &mut [Value],
    subagents: &[Value],
) -> HashSet<String> {
    let mut carded = HashSet::new();
    for item in turn_items.iter_mut() {
        if item["type"] != "command_execution" || item["status"] != "completed" {
            continue;
        }
        if !item["input"]
            .as_str()
            .is_some_and(|input| input.contains(DELEGATE_COMMAND))
        {
            continue;
        }
        let Some(output) = item["output"].as_str() else {
            continue;
        };
        let Some(subagent) = subagents.iter().find(|subagent| {
            subagent["childThreadId"]
                .as_str()
                .is_some_and(|child| output.contains(child))
        }) else {
            continue;
        };
        let Value::Object(mut base) = item.take() else {
            continue;
        };
        if let Some(child) = subagent["childThreadId"].as_str() {
            carded.insert(child.to_string());
        }
        for key in ["input", "output", "outputIndicatesFailure", "exitCode"] {
            base.remove(key);
        }
        *item = layered(
            Value::Object(base),
            json!({
                "type": "subagent",
                "title": subagent["title"],
                "subagentId": subagent["id"],
                "origin": "app_owned",
                "driver": subagent["driver"],
                "providerInstanceId": subagent["providerInstanceId"],
                "childThreadId": subagent["childThreadId"],
                "prompt": subagent["prompt"],
                "result": Value::Null
            }),
        );
    }
    carded
}
