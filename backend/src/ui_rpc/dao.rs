//! The conversation half of the shell listing's data access.
//!
//! One read, one row type. Everything a sidebar row needs that is not on
//! `tasks` — the newest turn, the Sandbox's lifecycle intent, when a person
//! last wrote, which model the newest turn is running, and whether the agent is
//! waiting on an answer — is derived here so the projection above never issues
//! a query per conversation.
//!
//! No T3 vocabulary appears in this file. The names are Moatless's; the
//! translation belongs one module up.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use tokio::sync::Semaphore;
use tracing::warn;

use crate::db::DbPool;
use crate::shared_models::AgentType;
use crate::task_listing::{self, ListingChange, ListingHead};
use moatless_api_schemas::enums::{SandboxDesiredState, SandboxStopReason};
use moatless_backend_core::scope::{ScopeContext, ScopedEntity};

/// One Task, with the derived values a sidebar row is assembled from.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Conversation {
    pub task_id: String,
    /// The Workspace this Task runs against. `None` for older and hand-written
    /// rows: creation makes a shadow Workspace when none is given, but
    /// provisioning still falls back to repository-only behaviour without one.
    pub workspace_id: Option<String>,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub agent_type: AgentType,
    /// Creation time as the caller recorded it (RFC 3339 text, `NOT NULL`).
    pub created_at: String,
    /// Creation time as the database recorded it. `None` for rows written
    /// before the column existed.
    pub created_at_db: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,

    /// The newest Turn, by turn number. `None` when the Task has never run.
    pub turn_number: Option<i32>,
    pub turn_status: Option<String>,
    pub turn_started_at: Option<DateTime<Utc>>,
    pub turn_ended_at: Option<DateTime<Utc>>,

    /// Durable Sandbox lifecycle intent. All four are `None` together when no
    /// Sandbox was ever requested — the lazy-absence model of `sandboxes`.
    pub sandbox_desired_state: Option<SandboxDesiredState>,
    pub sandbox_stop_reason: Option<SandboxStopReason>,
    pub sandbox_started_at: Option<DateTime<Utc>>,
    pub sandbox_updated_at: Option<DateTime<Utc>>,

    /// When a person last wrote. Counts only `message_type = 'user'` rows,
    /// which is this backend's own predicate for a person's message: the
    /// agent's own `user`-role rows (tool results streamed back by the SDK) are
    /// persisted as `system`. `tasks.last_message_at` is the nearest stored
    /// value and is the wrong one — it also advances when the agent finishes,
    /// stops or errors.
    pub latest_user_message_at: Option<DateTime<Utc>>,

    /// The model the newest Turn's agent replies reported they were produced
    /// by, newest first. `None` until something comes back.
    pub assistant_model: Option<String>,
    /// The model asked for when the newest Turn started, or — when it has not
    /// started — the choice carried by the messages queued for it.
    pub requested_model: Option<String>,

    /// An `AskUserQuestion` raised by the agent that nothing has answered.
    pub pending_question: bool,
    /// An `ExitPlanMode` plan proposed by the agent that nothing has answered.
    pub pending_plan: bool,
}

/// Read the conversations one viewer's sidebar is built from, and the position
/// a client watching it resumes from.
///
/// The whole of this module's internal interface: read the conversations this
/// viewer follows, re-read the ones a change names, and read the position.
///
/// A trait because the module's transport tests stub it; the only production
/// implementation is [`PgConversationDao`].
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ConversationDao: Send + Sync {
    /// Every Task the viewer follows **and** may still read, newest first.
    ///
    /// The two rules are deliberately different and both are load-bearing.
    /// Following is what makes this a sidebar rather than a deployment listing
    /// — an administrator's list is what they follow, like everyone else's.
    /// The read rule underneath is what stops a follow, which is durable, from
    /// outliving the access that produced it.
    async fn list_followed(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// The same listing, narrowed to Tasks a change has named.
    ///
    /// Both filters still apply, and that is the point of re-reading through
    /// this rather than projecting the change record itself: a change is a
    /// deployment-wide fact, and what a viewer may be shown of it is not. A
    /// Task the viewer does not follow, or may no longer read, simply comes
    /// back absent.
    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// The highest listing position this deployment has reached, and the
    /// identity of the database it reached it in.
    async fn listing_head(&self) -> Result<ListingHead, sqlx::Error>;

    /// Every listing change recorded after `after`, oldest first.
    async fn listing_changes_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<ListingChange>, sqlx::Error>;
}

/// A full shell snapshot is a deployment-sized read with no pagination in T3's
/// contract. One may run per backend replica; live change re-reads are narrowed
/// by task id and do not take this permit. This reserves the rest of the main
/// pool for task control, ingress, and reconciliation under shell overload.
#[derive(Clone)]
pub struct PgConversationDao {
    pool: DbPool,
    full_listing: Arc<Semaphore>,
}

const FULL_LISTING_CONCURRENCY: usize = 1;
const FULL_LISTING_STATEMENT_TIMEOUT: &str = "5s";
const LISTING_DURATION: &str = "moatless_ui_shell_listing_duration_seconds";
const LISTING_REJECTIONS: &str = "moatless_ui_shell_listing_rejections_total";

/// Tool calls the agent uses to hand the conversation back to a person. An
/// unanswered one is what "the agent is waiting" means in this backend.
const QUESTION_TOOL: &str = "AskUserQuestion";
const PLAN_TOOL: &str = "ExitPlanMode";

impl PgConversationDao {
    pub fn new(pool: DbPool) -> Self {
        metrics::describe_histogram!(
            LISTING_DURATION,
            "Full T3 shell listing duration by outcome"
        );
        metrics::describe_counter!(
            LISTING_REJECTIONS,
            "Full T3 shell listings rejected before completion by reason"
        );
        Self {
            pool,
            full_listing: Arc::new(Semaphore::new(FULL_LISTING_CONCURRENCY)),
        }
    }

    /// The one listing query, optionally narrowed to a set of Tasks.
    ///
    /// Narrowing is a `WHERE` clause on the same query rather than a second one
    /// because the filters underneath it — the read rule and the follow rule —
    /// are the part that must not drift. A live re-read that admitted a row the
    /// snapshot would not have is the same disclosure bug in slow motion.
    async fn list(
        &self,
        viewer: &ScopeContext,
        task_ids: Option<&[String]>,
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        let full_listing = task_ids.is_none();
        let _permit = if full_listing {
            let Ok(permit) = self.full_listing.try_acquire() else {
                metrics::counter!(LISTING_REJECTIONS, "reason" => "at_capacity").increment(1);
                warn!(
                    user_id = %viewer.user_id,
                    "T3 UI shell listing rejected; another full listing is in progress"
                );
                return Err(sqlx::Error::PoolTimedOut);
            };
            Some(permit)
        } else {
            None
        };
        let started = full_listing.then(Instant::now);

        // `$1`/`$2` are the access filter's admin flag and user id; `$3` is the
        // follower. The read rule is the shared fragment rather than a copy, so
        // a change to what "may read" means reaches this listing too.
        let access = viewer.access_filter(ScopedEntity::Task, "t.", 1);
        let follower = access.next_param();
        let question_tool = follower + 1;
        let plan_tool = follower + 2;
        let narrowed = task_ids
            .map(|_| format!("\n                 AND t.task_id = ANY(${})", plan_tool + 1))
            .unwrap_or_default();
        let sql = format!(
            r#"WITH listed AS MATERIALIZED (
                   SELECT t.*
                   FROM task_follows tf
                   JOIN tasks t ON t.task_id = tf.task_id
                   WHERE tf.user_id = ${follower}
                     AND {}{narrowed}
               )
               SELECT
                   t.task_id,
                   t.workspace_id,
                   t.name,
                   t.branch,
                   t.agent_type,
                   t.created_at,
                   t.created_at_db,
                   t.updated_at,
                   turn.turn_number,
                   turn.status                AS turn_status,
                   turn.started_at            AS turn_started_at,
                   turn.ended_at              AS turn_ended_at,
                   s.desired_state            AS sandbox_desired_state,
                   s.stop_reason              AS sandbox_stop_reason,
                   s.started_at               AS sandbox_started_at,
                   s.desired_state_updated_at AS sandbox_updated_at,
                   person.latest_user_message_at,
                   model.assistant_model,
                   model.requested_model,
                   COALESCE(pending.question, false) AS pending_question,
                   COALESCE(pending.plan, false)     AS pending_plan
               FROM listed t
               LEFT JOIN sandboxes s ON s.task_id = t.task_id
               LEFT JOIN LATERAL (
                   SELECT tt.turn_number, tt.status, tt.started_at, tt.ended_at
                   FROM task_turns tt
                   WHERE tt.task_id = t.task_id
                   ORDER BY tt.turn_number DESC
                   LIMIT 1
               ) turn ON TRUE
               LEFT JOIN LATERAL (
                   SELECT m.created_at AS latest_user_message_at
                   FROM task_messages m
                   WHERE m.task_id = t.task_id
                     AND m.message_type = 'user'
                   ORDER BY m.created_at DESC
                   LIMIT 1
               ) person ON TRUE
               -- UNCERTAIN: `task_messages."timestamp"` is `text`, so every
               -- ORDER BY below is a lexicographic sort standing in for a
               -- chronological one. It holds only while the format is uniform
               -- (RFC 3339, fixed precision, `Z`), which is what this backend
               -- writes today and what its own `(task_id, timestamp, id)` index
               -- assumes. A row written with an offset other than `Z` would
               -- sort wrong. `id` breaks ties, so same-instant rows are stable.
               LEFT JOIN LATERAL (
                   SELECT
                       (SELECT a.assistant_message_model
                          FROM task_messages a
                         WHERE a.task_id = t.task_id
                           -- NULL = NULL is never true, so a conversation with
                           -- no turn reports no model here and falls through to
                           -- the queued-message branch below. Intended.
                           AND a.turn_number = turn.turn_number
                           AND a.assistant_message_model IS NOT NULL
                         ORDER BY a."timestamp" DESC, a.id DESC
                         LIMIT 1) AS assistant_model,
                       COALESCE(
                           (SELECT u.model
                              FROM task_messages u
                             WHERE u.task_id = t.task_id
                               AND u.turn_number = turn.turn_number
                               AND u.model IS NOT NULL
                             ORDER BY u."timestamp" ASC, u.id ASC
                             LIMIT 1),
                           (SELECT u.model
                              FROM task_messages u
                             WHERE u.task_id = t.task_id
                               AND u.turn_number IS NULL
                               AND u.model IS NOT NULL
                             ORDER BY u."timestamp" DESC, u.id DESC
                             LIMIT 1)
                       ) AS requested_model
               ) model ON TRUE
               -- Only the newest assistant message is scanned. An unanswered
               -- question the agent then wrote past stops counting as pending,
               -- which is the narrow reading: the agent has moved on.
               --
               -- "Newest" is the highest `id`, and `id` is a `bigserial`
               -- assigned at INSERT but made visible at COMMIT, so a concurrent
               -- writer can make this flag stale: the read computes it from
               -- message 100 while 101 is still committing. That is acceptable
               -- because it is a point-in-time read that advances nothing —
               -- the next snapshot sees 101 and is correct, so it is
               -- self-healing.
               --
               -- It is **not** the hazard a polling cursor (`WHERE id > N`)
               -- has, which is the same mechanism with a different
               -- consequence: a watermark moved past an uncommitted id skips
               -- that row permanently. Do not apply a cursor mitigation here.
               -- It buys nothing and costs a join.
               --
               -- Ordering by `"timestamp"` instead would only trade this for
               -- the text-sort caveat above.
               LEFT JOIN LATERAL (
                   SELECT
                       bool_or(raised.tool_name = ${question_tool}) AS question,
                       bool_or(raised.tool_name = ${plan_tool})     AS plan
                   FROM (
                       SELECT block->>'name' AS tool_name,
                              block->>'id'   AS tool_use_id
                       FROM task_messages a,
                            LATERAL jsonb_array_elements(a.message_content) block
                       WHERE a.id = (
                               SELECT newest.id
                               FROM task_messages newest
                               WHERE newest.task_id = t.task_id
                                 AND newest.message_type = 'assistant'
                               ORDER BY newest.id DESC
                               LIMIT 1
                             )
                         AND jsonb_typeof(a.message_content) = 'array'
                         AND block->>'type' = 'tool_use'
                         AND block->>'name' IN (${question_tool}, ${plan_tool})
                   ) raised
                   WHERE NOT EXISTS (
                       SELECT 1
                       FROM task_messages answer,
                            LATERAL jsonb_array_elements(answer.message_content) reply
                       WHERE answer.task_id = t.task_id
                         AND jsonb_typeof(answer.message_content) = 'array'
                         AND reply->>'type' = 'tool_result'
                         AND reply->>'tool_use_id' = raised.tool_use_id
                   )
               ) pending ON TRUE
               -- Determinism for this query's own output, and nothing more.
               -- This is *not* the sidebar's order: T3 sorts client-side, in
               -- `resolveSettledTimestamp` (`Sidebar.logic.ts:484-502`), from
               -- the maximum of `latestUserMessageAt` and the three
               -- `latestTurn` stamps, falling back to `updatedAt` only when all
               -- four are absent. Reading this line as the sidebar's order
               -- leads to trying to keep the server's ordering live, which
               -- nothing observes.
               ORDER BY t.updated_at DESC NULLS LAST, t.task_id ASC"#,
            access.sql(),
        );

        let query = sqlx::query_as::<_, Conversation>(AssertSqlSafe(sql))
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .bind(&viewer.user_id)
            .bind(QUESTION_TOOL)
            .bind(PLAN_TOOL);
        let result = match task_ids {
            Some(task_ids) => query.bind(task_ids.to_vec()).fetch_all(&self.pool).await,
            None => {
                async {
                    let mut tx = self.pool.begin().await?;
                    sqlx::query("SELECT set_config('statement_timeout', $1, true)")
                        .bind(FULL_LISTING_STATEMENT_TIMEOUT)
                        .execute(&mut *tx)
                        .await?;
                    let rows = query.fetch_all(&mut *tx).await?;
                    tx.commit().await?;
                    Ok(rows)
                }
                .await
            }
        };
        if let Some(started) = started {
            let outcome = match &result {
                Ok(_) => "ok",
                Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("57014") => {
                    metrics::counter!(LISTING_REJECTIONS, "reason" => "statement_timeout")
                        .increment(1);
                    "statement_timeout"
                }
                Err(_) => "error",
            };
            metrics::histogram!(LISTING_DURATION, "outcome" => outcome)
                .record(started.elapsed().as_secs_f64());
        }
        result
    }
}

#[async_trait]
impl ConversationDao for PgConversationDao {
    async fn list_followed(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.list(viewer, None).await
    }

    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.list(viewer, Some(task_ids)).await
    }

    async fn listing_head(&self) -> Result<ListingHead, sqlx::Error> {
        task_listing::current_head(&self.pool).await
    }

    async fn listing_changes_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<ListingChange>, sqlx::Error> {
        task_listing::changes_after(&self.pool, after, limit).await
    }
}
