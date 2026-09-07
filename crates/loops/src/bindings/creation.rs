//! Authorized creation of external Task bindings.

use super::{access::ensure_task_access_with, dao::TaskBindingDao};
use crate::loops::{access::load_loop_for_auth_with, dao::LoopDao};
use chrono::Utc;
use moatless_adapter_model::access::load_adapter_connection_for_auth_with;
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_api_schemas::bindings::CreateTaskBindingRequest;
use moatless_ports::task::TaskReader;
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::TaskBinding;

/// Create a binding after authorizing its Task, connection, and optional Loop.
pub async fn create_task_binding(
    tasks: &dyn TaskReader,
    task_bindings: &dyn TaskBindingDao,
    adapter_connections: &dyn AdapterConnectionDao,
    loops: &LoopDao,
    ctx: &ScopeContext,
    task_id: &str,
    req: CreateTaskBindingRequest,
) -> Result<TaskBinding, AdapterError> {
    let task = ensure_task_access_with(tasks, ctx, task_id).await?;
    let adapter_kind = req.adapter_kind;
    let connection =
        load_adapter_connection_for_auth_with(adapter_connections, ctx, &req.connection_id).await?;
    if connection.adapter_kind != adapter_kind {
        return Err(SharedAdapterError::BadRequest(format!(
            "Connection {} is for adapter {}, not {}",
            connection.id, connection.adapter_kind, adapter_kind
        ))
        .into());
    }
    if let Some(loop_id) = req.loop_id.as_deref() {
        let trigger = load_loop_for_auth_with(loops, ctx, loop_id).await?;
        let crate::loops::dao::LoopSourceRow::AdapterEvent(source) = trigger.source else {
            return Err(SharedAdapterError::BadRequest(format!(
                "Loop {loop_id} is not an adapter-event Loop"
            ))
            .into());
        };
        if source.adapter_kind != adapter_kind {
            return Err(SharedAdapterError::BadRequest(format!(
                "Loop {} is for adapter {}, not {}",
                trigger.loop_.id, source.adapter_kind, adapter_kind
            ))
            .into());
        }
        if source.connection_id != req.connection_id {
            return Err(SharedAdapterError::BadRequest(format!(
                "Loop {} belongs to connection {}, not {}",
                trigger.loop_.id, source.connection_id, req.connection_id
            ))
            .into());
        }
    }
    let now = Utc::now();
    let row = TaskBinding {
        id: format!("tb_{}", uuid::Uuid::new_v4()),
        task_id: task.task_id,
        adapter_kind,
        subject_kind: req.subject_kind,
        subject_key: req.subject_key,
        connection_id: req.connection_id,
        loop_id: req.loop_id,
        destination_config: req
            .destination_config
            .unwrap_or_else(|| serde_json::json!({})),
        metadata: req.metadata.unwrap_or_else(|| serde_json::json!({})),
        last_external_event_id: None,
        origin: req.origin.unwrap_or_else(|| "api".to_string()),
        created_at: now,
        last_event_at: now,
        external_status: None,
        external_state_kind: None,
        external_status_raw: None,
        external_review_decision: None,
        external_mergeable_state: None,
        external_status_updated_at: None,
        active: true,
        deactivated_at: None,
    };
    let saved = task_bindings.upsert(&row).await?;
    Ok(saved)
}
