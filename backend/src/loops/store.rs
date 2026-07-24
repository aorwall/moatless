//! `LoopStore` — the single home for every read and write of the four tables
//! that persist a Loop: `triggers`, `trigger_rules`,
//! `adapter_event_trigger_sources`, and `schedule_trigger_sources`.
//!
//! A Loop is the standing definition of recurring agent work (CONTEXT.md,
//! ADR-0001). Its persistence was previously spread across `dao.rs`,
//! `rule_dao.rs`, HTTP handlers, the schedule runner, and the git-loop
//! reconciler, each hand-rolling overlapping SQL. This module consolidates all
//! of that behind one plain struct holding a `DbPool` (the
//! `TaskCreationService` precedent, ADR-0004) — no new single-impl trait.
//!
//! The SQL here is a verbatim relocation of the previous call sites: same
//! queries, same transactions, one home.

use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use moatless_backend_core::scope::{Scope, ScopeContext, ScopedEntity};

use crate::adapters::models::{RoutingMode, Schedule, LoopSpawnConfig, SubscriptionMatch};
use crate::db::DbPool;

// =============================================================================
// Joined trigger record — the read shape used by the HTTP surface.
// =============================================================================

#[derive(Debug, Clone)]
pub struct LoopRow {
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
pub struct LoopConfigRow {
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
pub struct SubscriptionRow {
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
pub struct ScheduleRow {
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
pub enum LoopSourceRow {
    AdapterEvent(SubscriptionRow),
    Schedule(ScheduleRow),
    Manual,
}

#[derive(Debug, Clone)]
pub struct LoopRecord {
    pub trigger: LoopRow,
    pub rule: LoopConfigRow,
    pub source: LoopSourceRow,
}

#[derive(sqlx::FromRow)]
struct JoinedLoopRecordRow {
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

impl JoinedLoopRecordRow {
    fn into_record(self) -> Result<LoopRecord, sqlx::Error> {
        let trigger = LoopRow {
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
        let rule = LoopConfigRow {
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
            "adapter_event" => LoopSourceRow::AdapterEvent(SubscriptionRow {
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
            "schedule" => LoopSourceRow::Schedule(ScheduleRow {
                trigger_id: self.id,
                cron_expression: self.cron_expression.ok_or(sqlx::Error::RowNotFound)?,
                timezone: self.timezone.unwrap_or_else(|| "UTC".to_string()),
                message_template: self.message_template.ok_or(sqlx::Error::RowNotFound)?,
                last_fired_at: self.last_fired_at,
                next_fire_at: self.next_fire_at.ok_or(sqlx::Error::RowNotFound)?,
                locked_by: self.locked_by,
                locked_until: self.locked_until,
            }),
            _ => LoopSourceRow::Manual,
        };

        Ok(LoopRecord {
            trigger,
            rule,
            source,
        })
    }
}

#[derive(Debug, Default)]
pub struct LoopListFilter<'a> {
    pub kind: Option<&'a str>,
    pub adapter_kind: Option<&'a str>,
}

const LOOP_RECORD_SELECT: &str = r#"
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

// =============================================================================
// Write inputs — resolved values ready to persist. Callers own validation,
// scope resolution, and coalescing; the store owns only the SQL.
// =============================================================================

/// A fully-resolved new Loop, ready to insert.
pub struct NewLoop {
    pub id: String,
    pub name: Option<String>,
    pub kind: String,
    pub active: bool,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub rule: NewLoopConfig,
    pub source: NewLoopSource,
}

/// Resolved `trigger_rules` values shared by create and update.
pub struct NewLoopConfig {
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

pub enum NewLoopSource {
    AdapterEvent(NewAdapterEventSource),
    Schedule(NewScheduleSource),
    Manual,
}

pub struct NewAdapterEventSource {
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

pub struct NewScheduleSource {
    pub cron_expression: String,
    pub timezone: String,
    pub message_template: String,
    pub next_fire_at: DateTime<Utc>,
}

/// Resolved `triggers` columns to update (identity + scope, not the rule/source).
pub struct LoopUpdate {
    pub name: Option<String>,
    pub active: bool,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
}

/// A resolved source update. Unlike create, `adapter_kind` is immutable so the
/// adapter variant carries no kind.
pub enum UpdateLoopSource {
    AdapterEvent(AdapterSourceUpdate),
    Schedule(NewScheduleSource),
}

pub struct AdapterSourceUpdate {
    pub connection_id: String,
    pub source_matcher: String,
    pub source_name: Option<String>,
    pub event_filter: serde_json::Value,
    pub adapter_config: Option<serde_json::Value>,
    pub allow_direct_send: bool,
    pub close_task_on_close: bool,
    pub notify_on_success: bool,
}

/// A managed (`managed_by = 'repo_loop'`) Loop reconciled from a repository's
/// `.moatless/loops/` tree, keyed by `(source_repository_id, loop_name)`.
pub struct ManagedLoopUpsert {
    pub source_repository_id: String,
    pub loop_name: String,
    pub trigger_name: Option<String>,
    pub kind: String,
    pub active: bool,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub routing_mode: RoutingMode,
    pub prompt: Option<String>,
    pub tag_ids: Vec<String>,
    pub run_as_user_id: Option<String>,
    pub agent_type: String,
    pub inactive_ttl_days: Option<i32>,
    pub adapter: Option<ManagedAdapterSource>,
    pub schedule: Option<NewScheduleSource>,
}

pub struct ManagedAdapterSource {
    pub adapter_kind: String,
    pub connection_id: String,
    pub source_matcher: String,
    pub event_filter: serde_json::Value,
    pub adapter_config: Option<serde_json::Value>,
}

// =============================================================================
// LoopStore
// =============================================================================

/// The single owner of persistence for the four Loop tables.
pub struct LoopStore {
    db: DbPool,
}

impl LoopStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    // ---------------------------------------------------------------------
    // HTTP surface: get / list / create / update / delete
    // ---------------------------------------------------------------------

    pub async fn get(&self, id: &str) -> Result<Option<LoopRecord>, sqlx::Error> {
        let row = sqlx::query_as::<_, JoinedLoopRecordRow>(AssertSqlSafe(format!(
            "{LOOP_RECORD_SELECT} WHERE t.id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.db)
        .await?;

        row.map(JoinedLoopRecordRow::into_record).transpose()
    }

    pub async fn list(
        &self,
        ctx: &ScopeContext,
        filter: LoopListFilter<'_>,
    ) -> Result<Vec<LoopRecord>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Resource, "t.", 1);
        let mut sql = format!("{LOOP_RECORD_SELECT} WHERE {}", access.sql());
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

        let mut query = sqlx::query_as::<_, JoinedLoopRecordRow>(AssertSqlSafe(sql))
            .bind(ctx.is_admin)
            .bind(&ctx.team_ids)
            .bind(&ctx.user_id);
        if let Some(kind) = filter.kind {
            query = query.bind(kind);
        }
        if let Some(adapter_kind) = filter.adapter_kind {
            query = query.bind(adapter_kind);
        }
        let rows = query.fetch_all(&self.db).await?;
        rows.into_iter()
            .map(JoinedLoopRecordRow::into_record)
            .collect()
    }

    /// Insert a Loop (trigger + rule + optional source) in one transaction.
    pub async fn create(&self, new: NewLoop) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;

        sqlx::query(
            r#"INSERT INTO triggers
               (id, name, kind, active, scope, team_id, owner_user_id,
                created_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)"#,
        )
        .bind(&new.id)
        .bind(&new.name)
        .bind(&new.kind)
        .bind(new.active)
        .bind(new.scope)
        .bind(&new.team_id)
        .bind(&new.owner_user_id)
        .bind(&new.created_by)
        .bind(new.created_at)
        .execute(&mut *tx)
        .await?;

        insert_config(&mut tx, &new.id, &new.rule).await?;

        match &new.source {
            NewLoopSource::AdapterEvent(source) => {
                insert_adapter_event_source(&mut tx, &new.id, source).await?;
            }
            NewLoopSource::Schedule(source) => {
                insert_schedule_source(&mut tx, &new.id, source).await?;
            }
            NewLoopSource::Manual => {}
        }

        tx.commit().await
    }

    /// Update a Loop's trigger row, and optionally its rule and/or source.
    pub async fn update(
        &self,
        id: &str,
        trigger: LoopUpdate,
        rule: Option<NewLoopConfig>,
        source: Option<UpdateLoopSource>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;

        sqlx::query(
            r#"UPDATE triggers
               SET name = $2,
                   active = $3,
                   scope = $4,
                   team_id = $5,
                   owner_user_id = $6,
                   updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(&trigger.name)
        .bind(trigger.active)
        .bind(trigger.scope)
        .bind(&trigger.team_id)
        .bind(&trigger.owner_user_id)
        .execute(&mut *tx)
        .await?;

        if let Some(rule) = rule {
            sqlx::query(
                r#"UPDATE trigger_rules
                   SET repository_id = $2,
                       routing_mode = $3,
                       ongoing_task_id = $4,
                       prompt = $5,
                       task_name = $6,
                       tag_ids = $7,
                       run_as_user_id = $8,
                       agent_type = $9,
                       inactive_ttl_days = $10
                   WHERE trigger_id = $1"#,
            )
            .bind(id)
            .bind(&rule.repository_id)
            .bind(rule.routing_mode)
            .bind(&rule.ongoing_task_id)
            .bind(&rule.prompt)
            .bind(&rule.task_name)
            .bind(&rule.tag_ids)
            .bind(&rule.run_as_user_id)
            .bind(&rule.agent_type)
            .bind(rule.inactive_ttl_days)
            .execute(&mut *tx)
            .await?;
        }

        match source {
            Some(UpdateLoopSource::AdapterEvent(source)) => {
                sqlx::query(
                    r#"UPDATE adapter_event_trigger_sources
                       SET connection_id = $2,
                           source_matcher = $3,
                           source_name = $4,
                           event_filter = $5,
                           adapter_config = $6,
                           allow_direct_send = $7,
                           close_task_on_close = $8,
                           notify_on_success = $9
                       WHERE trigger_id = $1"#,
                )
                .bind(id)
                .bind(&source.connection_id)
                .bind(&source.source_matcher)
                .bind(&source.source_name)
                .bind(&source.event_filter)
                .bind(&source.adapter_config)
                .bind(source.allow_direct_send)
                .bind(source.close_task_on_close)
                .bind(source.notify_on_success)
                .execute(&mut *tx)
                .await?;
            }
            Some(UpdateLoopSource::Schedule(source)) => {
                sqlx::query(
                    r#"UPDATE schedule_trigger_sources
                       SET cron_expression = $2,
                           timezone = $3,
                           message_template = $4,
                           next_fire_at = $5
                       WHERE trigger_id = $1"#,
                )
                .bind(id)
                .bind(&source.cron_expression)
                .bind(&source.timezone)
                .bind(&source.message_template)
                .bind(source.next_fire_at)
                .execute(&mut *tx)
                .await?;
            }
            None => {}
        }

        tx.commit().await
    }

    /// Delete a Loop by id. Cascades to `trigger_rules` and source rows.
    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM triggers WHERE id = $1")
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(result.rows_affected())
    }

    // ---------------------------------------------------------------------
    // Adapter-event route surface (canonical trigger+route view).
    // ---------------------------------------------------------------------

    pub async fn find_subscription_by_id(
        &self,
        id: &str,
    ) -> Result<Option<SubscriptionMatch>, sqlx::Error> {
        sqlx::query_as::<_, SubscriptionMatch>(AssertSqlSafe(format!(
            "{SUBSCRIPTION_MATCH_SELECT} WHERE t.id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.db)
        .await
    }

    pub async fn find_subscription_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Option<SubscriptionMatch>, sqlx::Error> {
        sqlx::query_as::<_, SubscriptionMatch>(AssertSqlSafe(format!(
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = $2 AND t.active = true ORDER BY t.created_at ASC LIMIT 1"
        )))
        .bind(provider)
        .bind(source_matcher)
        .fetch_optional(&self.db)
        .await
    }

    /// Return ALL rules for a given (provider, source_matcher) — used by
    /// Linear, where multiple listeners can share a team UUID.
    pub async fn list_subscriptions_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Vec<SubscriptionMatch>, sqlx::Error> {
        sqlx::query_as::<_, SubscriptionMatch>(AssertSqlSafe(format!(
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = $2 AND t.active = true ORDER BY t.created_at ASC"
        )))
        .bind(provider)
        .bind(source_matcher)
        .fetch_all(&self.db)
        .await
    }

    pub async fn find_subscription_wildcard(
        &self,
        provider: &str,
    ) -> Result<Option<SubscriptionMatch>, sqlx::Error> {
        sqlx::query_as::<_, SubscriptionMatch>(AssertSqlSafe(format!(
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = '*' AND t.active = true LIMIT 1"
        )))
        .bind(provider)
        .fetch_optional(&self.db)
        .await
    }

    pub async fn list_subscriptions_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<SubscriptionMatch>, sqlx::Error> {
        sqlx::query_as::<_, SubscriptionMatch>(AssertSqlSafe(format!(
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND t.active = true ORDER BY t.created_at ASC"
        )))
        .bind(provider)
        .fetch_all(&self.db)
        .await
    }

    pub async fn upsert_subscription(
        &self,
        row: &SubscriptionMatch,
    ) -> Result<SubscriptionMatch, sqlx::Error> {
        let mut tx = self.db.begin().await?;

        sqlx::query(
            r#"INSERT INTO triggers
               (id, name, kind, active, scope, team_id, owner_user_id,
                created_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (id) DO UPDATE
               SET name            = EXCLUDED.name,
                   kind            = EXCLUDED.kind,
                   active          = EXCLUDED.active,
                   scope           = EXCLUDED.scope,
                   team_id         = EXCLUDED.team_id,
                   owner_user_id   = EXCLUDED.owner_user_id,
                   updated_at      = EXCLUDED.updated_at"#,
        )
        .bind(&row.id)
        .bind(&row.source_name)
        .bind("adapter_event")
        .bind(row.active)
        .bind(row.scope)
        .bind(&row.team_id)
        .bind(&row.owner_user_id)
        .bind(&row.created_by)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO trigger_rules
               (trigger_id, repository_id, routing_mode, ongoing_task_id, prompt,
                task_name, tag_ids, run_as_user_id, agent_type, inactive_ttl_days)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (trigger_id) DO UPDATE
               SET repository_id     = EXCLUDED.repository_id,
                   routing_mode      = EXCLUDED.routing_mode,
                   ongoing_task_id   = EXCLUDED.ongoing_task_id,
                   prompt            = EXCLUDED.prompt,
                   task_name         = EXCLUDED.task_name,
                   tag_ids           = EXCLUDED.tag_ids,
                   run_as_user_id    = EXCLUDED.run_as_user_id,
                   agent_type        = EXCLUDED.agent_type,
                   inactive_ttl_days = EXCLUDED.inactive_ttl_days"#,
        )
        .bind(&row.id)
        .bind(&row.repository_id)
        .bind(row.routing_mode)
        .bind(&row.ongoing_task_id)
        .bind(&row.prompt_template)
        .bind(&row.task_name)
        .bind(&row.tag_ids)
        .bind(&row.run_as_user_id)
        .bind(&row.agent_type)
        .bind(row.inactive_ttl_days)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO adapter_event_trigger_sources
               (trigger_id, adapter_kind, connection_id, source_matcher, source_name,
                event_filter, adapter_config,
                allow_direct_send, close_task_on_close, notify_on_success)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (trigger_id) DO UPDATE
               SET adapter_kind         = EXCLUDED.adapter_kind,
                   connection_id        = EXCLUDED.connection_id,
                   source_matcher       = EXCLUDED.source_matcher,
                   source_name          = EXCLUDED.source_name,
                   event_filter         = EXCLUDED.event_filter,
                   adapter_config       = EXCLUDED.adapter_config,
                   allow_direct_send    = EXCLUDED.allow_direct_send,
                   close_task_on_close  = EXCLUDED.close_task_on_close,
                   notify_on_success    = EXCLUDED.notify_on_success"#,
        )
        .bind(&row.id)
        .bind(&row.provider)
        .bind(&row.connection_id)
        .bind(&row.source_matcher)
        .bind(&row.source_name)
        .bind(&row.event_filter)
        .bind(&row.provider_config)
        .bind(row.allow_direct_send)
        .bind(row.close_task_on_close)
        .bind(row.notify_on_success)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        self.find_subscription_by_id(&row.id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    // ---------------------------------------------------------------------
    // Telegram pairing: adapter_config read-modify-write.
    // ---------------------------------------------------------------------

    /// Read the `adapter_config` JSON for an adapter-event trigger's source.
    /// Returns `Ok(None)` when the trigger has no adapter-event source row (the
    /// outer `Option` is the `adapter_config` column, which is itself nullable).
    pub async fn get_adapter_config(
        &self,
        trigger_id: &str,
    ) -> Result<Option<serde_json::Value>, sqlx::Error> {
        let row: Option<(Option<serde_json::Value>,)> = sqlx::query_as(
            "SELECT adapter_config FROM adapter_event_trigger_sources WHERE trigger_id = $1",
        )
        .bind(trigger_id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.and_then(|(config,)| config))
    }

    /// Overwrite the `adapter_config` JSON for an adapter-event trigger's source
    /// and bump the trigger's `updated_at`, in one transaction.
    pub async fn set_adapter_config(
        &self,
        trigger_id: &str,
        config: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE adapter_event_trigger_sources SET adapter_config = $1 WHERE trigger_id = $2",
        )
        .bind(config)
        .bind(trigger_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE triggers SET updated_at = now() WHERE id = $1")
            .bind(trigger_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    // ---------------------------------------------------------------------
    // Repo-loop reconciliation (managed_by = 'repo_loop').
    // ---------------------------------------------------------------------

    /// Existing `repo_loop` triggers for a repository as `(trigger_id, loop_name)`.
    pub async fn list_managed_loops(
        &self,
        source_repository_id: &str,
    ) -> Result<Vec<(String, String)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, loop_name FROM triggers \
             WHERE source_repository_id = $1 AND managed_by = 'repo_loop' AND loop_name IS NOT NULL",
        )
        .bind(source_repository_id)
        .fetch_all(&self.db)
        .await
    }

    /// Insert or update a `repo_loop` Loop keyed by
    /// `(source_repository_id, loop_name)`. Source rows are replaced so a kind
    /// switch (adapter_event <-> schedule) is clean. Returns `true` when a fresh
    /// trigger was inserted, `false` when an existing one was updated.
    pub async fn upsert_managed_loop(
        &self,
        input: &ManagedLoopUpsert,
    ) -> Result<bool, sqlx::Error> {
        let now = Utc::now();
        let mut tx = self.db.begin().await?;

        let existing_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM triggers \
             WHERE source_repository_id = $1 AND loop_name = $2 AND managed_by = 'repo_loop'",
        )
        .bind(&input.source_repository_id)
        .bind(&input.loop_name)
        .fetch_optional(&mut *tx)
        .await?;

        let (trigger_id, created) = if let Some(id) = existing_id {
            sqlx::query(
                "UPDATE triggers \
                 SET name = $2, kind = $3, active = $4, scope = $5, team_id = $6, \
                     owner_user_id = $7, updated_at = $8 \
                 WHERE id = $1",
            )
            .bind(&id)
            .bind(&input.trigger_name)
            .bind(&input.kind)
            .bind(input.active)
            .bind(input.scope)
            .bind(&input.team_id)
            .bind(&input.owner_user_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            (id, false)
        } else {
            let id = format!("tr_{}", uuid::Uuid::new_v4());
            sqlx::query(
                "INSERT INTO triggers \
                 (id, name, kind, active, scope, team_id, owner_user_id, created_by, \
                  managed_by, source_repository_id, loop_name, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'repo_loop', $9, $10, $11, $11)",
            )
            .bind(&id)
            .bind(&input.trigger_name)
            .bind(&input.kind)
            .bind(input.active)
            .bind(input.scope)
            .bind(&input.team_id)
            .bind(&input.owner_user_id)
            .bind(&input.created_by)
            .bind(&input.source_repository_id)
            .bind(&input.loop_name)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            (id, true)
        };

        // Upsert the rule row (handles both insert + update paths).
        sqlx::query(
            "INSERT INTO trigger_rules \
             (trigger_id, repository_id, routing_mode, ongoing_task_id, prompt, task_name, \
              tag_ids, run_as_user_id, agent_type, inactive_ttl_days) \
             VALUES ($1, $2, $3, NULL, $4, NULL, $5, $6, $7, $8) \
             ON CONFLICT (trigger_id) DO UPDATE SET \
               repository_id = EXCLUDED.repository_id, \
               routing_mode = EXCLUDED.routing_mode, \
               prompt = EXCLUDED.prompt, \
               tag_ids = EXCLUDED.tag_ids, \
               run_as_user_id = EXCLUDED.run_as_user_id, \
               agent_type = EXCLUDED.agent_type, \
               inactive_ttl_days = EXCLUDED.inactive_ttl_days",
        )
        .bind(&trigger_id)
        .bind(&input.source_repository_id)
        .bind(input.routing_mode)
        .bind(&input.prompt)
        .bind(&input.tag_ids)
        .bind(&input.run_as_user_id)
        .bind(&input.agent_type)
        .bind(input.inactive_ttl_days)
        .execute(&mut *tx)
        .await?;

        // Replace source rows so a kind switch (github_pr <-> schedule) is clean.
        sqlx::query("DELETE FROM adapter_event_trigger_sources WHERE trigger_id = $1")
            .bind(&trigger_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM schedule_trigger_sources WHERE trigger_id = $1")
            .bind(&trigger_id)
            .execute(&mut *tx)
            .await?;

        if let Some(adapter) = &input.adapter {
            sqlx::query(
                "INSERT INTO adapter_event_trigger_sources \
                 (trigger_id, adapter_kind, connection_id, source_matcher, source_name, \
                  event_filter, adapter_config, allow_direct_send, close_task_on_close, notify_on_success) \
                 VALUES ($1, $2, $3, $4, NULL, $5, $6, false, true, false)",
            )
            .bind(&trigger_id)
            .bind(&adapter.adapter_kind)
            .bind(&adapter.connection_id)
            .bind(&adapter.source_matcher)
            .bind(&adapter.event_filter)
            .bind(&adapter.adapter_config)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(schedule) = &input.schedule {
            sqlx::query(
                "INSERT INTO schedule_trigger_sources \
                 (trigger_id, cron_expression, timezone, message_template, next_fire_at) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&trigger_id)
            .bind(&schedule.cron_expression)
            .bind(&schedule.timezone)
            .bind(&schedule.message_template)
            .bind(schedule.next_fire_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(created)
    }

    // ---------------------------------------------------------------------
    // Schedule runner: claim due schedules, record a fire, resolve the route.
    // ---------------------------------------------------------------------

    /// Select due, unlocked schedule jobs and lock each one for `worker_id`.
    /// Returns the jobs that were successfully locked. Mirrors the runner's
    /// previous two-statement due-select + per-row lock CTE.
    pub async fn claim_due_schedules(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<Vec<Schedule>, sqlx::Error> {
        let due_jobs = sqlx::query_as::<_, Schedule>(
            r#"SELECT t.id,
                      t.id AS route_id,
                      sts.cron_expression AS schedule_expr,
                      sts.timezone,
                      sts.message_template,
                      r.run_as_user_id,
                      t.active,
                      sts.last_fired_at,
                      sts.next_fire_at,
                      sts.locked_by,
                      sts.locked_until,
                      t.created_by,
                      t.scope,
                      t.team_id,
                      t.owner_user_id,
                      t.created_at,
                      t.updated_at
               FROM triggers t
               JOIN trigger_rules r ON r.trigger_id = t.id
               JOIN schedule_trigger_sources sts ON sts.trigger_id = t.id
               WHERE t.kind = 'schedule'
                 AND t.active = true
                 AND sts.next_fire_at <= now()
                 AND (sts.locked_until IS NULL OR sts.locked_until < now())
               ORDER BY sts.next_fire_at ASC
               LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        let mut locked = Vec::new();
        for job in due_jobs {
            let job_lock = sqlx::query_as::<_, Schedule>(
                r#"WITH locked_source AS (
                       UPDATE schedule_trigger_sources
                       SET locked_by = $2,
                           locked_until = now() + interval '2 minutes'
                       WHERE trigger_id = $1
                         AND next_fire_at <= now()
                         AND (locked_until IS NULL OR locked_until < now())
                       RETURNING *
                   ),
                   touched_trigger AS (
                       UPDATE triggers
                       SET updated_at = now()
                       WHERE id = $1 AND active = true AND kind = 'schedule'
                       RETURNING *
                   )
                   SELECT t.id,
                          t.id AS route_id,
                          s.cron_expression AS schedule_expr,
                          s.timezone,
                          s.message_template,
                          r.run_as_user_id,
                          t.active,
                          s.last_fired_at,
                          s.next_fire_at,
                          s.locked_by,
                          s.locked_until,
                          t.created_by,
                          t.scope,
                          t.team_id,
                          t.owner_user_id,
                          t.created_at,
                          t.updated_at
                   FROM touched_trigger t
                   JOIN trigger_rules r ON r.trigger_id = t.id
                   JOIN locked_source s ON s.trigger_id = t.id"#,
            )
            .bind(&job.id)
            .bind(worker_id)
            .fetch_optional(&self.db)
            .await?;

            if let Some(job_lock) = job_lock {
                locked.push(job_lock);
            }
        }

        Ok(locked)
    }

    /// Record a schedule fire: stamp `last_fired_at`, set the next fire time,
    /// and release the lock.
    pub async fn record_schedule_fire(
        &self,
        trigger_id: &str,
        next_fire_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE schedule_trigger_sources
               SET last_fired_at = now(),
                   next_fire_at = $2,
                   locked_by = NULL,
                   locked_until = NULL
               WHERE trigger_id = $1"#,
        )
        .bind(trigger_id)
        .bind(next_fire_at)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Resolve the `LoopSpawnConfig` (trigger + rule) for a schedule job.
    pub async fn find_loop_spawn_config(&self, trigger_id: &str) -> Result<LoopSpawnConfig, sqlx::Error> {
        sqlx::query_as::<_, LoopSpawnConfig>(
            r#"SELECT t.id,
                      r.repository_id,
                      r.routing_mode,
                      r.ongoing_task_id,
                      r.prompt AS prompt_template,
                      r.task_name,
                      r.tag_ids,
                      r.run_as_user_id,
                      r.agent_type,
                      r.inactive_ttl_days,
                      t.scope,
                      t.team_id,
                      t.owner_user_id,
                      t.created_by,
                      t.created_at,
                      t.updated_at
               FROM triggers t
               JOIN trigger_rules r ON r.trigger_id = t.id
               WHERE t.id = $1"#,
        )
        .bind(trigger_id)
        .fetch_one(&self.db)
        .await
    }

    /// Point a Loop's rule at an ongoing task and bump the trigger's
    /// `updated_at`. The single home for the `ongoing_task_id` update.
    pub async fn set_ongoing_task(
        &self,
        loop_id: &str,
        task_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"WITH updated_rule AS (
                   UPDATE trigger_rules
                   SET ongoing_task_id = $2
                   WHERE trigger_id = $1
                   RETURNING trigger_id
               )
               UPDATE triggers
               SET updated_at = now()
               WHERE id IN (SELECT trigger_id FROM updated_rule)"#,
        )
        .bind(loop_id)
        .bind(task_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }
}

const SUBSCRIPTION_MATCH_SELECT: &str = r#"
    SELECT t.id,
           aes.adapter_kind AS provider,
           aes.connection_id,
           aes.source_matcher AS source_matcher,
           aes.source_name,
           r.routing_mode,
           aes.event_filter,
           r.repository_id,
           r.prompt AS prompt_template,
           r.task_name,
           r.tag_ids,
           r.run_as_user_id,
           r.agent_type,
           r.inactive_ttl_days,
           aes.allow_direct_send,
           aes.adapter_config AS provider_config,
           r.ongoing_task_id,
           aes.close_task_on_close,
           aes.notify_on_success,
           t.active,
           t.created_by,
           t.scope,
           t.team_id,
           t.owner_user_id,
           t.created_at,
           t.updated_at
    FROM triggers t
    JOIN trigger_rules r ON r.trigger_id = t.id
    JOIN adapter_event_trigger_sources aes ON aes.trigger_id = t.id
"#;

// =============================================================================
// Private insert helpers shared by create.
// =============================================================================

async fn insert_config(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trigger_id: &str,
    rule: &NewLoopConfig,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO trigger_rules
           (trigger_id, repository_id, routing_mode, ongoing_task_id, prompt,
            task_name, tag_ids, run_as_user_id, agent_type, inactive_ttl_days)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(trigger_id)
    .bind(&rule.repository_id)
    .bind(rule.routing_mode)
    .bind(&rule.ongoing_task_id)
    .bind(&rule.prompt)
    .bind(&rule.task_name)
    .bind(&rule.tag_ids)
    .bind(&rule.run_as_user_id)
    .bind(&rule.agent_type)
    .bind(rule.inactive_ttl_days)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_adapter_event_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trigger_id: &str,
    source: &NewAdapterEventSource,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO adapter_event_trigger_sources
           (trigger_id, adapter_kind, connection_id, source_matcher, source_name,
            event_filter, adapter_config, allow_direct_send, close_task_on_close,
            notify_on_success)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(trigger_id)
    .bind(&source.adapter_kind)
    .bind(&source.connection_id)
    .bind(&source.source_matcher)
    .bind(&source.source_name)
    .bind(&source.event_filter)
    .bind(&source.adapter_config)
    .bind(source.allow_direct_send)
    .bind(source.close_task_on_close)
    .bind(source.notify_on_success)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_schedule_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    trigger_id: &str,
    source: &NewScheduleSource,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO schedule_trigger_sources
           (trigger_id, cron_expression, timezone, message_template, next_fire_at)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(trigger_id)
    .bind(&source.cron_expression)
    .bind(&source.timezone)
    .bind(&source.message_template)
    .bind(source.next_fire_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
