//! `LoopDao` — the single home for every read and write of the four tables
//! that persist a Loop: `loops`, `loop_configs`,
//! `subscriptions`, and `schedules`.
//!
//! A Loop is the standing definition of recurring agent work; see `CONTEXT.md`
//! for terminology. This module owns loop persistence behind one plain struct
//! holding a `DbPool`; do not add a single-impl trait.
//!
//! Keep loop SQL here so reads, writes, and transactions share one home.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use moatless_backend_core::scope::{Scope, ScopeContext, ScopedEntity};

use crate::adapters::models::{LoopSpawnConfig, RoutingMode, Schedule, SubscriptionMatch};
use moatless_core::db::DbPool;

// =============================================================================
// Joined loop record — the read shape used by the HTTP surface.
// =============================================================================

#[derive(Debug, Clone)]
pub struct LoopRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub config_source: String,
    pub source_repository_id: Option<String>,
    pub source_key: Option<String>,
    pub source_config_path: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone)]
pub struct LoopConfigRow {
    pub loop_id: String,
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
    pub loop_id: String,
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
    pub loop_id: String,
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
    pub loop_: LoopRow,
    pub config: LoopConfigRow,
    pub source: LoopSourceRow,
}

#[derive(sqlx::FromRow)]
struct JoinedLoopRecordRow {
    id: String,
    name: String,
    kind: String,
    active: bool,
    scope: Scope,
    owner_user_id: Option<String>,
    created_by: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    config_source: String,
    source_repository_id: Option<String>,
    source_key: Option<String>,
    source_config_path: Option<String>,
    deleted: bool,
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
        let loop_ = LoopRow {
            id: self.id.clone(),
            name: self.name,
            kind: self.kind.clone(),
            active: self.active,
            scope: self.scope,
            owner_user_id: self.owner_user_id,
            created_by: self.created_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
            config_source: self.config_source,
            source_repository_id: self.source_repository_id,
            source_key: self.source_key,
            source_config_path: self.source_config_path,
            deleted: self.deleted,
        };
        let config = LoopConfigRow {
            loop_id: self.id.clone(),
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
                loop_id: self.id,
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
                loop_id: self.id,
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
            loop_,
            config,
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
           t.owner_user_id,
           t.created_by,
           t.created_at,
           t.updated_at,
           t.config_source,
           t.source_repository_id,
           t.source_key,
           t.source_config_path,
           t.deleted,
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
    FROM loops t
    JOIN loop_configs r ON r.loop_id = t.id
    LEFT JOIN subscriptions aes ON aes.loop_id = t.id
    LEFT JOIN schedules sts ON sts.loop_id = t.id
"#;

// =============================================================================
// Write inputs — resolved values ready to persist. Callers own validation,
// scope resolution, and coalescing; the store owns only the SQL.
// =============================================================================

/// A fully-resolved new Loop, ready to insert.
pub struct NewLoop {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub config: NewLoopConfig,
    pub source: NewLoopSource,
}

/// Resolved `loop_configs` values shared by create and update.
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

/// Resolved `loops` columns to update (identity + scope, not config/source).
pub struct LoopUpdate {
    pub name: String,
    pub scope: Scope,
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

/// A Git-owned Loop reconciled from a repository's `.moatless/loops/` tree,
/// keyed by `(source_repository_id, source_key)`.
pub struct ManagedLoopUpsert {
    pub source_repository_id: String,
    pub source_key: String,
    pub source_config_path: String,
    pub trigger_name: String,
    pub kind: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: String,
    pub routing_mode: RoutingMode,
    pub prompt: Option<String>,
    pub tag_ids: Vec<String>,
    pub agent_type: String,
    pub inactive_ttl_days: Option<i32>,
    pub adapter: Option<NewAdapterEventSource>,
    pub schedule: Option<NewScheduleSource>,
}

#[derive(Debug, Default)]
pub struct ManagedLoopReconcileSummary {
    pub created: u32,
    pub updated: u32,
    pub removed: u32,
    pub approvals_revoked: u32,
    pub overrides_skipped: u32,
    pub skipped_overrides: Vec<ManagedLoopOverrideSkip>,
}

#[derive(Debug)]
pub struct ManagedLoopOverrideSkip {
    pub source_key: String,
    pub deleted: bool,
}

// =============================================================================
// LoopDao
// =============================================================================

/// The single owner of persistence for the four Loop tables.
pub struct LoopDao {
    db: DbPool,
}

impl LoopDao {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    // ---------------------------------------------------------------------
    // HTTP surface: get / list / create / update / delete
    // ---------------------------------------------------------------------

    pub async fn get(
        &self,
        id: &str,
        ctx: &ScopeContext,
    ) -> Result<Option<LoopRecord>, sqlx::Error> {
        let access = ctx.access_filter(ScopedEntity::Resource, "t.", 2);
        let sql = format!(
            "{LOOP_RECORD_SELECT} WHERE t.id = $1 AND {} AND (t.deleted = false OR $2)",
            access.sql()
        );
        let row = sqlx::query_as::<_, JoinedLoopRecordRow>(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
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
        let mut sql = format!(
            "{LOOP_RECORD_SELECT} WHERE {} AND (t.deleted = false OR $1)",
            access.sql()
        );
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

    /// Insert a Loop (row + config + optional source) in one transaction.
    pub async fn create(&self, new: NewLoop) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;

        sqlx::query(
            r#"INSERT INTO loops
               (id, name, kind, active, scope, owner_user_id,
                created_by, config_source, deleted, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'manual', false, $8, $8)"#,
        )
        .bind(&new.id)
        .bind(&new.name)
        .bind(&new.kind)
        .bind(new.active)
        .bind(new.scope)
        .bind(&new.owner_user_id)
        .bind(&new.created_by)
        .bind(new.created_at)
        .execute(&mut *tx)
        .await?;

        insert_config(&mut tx, &new.id, &new.config).await?;

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

    /// Update a Loop's row, and optionally its config and/or source.
    /// Returns `false` when the id is not found or the caller lacks control.
    pub async fn update(
        &self,
        id: &str,
        loop_update: LoopUpdate,
        config: Option<NewLoopConfig>,
        source: Option<UpdateLoopSource>,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.db.begin().await?;
        lock_git_origin(&mut tx, id).await?;
        let ctrl = ctx.control_filter("t.", 5);
        let sql = format!(
            r#"UPDATE loops t
               SET name = $2,
                   scope = $3,
                   owner_user_id = $4,
                   updated_at = now()
               WHERE t.id = $1
                 AND t.config_source = 'manual'
                 AND t.deleted = false
                 AND {}"#,
            ctrl.sql()
        );
        let result = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .bind(&loop_update.name)
            .bind(loop_update.scope)
            .bind(&loop_update.owner_user_id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }

        if let Some(config) = config {
            sqlx::query(
                r#"UPDATE loop_configs
                   SET repository_id = $2,
                       routing_mode = $3,
                       ongoing_task_id = $4,
                       prompt = $5,
                       task_name = $6,
                       tag_ids = $7,
                       run_as_user_id = $8,
                       agent_type = $9,
                       inactive_ttl_days = $10
                   WHERE loop_id = $1"#,
            )
            .bind(id)
            .bind(&config.repository_id)
            .bind(config.routing_mode)
            .bind(&config.ongoing_task_id)
            .bind(&config.prompt)
            .bind(&config.task_name)
            .bind(&config.tag_ids)
            .bind(&config.run_as_user_id)
            .bind(&config.agent_type)
            .bind(config.inactive_ttl_days)
            .execute(&mut *tx)
            .await?;
        }

        match source {
            Some(UpdateLoopSource::AdapterEvent(source)) => {
                sqlx::query(
                    r#"UPDATE subscriptions
                       SET connection_id = $2,
                           source_matcher = $3,
                           source_name = $4,
                           event_filter = $5,
                           adapter_config = $6,
                           allow_direct_send = $7,
                           close_task_on_close = $8,
                           notify_on_success = $9
                       WHERE loop_id = $1"#,
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
                    r#"UPDATE schedules
                       SET cron_expression = $2,
                           timezone = $3,
                           message_template = $4,
                           next_fire_at = $5
                       WHERE loop_id = $1"#,
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

        tx.commit().await?;
        Ok(true)
    }

    /// Delete a Loop by id. Git-origin Loops become local tombstones so
    /// repository sync cannot recreate them; plain manual Loops are removed.
    /// Returns `false` when the id is not found or the caller lacks control.
    pub async fn delete(&self, id: &str, ctx: &ScopeContext) -> Result<u64, sqlx::Error> {
        let mut tx = self.db.begin().await?;
        lock_git_origin(&mut tx, id).await?;
        let ctrl = ctx.control_filter("t.", 2);
        let sql = format!(
            "SELECT t.source_repository_id FROM loops t WHERE t.id = $1 AND {} FOR UPDATE",
            ctrl.sql()
        );
        let provenance: Option<Option<String>> = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(source_repository_id) = provenance else {
            return Ok(0);
        };

        if source_repository_id.is_some() {
            sqlx::query(
                "UPDATE loops
                 SET active = false, config_source = 'manual', deleted = true, updated_at = now()
                 WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE loop_configs SET run_as_user_id = NULL WHERE loop_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("DELETE FROM loops WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(1)
    }

    pub async fn activate(
        &self,
        id: &str,
        run_as_user_id: &str,
        ctx: &ScopeContext,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.db.begin().await?;
        lock_git_origin(&mut tx, id).await?;
        let ctrl = ctx.control_filter("t.", 2);
        let sql = format!(
            "SELECT t.id
             FROM loops t
             JOIN loop_configs r ON r.loop_id = t.id
             WHERE t.id = $1
               AND t.deleted = false
               AND {}
             FOR UPDATE OF t, r",
            ctrl.sql()
        );
        let loop_id: Option<String> = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if loop_id.is_none() {
            return Ok(false);
        }
        sqlx::query("UPDATE loop_configs SET run_as_user_id = $2 WHERE loop_id = $1")
            .bind(id)
            .bind(run_as_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE loops SET active = true, updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn pause(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let mut tx = self.db.begin().await?;
        lock_git_origin(&mut tx, id).await?;
        let ctrl = ctx.control_filter("t.", 2);
        let sql = format!(
            "SELECT t.id FROM loops t
             WHERE t.id = $1 AND t.deleted = false AND {}
             FOR UPDATE",
            ctrl.sql()
        );
        let loop_id: Option<String> = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if loop_id.is_none() {
            return Ok(false);
        }
        sqlx::query("UPDATE loops SET active = false, updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn resume(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let mut tx = self.db.begin().await?;
        lock_git_origin(&mut tx, id).await?;
        let ctrl = ctx.control_filter("t.", 2);
        let sql = format!(
            "SELECT t.id
             FROM loops t
             JOIN loop_configs r ON r.loop_id = t.id
             WHERE t.id = $1
               AND r.run_as_user_id IS NOT NULL
               AND t.deleted = false
               AND {}
             FOR UPDATE OF t, r",
            ctrl.sql()
        );
        let loop_id: Option<String> = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if loop_id.is_none() {
            return Ok(false);
        }
        sqlx::query("UPDATE loops SET active = true, updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn override_git(&self, id: &str, ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let mut tx = self.db.begin().await?;
        lock_git_origin(&mut tx, id).await?;
        let ctrl = ctx.control_filter("t.", 2);
        let sql = format!(
            "SELECT t.id FROM loops t
             WHERE t.id = $1
               AND t.config_source = 'git'
               AND t.deleted = false
               AND {}
             FOR UPDATE",
            ctrl.sql()
        );
        let loop_id: Option<String> = sqlx::query_scalar(AssertSqlSafe(sql))
            .bind(id)
            .bind(ctx.is_admin)
            .bind(&ctx.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if loop_id.is_none() {
            return Ok(false);
        }
        sqlx::query(
            "UPDATE loops
             SET config_source = 'manual', updated_at = now()
             WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    // ---------------------------------------------------------------------
    // Adapter-event subscription lookup surface.
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

    pub async fn find_runnable_subscription_by_id(
        &self,
        id: &str,
    ) -> Result<Option<SubscriptionMatch>, sqlx::Error> {
        sqlx::query_as::<_, SubscriptionMatch>(AssertSqlSafe(format!(
            "{SUBSCRIPTION_MATCH_SELECT}
             WHERE t.id = $1
               AND t.active = true
               AND t.deleted = false
               AND r.run_as_user_id IS NOT NULL"
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
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = $2 AND t.active = true AND t.deleted = false AND r.run_as_user_id IS NOT NULL ORDER BY t.created_at ASC LIMIT 1"
        )))
        .bind(provider)
        .bind(source_matcher)
        .fetch_optional(&self.db)
        .await
    }

    /// Return all subscriptions for a given (adapter, source_matcher) — used by
    /// Linear, where multiple listeners can share a team UUID.
    pub async fn list_subscriptions_by_source_matcher(
        &self,
        provider: &str,
        source_matcher: &str,
    ) -> Result<Vec<SubscriptionMatch>, sqlx::Error> {
        sqlx::query_as::<_, SubscriptionMatch>(AssertSqlSafe(format!(
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = $2 AND t.active = true AND t.deleted = false AND r.run_as_user_id IS NOT NULL ORDER BY t.created_at ASC"
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
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND aes.source_matcher = '*' AND t.active = true AND t.deleted = false AND r.run_as_user_id IS NOT NULL LIMIT 1"
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
            "{SUBSCRIPTION_MATCH_SELECT} WHERE aes.adapter_kind = $1 AND t.active = true AND t.deleted = false AND r.run_as_user_id IS NOT NULL ORDER BY t.created_at ASC"
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
            r#"INSERT INTO loops
               (id, name, kind, active, scope, owner_user_id,
                created_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (id) DO UPDATE
               SET name            = EXCLUDED.name,
                   kind            = EXCLUDED.kind,
                   active          = EXCLUDED.active,
                   scope           = EXCLUDED.scope,
                   owner_user_id   = EXCLUDED.owner_user_id,
                   updated_at      = EXCLUDED.updated_at"#,
        )
        .bind(&row.id)
        // `loops.name` is NOT NULL; an unnamed subscription falls back to
        // the matcher it listens on.
        .bind(row.source_name.as_deref().unwrap_or(&row.source_matcher))
        .bind("adapter_event")
        .bind(row.active)
        .bind(row.scope)
        .bind(&row.owner_user_id)
        .bind(&row.created_by)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO loop_configs
               (loop_id, repository_id, routing_mode, ongoing_task_id, prompt,
                task_name, tag_ids, run_as_user_id, agent_type, inactive_ttl_days)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (loop_id) DO UPDATE
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
            r#"INSERT INTO subscriptions
               (loop_id, adapter_kind, connection_id, source_matcher, source_name,
                event_filter, adapter_config,
                allow_direct_send, close_task_on_close, notify_on_success)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (loop_id) DO UPDATE
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
        .bind(&row.adapter_kind)
        .bind(&row.connection_id)
        .bind(&row.source_matcher)
        .bind(&row.source_name)
        .bind(&row.event_filter)
        .bind(&row.adapter_config)
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

    /// Read the `adapter_config` JSON for an adapter-event subscription.
    /// Returns `Ok(None)` when the Loop has no adapter-event source row (the
    /// outer `Option` is the `adapter_config` column, which is itself nullable).
    pub async fn get_adapter_config(
        &self,
        loop_id: &str,
    ) -> Result<Option<serde_json::Value>, sqlx::Error> {
        let row: Option<(Option<serde_json::Value>,)> =
            sqlx::query_as("SELECT adapter_config FROM subscriptions WHERE loop_id = $1")
                .bind(loop_id)
                .fetch_optional(&self.db)
                .await?;
        Ok(row.and_then(|(config,)| config))
    }

    /// Overwrite the `adapter_config` JSON for an adapter-event subscription
    /// and bump the Loop's `updated_at`, in one transaction.
    pub async fn set_adapter_config(
        &self,
        loop_id: &str,
        config: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;
        sqlx::query("UPDATE subscriptions SET adapter_config = $1 WHERE loop_id = $2")
            .bind(config)
            .bind(loop_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE loops SET updated_at = now() WHERE id = $1")
            .bind(loop_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    // ---------------------------------------------------------------------
    // Repository Loop reconciliation.
    // ---------------------------------------------------------------------

    /// Reconcile one complete repository snapshot in one transaction.
    ///
    /// `present_keys` includes malformed and unresolved Loop directories. That
    /// distinction prevents a bad `LOOP.md` from being interpreted as removal.
    /// Manual overrides and deletion tombstones retain their Git provenance and
    /// are skipped until an administrator explicitly resets them.
    pub async fn reconcile_managed_loops(
        &self,
        source_repository_id: &str,
        inputs: &[ManagedLoopUpsert],
        present_keys: &HashSet<String>,
    ) -> Result<ManagedLoopReconcileSummary, sqlx::Error> {
        let now = Utc::now();
        let mut tx = self.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(source_repository_id)
            .execute(&mut *tx)
            .await?;
        let mut summary = ManagedLoopReconcileSummary::default();

        for input in inputs {
            let existing: Option<(String, String, bool, bool)> = sqlx::query_as(
                "SELECT t.id, t.config_source, t.deleted,
                        (t.active OR r.run_as_user_id IS NOT NULL) AS approved
                 FROM loops t
                 JOIN loop_configs r ON r.loop_id = t.id
                 WHERE t.source_repository_id = $1 AND t.source_key = $2
                 FOR UPDATE",
            )
            .bind(source_repository_id)
            .bind(&input.source_key)
            .fetch_optional(&mut *tx)
            .await?;

            let loop_id = match existing {
                Some((_, config_source, deleted, _)) if config_source == "manual" || deleted => {
                    summary.overrides_skipped += 1;
                    summary.skipped_overrides.push(ManagedLoopOverrideSkip {
                        source_key: input.source_key.clone(),
                        deleted,
                    });
                    continue;
                }
                Some((id, _, _, approved)) => {
                    sqlx::query(
                        "UPDATE loops
                         SET name = $2, kind = $3, active = false, scope = $4,
                             owner_user_id = $5, source_config_path = $6,
                             deleted = false, updated_at = $7
                         WHERE id = $1",
                    )
                    .bind(&id)
                    .bind(&input.trigger_name)
                    .bind(&input.kind)
                    .bind(input.scope)
                    .bind(&input.owner_user_id)
                    .bind(&input.source_config_path)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                    summary.updated += 1;
                    if approved {
                        summary.approvals_revoked += 1;
                    }
                    id
                }
                None => {
                    let id = format!("tr_{}", uuid::Uuid::new_v4());
                    sqlx::query(
                        "INSERT INTO loops
                         (id, name, kind, active, scope, owner_user_id, created_by,
                          config_source, source_repository_id, source_key,
                          source_config_path, deleted, created_at, updated_at)
                         VALUES ($1, $2, $3, false, $4, $5, $6, 'git', $7, $8, $9,
                                 false, $10, $10)",
                    )
                    .bind(&id)
                    .bind(&input.trigger_name)
                    .bind(&input.kind)
                    .bind(input.scope)
                    .bind(&input.owner_user_id)
                    .bind(&input.created_by)
                    .bind(source_repository_id)
                    .bind(&input.source_key)
                    .bind(&input.source_config_path)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                    summary.created += 1;
                    id
                }
            };

            replace_managed_loop_rows(&mut tx, &loop_id, input).await?;
        }

        let existing: Vec<(String, String, String, bool)> = sqlx::query_as(
            "SELECT id, source_key, config_source, deleted
             FROM loops
             WHERE source_repository_id = $1 AND source_key IS NOT NULL
             FOR UPDATE",
        )
        .bind(source_repository_id)
        .fetch_all(&mut *tx)
        .await?;
        for (id, source_key, config_source, deleted) in existing {
            if present_keys.contains(&source_key) {
                continue;
            }
            if config_source == "git" {
                sqlx::query("DELETE FROM loops WHERE id = $1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                summary.removed += 1;
            } else {
                summary.overrides_skipped += 1;
                summary.skipped_overrides.push(ManagedLoopOverrideSkip {
                    source_key,
                    deleted,
                });
            }
        }

        tx.commit().await?;
        Ok(summary)
    }

    /// Force one override or tombstone back to its Git declaration. The caller
    /// must have fetched and resolved that exact source first.
    pub async fn reset_managed_loop(&self, input: &ManagedLoopUpsert) -> Result<bool, sqlx::Error> {
        let mut tx = self.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(&input.source_repository_id)
            .execute(&mut *tx)
            .await?;
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM loops
             WHERE source_repository_id = $1 AND source_key = $2
             FOR UPDATE",
        )
        .bind(&input.source_repository_id)
        .bind(&input.source_key)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(id) = id else {
            return Ok(false);
        };
        sqlx::query(
            "UPDATE loops
             SET name = $2, kind = $3, active = false, scope = $4,
                 owner_user_id = $5, config_source = 'git',
                 source_config_path = $6, deleted = false, updated_at = now()
             WHERE id = $1",
        )
        .bind(&id)
        .bind(&input.trigger_name)
        .bind(&input.kind)
        .bind(input.scope)
        .bind(&input.owner_user_id)
        .bind(&input.source_config_path)
        .execute(&mut *tx)
        .await?;
        replace_managed_loop_rows(&mut tx, &id, input).await?;
        tx.commit().await?;
        Ok(true)
    }

    // ---------------------------------------------------------------------
    // Schedule runner: claim due schedules, record a fire, resolve the route.
    // ---------------------------------------------------------------------

    /// Select due, unlocked schedules and lock each one for `worker_id`.
    /// Returns the schedules that were successfully locked. System-level query —
    /// no scope filter applied (the schedule runner acts across all owners).
    pub async fn claim_due_schedules(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<Vec<Schedule>, sqlx::Error> {
        let due_jobs = sqlx::query_as::<_, Schedule>(
            r#"SELECT t.id,
                      t.id AS loop_id,
                      sts.cron_expression,
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
                      t.owner_user_id,
                      t.created_at,
                      t.updated_at
               FROM loops t
               JOIN loop_configs r ON r.loop_id = t.id
               JOIN schedules sts ON sts.loop_id = t.id
               WHERE t.kind = 'schedule'
                 AND t.active = true
                 AND t.deleted = false
                 AND r.run_as_user_id IS NOT NULL
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
                       UPDATE schedules
                       SET locked_by = $2,
                           locked_until = now() + interval '2 minutes'
                       WHERE loop_id = $1
                         AND next_fire_at <= now()
                         AND (locked_until IS NULL OR locked_until < now())
                       RETURNING *
                   ),
                   touched_trigger AS (
                       UPDATE loops
                       SET updated_at = now()
                       WHERE id = $1 AND active = true AND deleted = false AND kind = 'schedule'
                       RETURNING *
                   )
                   SELECT t.id,
                          t.id AS loop_id,
                          s.cron_expression,
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
                          t.owner_user_id,
                          t.created_at,
                          t.updated_at
                   FROM touched_trigger t
                   JOIN loop_configs r ON r.loop_id = t.id
                   JOIN locked_source s ON s.loop_id = t.id
                   WHERE r.run_as_user_id IS NOT NULL"#,
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
        loop_id: &str,
        next_fire_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE schedules
               SET last_fired_at = now(),
                   next_fire_at = $2,
                   locked_by = NULL,
                   locked_until = NULL
               WHERE loop_id = $1"#,
        )
        .bind(loop_id)
        .bind(next_fire_at)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Resolve the `LoopSpawnConfig` for a schedule firing.
    pub async fn find_loop_spawn_config(
        &self,
        loop_id: &str,
    ) -> Result<LoopSpawnConfig, sqlx::Error> {
        // System-level query — schedule runner acts across all owners, no scope filter needed.
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
                      t.owner_user_id,
                      t.created_by,
                      t.created_at,
                      t.updated_at
               FROM loops t
               JOIN loop_configs r ON r.loop_id = t.id
               WHERE t.id = $1"#,
        )
        .bind(loop_id)
        .fetch_one(&self.db)
        .await
    }

    /// Point a Loop's config at an ongoing task and bump the Loop's
    /// `updated_at`. The single home for the `ongoing_task_id` update.
    pub async fn set_ongoing_task(&self, loop_id: &str, task_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"WITH updated_rule AS (
                   UPDATE loop_configs
                   SET ongoing_task_id = $2
                   WHERE loop_id = $1
                   RETURNING loop_id
               )
               UPDATE loops
               SET updated_at = now()
               WHERE id IN (SELECT loop_id FROM updated_rule)"#,
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
           aes.adapter_kind,
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
           aes.adapter_config AS adapter_config,
           r.ongoing_task_id,
           aes.close_task_on_close,
           aes.notify_on_success,
           t.active,
           t.created_by,
           t.scope,
           t.owner_user_id,
           t.created_at,
           t.updated_at
    FROM loops t
    JOIN loop_configs r ON r.loop_id = t.id
    JOIN subscriptions aes ON aes.loop_id = t.id
"#;

// =============================================================================
// Private insert helpers shared by create.
// =============================================================================

/// Serialize every mutation of a Git-origin Loop with repository reconciliation.
///
/// Provenance survives override and deletion, so the same lock also orders
/// local edits, tombstones, resets, and lifecycle changes against Git sync.
async fn lock_git_origin(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    loop_id: &str,
) -> Result<(), sqlx::Error> {
    let repository_id: Option<String> =
        sqlx::query_scalar("SELECT source_repository_id FROM loops WHERE id = $1")
            .bind(loop_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();
    if let Some(repository_id) = repository_id {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(repository_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn replace_managed_loop_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    loop_id: &str,
    input: &ManagedLoopUpsert,
) -> Result<(), sqlx::Error> {
    let ongoing_task_id: Option<String> =
        sqlx::query_scalar("SELECT ongoing_task_id FROM loop_configs WHERE loop_id = $1")
            .bind(loop_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();

    sqlx::query("DELETE FROM loop_configs WHERE loop_id = $1")
        .bind(loop_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM subscriptions WHERE loop_id = $1")
        .bind(loop_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM schedules WHERE loop_id = $1")
        .bind(loop_id)
        .execute(&mut **tx)
        .await?;

    insert_config(
        tx,
        loop_id,
        &NewLoopConfig {
            repository_id: input.source_repository_id.clone(),
            routing_mode: input.routing_mode,
            ongoing_task_id,
            prompt: input.prompt.clone(),
            task_name: None,
            tag_ids: input.tag_ids.clone(),
            run_as_user_id: None,
            agent_type: input.agent_type.clone(),
            inactive_ttl_days: input.inactive_ttl_days,
        },
    )
    .await?;

    if let Some(adapter) = &input.adapter {
        insert_adapter_event_source(tx, loop_id, adapter).await?;
    }
    if let Some(schedule) = &input.schedule {
        insert_schedule_source(tx, loop_id, schedule).await?;
    }
    Ok(())
}

async fn insert_config(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    loop_id: &str,
    config: &NewLoopConfig,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO loop_configs
           (loop_id, repository_id, routing_mode, ongoing_task_id, prompt,
            task_name, tag_ids, run_as_user_id, agent_type, inactive_ttl_days)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(loop_id)
    .bind(&config.repository_id)
    .bind(config.routing_mode)
    .bind(&config.ongoing_task_id)
    .bind(&config.prompt)
    .bind(&config.task_name)
    .bind(&config.tag_ids)
    .bind(&config.run_as_user_id)
    .bind(&config.agent_type)
    .bind(config.inactive_ttl_days)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_adapter_event_source(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    loop_id: &str,
    source: &NewAdapterEventSource,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO subscriptions
           (loop_id, adapter_kind, connection_id, source_matcher, source_name,
            event_filter, adapter_config, allow_direct_send, close_task_on_close,
            notify_on_success)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(loop_id)
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
    loop_id: &str,
    source: &NewScheduleSource,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO schedules
           (loop_id, cron_expression, timezone, message_template, next_fire_at)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(loop_id)
    .bind(&source.cron_expression)
    .bind(&source.timezone)
    .bind(&source.message_template)
    .bind(source.next_fire_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
