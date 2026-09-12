//! Where a Message came from, for the Messages that came from outside T3.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::transcript_dao::TranscriptMessage;

/// What the projection needs to say where a Message came from, beyond the
/// Message row itself: the deployment's own address, for a link back into
/// Moatless, and the names of the Tasks other Messages were authored by.
pub(crate) struct OriginContext<'a> {
    pub frontend_url: &'a str,
    pub source_task_names: &'a HashMap<String, String>,
}

/// The one adapter whose URL is derived rather than joined: the GitHub PR
/// adapter is GitHub-only and its subject key carries the coordinates.
const GITHUB_PR_ADAPTER: &str = "github_pr";

/// Where one Message came from, or `None` when it came from inside T3.
///
/// The first arm that matches wins: a Message authored by another Task is a
/// task origin even where it also carries adapter columns.
pub(super) fn message_origin(
    message: &TranscriptMessage,
    origins: &OriginContext<'_>,
) -> Option<Value> {
    if let Some(source_task_id) = message.source_task_id.as_deref() {
        let name = origins.source_task_names.get(source_task_id);
        return Some(json!({
            "kind": "task",
            "label": name,
            // A Task the reader cannot see yields no name and no link: a link
            // they could not follow would be worse than none.
            "url": name.map(|_| format!(
                "{}/tasks/{source_task_id}",
                origins.frontend_url.trim_end_matches('/')
            )),
            "user": Value::Null
        }));
    }
    let adapter = message.external_adapter.as_deref()?;
    // The handle the external system knows the writer by, or — on a row written
    // before that was captured — the Moatless user the send resolved to.
    let author = message
        .external_author
        .as_deref()
        .or(message.sent_by_name.as_deref());
    if adapter == GITHUB_PR_ADAPTER {
        let label = message
            .external_subject_key
            .as_deref()
            .or(message.external_source_name.as_deref());
        return Some(json!({
            "kind": adapter,
            "label": label,
            "url": github_pr_url(
                message.external_subject_key.as_deref(),
                message.external_event_id.as_deref()
            ),
            "user": author
        }));
    }
    // The Binding's own name for the subject first: `external_source_name` is
    // whatever matched the subscription, which for Linear is a team id.
    Some(json!({
        "kind": adapter,
        "label": message
            .binding_label
            .as_deref()
            .or(message.external_source_name.as_deref()),
        "url": message.binding_url.as_deref(),
        "user": author
    }))
}

/// The GitHub URL a pull-request comment opens at, from `owner/repo#N`.
///
/// The subject key carries the repository and PR number; the event id says
/// which kind of comment, which is the fragment. A key that is not `owner/repo#N`
/// yields no URL rather than a wrong one, and an event id this does not
/// recognise — or none — leaves the reader at the pull request itself.
pub(super) fn github_pr_url(
    subject_key: Option<&str>,
    external_event_id: Option<&str>,
) -> Option<String> {
    let (repo, number) = subject_key?.split_once('#')?;
    if repo.split('/').count() != 2 || repo.split('/').any(str::is_empty) {
        return None;
    }
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let base = format!("https://github.com/{repo}/pull/{number}");
    let fragment = external_event_id.and_then(|id| {
        id.strip_prefix("github:ic:")
            .map(|comment| format!("#issuecomment-{comment}"))
            .or_else(|| {
                id.strip_prefix("github:rc:")
                    .map(|comment| format!("#discussion_r{comment}"))
            })
    });
    Some(match fragment {
        Some(fragment) => format!("{base}{fragment}"),
        None => base,
    })
}
