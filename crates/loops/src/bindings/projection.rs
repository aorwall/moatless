//! Task binding and reply target wire projections.

use moatless_adapter_model::error::ReplyTarget;
use moatless_api_schemas::bindings::{ReplyTargetDescriptor, TaskBindingResponse};
use moatless_types::adapter::TaskBinding;

pub(crate) fn task_binding_response(row: &TaskBinding) -> TaskBindingResponse {
    TaskBindingResponse {
        id: row.id.clone(),
        task_id: row.task_id.clone(),
        adapter_kind: row.adapter_kind,
        connection_id: row.connection_id.clone(),
        loop_id: row.loop_id.clone(),
        subject_kind: row.subject_kind.clone(),
        subject_key: row.subject_key.clone(),
        destination_config: row.destination_config.clone(),
        metadata: row.metadata.clone(),
        last_external_event_id: row.last_external_event_id.clone(),
        origin: row.origin.clone(),
        created_at: row.created_at.to_rfc3339(),
        last_event_at: row.last_event_at.to_rfc3339(),
    }
}

pub(crate) fn binding_descriptor(binding: &TaskBinding) -> ReplyTargetDescriptor {
    ReplyTargetDescriptor {
        kind: "binding".to_string(),
        id: binding.id.clone(),
        label: format!("{} {}", binding.subject_kind, binding.subject_key),
        flag_hint: format!("--binding {}", binding.id),
    }
}

pub(crate) fn binding_reply_target(binding: &TaskBinding) -> ReplyTarget {
    ReplyTarget {
        kind: "binding".to_string(),
        id: binding.id.clone(),
        label: format!("{} {}", binding.subject_kind, binding.subject_key),
        flag_hint: format!("--binding {}", binding.id),
    }
}
