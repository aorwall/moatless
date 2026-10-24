//! Feedback DAO — queries for feedback persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;

use super::models::{Feedback, FeedbackFilter, FeedbackWithTask, LatestActivity};
use moatless_core::db::DbPool;
use moatless_types::access_control::ScopedEntity;

/// DAO trait for feedback persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait FeedbackDao: Send + Sync {
    /// Insert a new feedback row.
    async fn insert(&self, row: &Feedback) -> Result<(), sqlx::Error>;
    /// Find a feedback row by its public `feedback_id`.
    async fn find_by_id(&self, feedback_id: &str) -> Result<Option<Feedback>, sqlx::Error>;
    /// List all feedback rows for a task, ordered oldest → newest.
    async fn list_for_task(&self, task_id: &str) -> Result<Vec<Feedback>, sqlx::Error>;
    /// Set a row's triage status and return it, or `None` when no row carries
    /// `feedback_id`.
    ///
    /// `note` replaces the previous status's note rather than inheriting it —
    /// an unexplained change clears the old reason. Caller checks task access
    /// first: `feedback` carries no scope columns of its own.
    async fn update_status(
        &self,
        feedback_id: &str,
        status: &str,
        note: Option<String>,
        updated_by: &str,
    ) -> Result<Option<Feedback>, sqlx::Error>;
    /// List feedback across tasks the caller may read, newest first, with the
    /// total matching the filter before paging.
    async fn list(
        &self,
        filter: &FeedbackFilter,
    ) -> Result<(Vec<FeedbackWithTask>, i64), sqlx::Error>;
    /// Best-effort scope for feedback submitted with no `message_uuid`: the
    /// most recent tool call on the task, or, when it made none, its most
    /// recent message. `None` when the task has no messages yet.
    async fn latest_activity(&self, task_id: &str) -> Result<Option<LatestActivity>, sqlx::Error>;
}

/// PostgreSQL implementation of [`FeedbackDao`].
pub struct PgFeedbackDao(pub DbPool);

#[async_trait]
impl FeedbackDao for PgFeedbackDao {
    async fn insert(&self, row: &Feedback) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO feedback (
                feedback_id, task_id, message_uuid, tool_use_id,
                source, surface, category, comment, metadata, created_by, created_at,
                status, status_note, status_updated_by, status_updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"#,
        )
        .bind(&row.feedback_id)
        .bind(&row.task_id)
        .bind(&row.message_uuid)
        .bind(&row.tool_use_id)
        .bind(&row.source)
        .bind(&row.surface)
        .bind(&row.category)
        .bind(&row.comment)
        .bind(&row.metadata)
        .bind(&row.created_by)
        .bind(row.created_at)
        .bind(&row.status)
        .bind(&row.status_note)
        .bind(&row.status_updated_by)
        .bind(row.status_updated_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find_by_id(&self, feedback_id: &str) -> Result<Option<Feedback>, sqlx::Error> {
        sqlx::query_as::<_, Feedback>(
            r#"SELECT
                feedback_id, task_id, message_uuid, tool_use_id,
                source, surface, category, comment, metadata, created_by, created_at,
                status, status_note, status_updated_by, status_updated_at
               FROM feedback
               WHERE feedback_id = $1"#,
        )
        .bind(feedback_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn list_for_task(&self, task_id: &str) -> Result<Vec<Feedback>, sqlx::Error> {
        sqlx::query_as::<_, Feedback>(
            r#"SELECT
                feedback_id, task_id, message_uuid, tool_use_id,
                source, surface, category, comment, metadata, created_by, created_at,
                status, status_note, status_updated_by, status_updated_at
               FROM feedback
               WHERE task_id = $1
               ORDER BY created_at ASC, id ASC"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    async fn update_status(
        &self,
        feedback_id: &str,
        status: &str,
        note: Option<String>,
        updated_by: &str,
    ) -> Result<Option<Feedback>, sqlx::Error> {
        sqlx::query_as::<_, Feedback>(
            r#"UPDATE feedback
                  SET status = $2,
                      status_note = $3,
                      status_updated_by = $4,
                      status_updated_at = now()
                WHERE feedback_id = $1
            RETURNING
                feedback_id, task_id, message_uuid, tool_use_id,
                source, surface, category, comment, metadata, created_by, created_at,
                status, status_note, status_updated_by, status_updated_at"#,
        )
        .bind(feedback_id)
        .bind(status)
        .bind(&note)
        .bind(updated_by)
        .fetch_optional(&self.0)
        .await
    }

    async fn latest_activity(&self, task_id: &str) -> Result<Option<LatestActivity>, sqlx::Error> {
        // A tool call's `conversation_activities` row writes when its
        // `tool_use` streams in, before the call runs — so when an agent
        // invokes `moat feedback` via Bash, that call is already the latest
        // row. Intended: feedback lands on the turn where it was decided,
        // not the preceding call.
        if let Some(row) = sqlx::query_as::<_, LatestActivity>(
            r#"SELECT message_uuid::text AS message_uuid, tool_use_id
               FROM conversation_activities
               WHERE task_id = $1 AND kind = 'tool_call' AND message_uuid IS NOT NULL
               ORDER BY position DESC, ordinal DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await?
        {
            return Ok(Some(row));
        }

        sqlx::query_as::<_, LatestActivity>(
            r#"SELECT uuid::text AS message_uuid, NULL::text AS tool_use_id
               FROM conversation_messages
               WHERE task_id = $1
               ORDER BY position DESC
               LIMIT 1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn list(
        &self,
        filter: &FeedbackFilter,
    ) -> Result<(Vec<FeedbackWithTask>, i64), sqlx::Error> {
        // Page columns are optional because the query always emits the total,
        // even when the offset lies past the end — that case comes back as a
        // single row whose page columns are all NULL. See `into_item`.
        #[derive(sqlx::FromRow)]
        struct FeedbackPageRow {
            total_count: i64,
            feedback_id: Option<String>,
            task_id: Option<String>,
            message_uuid: Option<String>,
            tool_use_id: Option<String>,
            source: Option<String>,
            surface: Option<String>,
            category: Option<String>,
            comment: Option<String>,
            metadata: Option<serde_json::Value>,
            created_by: Option<String>,
            created_at: Option<DateTime<Utc>>,
            status: Option<String>,
            status_note: Option<String>,
            status_updated_by: Option<String>,
            status_updated_at: Option<DateTime<Utc>>,
            task_name: Option<String>,
            task_description: Option<String>,
            repository_id: Option<String>,
            workspace_id: Option<String>,
        }

        impl FeedbackPageRow {
            fn is_total_only(&self) -> bool {
                self.feedback_id.is_none()
            }

            /// `None` only when a `NOT NULL` column came back NULL on a row
            /// that isn't the total-only one.
            ///
            /// Callers must reject that rather than skip it: dropping a row
            /// from the page while still counting it in the total is the drift
            /// this query exists to prevent.
            fn into_item(self) -> Option<FeedbackWithTask> {
                let task_title = self
                    .task_name
                    .filter(|n| !n.is_empty())
                    .or_else(|| {
                        self.task_description
                            .as_deref()
                            .filter(|d| !d.is_empty())
                            .and_then(|d| d.lines().next())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "Task".to_string());
                Some(FeedbackWithTask {
                    feedback: Feedback {
                        feedback_id: self.feedback_id?,
                        task_id: self.task_id?,
                        message_uuid: self.message_uuid,
                        tool_use_id: self.tool_use_id,
                        source: self.source?,
                        surface: self.surface?,
                        category: self.category,
                        comment: self.comment?,
                        metadata: self.metadata,
                        created_by: self.created_by?,
                        created_at: self.created_at?,
                        status: self.status?,
                        status_note: self.status_note,
                        status_updated_by: self.status_updated_by,
                        status_updated_at: self.status_updated_at,
                    },
                    task_title,
                    repository_id: self.repository_id,
                    // Left for the handler to fill in: `repositories` is owned
                    // by `crates/workspaces`, so this DAO cannot join it.
                    repository_name: None,
                    workspace_id: self.workspace_id,
                })
            }
        }

        // Access control reads the joined task: `feedback` has no scope
        // columns of its own, so a row is readable exactly when its task is.
        //
        // `filtered` is the single source for both page and total, so they
        // can't drift under a filter change; `total LEFT JOIN page ON TRUE`
        // keeps the total available when the page is empty.
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 1);
        let sql = format!(
            r#"WITH filtered AS (
                SELECT
                    f.id AS seq, f.feedback_id, f.task_id, f.message_uuid, f.tool_use_id,
                    f.source, f.surface, f.category, f.comment, f.metadata,
                    f.created_by, f.created_at,
                    f.status, f.status_note, f.status_updated_by, f.status_updated_at,
                    t.name AS task_name, t.description AS task_description,
                    t.repository_id, t.workspace_id
                FROM feedback f
                JOIN tasks t ON t.task_id = f.task_id
                WHERE {}
                  AND ($3::text IS NULL OR t.workspace_id = $3)
                  AND ($4::text IS NULL OR t.repository_id = $4)
                  AND ($5::text IS NULL OR f.task_id = $5)
                  AND ($6::text IS NULL OR f.source = $6)
                  AND ($7::text IS NULL OR f.surface = $7)
                  AND ($8::text IS NULL OR f.category = $8)
                  AND ($9::text IS NULL OR f.created_by = $9)
                  AND ($10::timestamptz IS NULL OR f.created_at >= $10)
                  AND ($11::text IS NULL OR f.status = $11)
            ),
            total AS (SELECT COUNT(*) AS total_count FROM filtered),
            page AS (
                SELECT * FROM filtered
                ORDER BY created_at DESC, seq DESC
                LIMIT $12 OFFSET $13
            )
            SELECT tot.total_count, p.*
            FROM total tot
            LEFT JOIN page p ON TRUE
            ORDER BY p.created_at DESC, p.seq DESC"#,
            access.sql(),
        );

        let rows = sqlx::query_as::<_, FeedbackPageRow>(AssertSqlSafe(sql))
            .bind(filter.scope.is_admin)
            .bind(&filter.scope.user_id)
            .bind(&filter.workspace_id)
            .bind(&filter.repository_id)
            .bind(&filter.task_id)
            .bind(&filter.source)
            .bind(&filter.surface)
            .bind(&filter.category)
            .bind(&filter.created_by)
            .bind(filter.since)
            .bind(&filter.status)
            .bind(filter.limit)
            .bind(filter.offset)
            .fetch_all(&self.0)
            .await?;

        let total = rows.first().map_or(0, |r| r.total_count);
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            if row.is_total_only() {
                continue;
            }
            let item = row.into_item().ok_or_else(|| {
                sqlx::Error::Decode("feedback row has a NULL in a NOT NULL column".into())
            })?;
            items.push(item);
        }
        Ok((items, total))
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
