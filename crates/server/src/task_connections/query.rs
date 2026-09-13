//! Authorized Task connection reads and bounded live-status enrichment.

use super::projection::{binding_status_for, decode_connection, match_counts};
use futures::stream::{self, StreamExt};
use moatless_adapter_model::registry::Adapters;
use moatless_api_schemas::task_connections::{Connection, GetTaskConnectionsQuery};
use moatless_loops::bindings::{operations::Bindings, status_service};
use moatless_tasks::{
    message::operations::Messages,
    task::{error::TaskError, operations::TaskCommands},
};
use moatless_types::{
    access_control::ScopeContext,
    adapter::{ExternalStatus, TaskBinding},
};
use std::collections::HashMap;
use std::sync::Arc;

const STATUS_STALE_SECS: i64 = 300;
const REFRESH_CONCURRENCY: usize = 8;

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
        refresh_stale_statuses(adapters, task_bindings, &row, &mut bindings).await;
    }

    let count_by_binding = match_counts(&bindings, &counts);

    let connections: Vec<Connection> = bindings
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
    row: &moatless_tasks::task::models::Task,
    bindings: &mut [TaskBinding],
) {
    let creator = row.created_by.clone().unwrap_or_default();

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
            let creator = creator.clone();
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
    use moatless_tasks::message::test_support::messages_without_a_database;
    use moatless_tasks::task::test_support::ScriptedTasks;

    fn disconnected_pool() -> sqlx::PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap()
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
