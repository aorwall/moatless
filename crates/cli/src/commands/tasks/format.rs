//! Rendering a task for a person: the list table, the detail block, and the
//! backend's own message out of a refusal.

use crate::output;
use moatless_api_schemas::enums::TaskStatus;
use moatless_api_schemas::task::{ListTasksResponse, TaskResponse};

/// The backend's `{"error": …}` message out of an error body.
///
/// `None` for a body that is not that envelope, which leaves each caller to
/// decide between the raw body and returning the error unchanged.
pub(super) fn api_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
}

pub(super) fn parse_task_status(s: &str) -> Option<TaskStatus> {
    match s {
        "open" => Some(TaskStatus::Open),
        "closed" => Some(TaskStatus::Closed),
        "error" => Some(TaskStatus::Error),
        _ => None,
    }
}

pub(super) fn format_task_list(list: &ListTasksResponse) {
    let rows: Vec<Vec<String>> = list.items.iter().map(format_task_row).collect();
    output::print_table(
        &[
            "TASK_ID",
            "SHORT",
            "STATUS",
            "TITLE",
            "REPO",
            "WORKSPACE",
            "CREATED_BY",
        ],
        &rows,
    );
}

fn format_task_row(t: &TaskResponse) -> Vec<String> {
    vec![
        t.task_id.clone(),
        t.short_id.as_deref().unwrap_or("-").to_string(),
        t.status.to_string(),
        t.title.chars().take(50).collect(),
        t.repository_name.as_deref().unwrap_or("-").to_string(),
        label(t.workspace_name.as_deref(), t.workspace_id.as_deref()).to_string(),
        label(
            t.created_by_display_name.as_deref(),
            t.created_by.as_deref(),
        )
        .to_string(),
    ]
}

/// Label a foreign key for display: its resolved name, else the raw id, else
/// `-`. Falling back to the id matters — a task that *has* a workspace whose
/// name failed to resolve must not render identically to one that has none.
fn label<'a>(name: Option<&'a str>, id: Option<&'a str>) -> &'a str {
    name.or(id).unwrap_or("-")
}

pub(super) fn format_task_detail(t: &TaskResponse) {
    println!("Task:    {}", t.task_id);
    println!("ShortId: {}", t.short_id.as_deref().unwrap_or("-"));
    println!("Title:   {}", t.title);
    println!("Status:  {}", t.status);
    println!("Repo:    {}", t.repository_name.as_deref().unwrap_or("-"));
    println!(
        "Workspace: {}",
        label(t.workspace_name.as_deref(), t.workspace_id.as_deref())
    );
    println!("Branch:  {}", t.branch.as_deref().unwrap_or("-"));
    println!(
        "Creator: {}",
        label(
            t.created_by_display_name.as_deref(),
            t.created_by.as_deref()
        )
    );
    println!("Created: {}", t.created_at);
    println!("Cost:    ${:.4}", t.total_cost_usd);
    println!("Parent:  {}", t.parent_task_id.as_deref().unwrap_or("none"));
    if !t.relations.is_empty() {
        println!("Relations:");
        for r in &t.relations {
            let via = r
                .context
                .get("via")
                .and_then(|v| v.as_str())
                .map(|s| format!(" (via {s})"))
                .unwrap_or_default();
            println!("  {} → {}{}", r.relation_type.as_str(), r.to_task_id, via);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_prefers_name_then_id_then_dash() {
        assert_eq!(label(Some("acme-ws"), Some("ws_1")), "acme-ws");
        // Name lookup failed but the task does have a workspace — showing the
        // id keeps that distinguishable from having none.
        assert_eq!(label(None, Some("ws_1")), "ws_1");
        assert_eq!(label(None, None), "-");
    }
}
