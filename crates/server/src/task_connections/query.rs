//! Authorized Task connection reads and bounded live-status enrichment.

use super::projection::{
    binding_status_for, decode_connection, group_bindings_by_task, group_counts_by_task,
    match_counts,
};
use futures::stream::{self, StreamExt};
use moatless_adapter_model::registry::Adapters;
use moatless_api_schemas::task::PaginationResponse;
use moatless_api_schemas::task_connections::{
    Connection, GetTaskConnectionsQuery, ListTaskConnectionsQuery, TaskConnectionRow,
    TaskConnectionsResponse,
};
use moatless_loops::bindings::{operations::Bindings, status_service};
use moatless_ports::identity::UserDisplay;
use moatless_ports::workspace::RepositoryReader;
use moatless_tasks::{
    message::models::ConnectionMessageCount,
    message::operations::Messages,
    task::{error::TaskError, models::TaskFilter, operations::TaskCommands},
};
use moatless_types::{
    access_control::ScopeContext,
    adapter::{ExternalStatus, TaskBinding},
    task::TaskStatus,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const STATUS_STALE_SECS: i64 = 300;
const REFRESH_CONCURRENCY: usize = 8;

/// Read ports for the administrator Task connection view.
pub struct TaskConnectionReads<'a> {
    pub tasks: &'a TaskCommands,
    pub users: &'a dyn UserDisplay,
    pub repositories: &'a dyn RepositoryReader,
    pub task_bindings: &'a Bindings,
    pub messages: &'a Messages,
    pub adapters: Option<&'a Arc<Adapters>>,
}

/// List live Tasks and enrich their connections for an administrator.
pub async fn list_task_connections(
    reads: TaskConnectionReads<'_>,
    ctx: &ScopeContext,
    query: ListTaskConnectionsQuery,
) -> Result<TaskConnectionsResponse, TaskError> {
    let TaskConnectionReads {
        tasks,
        users,
        repositories,
        task_bindings,
        messages,
        adapters,
    } = reads;
    if !ctx.is_admin {
        return Err(TaskError::Forbidden(
            "admin role required for task-connections".into(),
        ));
    }

    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0);
    let refresh = query.refresh.unwrap_or(false);

    let filter = TaskFilter {
        scope: ctx.clone(),
        limit,
        offset,
        status: None,
        // Closed tasks are finished work; the operational view shows only live tasks.
        exclude_status: Some(TaskStatus::Closed),
        repository_id: None,
        created_by: None,
        tag: None,
        parent_task_id: None,
        followed_by: None,
    };

    let (rows, total) = tasks.list_tasks(&filter).await?;
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
        let owner_uuids: Vec<uuid::Uuid> =
            owner_ids.iter().filter_map(|id| id.parse().ok()).collect();
        match users.names(&owner_uuids).await {
            Ok(names) => names
                .into_iter()
                .map(|(id, name)| (id.to_string(), name))
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
    let repo_names: HashMap<String, String> =
        repositories.names(&repo_ids).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve repository names");
            HashMap::new()
        });

    // Batch resolve bindings (full rows, incl. destination_config + status).
    let mut bindings = task_bindings
        .for_tasks(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve task bindings");
            vec![]
        });

    // Batch resolve per-connection message counts.
    let counts = messages
        .get_connection_message_counts(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to batch resolve connection message counts");
            vec![]
        });

    if refresh {
        refresh_stale_statuses(adapters, task_bindings, &rows, &mut bindings).await;
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
                .map(|b| {
                    decode_connection(
                        binding_status_for(adapters, b.adapter_kind.as_str()),
                        b,
                        &count_by_binding,
                    )
                })
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

    Ok(TaskConnectionsResponse {
        items,
        pagination: PaginationResponse {
            limit,
            offset,
            total,
            has_more: offset + limit < total,
        },
    })
}

/// Read connections for one Task visible to the caller.
pub async fn get_task_connections(
    tasks: &TaskCommands,
    task_bindings: &Bindings,
    messages: &Messages,
    adapters: Option<&Arc<Adapters>>,
    ctx: &ScopeContext,
    task_id: String,
    query: GetTaskConnectionsQuery,
) -> Result<Vec<Connection>, TaskError> {
    // Access check mirrors `get_task`: scope-checked, not admin-gated.
    let row = tasks
        .find_by_id(&task_id, ctx)
        .await?
        .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;

    let task_ids = [task_id.clone()];

    let mut bindings = task_bindings
        .for_tasks(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to resolve task bindings");
            vec![]
        });

    let counts = messages
        .get_connection_message_counts(&task_ids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to resolve connection message counts");
            vec![]
        });

    if query.refresh == Some(true) {
        // `refresh_stale_statuses` has its own 300s staleness guard and skips
        // finished subjects entirely, so this stays cheap and bounded even when
        // called per-request.
        refresh_stale_statuses(
            adapters,
            task_bindings,
            std::slice::from_ref(&row),
            &mut bindings,
        )
        .await;
    }

    let binding_refs: Vec<&TaskBinding> = bindings.iter().collect();
    let count_refs: Vec<&ConnectionMessageCount> = counts.iter().collect();
    let count_by_binding = match_counts(&binding_refs, &count_refs);

    let connections: Vec<Connection> = binding_refs
        .iter()
        .map(|b| {
            decode_connection(
                binding_status_for(adapters, b.adapter_kind.as_str()),
                b,
                &count_by_binding,
            )
        })
        .collect();

    Ok(connections)
}

async fn refresh_stale_statuses(
    adapters: Option<&Arc<Adapters>>,
    task_bindings: &Bindings,
    rows: &[moatless_tasks::task::models::Task],
    bindings: &mut [TaskBinding],
) {
    let created_by: HashMap<&str, &str> = rows
        .iter()
        .filter_map(|r| r.created_by.as_deref().map(|c| (r.task_id.as_str(), c)))
        .collect();

    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::seconds(STATUS_STALE_SECS);

    let Some(adapters) = adapters else {
        return;
    };

    // Which bindings are worth polling is the adapter's answer, not ours: it
    // names the subject and classifies terminal state kinds.
    let targets: Vec<TaskBinding> = bindings
        .iter()
        .filter(|b| {
            adapters
                .binding_status_by_name(b.adapter_kind.as_str())
                .is_some_and(|status| status_service::should_refresh(status, b, cutoff))
        })
        .cloned()
        .collect();

    if targets.is_empty() {
        return;
    }

    let fetched: HashMap<String, ExternalStatus> = stream::iter(targets)
        .map(|binding| {
            let creator = created_by
                .get(binding.task_id.as_str())
                .map_or_else(String::new, |c| (*c).to_string());
            async move {
                status_service::get_or_fetch(Some(adapters), task_bindings, &binding, &creator)
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

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_ports::identity::MockUserDisplay;
    use moatless_ports::workspace::MockRepositoryReader;
    use moatless_tasks::message::test_support::messages_without_a_database;
    use moatless_tasks::task::test_support::ScriptedTasks;

    fn disconnected_pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap()
    }

    #[tokio::test]
    async fn non_admin_list_is_refused_before_any_read() {
        let (tasks, calls) = ScriptedTasks::new().commands();
        let users = MockUserDisplay::new();
        let repositories = MockRepositoryReader::new();
        let bindings = moatless_loops::compose::loop_graph(disconnected_pool()).bindings();
        let messages = messages_without_a_database();
        let result = list_task_connections(
            TaskConnectionReads {
                tasks: &tasks,
                users: &users,
                repositories: &repositories,
                task_bindings: &bindings,
                messages: &messages,
                adapters: None,
            },
            &ScopeContext {
                user_id: "reader".into(),
                is_admin: false,
            },
            ListTaskConnectionsQuery::default(),
        )
        .await;
        assert!(matches!(result, Err(TaskError::Forbidden(_))));
        assert!(calls.list_tasks().is_empty());
    }

    #[tokio::test]
    async fn empty_page_retains_total_and_live_task_filter() {
        let (tasks, calls) = ScriptedTasks::new().listing(Vec::new(), 51).commands();
        let users = MockUserDisplay::new();
        let mut repositories = MockRepositoryReader::new();
        repositories
            .expect_names()
            .withf(<[String]>::is_empty)
            .return_once(|_| Ok(HashMap::new()));
        let bindings = moatless_loops::compose::loop_graph(disconnected_pool()).bindings();
        let messages = messages_without_a_database();
        let result = list_task_connections(
            TaskConnectionReads {
                tasks: &tasks,
                users: &users,
                repositories: &repositories,
                task_bindings: &bindings,
                messages: &messages,
                adapters: None,
            },
            &ScopeContext {
                user_id: "admin".into(),
                is_admin: true,
            },
            ListTaskConnectionsQuery {
                limit: Some(20),
                offset: Some(60),
                refresh: Some(false),
            },
        )
        .await
        .unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.pagination.total, 51);
        assert!(!result.pagination.has_more);
        let filters = calls.list_tasks();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].exclude_status, Some(TaskStatus::Closed));
        assert_eq!(filters[0].limit, 20);
        assert_eq!(filters[0].offset, 60);
    }

    #[tokio::test]
    async fn per_task_lookup_preserves_caller_scope_before_enrichment() {
        let (tasks, calls) = ScriptedTasks::new().holding(None).commands();
        let result = get_task_connections(
            &tasks,
            &moatless_loops::compose::loop_graph(disconnected_pool()).bindings(),
            &messages_without_a_database(),
            None,
            &ScopeContext {
                user_id: "reader".into(),
                is_admin: false,
            },
            "hidden".into(),
            GetTaskConnectionsQuery {
                refresh: Some(true),
            },
        )
        .await;
        assert!(matches!(result, Err(TaskError::NotFound(id)) if id == "hidden"));
        let reads = calls.find_by_id();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].0, "hidden");
        assert_eq!(reads[0].1.user_id, "reader");
        assert!(!reads[0].1.is_admin);
    }
}
