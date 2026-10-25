//! Authorized creation of external Task bindings.

use super::access::ensure_task_access_with;
use super::operations::Bindings;
use crate::loops::operations::Loops;
use chrono::Utc;
use moatless_adapter_model::binding::BindScope;
use moatless_adapter_model::connections::AdapterConnections;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_api_schemas::bindings::CreateTaskBindingRequest;
use moatless_ports::task::TaskReader;
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::TaskBinding;

/// Create a binding after authorizing its Task, connection, and optional Loop.
pub(crate) async fn create_task_binding(
    tasks: &dyn TaskReader,
    task_bindings: &Bindings,
    adapter_connections: &AdapterConnections,
    loops: &Loops,
    ctx: &ScopeContext,
    task_id: &str,
    req: CreateTaskBindingRequest,
) -> Result<TaskBinding, AdapterError> {
    let task = ensure_task_access_with(tasks, ctx, task_id).await?;
    let adapter_kind = req.adapter_kind;
    let connection = adapter_connections
        .for_auth(ctx, &req.connection_id)
        .await?;
    if connection.adapter_kind != adapter_kind {
        return Err(SharedAdapterError::BadRequest(format!(
            "Connection {} is for adapter {}, not {}",
            connection.id, connection.adapter_kind, adapter_kind
        ))
        .into());
    }
    if let Some(loop_id) = req.loop_id.as_deref() {
        let trigger = loops.load_for_auth(ctx, loop_id).await?;
        let crate::loops::models::LoopSourceRow::AdapterEvent(source) = trigger.source else {
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
        metadata: binding_metadata(req.metadata, req.close_task_on_close)?,
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
        watching: true,
    };
    let saved = task_bindings.upsert(&row).await?;
    Ok(saved)
}

/// The caller's `metadata`, with the close opt-in written under the key
/// routing reads.
fn binding_metadata(
    metadata: Option<serde_json::Value>,
    close_task_on_close: bool,
) -> Result<serde_json::Value, AdapterError> {
    let metadata = metadata.unwrap_or_else(|| serde_json::json!({}));
    if !close_task_on_close {
        return Ok(metadata);
    }
    let serde_json::Value::Object(mut metadata) = metadata else {
        return Err(SharedAdapterError::BadRequest(
            "metadata must be a JSON object to carry close_task_on_close".into(),
        )
        .into());
    };
    BindScope {
        close_on_subject_close: true,
        ..BindScope::default()
    }
    .write_into(&mut metadata);
    Ok(metadata.into())
}

#[cfg(test)]
mod tests {
    use super::binding_metadata;

    #[test]
    fn the_close_opt_in_joins_the_callers_metadata() {
        let metadata = binding_metadata(Some(serde_json::json!({ "k": 1 })), true).unwrap();
        assert_eq!(
            metadata,
            serde_json::json!({ "k": 1, "close_on_subject_close": true })
        );
        assert_eq!(
            binding_metadata(None, false).unwrap(),
            serde_json::json!({})
        );
        assert!(binding_metadata(Some(serde_json::json!([])), true).is_err());
    }
}
