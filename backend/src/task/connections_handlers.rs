//! Admin task-connections view.
//!
//! Lists tasks with their external connections (GitHub PR / Linear issue /
//! Slack thread), each connection's persisted external status, and
//! per-connection inbound/outgoing message counts. Modeled on
//! [`super::handlers::list_tasks`]: list tasks → batch bindings → batch message
//! counts → decode `destination_config` into typed connections → optional live
//! status refresh.

use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State};
use axum::Json;
use futures::stream::{self, StreamExt};

use moatless_api_schemas::enums::{AdapterKind, TaskStatus};
use moatless_api_schemas::task::PaginationResponse;
use moatless_api_schemas::task_connections::{
    Connection, ListTaskConnectionsQuery, TaskConnectionRow, TaskConnectionsResponse,
};

use crate::adapters::models::TaskBinding;
use crate::bindings::dao::PgTaskBindingRepository;
use crate::bindings::status_service;
use crate::extractors::AuthUser;
use crate::message::dao::ConnectionMessageCount;
use crate::state::AppState;

use super::dao::TaskFilter;
use super::error::TaskError;

/// Live-refresh a connection's status only when its persisted status is older
/// than this (or never set).
const STATUS_STALE_SECS: i64 = 300;

/// Bounded concurrency for live status fetches during `?refresh=true`.
const REFRESH_CONCURRENCY: usize = 8;

/// List tasks with their external connections, statuses, and message counts.
#[utoipa::path(
    get,
    path = "/api/v1/admin/task-connections",
    tag = "tasks",
    params(ListTaskConnectionsQuery),
    responses(
        (status = 200, description = "Tasks with their external connections", body = TaskConnectionsResponse),
        (status = 403, description = "Admin role required", body = crate::error::ErrorBody)
    )
)]
pub async fn list_task_connections(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListTaskConnectionsQuery>,
) -> Result<Json<TaskConnectionsResponse>, TaskError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(TaskError::Forbidden(
            "admin role required for task-connections".into(),
        ));
    }

    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0);
    let refresh = query.refresh.unwrap_or(false);

    let filter = TaskFilter {
        scope: auth.scope_context(),
        limit,
        offset,
        status: None,
        // Closed tasks are finished work; the operational view shows only live tasks.
        exclude_status: Some(TaskStatus::Closed),
        repository_id: None,
        created_by: None,
        tag: None,
        parent_task_id: None,
    };

    let (rows, total) = state.tasks.list_tasks(&filter).await?;
    let task_ids: Vec<String> = rows.iter().map(|r| r.task_id.clone()).collect();

    // Batch resolve owner display names (mirrors `list_tasks`'
    // `resolve_user_display_names_batch`, inlined to keep this module
    // self-contained).
    let owner_ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.created_by.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let owner_names: HashMap<String, String> = if owner_ids.is_empty() {
        HashMap::new()
    } else {
        match state.users.get_users_by_ids(&owner_ids).await {
            Ok(users) => users
                .into_iter()
                .map(|u| (u.id, u.name.unwrap_or(u.login)))
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to batch resolve owner display names");
                HashMap::new()
            }
        }
    };

    // Batch resolve repository names.
    let repo_ids: Vec<String> = rows
        .iter()
        .map(|r| r.repository_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let repo_names: HashMap<String, String> = state
        .repositories
        .find_names_by_ids(&repo_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve repository names");
            vec![]
        })
        .into_iter()
        .collect();

    // Batch resolve bindings (full rows, incl. destination_config + status).
    let mut bindings = PgTaskBindingRepository(state.db.clone())
        .find_by_task_ids(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve task bindings");
            vec![]
        });

    // Batch resolve per-connection message counts.
    let counts = state
        .messages
        .get_connection_message_counts(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve connection message counts");
            vec![]
        });

    if refresh {
        refresh_stale_statuses(&state, &rows, &mut bindings).await;
    }

    // Index counts and bindings by task for O(1) assembly.
    let counts_by_task = group_counts_by_task(&counts);
    let bindings_by_task = group_bindings_by_task(&bindings);

    let items: Vec<TaskConnectionRow> = rows
        .iter()
        .map(|task| {
            let task_bindings: &[&TaskBinding] = bindings_by_task
                .get(task.task_id.as_str())
                .map_or(&[], Vec::as_slice);
            let empty = Vec::new();
            let task_counts = counts_by_task.get(task.task_id.as_str()).unwrap_or(&empty);
            let count_by_binding = match_counts(task_bindings, task_counts);

            let connections: Vec<Connection> = task_bindings
                .iter()
                .map(|b| decode_connection(b, &count_by_binding))
                .collect();

            TaskConnectionRow {
                task_id: task.task_id.clone(),
                short_id: task.short_id.clone(),
                title: task
                    .name
                    .clone()
                    .or_else(|| task.short_id.clone())
                    .unwrap_or_else(|| task.task_id.clone()),
                status: task.status,
                repository_name: repo_names.get(&task.repository_id).cloned(),
                owner: task
                    .created_by
                    .as_ref()
                    .and_then(|id| owner_names.get(id).cloned()),
                connections,
            }
        })
        .collect();

    Ok(Json(TaskConnectionsResponse {
        items,
        pagination: PaginationResponse {
            limit,
            offset,
            total,
            has_more: offset + limit < total,
        },
    }))
}

/// Live-refresh bindings whose persisted status is missing or stale. Mutates
/// `bindings` in place with the freshly fetched status. Bounded concurrency.
async fn refresh_stale_statuses(
    state: &AppState,
    rows: &[crate::task::models::Task],
    bindings: &mut [TaskBinding],
) {
    let created_by: HashMap<&str, &str> = rows
        .iter()
        .filter_map(|r| r.created_by.as_deref().map(|c| (r.task_id.as_str(), c)))
        .collect();

    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::seconds(STATUS_STALE_SECS);

    let targets: Vec<TaskBinding> = bindings
        .iter()
        .filter(|b| {
            is_refreshable(b)
                && b.external_status_updated_at.is_none_or(|ts| ts < cutoff)
        })
        .cloned()
        .collect();

    if targets.is_empty() {
        return;
    }

    let fetched: HashMap<String, status_service::ExternalStatus> = stream::iter(targets)
        .map(|binding| {
            let state = state.clone();
            let creator = created_by
                .get(binding.task_id.as_str())
                .map_or_else(String::new, |c| (*c).to_string());
            async move {
                status_service::get_or_fetch(&state, &binding, &creator)
                    .await
                    .map(|status| (binding.id.clone(), status))
            }
        })
        .buffer_unordered(REFRESH_CONCURRENCY)
        .filter_map(|x| async move { x })
        .collect()
        .await;

    for b in bindings.iter_mut() {
        if let Some(status) = fetched.get(&b.id) {
            b.external_status = status.status.clone();
            b.external_state_kind = status.state_kind.clone();
            b.external_status_raw = Some(status.raw.clone());
            b.external_status_updated_at = Some(now);
        }
    }
}

/// Whether a binding has a live status we know how to fetch.
fn is_refreshable(b: &TaskBinding) -> bool {
    matches!(b.provider.as_str(), "github" | "github_pr" | "linear")
}

/// Normalize an adapter/provider string to a canonical bucket so that the
/// GitHub binding provider (`github`) and the GitHub message adapter label
/// (`github_pr`) compare equal.
fn normalize_adapter(s: &str) -> &str {
    match s {
        "github" | "github_pr" => "github",
        other => other,
    }
}

fn group_counts_by_task(
    counts: &[ConnectionMessageCount],
) -> HashMap<&str, Vec<&ConnectionMessageCount>> {
    let mut map: HashMap<&str, Vec<&ConnectionMessageCount>> = HashMap::new();
    for c in counts {
        map.entry(c.task_id.as_str()).or_default().push(c);
    }
    map
}

fn group_bindings_by_task(bindings: &[TaskBinding]) -> HashMap<&str, Vec<&TaskBinding>> {
    let mut map: HashMap<&str, Vec<&TaskBinding>> = HashMap::new();
    for b in bindings {
        map.entry(b.task_id.as_str()).or_default().push(b);
    }
    map
}

/// Attribute message counts to bindings. Exact `subject_key` match wins;
/// counts whose subject_key matches no binding fall back to a per-adapter
/// bucket, applied only when that adapter has a single binding on the task.
fn match_counts(
    task_bindings: &[&TaskBinding],
    rows: &[&ConnectionMessageCount],
) -> HashMap<String, (i64, i64)> {
    let mut out: HashMap<String, (i64, i64)> = HashMap::new();

    let mut subject_to_binding: HashMap<&str, &TaskBinding> = HashMap::new();
    for b in task_bindings {
        subject_to_binding
            .entry(b.subject_key.as_str())
            .or_insert(b);
    }

    let mut bindings_by_adapter: HashMap<&str, Vec<&TaskBinding>> = HashMap::new();
    for b in task_bindings {
        bindings_by_adapter
            .entry(normalize_adapter(&b.provider))
            .or_default()
            .push(b);
    }

    let mut leftover: HashMap<String, (i64, i64)> = HashMap::new();
    for row in rows {
        if let Some(sk) = row.external_subject_key.as_deref() {
            if let Some(b) = subject_to_binding.get(sk) {
                let entry = out.entry(b.id.clone()).or_insert((0, 0));
                entry.0 += row.incoming;
                entry.1 += row.outgoing;
                continue;
            }
        }
        let adapter = row
            .external_adapter
            .as_deref()
            .map_or("", normalize_adapter)
            .to_string();
        let entry = leftover.entry(adapter).or_insert((0, 0));
        entry.0 += row.incoming;
        entry.1 += row.outgoing;
    }

    for (adapter, (inc, outg)) in leftover {
        if let Some(bs) = bindings_by_adapter.get(adapter.as_str()) {
            if bs.len() == 1 {
                let entry = out.entry(bs[0].id.clone()).or_insert((0, 0));
                entry.0 += inc;
                entry.1 += outg;
            }
        }
    }

    out
}

/// Decode a binding's `destination_config` + persisted status into a typed
/// [`Connection`].
fn decode_connection(b: &TaskBinding, counts: &HashMap<String, (i64, i64)>) -> Connection {
    let (label, url) = label_and_url(b);
    let (incoming, outgoing) = counts.get(&b.id).copied().unwrap_or((0, 0));

    Connection {
        binding_id: b.id.clone(),
        adapter_kind: map_provider(&b.provider),
        kind: connection_kind(&b.subject_kind),
        label,
        url,
        status: b.external_status.clone(),
        state_kind: b.external_state_kind.clone(),
        status_updated_at: b.external_status_updated_at.map(|t| t.to_rfc3339()),
        incoming,
        outgoing,
        last_event_at: Some(b.last_event_at.to_rfc3339()),
        active: b.active,
        deactivated_at: b.deactivated_at.map(|t| t.to_rfc3339()),
    }
}

fn map_provider(provider: &str) -> AdapterKind {
    match provider {
        "github" | "github_pr" => AdapterKind::Github,
        "linear" => AdapterKind::Linear,
        "telegram" => AdapterKind::Telegram,
        "webhook" => AdapterKind::Webhook,
        _ => AdapterKind::Slack,
    }
}

fn connection_kind(subject_kind: &str) -> String {
    match subject_kind {
        "github_pr" => "github_pr".to_string(),
        "linear_issue" | "linear_agent_session" => "linear_issue".to_string(),
        other => other.to_string(),
    }
}

/// Build a human label and web URL from a binding's destination config.
fn label_and_url(b: &TaskBinding) -> (String, Option<String>) {
    let cfg = &b.destination_config;
    match b.provider.as_str() {
        "github" | "github_pr" => {
            // subject_key is already "owner/repo#N".
            let url = github_pr_url(cfg).or_else(|| {
                b.external_status_raw
                    .as_ref()
                    .and_then(|r| r.get("url"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });
            (b.subject_key.clone(), url)
        }
        "linear" => {
            let label = json_str(cfg, "identifier")
                .or_else(|| json_str(cfg, "issue_identifier"))
                .unwrap_or_else(|| b.subject_key.clone());
            let url = json_str(cfg, "issue_url").or_else(|| json_str(cfg, "url"));
            (label, url)
        }
        _ => (b.subject_key.clone(), None),
    }
}

fn github_pr_url(cfg: &serde_json::Value) -> Option<String> {
    let owner = cfg.get("owner").and_then(serde_json::Value::as_str)?;
    let repo = cfg.get("repo").and_then(serde_json::Value::as_str)?;
    let number = cfg.get("pr_number").and_then(serde_json::Value::as_u64)?;
    Some(format!("https://github.com/{owner}/{repo}/pull/{number}"))
}

fn json_str(cfg: &serde_json::Value, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn binding(id: &str, provider: &str, subject_kind: &str, subject_key: &str) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: id.to_string(),
            task_id: "task_1".to_string(),
            provider: provider.to_string(),
            subject_kind: subject_kind.to_string(),
            subject_key: subject_key.to_string(),
            connection_id: "conn_1".to_string(),
            trigger_id: None,
            destination_config: serde_json::json!({}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "ingress".to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        }
    }

    fn count(
        subject_key: Option<&str>,
        adapter: Option<&str>,
        incoming: i64,
        outgoing: i64,
    ) -> ConnectionMessageCount {
        ConnectionMessageCount {
            task_id: "task_1".to_string(),
            external_adapter: adapter.map(str::to_string),
            external_subject_key: subject_key.map(str::to_string),
            incoming,
            outgoing,
        }
    }

    #[test]
    fn github_label_is_subject_key_and_url_is_derived() {
        let mut b = binding("b1", "github", "github_pr", "o/r#42");
        b.destination_config = serde_json::json!({"owner": "o", "repo": "r", "pr_number": 42});
        let (label, url) = label_and_url(&b);
        assert_eq!(label, "o/r#42");
        assert_eq!(url.as_deref(), Some("https://github.com/o/r/pull/42"));
    }

    #[test]
    fn linear_label_prefers_identifier() {
        let mut b = binding("b1", "linear", "linear_agent_session", "session-uuid");
        b.destination_config = serde_json::json!({
            "identifier": "SP-1721",
            "issue_url": "https://linear.app/x/issue/SP-1721",
        });
        let (label, url) = label_and_url(&b);
        assert_eq!(label, "SP-1721");
        assert_eq!(url.as_deref(), Some("https://linear.app/x/issue/SP-1721"));
    }

    #[test]
    fn linear_kind_normalizes_both_subject_kinds() {
        assert_eq!(connection_kind("linear_agent_session"), "linear_issue");
        assert_eq!(connection_kind("linear_issue"), "linear_issue");
        assert_eq!(connection_kind("github_pr"), "github_pr");
        assert_eq!(connection_kind("slack_thread"), "slack_thread");
    }

    #[test]
    fn counts_match_on_subject_key() {
        let b1 = binding("b1", "github", "github_pr", "o/r#1");
        let b2 = binding("b2", "github", "github_pr", "o/r#2");
        let bindings = vec![&b1, &b2];
        let c1 = count(Some("o/r#1"), Some("github_pr"), 3, 1);
        let c2 = count(Some("o/r#2"), Some("github_pr"), 5, 2);
        let rows = vec![&c1, &c2];

        let matched = match_counts(&bindings, &rows);
        assert_eq!(matched.get("b1").copied(), Some((3, 1)));
        assert_eq!(matched.get("b2").copied(), Some((5, 2)));
    }

    #[test]
    fn counts_without_subject_fall_back_to_sole_adapter_binding() {
        // Single linear binding; a legacy row with null subject_key should be
        // attributed to it.
        let b1 = binding("b1", "linear", "linear_agent_session", "session-uuid");
        let bindings = vec![&b1];
        let legacy = count(None, Some("linear"), 4, 6);
        let rows = vec![&legacy];

        let matched = match_counts(&bindings, &rows);
        assert_eq!(matched.get("b1").copied(), Some((4, 6)));
    }

    #[test]
    fn ambiguous_adapter_fallback_is_dropped() {
        // Two bindings of the same adapter — a null-subject row can't be
        // attributed unambiguously, so it's dropped rather than double-counted.
        let b1 = binding("b1", "github", "github_pr", "o/r#1");
        let b2 = binding("b2", "github", "github_pr", "o/r#2");
        let bindings = vec![&b1, &b2];
        let orphan = count(None, Some("github_pr"), 9, 9);
        let rows = vec![&orphan];

        let matched = match_counts(&bindings, &rows);
        assert!(matched.is_empty());
    }

    #[test]
    fn decode_connection_carries_active_state() {
        let counts: HashMap<String, (i64, i64)> = HashMap::new();

        let active = binding("b1", "github", "github_pr", "o/r#1");
        let decoded = decode_connection(&active, &counts);
        assert!(decoded.active);
        assert_eq!(decoded.deactivated_at, None);

        let mut closed = binding("b2", "github", "github_pr", "o/r#2");
        closed.active = false;
        closed.deactivated_at = Some(Utc::now());
        let decoded = decode_connection(&closed, &counts);
        assert!(!decoded.active);
        assert!(decoded.deactivated_at.is_some());
    }
}
