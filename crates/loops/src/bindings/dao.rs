//! The `task_bindings` store.
//!
//! Private to this crate. [`crate::bindings::operations::Bindings`] is the one
//! surface on these queries, and the narrow read seams in `bindings::ports`
//! answer a peer that only asks about Bindings.

use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use moatless_core::db::DbPool;

use super::persistence::TaskBindingRow;
use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
use moatless_types::adapter::TaskBinding;

/// Flat row returned by [`PgTaskBindingDao::find_recent_for_loop`]. Binding
/// columns are optional — schedule loops spawn tasks without a binding.
#[derive(sqlx::FromRow, Debug)]
pub(crate) struct LoopRecentTaskRow {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub task_created_at: String,
    // binding fields (None for schedule loops — no return-channel binding)
    pub binding_id: Option<String>,
    pub adapter_kind: Option<String>,
    pub subject_kind: Option<String>,
    pub subject_key: Option<String>,
    pub connection_id: Option<String>,
    pub loop_id: Option<String>,
    pub destination_config: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub last_external_event_id: Option<String>,
    pub origin: Option<String>,
    pub binding_created_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
}

/// Flat row returned by the batch binding query.
#[derive(sqlx::FromRow, Debug)]
pub(crate) struct TaskBindingInfo {
    pub task_id: String,
    pub loop_id: String,
    pub adapter_kind: String,
    pub source_id: String,
    pub source_name: Option<String>,
}

pub(crate) struct PgTaskBindingDao(DbPool);

impl PgTaskBindingDao {
    /// Construct over a pool. The only way to build one — ad-hoc construction
    /// at use sites is what the composition-root invariant bans, so the field
    /// is private and [`crate::compose::loop_graph`] is the only caller.
    pub(crate) fn new(pool: DbPool) -> Self {
        Self(pool)
    }

    /// Batch-fetch the task-list display projection — one row per binding, with
    /// the owning loop's `source_matcher`/`source_name` joined in from
    /// `subscriptions` for labelling.
    pub(crate) async fn get_bindings_for_tasks(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<TaskBindingInfo>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_as::<_, TaskBindingInfo>(
            r#"SELECT tb.task_id,
                  COALESCE(aes.loop_id, '') AS loop_id,
                  tb.adapter_kind AS adapter_kind,
                  COALESCE(aes.source_matcher, tb.subject_key) AS source_id,
                  aes.source_name
           FROM task_bindings tb
           -- subscriptions is owned by loops::dao; this is a
           -- read-only enrichment join (source_matcher/source_name for display).
           LEFT JOIN subscriptions aes ON aes.loop_id = tb.loop_id
           WHERE tb.task_id = ANY($1)"#,
        )
        .bind(task_ids)
        .fetch_all(&self.0)
        .await
    }
}

const TASK_BINDING_SELECT: &str = r#"
    SELECT id,
           task_id,
           adapter_kind AS adapter_kind,
           subject_kind,
           subject_key,
           connection_id,
           loop_id AS loop_id,
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

/// The queries, reached only through [`crate::bindings::operations::Bindings`].
impl PgTaskBindingDao {
    pub(crate) async fn find_by_id(&self, id: &str) -> Result<Option<TaskBinding>, sqlx::Error> {
        let row = sqlx::query_as::<_, TaskBindingRow>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE id = $1"
        )))
        .bind(id)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(TaskBinding::from))
    }

    pub(crate) async fn find_by_subject(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        let rows = sqlx::query_as::<_, TaskBindingRow>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE adapter_kind = $1 AND subject_kind = $2 AND subject_key = $3 ORDER BY created_at ASC"
        )))
        .bind(adapter_kind)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().map(TaskBinding::from).collect())
    }

    pub(crate) async fn find_by_task_id(
        &self,
        task_id: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        let rows = sqlx::query_as::<_, TaskBindingRow>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE task_id = $1 ORDER BY created_at ASC"
        )))
        .bind(task_id)
        .fetch_all(&self.0)
        .await?;
        Ok(rows.into_iter().map(TaskBinding::from).collect())
    }

    pub(crate) async fn find_by_task_id_and_subject(
        &self,
        task_id: &str,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        let row = sqlx::query_as::<_, TaskBindingRow>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE task_id = $1 AND adapter_kind = $2 AND subject_kind = $3 AND subject_key = $4 LIMIT 1"
        )))
        .bind(task_id)
        .bind(adapter_kind)
        .bind(subject_kind)
        .bind(subject_key)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(TaskBinding::from))
    }

    pub(crate) async fn find_provider_event_created_for_subject(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        let row = sqlx::query_as::<_, TaskBindingRow>(AssertSqlSafe(format!(
            "{TASK_BINDING_SELECT} WHERE adapter_kind = $1 AND subject_kind = $2 AND subject_key = $3 AND origin = $4 ORDER BY created_at ASC LIMIT 1"
        )))
        .bind(adapter_kind)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(ADAPTER_EVENT_BINDING_ORIGIN)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(TaskBinding::from))
    }

    /// Whitelist of allowed `destination_config` keys for
    /// [`Self::find_active_by_destination_field`]. The field name is
    /// interpolated into the SQL, so untrusted values would be a SQL
    /// injection vector.
    const ALLOWED_DESTINATION_FIELDS: &'static [&'static str] = &["issue_id"];

    /// Look up active bindings whose `destination_config->>field_name` equals
    /// `field_value`. Routes an inbound event that carries a destination-scoped
    /// identifier (e.g. Linear `issue_id`) rather than the binding's
    /// `subject_key`.
    pub(crate) async fn find_active_by_destination_field(
        &self,
        adapter_kind: &str,
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
        let rows = sqlx::query_as::<_, TaskBindingRow>(AssertSqlSafe(sql))
            .bind(adapter_kind)
            .bind(subject_kind)
            .bind(field_value)
            .fetch_all(&self.0)
            .await?;
        Ok(rows.into_iter().map(TaskBinding::from).collect())
    }

    pub(crate) async fn upsert(&self, row: &TaskBinding) -> Result<TaskBinding, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let row = sqlx::query_as::<_, TaskBindingRow>(
            r#"INSERT INTO task_bindings
               (id, task_id, adapter_kind, subject_kind, subject_key, connection_id,
                loop_id, destination_config, metadata,
                last_external_event_id, origin, created_at, last_event_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               ON CONFLICT (task_id, adapter_kind, subject_kind, subject_key) DO UPDATE
               SET connection_id            = EXCLUDED.connection_id,
                   loop_id               = COALESCE(task_bindings.loop_id, EXCLUDED.loop_id),
                   destination_config      = EXCLUDED.destination_config,
                   metadata                 = task_bindings.metadata || EXCLUDED.metadata,
                   last_external_event_id   = COALESCE(EXCLUDED.last_external_event_id, task_bindings.last_external_event_id),
                   last_event_at            = GREATEST(task_bindings.last_event_at, EXCLUDED.last_event_at)
               RETURNING id,
                         task_id,
                         adapter_kind AS adapter_kind,
                         subject_kind,
                         subject_key,
                         connection_id,
                         loop_id AS loop_id,
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
        .bind(row.adapter_kind.as_str())
        .bind(&row.subject_kind)
        .bind(&row.subject_key)
        .bind(&row.connection_id)
        .bind(&row.loop_id)
        .bind(&row.destination_config)
        .bind(&row.metadata)
        .bind(&row.last_external_event_id)
        .bind(&row.origin)
        .bind(row.created_at)
        .bind(row.last_event_at)
        .fetch_one(&mut *tx)
        .await?;
        // Last in the transaction: `record_change` takes the listing counter,
        // and a writer that takes it before its own row is the deadlock the
        // ordering prevents. See `moatless_core::change_feed::listing`.
        moatless_core::change_feed::listing::record_change(&mut tx, &row.task_id).await?;
        tx.commit().await?;
        Ok(TaskBinding::from(row))
    }

    /// Set whether the Binding joining one Task to one subject routes, and
    /// report whether the row changed.
    ///
    /// The `active <> $5` predicate is what makes this idempotent, so a second
    /// unlink of the same pull request answers `false` rather than re-stamping
    /// `deactivated_at`.
    pub(crate) async fn set_active_for_subject(
        &self,
        task_id: &str,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
        active: bool,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let changed = sqlx::query(
            "UPDATE task_bindings \
             SET active = $5, deactivated_at = CASE WHEN $5 THEN NULL ELSE now() END \
             WHERE task_id = $1 AND adapter_kind = $2 AND subject_kind = $3 \
               AND subject_key = $4 AND active <> $5",
        )
        .bind(task_id)
        .bind(adapter_kind)
        .bind(subject_kind)
        .bind(subject_key)
        .bind(active)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if changed {
            // Ordered as in [`Self::upsert`].
            moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// Deactivate every active binding for a task — called when the task closes
    /// (a closed task is terminal, so its external listeners must stop routing
    /// inbound events). Rows are kept — see [`TaskBinding::active`].
    /// Idempotent: the `active = TRUE` predicate makes re-runs no-ops. Returns
    /// the number of bindings deactivated.
    pub(crate) async fn deactivate_by_task(&self, task_id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE task_bindings SET active = FALSE, deactivated_at = now() \
             WHERE task_id = $1 AND active = TRUE",
        )
        .bind(task_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected())
    }

    pub(crate) async fn touch_last_event(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE task_bindings SET last_event_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    /// Record that this binding's thread has carried a task-provenance footer,
    /// so later replies in the same thread do not repeat it. Merges into
    /// `metadata` rather than taking a column: the footer is presentation
    /// state, not part of the binding's identity.
    pub(crate) async fn mark_task_link_sent(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE task_bindings
               SET metadata = COALESCE(metadata, '{}'::jsonb) || '{"task_link_sent": true}'::jsonb
               WHERE id = $1"#,
        )
        .bind(id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    /// Fetch the most recent tasks created via the given Loop. Drives from
    /// `tasks.loop_id` so all Loop kinds — including schedule loops
    /// that never create a binding — are reachable. A LEFT JOIN LATERAL pulls
    /// the earliest matching binding for adapter-event loops; schedule
    /// loops yield NULL binding columns. Tasks scoped via `loop_id`
    /// inherit the Loop's scope by construction, so no additional
    /// task-level scope predicate is needed.
    pub(crate) async fn find_recent_for_loop(
        &self,
        loop_id: &str,
        limit: i64,
    ) -> Result<Vec<LoopRecentTaskRow>, sqlx::Error> {
        sqlx::query_as::<_, LoopRecentTaskRow>(
            r#"SELECT t.task_id, t.short_id, t.name AS title, t.status,
                      t.created_at AS task_created_at,
                      tb.id AS binding_id, tb.adapter_kind AS adapter_kind, tb.subject_kind, tb.subject_key,
                      tb.connection_id, tb.loop_id, tb.destination_config, tb.metadata,
                      tb.last_external_event_id, tb.origin,
                      tb.created_at AS binding_created_at, tb.last_event_at
               FROM tasks t
               LEFT JOIN LATERAL (
                   SELECT id, adapter_kind, subject_kind, subject_key, connection_id,
                          loop_id, destination_config, metadata,
                          last_external_event_id, origin, created_at, last_event_at
                   FROM task_bindings
                   WHERE task_id = t.task_id AND loop_id = $1
                   ORDER BY created_at ASC
                   LIMIT 1
               ) tb ON true
               WHERE t.loop_id = $1
               ORDER BY t.created_at DESC
               LIMIT $2"#,
        )
        .bind(loop_id)
        .bind(limit)
        .fetch_all(&self.0)
        .await
    }

    /// Stamp the latest external id we've seen on this binding so context
    /// replay can skip the agent's own posts. Also bumps `last_event_at`.
    pub(crate) async fn touch_cursor(
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

    /// Persist the external status of a binding's connected subject (GitHub PR
    /// / Linear issue). Writes the dedicated `external_status*` columns and
    /// stamps `external_status_updated_at = now()`. Leaves
    /// `destination_config` untouched (which `upsert` would clobber).
    pub(crate) async fn update_external_status(
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

    /// Persist the PR's latest review decision (approved / `changes_requested` /
    /// …) onto a binding. Leaves the `external_status*` columns untouched.
    pub(crate) async fn update_review_decision(
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
    pub(crate) async fn update_mergeable_state(
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
}
