//! Loop wire responses and read filters.

use crate::loops::models::{LoopRecord, LoopSourceRow};
use moatless_api_schemas::loops::{
    Loop, LoopConfig, LoopExecutionState, LoopSource, ManualSource, Schedule, Subscription,
};
use moatless_types::adapter::AdapterKind;
use moatless_types::loops::{LoopConfigSource, LoopKind};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopFilter {
    pub kind: Option<LoopKind>,
    pub adapter_kind: Option<AdapterKind>,
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
            workspace_id: record.config.workspace_id.clone(),
            routing_mode: record.config.routing_mode,
            ongoing_task_id: record.config.ongoing_task_id.clone(),
            task_name: record.config.task_name.clone(),
            prompt: record.config.prompt.clone(),
            tag_ids: record.config.tag_ids.clone(),
            skills: record.config.skills.clone(),
            run_as_user_id: record.config.run_as_user_id.clone(),
            agent_profile: record.config.agent_profile.as_str().to_string(),
            model: record.config.model.clone(),
            close_after_settled_days: record.config.close_after_settled_days,
            settle_after_hours: record.config.settle_after_hours,
            node_selector: record.config.node_selector.clone(),
            stop_when_done: record.config.stop_when_done,
            message_buffer_seconds: record.config.message_buffer_seconds,
            fallback_to_other_agent: record.config.fallback_to_other_agent,
        },
        source,
    }
}
