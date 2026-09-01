//! Building the API and reply shapes the adapter/loop/binding routes return
//! from stored rows, plus the query filters those reads take as input.

use cron::Schedule as CronSchedule;
use serde::Deserialize;

use moatless_adapter_model::error::{AdapterError, ReplyTarget, SharedAdapterError};
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::TaskBinding;
use moatless_api_schemas::adapters::AdapterConnectionResponse;
use moatless_api_schemas::bindings::{ReplyTargetDescriptor, TaskBindingResponse};
use moatless_api_schemas::loops::{
    Loop, LoopConfig, LoopExecutionState, LoopSource, ManualSource, Schedule, Subscription,
};
use moatless_types::adapter::AdapterKind;
use moatless_types::loops::{LoopConfigSource, LoopKind};

use crate::loops::dao::{LoopRecord, LoopSourceRow};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterFilter {
    pub adapter_kind: Option<AdapterKind>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopFilter {
    pub kind: Option<LoopKind>,
    pub adapter_kind: Option<AdapterKind>,
}

pub fn adapter_connection_response(row: &AdapterConnection) -> AdapterConnectionResponse {
    AdapterConnectionResponse {
        id: row.id.clone(),
        adapter_kind: row.adapter_kind,
        connection_kind: row.connection_kind.clone(),
        external_account_id: row.scope_value.clone(),
        created_by: row.created_by.clone(),
        scope: row.scope,
        owner_user_id: row.owner_user_id.clone(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

pub(crate) fn loop_response(record: &LoopRecord) -> Loop {
    let kind = LoopKind::try_from(record.loop_.kind.as_str()).unwrap_or(LoopKind::Manual);
    let config_source = LoopConfigSource::try_from(record.loop_.config_source.as_str())
        .unwrap_or(LoopConfigSource::Manual);
    let execution_state = if record.loop_.active {
        LoopExecutionState::Active
    } else if record.config.run_as_user_id.is_some() {
        LoopExecutionState::Paused
    } else {
        LoopExecutionState::AwaitingApproval
    };
    let source = match &record.source {
        LoopSourceRow::AdapterEvent(source) => LoopSource::AdapterEvent(Subscription {
            adapter_kind: source.adapter_kind,
            connection_id: source.connection_id.clone(),
            source_matcher: source.source_matcher.clone(),
            source_name: source.source_name.clone(),
            event_filter: source.event_filter.clone(),
            adapter_config: source.adapter_config.clone(),
            allow_direct_send: source.allow_direct_send,
            close_task_on_close: source.close_task_on_close,
            notify_on_success: source.notify_on_success,
        }),
        LoopSourceRow::Schedule(source) => LoopSource::Schedule(Schedule {
            cron_expression: source.cron_expression.clone(),
            timezone: source.timezone.clone(),
            message_template: source.message_template.clone(),
            last_fired_at: source.last_fired_at.map(|dt| dt.to_rfc3339()),
            next_fire_at: source.next_fire_at.to_rfc3339(),
            locked_by: source.locked_by.clone(),
            locked_until: source.locked_until.map(|dt| dt.to_rfc3339()),
        }),
        LoopSourceRow::Manual => LoopSource::Manual(ManualSource {}),
    };

    Loop {
        id: record.loop_.id.clone(),
        name: record.loop_.name.clone(),
        kind,
        active: record.loop_.active,
        scope: record.loop_.scope,
        owner_user_id: record.loop_.owner_user_id.clone(),
        created_by: record.loop_.created_by.clone(),
        created_at: record.loop_.created_at.to_rfc3339(),
        updated_at: record.loop_.updated_at.to_rfc3339(),
        config_source,
        synced_from_git: config_source == LoopConfigSource::Git && !record.loop_.deleted,
        source_repository_id: record.loop_.source_repository_id.clone(),
        source_key: record.loop_.source_key.clone(),
        source_config_path: record.loop_.source_config_path.clone(),
        deleted: record.loop_.deleted,
        execution_state,
        config: LoopConfig {
            repository_id: record.config.repository_id.clone(),
            routing_mode: record.config.routing_mode,
            ongoing_task_id: record.config.ongoing_task_id.clone(),
            task_name: record.config.task_name.clone(),
            prompt: record.config.prompt.clone(),
            tag_ids: record.config.tag_ids.clone(),
            skills: record.config.skills.clone(),
            run_as_user_id: record.config.run_as_user_id.clone(),
            agent_type: record.config.agent_type.as_str().to_string(),
            inactive_ttl_days: record.config.inactive_ttl_days,
        },
        source,
    }
}

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

pub(crate) fn next_fire_at(
    schedule_expr: &str,
) -> Result<chrono::DateTime<chrono::Utc>, AdapterError> {
    let schedule: CronSchedule = schedule_expr
        .parse()
        .map_err(|e| SharedAdapterError::BadRequest(format!("Invalid cron expression: {e}")))?;
    schedule.after(&chrono::Utc::now()).next().ok_or_else(|| {
        SharedAdapterError::BadRequest("Schedule has no future fire time".to_string()).into()
    })
}
