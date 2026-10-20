//! Persisted Loop configuration, subscription, and schedule rows.

use crate::loops::models::{LoopSpawnConfig, Schedule};
use chrono::{DateTime, Utc};
use moatless_types::access_control::Scope;
use moatless_types::adapter::AdapterKind;
use moatless_types::agent::AgentProfile;
use moatless_types::loops::{RoutingMode, SubscriptionMatch};
use std::collections::BTreeMap;

/// Row from the `loop_configs` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct LoopSpawnConfigRow {
    pub id: String,
    pub workspace_id: String,
    #[sqlx(try_from = "String")]
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub memory_preload: Vec<String>,
    pub run_as_user_id: Option<String>,
    #[sqlx(try_from = "String")]
    pub agent_profile: AgentProfile,
    pub model: Option<String>,
    pub inactive_ttl_days: Option<i32>,
    pub node_selector: Option<sqlx::types::Json<BTreeMap<String, String>>>,
    pub stop_when_done: Option<bool>,
    pub message_buffer_seconds: Option<i32>,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<LoopSpawnConfigRow> for LoopSpawnConfig {
    fn from(row: LoopSpawnConfigRow) -> Self {
        Self {
            agent_profile: row.agent_profile,
            model: row.model,
            id: row.id,
            workspace_id: row.workspace_id,
            routing_mode: row.routing_mode,
            ongoing_task_id: row.ongoing_task_id,
            prompt_template: row.prompt_template,
            task_name: row.task_name,
            tag_ids: row.tag_ids,
            skills: row.skills,
            memory_preload: row.memory_preload,
            run_as_user_id: row.run_as_user_id,
            inactive_ttl_days: row.inactive_ttl_days,
            node_selector: row.node_selector.map(|json| json.0),
            stop_when_done: row.stop_when_done,
            message_buffer_seconds: row.message_buffer_seconds,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Row from the query that resolves a Subscription against its Loop and config.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct SubscriptionMatchRow {
    pub id: String,
    pub name: String,
    pub adapter_kind: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    #[sqlx(try_from = "String")]
    pub routing_mode: RoutingMode,
    pub event_filter: serde_json::Value,
    pub workspace_id: String,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub memory_preload: Vec<String>,
    pub run_as_user_id: Option<String>,
    #[sqlx(try_from = "String")]
    pub agent_profile: AgentProfile,
    pub model: Option<String>,
    pub inactive_ttl_days: Option<i32>,
    pub node_selector: Option<sqlx::types::Json<BTreeMap<String, String>>>,
    pub stop_when_done: Option<bool>,
    pub message_buffer_seconds: Option<i32>,
    pub allow_direct_send: bool,
    pub adapter_config: Option<serde_json::Value>,
    pub ongoing_task_id: Option<String>,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
    pub active: bool,
    pub created_by: String,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<SubscriptionMatchRow> for SubscriptionMatch {
    fn from(row: SubscriptionMatchRow) -> Self {
        Self {
            adapter_kind: AdapterKind::parse_or_webhook(&row.adapter_kind),
            agent_profile: row.agent_profile,
            model: row.model,
            id: row.id,
            name: row.name,
            connection_id: row.connection_id,
            source_matcher: row.source_matcher,
            source_name: row.source_name,
            routing_mode: row.routing_mode,
            event_filter: row.event_filter,
            workspace_id: row.workspace_id,
            prompt_template: row.prompt_template,
            task_name: row.task_name,
            tag_ids: row.tag_ids,
            skills: row.skills,
            memory_preload: row.memory_preload,
            run_as_user_id: row.run_as_user_id,
            inactive_ttl_days: row.inactive_ttl_days,
            node_selector: row.node_selector.map(|json| json.0),
            stop_when_done: row.stop_when_done,
            message_buffer_seconds: row.message_buffer_seconds,
            allow_direct_send: row.allow_direct_send,
            adapter_config: row.adapter_config,
            ongoing_task_id: row.ongoing_task_id,
            close_task_on_close: row.close_task_on_close,
            notify_on_success: row.notify_on_success,
            active: row.active,
            created_by: row.created_by,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Row from the `schedules` table.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ScheduleRow {
    pub id: String,
    pub loop_id: String,
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub run_as_user_id: Option<String>,
    pub active: bool,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
    pub created_by: String,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<ScheduleRow> for Schedule {
    fn from(row: ScheduleRow) -> Self {
        Self {
            id: row.id,
            loop_id: row.loop_id,
            cron_expression: row.cron_expression,
            timezone: row.timezone,
            message_template: row.message_template,
            run_as_user_id: row.run_as_user_id,
            active: row.active,
            last_fired_at: row.last_fired_at,
            next_fire_at: row.next_fire_at,
            locked_by: row.locked_by,
            locked_until: row.locked_until,
            created_by: row.created_by,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
