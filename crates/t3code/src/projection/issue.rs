//! The Linear issues bound to a conversation.

use serde_json::{Value, json};

use super::scalar::instant;
use super::scalar::non_empty;
use crate::dao::Conversation;
use crate::issues::{LINEAR_HOST, LINEAR_TRACKER};

/// Every Linear issue a Task is bound to, as T3's `ThreadIssueLink`, read
/// through `project_id`.
pub(super) fn thread_issues(conversation: &Conversation, project_id: &str) -> Vec<Value> {
    let Some(bound) = conversation.issues.as_deref() else {
        return Vec::new();
    };
    bound
        .iter()
        .filter_map(|issue| {
            let key = non_empty(&issue.identifier)?;
            let (scope, _) = key.split_once('-')?;
            let url = issue
                .url
                .as_deref()
                .and_then(non_empty)
                .unwrap_or_else(|| format!("https://{LINEAR_HOST}/issue/{key}"));
            Some(json!({
                "tracker": LINEAR_TRACKER,
                "host": LINEAR_HOST,
                "scope": scope,
                "key": key,
                "projectId": project_id,
                "url": url,
                "source": thread_issue_source(&issue.origin, issue.bind_reason.as_deref()),
                "linkedAt": instant(issue.linked_at),
                "displayKey": key,
                "title": issue.title.as_deref().and_then(non_empty).unwrap_or_else(|| key.clone()),
            }))
        })
        .collect()
}

/// Who linked an issue, as T3 names it: `manual` for a person's link from T3
/// (`BindReason::Contributor`, spelled out as `thread_pull_request_source`
/// spells its names), else the agent or an event on the issue.
fn thread_issue_source(origin: &str, bind_reason: Option<&str>) -> &'static str {
    match (origin, bind_reason) {
        (_, Some("contributor")) => "manual",
        _ => "agent",
    }
}
