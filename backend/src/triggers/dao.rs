//! Persistence for unified triggers.

use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use crate::db::DbPool;
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedEntity};

use crate::adapters::models::RoutingMode;

#[derive(Debug, Clone)]
pub struct TriggerRow {
    pub id: String,
    pub name: Option<String>,
    pub kind: String,
    pub active: bool,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TriggerRuleRow {
    pub trigger_id: String,
    pub repository_id: String,
    pub routing_mode: RoutingMode,
    pub ongoing_task_id: Option<String>,
    pub prompt: Option<String>,
    pub task_name: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub inactive_ttl_days: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct AdapterEventTriggerSourceRow {
    pub trigger_id: String,
    pub adapter_kind: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub adapter_config: Option<serde_json::Value>,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
}

#[derive(Debug, Clone)]
pub struct ScheduleTriggerSourceRow {
    pub trigger_id: String,
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: DateTime<Utc>,
    pub locked_by: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub enum TriggerSourceRow {
    AdapterEvent(AdapterEventTriggerSourceRow),
    Schedule(ScheduleTriggerSourceRow),
    Manual,
}

#[derive(Debug, Clone)]
pub struct TriggerRecord {
    pub trigger: TriggerRow,
    pub rule: TriggerRuleRow,
    pub source: TriggerSourceRow,
}

#[derive(sqlx::FromRow)]
struct JoinedTriggerRecordRow {
    id: String,
    name: Option<String>,
    kind: String,
    active: bool,
    scope: Scope,
    team_id: Option<String>,
    owner_user_id: Option<String>,
    created_by: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    repository_id: String,
    routing_mode: RoutingMode,
    ongoing_task_id: Option<String>,
    prompt: Option<String>,
    task_name: Option<String>,
    tag_ids: Vec<String>,
    run_as_user_id: Option<String>,
    agent_type: String,
    inactive_ttl_days: Option<i32>,
    adapter_kind: Option<String>,
    connection_id: Option<String>,
    source_matcher: Option<String>,
    source_name: Option<String>,
    event_filter: Option<serde_json::Value>,
    adapter_config: Option<serde_json::Value>,
    allow_direct_send: Option<bool>,
    close_task_on_close: Option<bool>,
    notify_on_success: Option<bool>,
    cron_expression: Option<String>,
    timezone: Option<String>,
    message_template: Option<String>,
    last_fired_at: Option<DateTime<Utc>>,
    next_fire_at: Option<DateTime<Utc>>,
    locked_by: Option<String>,
    locked_until: Option<DateTime<Utc>>,
}

impl JoinedTriggerRecordRow {
    fn into_record(self) -> Result<TriggerRecord, sqlx::Error> {
        let trigger = TriggerRow {
            id: self.id.clone(),
            name: self.name,
            kind: self.kind.clone(),
            active: self.active,
            scope: self.scope,
            team_id: self.team_id,
            owner_user_id: self.owner_user_id,
            created_by: self.created_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        };
        let rule = TriggerRuleRow {
            trigger_id: self.id.clone(),
            repository_id: self.repository_id,
            routing_mode: self.routing_mode,
            ongoing_task_id: self.ongoing_task_id,
            prompt: self.prompt,
            task_name: self.task_name,
            tag_ids: self.tag_ids,
            run_as_user_id: self.run_as_user_id,
            agent_type: self.agent_type,
            inactive_ttl_days: self.inactive_ttl_days,
        };
        let source = match self.kind.as_str() {
            "adapter_event" => TriggerSourceRow::AdapterEvent(AdapterEventTriggerSourceRow {
                trigger_id: self.id,
                adapter_kind: self.adapter_kind.ok_or(sqlx::Error::RowNotFound)?,
                connection_id: self.connection_id.ok_or(sqlx::Error::RowNotFound)?,
                source_matcher: self.source_matcher.ok_or(sqlx::Error::RowNotFound)?,
                source_name: self.source_name,
                event_filter: self.event_filter.unwrap_or_else(|| serde_json::json!([])),
                adapter_config: self.adapter_config,
                allow_direct_send: self.allow_direct_send.unwrap_or(false),
                close_task_on_close: self.close_task_on_close.unwrap_or(true),
                notify_on_success: self.notify_on_success.unwrap_or(false),
            }),
            "schedule" => TriggerSourceRow::Schedule(ScheduleTriggerSourceRow {
                trigger_id: self.id,
                cron_expression: self.cron_expression.ok_or(sqlx::Error::RowNotFound)?,
                timezone: self.timezone.unwrap_or_else(|| "UTC".to_string()),
                message_template: self.message_template.ok_or(sqlx::Error::RowNotFound)?,
                last_fired_at: self.last_fired_at,
                next_fire_at: self.next_fire_at.ok_or(sqlx::Error::RowNotFound)?,
                locked_by: self.locked_by,
                locked_until: self.locked_until,
            }),
            _ => TriggerSourceRow::Manual,
        };

        Ok(TriggerRecord {
            trigger,
            rule,
            source,
        })
    }
}

pub struct PgTriggerRepository(pub DbPool);

#[derive(Debug, Default)]
pub struct TriggerListFilter<'a> {
    pub kind: Option<&'a str>,
    pub adapter_kind: Option<&'a str>,
}

const TRIGGER_RECORD_SELECT: &str = r#"
    SELECT t.id,
           t.name,
           t.kind,
           t.active,
           t.scope,
           t.team_id,
           t.owner_user_id,
           t.created_by,
           t.created_at,
           t.updated_at,
           r.repository_id,
           r.routing_mode,
           r.ongoing_task_id,
           r.prompt,
           r.task_name,
           r.tag_ids,
           r.run_as_user_id,
           r.agent_type,
           r.inactive_ttl_days,
           aes.adapter_kind,
           aes.connection_id,
           aes.source_matcher,
           aes.source_name,
           aes.event_filter,
           aes.adapter_config,
           aes.allow_direct_send,
           aes.close_task_on_close,
           aes.notify_on_success,
           sts.cron_expression,
           sts.timezone,
           sts.message_template,
           sts.last_fired_at,
           sts.next_fire_at,
           sts.locked_by,
           sts.locked_until
    FROM triggers t
    JOIN trigger_rules r ON r.trigger_id = t.id
    LEFT JOIN adapter_event_trigger_sources aes ON aes.trigger_id = t.id
    LEFT JOIN schedule_trigger_sources sts ON sts.trigger_id = t.id
"#;

impl PgTriggerRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<TriggerRecord>, sqlx::Error> {
        let row = sqlx::query_as::<_, JoinedTriggerRecordRow>(AssertSqlSafe(format!(
            "{TRIGGER_RECORD_SELECT} WHERE t.id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.0)
        .await?;

        row.map(JoinedTriggerRecordRow::into_record).transpose()
    }

    pub async fn list(
        &self,
        ctx: &ScopeContext,
        filter: TriggerListFilter<'_>,
    ) -> Result<Vec<TriggerRecord>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Resource, "t.", 1);
        let mut sql = format!("{TRIGGER_RECORD_SELECT} WHERE {}", access.sql());
        let mut next_param = access.next_param();
        if filter.kind.is_some() {
            sql.push_str(" AND t.kind = $");
            sql.push_str(&next_param.to_string());
            next_param += 1;
        }
        if filter.adapter_kind.is_some() {
            sql.push_str(" AND aes.adapter_kind = $");
            sql.push_str(&next_param.to_string());
        }
        sql.push_str(" ORDER BY t.created_at DESC");

        let mut query = sqlx::query_as::<_, JoinedTriggerRecordRow>(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id);
        if let Some(kind) = filter.kind {
            query = query.bind(kind);
        }
        if let Some(adapter_kind) = filter.adapter_kind {
            query = query.bind(adapter_kind);
        }
        let rows = query.fetch_all(&self.0).await?;
        rows.into_iter()
            .map(JoinedTriggerRecordRow::into_record)
            .collect()
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM triggers WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }
}
