//! The conversation half of the shell listing's data access.
//!
//! One read, one row type. Everything a sidebar row needs that is not on
//! `tasks` — the newest turn, the Sandbox's lifecycle intent, when a person
//! last wrote, which model the newest turn is running, and whether the agent is
//! waiting on an answer — is derived here so the projection above never issues
//! a query per conversation.
//!
//! Opening a conversation reads its transcript through the same module: what
//! was said, what happened on the way, and — once it is open — the changes
//! recorded against it and the rows those changes name.
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
use crate::message::models::{MessageRole, MessageType};
use crate::shared_models::AgentType;
use crate::task_activities::dao::{self as activities, Activity};
use crate::task_activities::{self, ConversationChange, ConversationHead};
use crate::task_listing::{self, ListingChange, ListingHead};
use moatless_api_schemas::enums::{SandboxDesiredState, SandboxStopReason, TaskStatus};
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

/// One Message of a Task's transcript, with the columns a transcript is
/// rendered from and nothing else.
///
/// Deliberately not [`crate::message::models::TaskMessage`]: that row carries
/// forty columns because it is the write model, and a transcript read that
/// selected all of them would pull every tool result's raw payload and every
/// run's metrics out of the database to project six fields.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TranscriptMessage {
    /// The insertion order this transcript is assembled in.
    pub id: i32,
    pub uuid: String,
    pub message_type: MessageType,
    pub message_role: MessageRole,
    /// What was said, and only that. A user Message holds the person's own
    /// prose: the files, contexts and attribution the agent is sent are rebuilt
    /// at delivery time and were never part of this column.
    pub message_text: Option<String>,
    /// Which kind of `user` row this is. The agent's tool responses are
    /// persisted as `user` too, and are not something a person said.
    pub user_type: String,
    /// The Turn this Message was acknowledged into. `None` while it is queued.
    pub turn_number: Option<i32>,
    /// When the writer said it happened (RFC 3339 text, `NOT NULL`).
    pub timestamp: String,
    /// When the database recorded it. `None` is impossible under the current
    /// schema's default and is treated as a missing value rather than assumed.
    pub created_at: Option<DateTime<Utc>>,
}

/// A Task's transcript, and whether it is all of it.
pub struct Transcript {
    /// Oldest first — the order a transcript is read in.
    pub messages: Vec<TranscriptMessage>,
    /// What happened on the way, oldest first. Not a subset of `messages`: a
    /// turn whose agent only called tools said nothing and has no Message row
    /// at all, and its calls reach a reader only through here.
    pub activities: Vec<Activity>,
    /// The rows past [`TRANSCRIPT_LIMIT`] that were left out, oldest first.
    /// Zero for every transcript that fits, which is nearly all of them.
    pub omitted: i64,
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
    /// Every **open** Task the viewer follows **and** may still read, newest
    /// first.
    ///
    /// The three rules are deliberately different and all are load-bearing.
    /// Following is what makes this a sidebar rather than a deployment listing
    /// — an administrator's list is what they follow, like everyone else's.
    /// The read rule underneath is what stops a follow, which is durable, from
    /// outliving the access that produced it. The status rule is what stops a
    /// follow from outliving the *work*: closing a Task is Moatless's own way of
    /// saying it is done, following survives it, and a sidebar that kept the row
    /// would grow without bound and never shrink.
    ///
    /// `error` is left out with `closed`, and that is the narrow reading of
    /// "open": it is the status the Moatless sidebar itself filters on by
    /// default (`sidebar-task-filter-values.ts`), and a Task returns to `open`
    /// the moment its agent produces anything
    /// (`TaskDao::update_status_if_not_closed`), so nothing that is running is
    /// hidden by it.
    async fn list_followed(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// The same listing, narrowed to Tasks a change has named.
    ///
    /// All three filters still apply, and that is the point of re-reading
    /// through this rather than projecting the change record itself: a change is
    /// a deployment-wide fact, and what a viewer may be shown of it is not. A
    /// Task the viewer does not follow, may no longer read, or that has been
    /// closed, simply comes back absent — and absent is what the feed above
    /// turns into the removal that takes the row off the screen.
    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// One Task's transcript, oldest first, capped at [`TRANSCRIPT_LIMIT`].
    ///
    /// Unscoped, and deliberately: every caller reaches this having already
    /// resolved the same Task through [`ConversationDao::list_followed_by_ids`],
    /// which is where the follow rule and the read rule live. Repeating them
    /// here would be a second copy of the access filter to keep in step with
    /// the first, which is the drift the narrowed listing query exists to
    /// avoid.
    async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error>;

    /// The highest position this conversation's change log has reached.
    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error>;

    /// This conversation's changes at or after `from`, oldest first, capped at
    /// `limit`. At `from` rather than after it — see
    /// [`task_activities::changes_from`].
    async fn conversation_changes(
        &self,
        task_id: &str,
        from: i64,
        limit: i64,
    ) -> Result<Vec<ConversationChange>, sqlx::Error>;

    /// The rows a batch of changes names — the Messages, and the activities
    /// under those uuids — oldest first.
    ///
    /// A [`Transcript`] rather than a type of its own, because it is one: the
    /// projection that turns a transcript into messages and activities is the
    /// same projection that turns this into the events for them, and two shapes
    /// would be two of it. Nothing is elided from a read this narrow, so
    /// [`Transcript::omitted`] is always zero.
    ///
    /// Unscoped for the reason [`ConversationDao::transcript`] is: the caller
    /// has already resolved this Task through the narrowed listing.
    async fn changed_messages(
        &self,
        task_id: &str,
        uuids: &[String],
    ) -> Result<Transcript, sqlx::Error>;

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

/// How many of a Task's Messages one transcript read returns.
///
/// T3's thread contract carries the whole transcript in one snapshot and offers
/// no pagination, so this cap is the only thing between an unusually long
/// conversation and a multi-megabyte frame. The **newest** rows are the ones
/// kept: an open conversation is read from its end, and a reader who has lost
/// the opening prompt is better served than one who has lost what just
/// happened. What was dropped is reported rather than silently absent — see
/// [`Transcript::omitted`] and its projection.
///
/// A thousand rows is roughly a megabyte once projected, which is the size a
/// subscription re-sends whenever the conversation moves. That is the number
/// this cap is chosen against, not the database's comfort.
const TRANSCRIPT_LIMIT: i64 = 1_000;
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
        //
        // The status rule is the third filter, and it is the enum's own spelling
        // rather than a literal typed twice: `tasks.status` is `text` with no
        // CHECK constraint, so a misspelling here would be a listing that is
        // always empty and a database that never complains.
        let open = TaskStatus::Open.as_str();
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
                     AND t.status = '{open}'
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
               -- Unanswered is read off the call's own row: an activity leaves
               -- `in_progress` when its result lands, so this needs no search
               -- for the answer.
               --
               -- Only the newest turn that did anything is scanned. An
               -- unanswered question the agent then called past stops counting
               -- as pending, which is the narrow reading: the agent has moved
               -- on. "Newest" is the highest `position` — the conversation's
               -- own ordering, allocated at write time, so unlike the `id` this
               -- used to read it is not a serial made visible at COMMIT.
               --
               -- What this no longer clears on is prose: a question followed by
               -- a reply that called nothing keeps reading as pending until the
               -- agent's next call. A Message carries no position, so the two
               -- cannot be ordered against each other in one index scan, and
               -- the case is the agent answering its own question in words —
               -- which leaves a person with something to click either way.
               LEFT JOIN LATERAL (
                   SELECT
                       bool_or(raised.summary = ${question_tool}) AS question,
                       bool_or(raised.summary = ${plan_tool})     AS plan
                   FROM task_activities raised
                   WHERE raised.task_id = t.task_id
                     AND raised.parent_uuid IS NULL
                     AND raised.kind = 'tool_call'
                     AND raised.status = 'in_progress'
                     AND raised.summary IN (${question_tool}, ${plan_tool})
                     AND raised.message_uuid = (
                           SELECT newest.message_uuid
                           FROM task_activities newest
                           WHERE newest.task_id = t.task_id
                             AND newest.parent_uuid IS NULL
                             AND newest.kind = 'tool_call'
                           ORDER BY newest.position DESC
                           LIMIT 1
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

    async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error> {
        // Newest first so the cap keeps the end of the conversation, then
        // reversed below into the order it is read in. The ordering matches the
        // shell listing's — and carries its caveat: `"timestamp"` is `text`, so
        // this is a lexicographic sort standing in for a chronological one,
        // sound only while every writer emits RFC 3339 at fixed precision in
        // `Z`. `id` breaks ties. It is also exactly `task_messages_task_idx`,
        // read backwards, so the cap is a bounded index scan rather than a sort
        // of the whole conversation.
        //
        // Subagent rows are excluded here rather than after the cap: they are
        // the bulk of a long transcript, they are never rendered inline
        // (`message/transform.rs` assembles them on demand behind their
        // spawning tool call), and counting them against the cap would evict
        // the conversation a person is reading.
        let mut messages = sqlx::query_as::<_, TranscriptMessage>(
            r#"SELECT id, uuid, message_type, message_role, message_text,
                      user_type, turn_number, "timestamp", created_at
               FROM task_messages
               WHERE task_id = $1
                 AND parent_uuid IS NULL
               ORDER BY "timestamp" DESC, id DESC
               LIMIT $2"#,
        )
        .bind(task_id)
        .bind(TRANSCRIPT_LIMIT)
        .fetch_all(&self.pool)
        .await?;
        messages.reverse();

        // Only when the cap was actually reached. Nearly every conversation is
        // shorter than it, and paying for a count on all of them to discover a
        // zero would be the wrong trade.
        let omitted = if messages.len() as i64 == TRANSCRIPT_LIMIT {
            let total: i64 = sqlx::query_scalar(
                r#"SELECT count(*)
                   FROM task_messages
                   WHERE task_id = $1
                     AND parent_uuid IS NULL"#,
            )
            .bind(task_id)
            .fetch_one(&self.pool)
            .await?;
            (total - TRANSCRIPT_LIMIT).max(0)
        } else {
            0
        };

        // From the oldest Message kept, not by that Message's uuid: a turn that
        // only called tools has no Message row to be named by, and its calls
        // are exactly what the cap must not drop. `None` — an empty
        // conversation, or one whose newest rows predate `created_at` — reads
        // the activities whole, which for an empty conversation is none.
        let activities = activities::for_task_since(
            &self.pool,
            task_id,
            messages.first().and_then(|message| message.created_at),
        )
        .await?;

        Ok(Transcript {
            messages,
            activities,
            omitted,
        })
    }

    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
        task_activities::current_head(&self.pool, task_id).await
    }

    async fn conversation_changes(
        &self,
        task_id: &str,
        from: i64,
        limit: i64,
    ) -> Result<Vec<ConversationChange>, sqlx::Error> {
        task_activities::changes_from(&self.pool, task_id, from, limit).await
    }

    async fn changed_messages(
        &self,
        task_id: &str,
        uuids: &[String],
    ) -> Result<Transcript, sqlx::Error> {
        if uuids.is_empty() {
            return Ok(Transcript {
                messages: Vec::new(),
                activities: Vec::new(),
                omitted: 0,
            });
        }
        // Just the named rows. A result no longer lands on a Message of its
        // own: it settles the activity the call already wrote, and the change
        // that records it names the Message that *made* the call
        // (`agent_handlers::anchor_message_uuid`). So the row a change names is
        // always the row the change is about, and the run of Messages this used
        // to read before each result-carrying one is gone with the reason for
        // it.
        //
        // `parent_uuid IS NULL` for the reason the transcript has it: subagent
        // rows are never rendered inline.
        let messages = sqlx::query_as::<_, TranscriptMessage>(
            r#"SELECT id, uuid, message_type, message_role, message_text,
                      user_type, turn_number, "timestamp", created_at
               FROM task_messages
               WHERE task_id = $1
                 AND uuid = ANY($2)
                 AND parent_uuid IS NULL
               ORDER BY id"#,
        )
        .bind(task_id)
        .bind(uuids)
        .fetch_all(&self.pool)
        .await?;

        // By the named uuids, not by the Messages just read: a change can name
        // a uuid that has no Message row at all — a turn that only called
        // tools — and those activities are the whole of what it changed.
        let activities = activities::for_messages(&self.pool, task_id, uuids).await?;

        Ok(Transcript {
            messages,
            activities,
            omitted: 0,
        })
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
