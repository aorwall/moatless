//! The persisted shapes of the Adapter, Loop, Subscription, and Binding
//! entities, and the decode from a stored row to its domain type.
//!
//! The domain types live in `moatless_types::adapter` and
//! `moatless_types::loops`. What is here is the shape the tables have — an
//! `adapter_kind` text column, an open `agent_type` string — and the
//! `From<…Row>` that turns one into the other. The decode of the `adapter_kind`
//! column is [`AdapterKind::parse_or_webhook`], which lives with the type in
//! `moatless-types`. The row shapes decode only when the store that reads them
//! is compiled, so the whole module is gated on `postgres`.

// The row shapes below decode into the domain types, so these names are only
// reachable when the store that decodes them is compiled.
#[cfg(feature = "postgres")]
use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use moatless_types::access_control::Scope;
#[cfg(feature = "postgres")]
use moatless_types::adapter::{AdapterConnection, AdapterKind, TaskBinding};
#[cfg(feature = "postgres")]
use moatless_types::agent::AgentType;
#[cfg(feature = "postgres")]
use moatless_types::loops::{LoopSpawnConfig, RoutingMode, Schedule, SubscriptionMatch};

/// Row from the `adapter_connections` table.
#[cfg(feature = "postgres")]
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct AdapterConnectionRow {
    pub id: String,
    pub adapter_kind: String,
    pub connection_kind: String,
    pub scope_value: String,
    pub credential_ref: Option<String>,
    pub webhook_secret: Option<String>,
    pub transport_config: serde_json::Value,
    pub created_by: String,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl From<AdapterConnectionRow> for AdapterConnection {
    fn from(row: AdapterConnectionRow) -> Self {
        Self {
            adapter_kind: AdapterKind::parse_or_webhook(&row.adapter_kind),
            id: row.id,
            connection_kind: row.connection_kind,
            scope_value: row.scope_value,
            credential_ref: row.credential_ref,
            webhook_secret: row.webhook_secret,
            transport_config: row.transport_config,
            created_by: row.created_by,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Row from the `loop_configs` table.
#[cfg(feature = "postgres")]
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct LoopSpawnConfigRow {
    pub id: String,
    pub repository_id: String,
    #[sqlx(try_from = "String")]
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub model: Option<String>,
    pub inactive_ttl_days: Option<i32>,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
impl From<LoopSpawnConfigRow> for LoopSpawnConfig {
    fn from(row: LoopSpawnConfigRow) -> Self {
        Self {
            agent_type: AgentType(row.agent_type),
            model: row.model,
            id: row.id,
            repository_id: row.repository_id,
            routing_mode: row.routing_mode,
            ongoing_task_id: row.ongoing_task_id,
            prompt_template: row.prompt_template,
            task_name: row.task_name,
            tag_ids: row.tag_ids,
            skills: row.skills,
            run_as_user_id: row.run_as_user_id,
            inactive_ttl_days: row.inactive_ttl_days,
            scope: row.scope,
            owner_user_id: row.owner_user_id,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Row from the query that resolves a Subscription against its Loop and config.
#[cfg(feature = "postgres")]
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
    pub repository_id: String,
    pub prompt_template: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub skills: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub model: Option<String>,
    pub inactive_ttl_days: Option<i32>,
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

#[cfg(feature = "postgres")]
impl From<SubscriptionMatchRow> for SubscriptionMatch {
    fn from(row: SubscriptionMatchRow) -> Self {
        Self {
            adapter_kind: AdapterKind::parse_or_webhook(&row.adapter_kind),
            agent_type: AgentType(row.agent_type),
            model: row.model,
            id: row.id,
            name: row.name,
            connection_id: row.connection_id,
            source_matcher: row.source_matcher,
            source_name: row.source_name,
            routing_mode: row.routing_mode,
            event_filter: row.event_filter,
            repository_id: row.repository_id,
            prompt_template: row.prompt_template,
            task_name: row.task_name,
            tag_ids: row.tag_ids,
            skills: row.skills,
            run_as_user_id: row.run_as_user_id,
            inactive_ttl_days: row.inactive_ttl_days,
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

/// Row from the `task_bindings` table.
///
/// The `external_*` and `active` columns are `#[sqlx(default)]` because many
/// existing `SELECT`s do not project them. `active` defaults to `false`, the
/// fail-safe direction: a forgotten projection drops delivery rather than
/// resurrecting a Task. The shared `TASK_BINDING_SELECT` and the `upsert`
/// `RETURNING` both project it, so live paths see the real value.
#[cfg(feature = "postgres")]
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskBindingRow {
    pub id: String,
    pub task_id: String,
    pub adapter_kind: String,
    pub subject_kind: String,
    pub subject_key: String,
    pub connection_id: String,
    pub loop_id: Option<String>,
    pub destination_config: serde_json::Value,
    pub metadata: serde_json::Value,
    pub last_external_event_id: Option<String>,
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
    #[sqlx(default)]
    pub external_status: Option<String>,
    #[sqlx(default)]
    pub external_state_kind: Option<String>,
    #[sqlx(default)]
    pub external_status_raw: Option<serde_json::Value>,
    #[sqlx(default)]
    pub external_review_decision: Option<String>,
    #[sqlx(default)]
    pub external_mergeable_state: Option<String>,
    #[sqlx(default)]
    pub external_status_updated_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub active: bool,
    #[sqlx(default)]
    pub deactivated_at: Option<DateTime<Utc>>,
}

#[cfg(feature = "postgres")]
impl From<TaskBindingRow> for TaskBinding {
    fn from(row: TaskBindingRow) -> Self {
        Self {
            adapter_kind: AdapterKind::parse_or_webhook(&row.adapter_kind),
            id: row.id,
            task_id: row.task_id,
            subject_kind: row.subject_kind,
            subject_key: row.subject_key,
            connection_id: row.connection_id,
            loop_id: row.loop_id,
            destination_config: row.destination_config,
            metadata: row.metadata,
            last_external_event_id: row.last_external_event_id,
            origin: row.origin,
            created_at: row.created_at,
            last_event_at: row.last_event_at,
            external_status: row.external_status,
            external_state_kind: row.external_state_kind,
            external_status_raw: row.external_status_raw,
            external_review_decision: row.external_review_decision,
            external_mergeable_state: row.external_mergeable_state,
            external_status_updated_at: row.external_status_updated_at,
            active: row.active,
            deactivated_at: row.deactivated_at,
        }
    }
}

/// Row from the `schedules` table.
#[cfg(feature = "postgres")]
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

#[cfg(feature = "postgres")]
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
