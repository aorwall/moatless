//! Persistence helpers for `task_bindings`.

use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use crate::db::DbPool;

use crate::adapters::models::{TaskBinding, ADAPTER_EVENT_BINDING_ORIGIN};

#[async_trait::async_trait]
pub trait TaskBindingRepository: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskBinding>, sqlx::Error>;

    async fn find_by_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn find_all_by_subject_excluding_task(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
        exclude_task_id: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn find_by_task_id(&self, task_id: &str) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn find_by_task_id_and_subject(
        &self,
        task_id: &str,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error>;

    async fn find_provider_event_created_for_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error>;

    /// Look up active bindings whose `destination_config->>field_name` equals
    /// `field_value`. Used to route inbound events that carry a destination-
    /// scoped identifier (e.g. Linear `issue_id`) rather than the binding's
    /// `subject_key`. `field_name` is whitelisted to prevent SQL injection
    /// through the JSONB path expression.
    async fn find_active_by_destination_field(
        &self,
        provider: &str,
        subject_kind: &str,
        field_name: &str,
        field_value: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn upsert(&self, row: &TaskBinding) -> Result<TaskBinding, sqlx::Error>;

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error>;

    async fn touch_last_event(&self, id: &str) -> Result<(), sqlx::Error>;

    async fn touch_cursor(&self, id: &str, last_external_event_id: &str)
        -> Result<(), sqlx::Error>;

    /// Persist the external status of a binding's connected subject (GitHub PR
    /// / Linear issue). Writes the dedicated `external_status*` columns and
    /// stamps `external_status_updated_at = now()`. Leaves
    /// `destination_config` untouched (which `upsert` would clobber).
    async fn update_external_status(
        &self,
        id: &str,
        status: Option<&str>,
        state_kind: Option<&str>,
        raw: &serde_json::Value,
    ) -> Result<(), sqlx::Error>;

    /// Batch-fetch full binding rows (including `destination_config` and the
    /// `external_status*` columns) for a set of tasks. Drives the
    /// task-connections admin view.
    async fn find_by_task_ids(&self, task_ids: &[String]) -> Result<Vec<TaskBinding>, sqlx::Error>;
}

pub struct PgTaskBindingRepository(pub DbPool);

/// Flat row returned by `find_recent_for_loop`. Binding columns are
/// optional — schedule triggers spawn tasks without a binding.
#[derive(sqlx::FromRow, Debug)]
pub struct LoopRecentTaskRow {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub task_created_at: String,
    // binding fields (None for schedule triggers — no return-channel binding)
    pub binding_id: Option<String>,
    pub provider: Option<String>,
    pub subject_kind: Option<String>,
    pub subject_key: Option<String>,
    pub connection_id: Option<String>,
    pub trigger_id: Option<String>,
    pub destination_config: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub last_external_event_id: Option<String>,
    pub origin: Option<String>,
    pub binding_created_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
}

/// Flat row returned by the batch binding query.
#[derive(sqlx::FromRow, Debug)]
pub struct TaskBindingInfo {
    pub task_id: String,
    pub trigger_id: String,
    pub provider: String,
    pub source_id: String,
    pub source_name: Option<String>,
}

#[async_trait::async_trait]
impl TaskBindingRepository for PgTaskBindingRepository {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_by_id(self, id).await
    }

    async fn find_by_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_by_subject(self, provider, subject_kind, subject_key).await
    }

    async fn find_all_by_subject_excluding_task(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
        exclude_task_id: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_all_by_subject_excluding_task(
            self,
            provider,
            subject_kind,
            subject_key,
            exclude_task_id,
        )
        .await
    }

    async fn find_by_task_id(&self, task_id: &str) -> Result<Vec<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_by_task_id(self, task_id).await
    }

    async fn find_by_task_id_and_subject(
        &self,
        task_id: &str,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_by_task_id_and_subject(
            self,
            task_id,
            provider,
            subject_kind,
            subject_key,
        )
        .await
    }

    async fn find_provider_event_created_for_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_provider_event_created_for_subject(
            self,
            provider,
            subject_kind,
            subject_key,
        )
        .await
    }

    async fn find_active_by_destination_field(
        &self,
        provider: &str,
        subject_kind: &str,
        field_name: &str,
        field_value: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_active_by_destination_field(
            self,
            provider,
            subject_kind,
            field_name,
            field_value,
        )
        .await
    }

    async fn upsert(&self, row: &TaskBinding) -> Result<TaskBinding, sqlx::Error> {
        PgTaskBindingRepository::upsert(self, row).await
    }

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        PgTaskBindingRepository::delete(self, id).await
    }

    async fn touch_last_event(&self, id: &str) -> Result<(), sqlx::Error> {
        PgTaskBindingRepository::touch_last_event(self, id).await
    }

    async fn touch_cursor(
        &self,
        id: &str,
        last_external_event_id: &str,
    ) -> Result<(), sqlx::Error> {
        PgTaskBindingRepository::touch_cursor(self, id, last_external_event_id).await
    }

    async fn update_external_status(
        &self,
        id: &str,
        status: Option<&str>,
        state_kind: Option<&str>,
        raw: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        PgTaskBindingRepository::update_external_status(self, id, status, state_kind, raw).await
    }

    async fn find_by_task_ids(&self, task_ids: &[String]) -> Result<Vec<TaskBinding>, sqlx::Error> {
        PgTaskBindingRepository::find_by_task_ids(self, task_ids).await
    }
}

/// Batch-fetch task binding source info for multiple tasks in a single query.
pub async fn get_bindings_for_tasks(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<Vec<TaskBindingInfo>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(vec![]);
    }
    sqlx::query_as::<_, TaskBindingInfo>(
        r#"SELECT tb.task_id,
                  COALESCE(aes.trigger_id, '') AS trigger_id,
                  tb.adapter_kind AS provider,
                  COALESCE(aes.source_matcher, tb.subject_key) AS source_id,
                  aes.source_name
           FROM task_bindings tb
           LEFT JOIN adapter_event_trigger_sources aes ON aes.trigger_id = tb.trigger_id
           WHERE tb.task_id = ANY($1)"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await
}

const TASK_BINDING_SELECT: &str = r#"
    SELECT id,
           task_id,
           adapter_kind AS provider,
           subject_kind,
           subject_key,
           connection_id,
           trigger_id AS trigger_id,
           destination_config AS destination_config,
           metadata,
           last_external_event_id AS last_external_event_id,
           origin AS origin,
           created_at,
           last_event_at,
           external_review_decision,
           external_mergeable_state,
           active,
           deactivated_at
    FROM task_bindings
"#;

impl PgTaskBindingRepository {
    pub async fn find_by_id(&self, id: &str) -> Result<Option<TaskBinding>, sqlx::Error> {
        sqlx::query_as::<_, TaskBinding>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_by_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        sqlx::query_as::<_, TaskBinding>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE adapter_kind = $1 AND subject_kind = $2 AND subject_key = $3 ORDER BY created_at ASC"
        )))
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_all(&self.0)
        .await
    }

    pub async fn find_all_by_subject_excluding_task(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
        exclude_task_id: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        sqlx::query_as::<_, TaskBinding>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE adapter_kind = $1 AND subject_kind = $2 AND subject_key = $3 AND task_id <> $4 ORDER BY created_at ASC"
        )))
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(exclude_task_id)
        .fetch_all(&self.0)
        .await
    }

    pub async fn find_by_task_id(&self, task_id: &str) -> Result<Vec<TaskBinding>, sqlx::Error> {
        sqlx::query_as::<_, TaskBinding>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE task_id = $1 ORDER BY created_at ASC"
        )))
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    pub async fn find_by_task_id_and_subject(
        &self,
        task_id: &str,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        sqlx::query_as::<_, TaskBinding>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE task_id = $1 AND adapter_kind = $2 AND subject_kind = $3 AND subject_key = $4 LIMIT 1"
        )))
        .bind(task_id)
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_optional(&self.0)
        .await
    }

    pub async fn find_provider_event_created_for_subject(
        &self,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        sqlx::query_as::<_, TaskBinding>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE adapter_kind = $1 AND subject_kind = $2 AND subject_key = $3 AND origin = $4 ORDER BY created_at ASC LIMIT 1"
        )))
        .bind(provider)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(ADAPTER_EVENT_BINDING_ORIGIN)
        .fetch_optional(&self.0)
        .await
    }

    /// Whitelist of allowed `destination_config` keys for
    /// [`Self::find_active_by_destination_field`]. The field name is
    /// interpolated into the SQL, so untrusted values would be a SQL
    /// injection vector.
    const ALLOWED_DESTINATION_FIELDS: &'static [&'static str] = &["issue_id"];

    pub async fn find_active_by_destination_field(
        &self,
        provider: &str,
        subject_kind: &str,
        field_name: &str,
        field_value: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        if !Self::ALLOWED_DESTINATION_FIELDS.contains(&field_name) {
            return Err(sqlx::Error::Protocol(format!(
                "find_active_by_destination_field: field_name '{field_name}' not allowed"
            )));
        }
        let sql = format!(
            "{TASK_BINDING_SELECT} WHERE adapter_kind = $1 \
               AND subject_kind = $2 \
               AND destination_config->>'{field_name}' = $3 \
               AND COALESCE(destination_config->>'pending', 'false') <> 'true' \
             ORDER BY created_at ASC"
        );
        sqlx::query_as::<_, TaskBinding>(AssertSqlSafe(sql))
            .bind(provider)
            .bind(subject_kind)
            .bind(field_value)
            .fetch_all(&self.0)
            .await
    }

    pub async fn upsert(&self, row: &TaskBinding) -> Result<TaskBinding, sqlx::Error> {
        sqlx::query_as::<_, TaskBinding>(
            r#"INSERT INTO task_bindings
               (id, task_id, adapter_kind, subject_kind, subject_key, connection_id,
                trigger_id, destination_config, metadata,
                last_external_event_id, origin, created_at, last_event_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               ON CONFLICT (task_id, adapter_kind, subject_kind, subject_key) DO UPDATE
               SET connection_id            = EXCLUDED.connection_id,
                   trigger_id               = COALESCE(task_bindings.trigger_id, EXCLUDED.trigger_id),
                   destination_config      = EXCLUDED.destination_config,
                   metadata                 = task_bindings.metadata || EXCLUDED.metadata,
                   last_external_event_id   = COALESCE(EXCLUDED.last_external_event_id, task_bindings.last_external_event_id),
                   last_event_at            = GREATEST(task_bindings.last_event_at, EXCLUDED.last_event_at)
               RETURNING id,
                         task_id,
                         adapter_kind AS provider,
                         subject_kind,
                         subject_key,
                         connection_id,
                         trigger_id AS trigger_id,
                         destination_config AS destination_config,
                         metadata,
                         last_external_event_id AS last_external_event_id,
                         origin AS origin,
                         created_at,
                         last_event_at,
                         active,
                         deactivated_at"#,
        )
        .bind(&row.id)
        .bind(&row.task_id)
        .bind(&row.provider)
        .bind(&row.subject_kind)
        .bind(&row.subject_key)
        .bind(&row.connection_id)
        .bind(&row.trigger_id)
        .bind(&row.destination_config)
        .bind(&row.metadata)
        .bind(&row.last_external_event_id)
        .bind(&row.origin)
        .bind(row.created_at)
        .bind(row.last_event_at)
        .fetch_one(&self.0)
        .await
    }

    pub async fn delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM task_bindings WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }

    /// Deactivate every active binding for a task — called when the task closes
    /// (a closed task is terminal, so its external listeners must stop routing
    /// inbound events). Rows are kept for the admin Task Connections view.
    /// Idempotent: the `active = TRUE` predicate makes re-runs no-ops. Returns
    /// the number of bindings deactivated.
    pub async fn deactivate_by_task(&self, task_id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE task_bindings SET active = FALSE, deactivated_at = now() \
             WHERE task_id = $1 AND active = TRUE",
        )
        .bind(task_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn touch_last_event(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE task_bindings SET last_event_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    /// Fetch the most recent tasks created via the given trigger. Drives from
    /// `tasks.trigger_id` so all trigger kinds — including schedule triggers
    /// that never create a binding — are reachable. A LEFT JOIN LATERAL pulls
    /// the earliest matching binding for adapter-event triggers; schedule
    /// triggers yield NULL binding columns. Tasks scoped via `trigger_id`
    /// inherit the trigger's scope by construction, so no additional
    /// task-level scope predicate is needed.
    pub async fn find_recent_for_loop(
        &self,
        trigger_id: &str,
        limit: i64,
    ) -> Result<Vec<LoopRecentTaskRow>, sqlx::Error> {
        sqlx::query_as::<_, LoopRecentTaskRow>(
            r#"SELECT t.task_id, t.short_id, t.name AS title, t.status,
                      t.created_at AS task_created_at,
                      tb.id AS binding_id, tb.adapter_kind AS provider, tb.subject_kind, tb.subject_key,
                      tb.connection_id, tb.trigger_id, tb.destination_config, tb.metadata,
                      tb.last_external_event_id, tb.origin,
                      tb.created_at AS binding_created_at, tb.last_event_at
               FROM tasks t
               LEFT JOIN LATERAL (
                   SELECT id, adapter_kind, subject_kind, subject_key, connection_id,
                          trigger_id, destination_config, metadata,
                          last_external_event_id, origin, created_at, last_event_at
                   FROM task_bindings
                   WHERE task_id = t.task_id AND trigger_id = $1
                   ORDER BY created_at ASC
                   LIMIT 1
               ) tb ON true
               WHERE t.trigger_id = $1
               ORDER BY t.created_at DESC
               LIMIT $2"#,
        )
        .bind(trigger_id)
        .bind(limit)
        .fetch_all(&self.0)
        .await
    }

    /// Stamp the latest external id we've seen on this binding so context
    /// replay can skip the agent's own posts. Also bumps `last_event_at`.
    pub async fn touch_cursor(
        &self,
        id: &str,
        last_external_event_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE task_bindings
               SET last_external_event_id = $2,
                   last_event_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(last_external_event_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    pub async fn update_external_status(
        &self,
        id: &str,
        status: Option<&str>,
        state_kind: Option<&str>,
        raw: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE task_bindings
               SET external_status            = $2,
                   external_state_kind        = $3,
                   external_status_raw        = $4,
                   external_status_updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(status)
        .bind(state_kind)
        .bind(raw)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// Persist the PR's latest review decision (approved / changes_requested /
    /// …) onto a binding. Leaves the `external_status*` columns untouched.
    pub async fn update_review_decision(
        &self,
        id: &str,
        review_decision: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE task_bindings
               SET external_review_decision = $2
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(review_decision)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// Persist the PR's mergeable state (clean / dirty / …) onto a binding.
    /// Leaves the `external_status*` columns untouched.
    pub async fn update_mergeable_state(
        &self,
        id: &str,
        mergeable_state: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE task_bindings
               SET external_mergeable_state = $2
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(mergeable_state)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    pub async fn find_by_task_ids(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as::<_, TaskBinding>(
            r#"SELECT id,
                      task_id,
                      adapter_kind AS provider,
                      subject_kind,
                      subject_key,
                      connection_id,
                      trigger_id,
                      destination_config,
                      metadata,
                      last_external_event_id,
                      origin,
                      created_at,
                      last_event_at,
                      external_status,
                      external_state_kind,
                      external_status_raw,
                      external_status_updated_at,
                      external_review_decision,
                      external_mergeable_state,
                      active,
                      deactivated_at
               FROM task_bindings
               WHERE task_id = ANY($1)
               ORDER BY created_at ASC"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await
    }
}
