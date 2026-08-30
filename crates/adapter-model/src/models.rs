//! Persisted shapes for the Adapter, Loop, Subscription, and Binding
//! entities, plus the adapter access-control vocabularies.
//!
//! The domain types live in `moatless_types::adapter` and
//! `moatless_types::loops`. What is here is the shape the tables have — an
//! `adapter_kind` text column, an open `agent_type` string — and the decode
//! from one to the other.

use serde::{Deserialize, Serialize};

use moatless_types::loops::LoopContext;

// The row shapes below decode into the domain types, so these names are only
// reachable when the store that decodes them is compiled.
#[cfg(feature = "postgres")]
use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use moatless_types::access_control::Scope;
#[cfg(feature = "postgres")]
use moatless_types::agent::AgentType;
#[cfg(feature = "postgres")]
use moatless_types::loops::RoutingMode;

/// The entity vocabulary a caller reaches through this module.
pub use moatless_types::adapter::{AdapterConnection, AdapterKind, BindingContext, TaskBinding};
pub use moatless_types::loops::{LoopSpawnConfig, Schedule, SubscriptionMatch};




/// Every `task_bindings.origin` value the system writes, and the whole of the
/// role taxonomy: the direction in which the task and the external subject met.
///
/// [`ADAPTER_EVENT_BINDING_ORIGIN`] is the responder direction — the subject
/// existed first and a loop spawned a task from it, so this is the one origin
/// the router writes from an inbound event. **Every other origin is a producer**
/// by direction: the task existed first and reached out. Routing decides the
/// role by negating the responder rather than matching a list of producers
/// (`recipients`, in the binding layer), so a fourth origin added here is classified
/// correctly the day it is written rather than the day someone remembers to
/// update a match. That is not hypothetical: `task_owned` lived as a private
/// string inside the Linear API routes, and the recipient resolver — written
/// against the two constants that were declared — read every Linear producer as
/// a responder.
///
/// The stored values remain what they always were, for existing database rows
/// and API compatibility.
pub const ADAPTER_EVENT_BINDING_ORIGIN: &str = "ingress";
/// Producer: the task replied into a subject, or was connected to one.
pub const CONNECTED_THREAD_BINDING_ORIGIN: &str = "connected_thread";
/// Producer: a CLI-driven endpoint filed the subject on the task's behalf.
pub const TASK_OWNED_BINDING_ORIGIN: &str = "task_owned";


// =============================================================================
// Target architecture tables (DB)
// =============================================================================

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
            adapter_kind: parse_adapter_kind(&row.adapter_kind),
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
            adapter_kind: parse_adapter_kind(&row.adapter_kind),
            agent_type: AgentType(row.agent_type),
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
            adapter_kind: parse_adapter_kind(&row.adapter_kind),
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

// =============================================================================
// Domain types (internal, not serialized to API)
// =============================================================================

/// Normalize a stored `adapter_kind` string to its enum, treating an unknown
/// value as `Webhook`.
pub fn parse_adapter_kind(adapter_kind: &str) -> AdapterKind {
    AdapterKind::parse(adapter_kind).unwrap_or(AdapterKind::Webhook)
}

/// Build a synthetic `LoopContext` from a subscription view for adapter
/// trait methods that still accept a config-shaped value.
pub fn subscription_context(subscription: &SubscriptionMatch) -> LoopContext {
    LoopContext {
        id: subscription.id.clone(),
        adapter_kind: subscription.adapter_kind.as_str().to_string(),
        source_id: subscription.source_matcher.clone(),
        source_name: subscription.source_name.clone(),
        repository_id: subscription.repository_id.clone(),
        routing_mode: subscription.routing_mode,
        prompt_template: subscription.prompt_template.clone(),
        active: subscription.active,
        allow_direct_send: subscription.allow_direct_send,
        adapter_config: subscription.adapter_config.clone(),
        ongoing_task_id: subscription.ongoing_task_id.clone(),
        scope: subscription.scope,
        task_name: subscription.task_name.clone(),
        tag_ids: subscription.tag_ids.clone(),
        skills: subscription.skills.clone(),
        run_as_user_id: subscription.run_as_user_id.clone(),
        agent_type: subscription.agent_type.clone(),
        inactive_ttl_days: subscription.inactive_ttl_days,
        created_by: subscription.created_by.clone(),
        created_at: subscription.created_at,
        updated_at: subscription.updated_at,
    }
}

/// Build a provider binding context from a task binding. Never persist this row.
pub fn binding_context(binding: &TaskBinding) -> BindingContext {
    let mut destination_config = binding.destination_config.clone();
    if let Some(obj) = destination_config.as_object_mut() {
        if let Some(ext_id) = binding.last_external_event_id.as_ref() {
            obj.insert("last_external_event_id".into(), serde_json::json!(ext_id));
        }
        if let Some(filter_uid) = binding.metadata.get("filter_user_id").cloned() {
            obj.insert("filter_user_id".into(), filter_uid);
        }
    }

    BindingContext {
        binding_id: binding.id.clone(),
        loop_id: binding.loop_id.clone(),
        task_id: binding.task_id.clone(),
        subject_kind: binding.subject_kind.clone(),
        subject_key: binding.subject_key.clone(),
        destination_config,
        metadata: binding.metadata.clone(),
        last_external_event_id: binding.last_external_event_id.clone(),
        last_message_at: binding.last_event_at,
        created_at: binding.created_at,
    }
}


/// The external system's handle for whoever wrote an inbound event, or `None`
/// where it named nobody.
///
/// Adapters read this off webhook fields that are `#[serde(default)]`, so a
/// missing sender arrives as an empty string rather than as an absent one.
pub fn author_handle(handle: &str) -> Option<String> {
    let trimmed = handle.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}



/// Info returned by `send_message` to create a new `BindingContext`.
#[derive(Debug)]
pub struct NewThreadInfo {
    /// Adapter-specific thread/message ID (e.g., Slack message `ts`).
    pub external_thread_id: String,
    /// Adapter-specific properties for the thread (e.g., `channel`, `thread_ts`).
    pub properties: serde_json::Value,
}

// =============================================================================
// Shared access-control types (used by Telegram, Slack, and future providers)
// =============================================================================

/// Access policy for an external source (DM, group, or Slack channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceAccessPolicy {
    /// Accept messages from any source.
    #[default]
    Open,
    /// Reject all messages.
    Disabled,
    /// Accept only from sources in the allowlist.
    Allowlist,
    /// Allowlist + prompt unknown users with a pairing code (DM only).
    Pairing,
}

/// When to trigger processing of an inbound message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageTrigger {
    /// Only process messages that mention the bot.
    #[default]
    MentionOnly,
    /// Process all messages in the source.
    AllMessages,
}

/// Result of an access control check.
#[derive(Debug)]
pub enum AccessDecision {
    /// Message is allowed through.
    Allowed,
    /// Message is blocked (silently dropped).
    Blocked,
    /// A pairing code was sent to the user; message is not processed.
    PairingSent,
}

#[cfg(test)]
mod tests {
    use super::author_handle;

    /// The three shapes a webhook field arrives in. Whitespace and the empty
    /// string are the same answer as an absent field: the origin chip names who
    /// wrote it or names nobody, and never shows a blank.
    #[test]
    fn a_handle_that_names_nobody_is_nobody() {
        assert_eq!(author_handle("alice").as_deref(), Some("alice"));
        assert_eq!(author_handle("  alice  ").as_deref(), Some("alice"));
        assert_eq!(author_handle(""), None);
        assert_eq!(author_handle("   "), None);
    }
}
