//! The conversation half of the shell listing's data access.
//!
//! One read, one row type: everything a sidebar row needs that is not on
//! `tasks` is derived here, so the projection above never issues a query per
//! conversation — including, once a conversation is open, its transcript and
//! the changes recorded against it.
//!
//! No T3 vocabulary appears in this file; the names are Moatless's.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::AssertSqlSafe;
use sqlx::types::Json;
use tokio::sync::Semaphore;
use tracing::warn;

use crate::UiRpcDbPool;
use crate::listing::{UiRpcListingChange, UiRpcListingHead};
use crate::task_dao::{T3TaskDao, TaskView};
use crate::transcript_dao::Transcript;
use crate::workspace::UiRpcListedWorkspace;
use crate::workspace_dao::T3WorkspaceDao;
use moatless_core::change_feed::conversation::{ConversationChange, ConversationHead};
use moatless_ports::activity::ActivitySource;
use moatless_types::access_control::{ScopeContext, ScopedEntity};
use moatless_types::activity::{PLAN_TOOL, QUESTION_TOOL};
use moatless_types::agent::AgentProfile;
use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason};
use moatless_types::task::{TaskStatus, Visibility};
use moatless_workspace_types::schemas::workspace::ScriptConfig;

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
    #[sqlx(try_from = "String")]
    pub agent_profile: AgentProfile,
    /// Creation time as the caller recorded it (RFC 3339 text, `NOT NULL`).
    pub created_at: String,
    /// Creation time as the database recorded it. `None` for rows written
    /// before the column existed.
    pub created_at_db: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    /// When this Task was closed. `None` while it is open, which is every row
    /// the live listing returns.
    pub closed_at: Option<DateTime<Utc>>,
    /// Who may read this Task. `NOT NULL`, defaulting to `Private`.
    #[sqlx(try_from = "String")]
    pub visibility: Visibility,
    /// The owner's display name, `None` when the viewer owns the Task or the
    /// owner resolves to no user — a per-viewer value, like the follower state
    /// below. Owner is the access rule's own `COALESCE(owner_user_id,
    /// created_by)`.
    pub owner_name: Option<String>,

    /// The newest Turn, by turn number. `None` when the Task has never run.
    pub turn_number: Option<i32>,
    pub turn_status: Option<String>,
    pub turn_started_at: Option<DateTime<Utc>>,
    pub turn_ended_at: Option<DateTime<Utc>>,

    /// Durable Sandbox lifecycle intent; all four are `None` together when no
    /// Sandbox was ever requested — the lazy-absence model of `sandboxes`.
    ///
    /// Persisted column; read the typed value through
    /// [`Conversation::sandbox_desired_state`].
    pub sandbox_desired_state_raw: Option<String>,
    /// Persisted column; read the typed value through
    /// [`Conversation::sandbox_stop_reason`].
    pub sandbox_stop_reason_raw: Option<String>,
    pub sandbox_started_at: Option<DateTime<Utc>>,
    pub sandbox_updated_at: Option<DateTime<Utc>>,
    /// `moat cmd` commands the Sandbox is running, as its pod last reported.
    /// `0` for a Task with no Sandbox row, which can be running nothing.
    pub running_commands: i32,

    /// When a person last wrote: the newest `user` row that is not a sandbox
    /// wake (`HARNESS_USER_TYPE`); the agent's tool results are `system`.
    ///
    /// Not `tasks.last_message_at`: that also advances when the agent
    /// finishes, stops, or errors.
    pub latest_user_message_at: Option<DateTime<Utc>>,

    /// The newest inbox item including sandbox wakes: what running state
    /// reads, since a queued wake owes a turn as much as a person's message.
    pub latest_queued_message_at: Option<DateTime<Utc>>,

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

    pub settled_override: Option<String>,
    pub settled_at: Option<DateTime<Utc>>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub snoozed_at: Option<DateTime<Utc>>,

    /// Every GitHub pull request this Task is bound to, primary first.
    ///
    /// Empty rather than `None` when there's no active binding — `None` is only
    /// the absent column, which no listing produces. The order matches
    /// [`crate::ports::primary_pull_request`], restated in the query's `ORDER
    /// BY` since the two read different paths and must not disagree on which is
    /// primary.
    pub pull_requests: Option<Json<Vec<ConversationPullRequest>>>,
}

/// One GitHub pull request bound to a Task, as the listing query aggregates it.
///
/// `subject_key` alone makes a row reportable, linking from the moment it's
/// written; `title`, `base_ref` and `head_ref` come from a refresh's status
/// blob and are `None` together until one runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationPullRequest {
    pub repository: String,
    pub number: i64,
    pub url: String,
    pub title: Option<String>,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    /// The binding's normalized state kind, in its own column rather than the
    /// blob: `open`, `draft`, `merged` or `closed`, and `None` until the
    /// adapter has classified one. Read by `t3_pr_state` in [`crate::ports`].
    pub state_kind: Option<String>,
    /// When this deployment last read the pull request's state from GitHub,
    /// falling back to the binding's `NOT NULL` `last_event_at` so it resolves
    /// for an unrefreshed binding too.
    ///
    /// The column [`crate::ports::BoundPullRequest::status_read_at`] reports.
    pub status_read_at: DateTime<Utc>,
    /// When the binding was written. T3's `linkedAt`, and what orders links of
    /// equal standing there.
    pub linked_at: DateTime<Utc>,
    /// `task_bindings.origin`, translated to a T3 link source by
    /// [`crate::projection::pull_request::thread_pull_request_source`].
    pub origin: String,
    /// Latest review verdict, GitHub's spelling. `None` until one is recorded.
    pub review_decision: Option<String>,
    /// GitHub's coarse mergeable state. `None` until it is computed.
    pub mergeable_state: Option<String>,
}

impl Conversation {
    /// Persisted intent for this conversation's Sandbox.
    pub fn sandbox_desired_state(&self) -> Option<SandboxDesiredState> {
        let raw = self.sandbox_desired_state_raw.as_deref()?;
        match SandboxDesiredState::try_from(raw) {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!(task_id = %self.task_id, %error, "unreadable desired_state column");
                None
            }
        }
    }

    /// Why the Sandbox was moved away from running, when it was.
    pub fn sandbox_stop_reason(&self) -> Option<SandboxStopReason> {
        let raw = self.sandbox_stop_reason_raw.as_deref()?;
        match SandboxStopReason::try_from(raw) {
            Ok(reason) => Some(reason),
            Err(error) => {
                tracing::warn!(task_id = %self.task_id, %error, "unreadable stop_reason column");
                None
            }
        }
    }
}

/// One conversation a search matched, and the text that matched in it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConversationMatch {
    pub task_id: String,
    /// `None` for older and hand-written rows, as on [`Conversation`]. A match
    /// without one is dropped by the projection: the contract's match carries a
    /// project id and the client uses it to route.
    pub workspace_id: Option<String>,
    /// Whether a person or the agent wrote the matched text. A Task *name*
    /// matched is reported as the person's — it is Moatless's handle for the
    /// Task and the contract has no third source to call it.
    pub from_user: bool,
    /// What matched: a Message's text, or the Task's name.
    pub matched_text: String,
    /// When the matched Message was written. `None` for a name match, which the
    /// contract allows precisely so a match need not be a Message.
    pub matched_at: Option<DateTime<Utc>>,
}

/// The stored tool call an answer names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveCall {
    /// The tool that asked, as the call recorded it.
    pub tool_name: String,
    /// Whether it has already come back. An answer to a settled call changes
    /// nothing, and saying so is more honest than reporting a send failure.
    pub settled: bool,
}

/// What narrows a browse of conversations the viewer does **not** follow.
///
/// At least one of `owner_user_id` and `tag` is always set — enforced by the
/// only constructor, [`ConversationBrowse::new`] — since the read behind this
/// is [`Reach::Readable`] and unfiltered would be every Task an administrator
/// may read: the whole deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationBrowse {
    /// Whose Tasks, as `COALESCE(owner_user_id, created_by)` — the same
    /// expression the listing resolves `owner_name` against, so picking a name
    /// off a row finds the rest of that person's rows.
    pub owner_user_id: Option<String>,
    /// A tag by `tags.name`, the handle the Moatless UI and the REST listing
    /// both filter on (`ListTasksQuery::tag`).
    pub tag: Option<String>,
    /// Whether closed Tasks come back alongside open ones. Closing is what T3
    /// calls archiving, so this is the archived half folded into the same read.
    pub include_closed: bool,
}

impl ConversationBrowse {
    /// `None` where neither filter is set — see the type's own doc.
    ///
    /// A value is trimmed before being judged empty, as
    /// `snapshots::workspace_cwd` does for the same kind of client-typed field:
    /// whitespace around a tag matches nothing, and the refusal says so where
    /// an empty listing would not.
    pub fn new(
        owner_user_id: Option<String>,
        tag: Option<String>,
        include_closed: bool,
    ) -> Option<Self> {
        let trimmed = |value: String| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        };
        let owner_user_id = owner_user_id.and_then(trimmed);
        let tag = tag.and_then(trimmed);
        if owner_user_id.is_none() && tag.is_none() {
            return None;
        }
        Some(Self {
            owner_user_id,
            tag,
            include_closed,
        })
    }

    fn statuses(&self) -> &'static [TaskStatus] {
        if self.include_closed {
            LISTED_STATUSES
        } else {
            &[TaskStatus::Open]
        }
    }
}

/// Read the conversations one viewer's sidebar is built from, and the position
/// a client watching it resumes from.
///
/// The whole of this module's internal interface: read what this viewer
/// follows, re-read what a change names, and read the position. A trait because
/// the module's transport tests stub it; the only production implementation is
/// [`PgConversationDao`].
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ConversationDao: Send + Sync {
    /// Every **open** Task the viewer follows **and** may still read, newest
    /// first.
    ///
    /// Three independently load-bearing rules: following makes this a sidebar
    /// rather than a deployment listing; the read rule stops a durable follow
    /// from outliving the access that produced it; the status rule stops it
    /// from outliving the *work* — closing is done, and a sidebar that kept
    /// closed rows would never shrink. `error` counts as not-open too, matching
    /// the Moatless sidebar's own filter; a Task returns to `open` the instant
    /// its agent produces anything, so nothing running is hidden.
    async fn list_followed(&self, viewer: &ScopeContext) -> Result<Vec<Conversation>, sqlx::Error>;

    /// The other half: the newest **closed** Tasks the viewer follows and may
    /// still read, older than `before` when given — at most [`ARCHIVED_LIMIT`]
    /// plus one, which a caller drops.
    ///
    /// Closing is what T3 calls archiving. Only the status rule inverts from
    /// [`Self::list_followed`], so the two partition what a person follows.
    async fn list_followed_closed(
        &self,
        viewer: &ScopeContext,
        before: Option<ArchiveCursor>,
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// Every Task the viewer may **read** that `filter` names, newest first,
    /// whether or not they follow it.
    ///
    /// At most [`BROWSE_LIMIT`] rows plus one — the extra signals a filter that
    /// matched more than the cap, and a caller must drop it. Drops the follow
    /// rule entirely (this answers "what is there", not "what am I involved
    /// in"); the read rule stays [`Reach::Readable`], same as
    /// [`Self::readable_by_id`]; the status rule moves into
    /// [`ConversationBrowse::include_closed`]. Follower state is absent on an
    /// unfollowed row, since following is what a client offers there instead.
    async fn list_browsable(
        &self,
        viewer: &ScopeContext,
        filter: &ConversationBrowse,
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// Both halves at once, narrowed to Tasks a change has named.
    ///
    /// The follow and read rules still apply — a change is deployment-wide but
    /// visibility isn't — so an unfollowed or unreadable Task comes back
    /// absent, which the feed turns into a removal. The status rule widens: a
    /// closed Task that is still followed and readable comes back present with
    /// `archivedAt`, not absent, since the feed may only retract an id it can
    /// prove it sent (a resumed cursor can't), so only an upsert reaches a
    /// resumed sidebar. `error` stays out of both halves — upserting it would
    /// return the row to the sidebar instead of leaving it.
    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// One conversation this viewer may **read**, whether or not they follow
    /// it.
    ///
    /// What a link resolves through: a URL is about access, not involvement, so
    /// a readable Task opens even when unfollowed — writing to it is what
    /// acquires the Follow, joining the sidebar only once the reader takes
    /// part. The read and status rules are unchanged; absence still means gone,
    /// unreadable, or at a status neither listing claims.
    async fn readable_by_id(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Conversation>, sqlx::Error>;

    /// The same read over a set of Tasks, in one query — what the subtask
    /// listing needs for the children a relation named.
    ///
    /// [`Self::readable_by_id`]'s rules unchanged: an unreadable id comes back
    /// absent, not as a row with fields blanked. Order is not specified; the
    /// caller sorts.
    async fn readable_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// Whether this viewer may *operate* the named Task — the `control_filter`
    /// rule: the write rule, widened by `visibility = 'public'`.
    ///
    /// The gate on every path that runs rather than changes something
    /// (terminal, script, Sandbox, turn). Asked directly rather than after a
    /// Follow, since a Follow is acquired by posting one message — which a
    /// public Task accepts from anyone — so it is not a restriction a
    /// controller should have to acquire.
    async fn controls_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Conversations whose name or Message text contains `query`, at most one
    /// match each, best first.
    ///
    /// Filtered by the same follow, access and open-status rules as
    /// [`ConversationDao::list_followed`] (reused fragments, not restated) — a
    /// search result the sidebar wouldn't show is a disclosure. One row per
    /// Task; a Message match beats a name match, and a person's beats the
    /// agent's, matching the reference server's ranking
    /// (`ProjectionSnapshotQuery.ts:725`).
    async fn search_conversations(
        &self,
        viewer: &ScopeContext,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ConversationMatch>, sqlx::Error>;

    /// One Task's transcript, oldest first, capped at `TRANSCRIPT_LIMIT`.
    ///
    /// Unscoped, deliberately: every caller has already resolved the same Task
    /// through [`ConversationDao::list_followed_by_ids`], where the follow and
    /// read rules live — repeating them here would be a second access filter to
    /// keep in step with the first.
    async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error>;

    /// The names of the Tasks a transcript's Messages came from, by task id.
    ///
    /// Scoped, unlike [`ConversationDao::transcript`]: these are *other* Tasks
    /// the caller has resolved nothing about. An unreadable Task yields no
    /// entry, so the origin built from it names no Task and links nowhere — the
    /// same answer as a deleted one.
    async fn source_task_names(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<HashMap<String, String>, sqlx::Error>;

    /// The interactive call an answer names, by the id it was made under.
    /// `None` where no call by that id was ever stored.
    ///
    /// An answer's shape depends on which tool asked — a question takes answers
    /// per question, a plan takes a yes or no — so the command reads the name
    /// off the stored call rather than assuming one. Unscoped for the reason
    /// [`ConversationDao::transcript`] is.
    async fn interactive_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<InteractiveCall>, sqlx::Error>;

    /// The highest position this conversation's change log has reached.
    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error>;

    /// This conversation's changes at or after `from`, oldest first, capped at
    /// `limit`. At `from` rather than after it — see
    /// [`moatless_core::change_feed::conversation::changes_from`].
    async fn conversation_changes(
        &self,
        task_id: &str,
        from: i64,
        limit: i64,
    ) -> Result<Vec<ConversationChange>, sqlx::Error>;

    /// The rows a batch of changes names, oldest first: Messages, the
    /// activities under them or named by their own id, and the interactive
    /// calls those answer, each with the Message it sits on.
    ///
    /// Elides nothing, so [`Transcript::omitted`] is zero, and is unscoped for
    /// the reason [`ConversationDao::transcript`] is.
    async fn changed_messages(
        &self,
        task_id: &str,
        uuids: &[String],
        activity_ids: &[String],
    ) -> Result<Transcript, sqlx::Error>;

    /// The highest listing position this deployment has reached, and the
    /// identity of the database it reached it in.
    async fn listing_head(&self) -> Result<UiRpcListingHead, sqlx::Error>;

    /// Every listing change recorded after `after`, oldest first.
    async fn listing_changes_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<UiRpcListingChange>, sqlx::Error>;
}

/// A full shell snapshot is a deployment-sized read with no pagination in T3's
/// contract, and the archived listing is the same read at the other status.
///
/// One of either may run per backend replica — live change re-reads are
/// narrowed by task id and don't take this permit — reserving the rest of the
/// main pool for task control, ingress, and reconciliation under shell
/// overload.
#[derive(Clone)]
pub struct PgConversationDao {
    pool: UiRpcDbPool,
    /// The Task band's activity rows. This crate projects them; it does not own
    /// the table, so it reads them through the published seam.
    activities: Arc<dyn ActivitySource>,
    full_listing: Arc<Semaphore>,
}

/// A published-seam read that failed, in the `sqlx::Error` the T3 DAO trait
/// returns. The whole chain is kept as the protocol text so the response
/// boundary logs what actually failed.
pub(crate) fn store_read(error: moatless_types::store::StoreError) -> sqlx::Error {
    sqlx::Error::Protocol(moatless_core::error::causal_chain(&error))
}

const FULL_LISTING_CONCURRENCY: usize = 1;
const FULL_LISTING_STATEMENT_TIMEOUT: &str = "5s";

/// How many rows a browse returns: the newest, since unlike the follow-bounded
/// listings a browse is bounded only by the filter and has no natural ceiling.
///
/// The query asks for one extra row past this so `snapshots::browse_snapshot`
/// can tell a cut list from one that ends exactly on the cap, rather than
/// calling a complete 200-row list truncated.
pub(crate) const BROWSE_LIMIT: i64 = 200;

/// How many rows the archived listing returns: the newest. Unlike the open
/// half, which shrinks as work closes, the archive only grows, so every close
/// would otherwise add a row to every later read. Asked for plus one, as
/// [`BROWSE_LIMIT`] is.
pub(crate) const ARCHIVED_LIMIT: i64 = 200;

/// Where an archive page resumes: the sort key of the last row the page
/// before it returned, in the listing's `updated_at DESC NULLS LAST, task_id`
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveCursor {
    updated_at: Option<DateTime<Utc>>,
    task_id: String,
}

impl ArchiveCursor {
    /// The cursor resuming after `conversation`.
    pub fn after(conversation: &Conversation) -> Self {
        Self {
            updated_at: conversation.updated_at,
            task_id: conversation.task_id.clone(),
        }
    }

    /// The opaque text a client echoes back: `<updated_at>|<task_id>`, the
    /// time empty when absent. Microsecond precision, since that is what
    /// Postgres stores and a coarser time would skip or repeat rows.
    pub fn encode(&self) -> String {
        let updated_at = self
            .updated_at
            .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
            .unwrap_or_default();
        format!("{updated_at}|{}", self.task_id)
    }

    /// `None` for text [`Self::encode`] did not produce.
    pub fn decode(text: &str) -> Option<Self> {
        let (updated_at, task_id) = text.split_once('|')?;
        if task_id.is_empty() {
            return None;
        }
        let updated_at = match updated_at {
            "" => None,
            at => Some(DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc)),
        };
        Some(Self {
            updated_at,
            task_id: task_id.to_owned(),
        })
    }
}

/// A capped read: `cap` rows, newest first, after `before` when given.
struct Page<'a> {
    cap: i64,
    before: Option<&'a ArchiveCursor>,
}

/// The statuses a conversation can be listed under: the sidebar and the archive
/// together. A Task at any other status is in neither, and comes back absent
/// from a re-read the same way one the viewer no longer follows does.
const LISTED_STATUSES: &[TaskStatus] = &[TaskStatus::Open, TaskStatus::Closed];

/// How wide a conversation read reaches: both variants share the same read rule
/// — `Readable` widens past the follow join only, never past access — so
/// follower state (`settled_*`, `snoozed_*`) stays absent for a viewer who does
/// not follow, which is what the outer join expresses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// The sidebar listings: involvement only.
    Followed,
    /// Opening one by id: a link resolves whether or not it's followed;
    /// writing to it is what follows it, via `message::service`.
    Readable,
}

/// A search term as a `LIKE` operand rather than a `LIKE` pattern.
///
/// `%`, `_`, and the escape character itself (`\`) are escaped, since they're
/// ordinary characters to the person typing them; every query using this must
/// declare `ESCAPE '\'` explicitly, since Postgres's default depends on
/// `standard_conforming_strings`.
fn escape_like(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len());
    for character in query.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
const LISTING_DURATION: &str = "moatless_ui_shell_listing_duration_seconds";
const LISTING_REJECTIONS: &str = "moatless_ui_shell_listing_rejections_total";

impl PgConversationDao {
    pub fn new(pool: UiRpcDbPool, activities: Arc<dyn ActivitySource>) -> Self {
        metrics::describe_histogram!(
            LISTING_DURATION,
            "Full T3 shell listing duration by kind and outcome"
        );
        metrics::describe_counter!(
            LISTING_REJECTIONS,
            "Full T3 shell listings rejected before completion by reason and kind"
        );
        Self {
            pool,
            activities,
            full_listing: Arc::new(Semaphore::new(FULL_LISTING_CONCURRENCY)),
        }
    }

    /// The one listing query at a set of statuses, optionally narrowed to a set
    /// of Tasks or a browse filter, and read a `page` at a time.
    ///
    /// Every narrowing is a `WHERE` clause on this query, so the read and follow
    /// rules cannot drift between a snapshot and a live re-read of it.
    async fn list(
        &self,
        viewer: &ScopeContext,
        reach: Reach,
        statuses: &[TaskStatus],
        task_ids: Option<&[String]>,
        browse: Option<&ConversationBrowse>,
        page: Option<Page<'_>>,
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        let full_listing = task_ids.is_none();
        // A browse takes the same permit, deliberately: it is the other read
        // here with no follow join to narrow it, and its `LIMIT` bounds the rows
        // it returns rather than the scan that finds them. A permit of its own
        // would let two deployment-sized reads onto the pool at once, which is
        // the thing this bounds. So the two reject each other, and `kind` is
        // what tells them apart afterwards — `task_ids.is_none()` no longer
        // means "the sidebar's listing".
        let kind = if browse.is_some() {
            "browse"
        } else {
            "listing"
        };
        let _permit = if full_listing {
            let Ok(permit) = self.full_listing.try_acquire() else {
                metrics::counter!(LISTING_REJECTIONS, "reason" => "at_capacity", "kind" => kind)
                    .increment(1);
                warn!(
                    user_id = %viewer.user_id,
                    kind,
                    "T3 UI full listing rejected; another one is in progress"
                );
                return Err(sqlx::Error::PoolTimedOut);
            };
            Some(permit)
        } else {
            None
        };
        let started = full_listing.then(Instant::now);

        // `$1`/`$2` are the access filter's admin flag and user id; `$3` is the
        // follower, and the same viewer the owner name is decided against. The
        // read rule is the shared fragment rather than a copy, so
        // a change to what "may read" means reaches this listing too.
        //
        // The status rule is the third filter, and it is the enum's own spelling
        // rather than a literal typed twice: `tasks.status` is `text` with no
        // CHECK constraint, so a misspelling here would be a listing that is
        // always empty and a database that never complains.
        let statuses = statuses
            .iter()
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 1);
        let follower = access.next_param();
        let question_tool = follower + 1;
        let plan_tool = follower + 2;
        // Every clause below allocates the next position in turn, and the binds
        // at the bottom of this function follow the same order.
        let mut next_param = plan_tool;
        let mut param = move || {
            next_param += 1;
            next_param
        };
        let narrowed = task_ids
            .map(|_| format!("\n                 AND t.task_id = ANY(${})", param()))
            .unwrap_or_default();
        // Each browse filter is a clause only when it is set, rather than a
        // `$n IS NULL OR` pair: an unset one has to disappear from the query
        // for the index behind it to be reachable, and the browse read is the
        // one here with no follow join to narrow it first.
        //
        // Predicates only. The cap is appended after them and holds no
        // parameter, so a filter added here cannot land behind `LIMIT` and
        // produce SQL that fails at runtime instead of at compile time.
        let mut predicates: Vec<String> = Vec::new();
        if let Some(browse) = browse {
            if browse.owner_user_id.is_some() {
                predicates.push(format!(
                    "AND COALESCE(t.owner_user_id, t.created_by) = ${}",
                    param()
                ));
            }
            if browse.tag.is_some() {
                predicates.push(format!(
                    "AND EXISTS (
                          SELECT 1 FROM task_tags tt
                          JOIN tags tg ON tg.tag_id = tt.tag_id
                          WHERE tt.task_id = t.task_id AND tg.name = ${}
                      )",
                    param()
                ));
            }
        }
        // Keyset rather than `OFFSET`, so a Task closed between two pages
        // shifts no row across the boundary. `-infinity` stands in for a NULL
        // `updated_at`, which `NULLS LAST` sorts in the same place.
        let before = page.as_ref().and_then(|page| page.before);
        if before.is_some() {
            let (at, id) = (param(), param());
            predicates.push(format!(
                "AND (COALESCE(t.updated_at, '-infinity') < COALESCE(${at}::timestamptz, '-infinity')
                       OR (COALESCE(t.updated_at, '-infinity') = COALESCE(${at}::timestamptz, '-infinity')
                           AND t.task_id > ${id}))"
            ));
        }
        let predicates = predicates
            .iter()
            .flat_map(|predicate| ["\n                 ", predicate.as_str()])
            .collect::<String>();
        // The cap is inside the CTE, which is also where the order it takes the
        // newest rows by has to live: the outer `ORDER BY` runs after the
        // laterals, and ordering there would cap an arbitrary set.
        let capped = page
            .as_ref()
            .map(|Page { cap, .. }| {
                format!(
                    "\n                 ORDER BY t.updated_at DESC NULLS LAST, \
                     t.task_id ASC LIMIT {cap}"
                )
            })
            .unwrap_or_default();
        // The follow rule is the join, so widening past it is the join kind and
        // nothing else: `${follower}` moves into the `ON` clause, leaving the
        // status rule, the read rule and the narrowing exactly as they are, at
        // the same parameter positions.
        let follows = match reach {
            Reach::Followed => format!(
                "FROM task_follows tf
                   JOIN tasks t ON t.task_id = tf.task_id
                   WHERE tf.user_id = ${follower}
                     AND"
            ),
            Reach::Readable => format!(
                "FROM tasks t
                   LEFT JOIN task_follows tf
                     ON tf.task_id = t.task_id AND tf.user_id = ${follower}
                   WHERE"
            ),
        };
        let harness = moatless_types::message::HARNESS_USER_TYPE;
        let sql = format!(
            r#"WITH listed AS MATERIALIZED (
                   SELECT t.*, tf.settled_override, tf.settled_at, tf.snoozed_until, tf.snoozed_at
                   {follows} t.status IN ({statuses})
                     AND {}{narrowed}{predicates}{capped}
               )
               SELECT
                   t.task_id,
                   t.workspace_id,
                   t.name,
                   t.branch,
                   t.agent_profile,
                   t.created_at,
                   t.created_at_db,
                   t.updated_at,
                   t.closed_at,
                   t.visibility,
                   COALESCE(owner.name, owner.login) AS owner_name,
                   turn.turn_number,
                   turn.status                AS turn_status,
                   turn.started_at            AS turn_started_at,
                   turn.ended_at              AS turn_ended_at,
                   s.desired_state            AS sandbox_desired_state_raw,
                   s.stop_reason              AS sandbox_stop_reason_raw,
                   s.started_at               AS sandbox_started_at,
                   s.desired_state_updated_at AS sandbox_updated_at,
                   COALESCE(s.running_commands, 0) AS running_commands,
                   person.latest_user_message_at,
                   queued.latest_queued_message_at,
                   model.assistant_model,
                   model.requested_model,
                   COALESCE(pending.question, false) AS pending_question,
                   COALESCE(pending.plan, false)     AS pending_plan,
                   t.settled_override,
                   t.settled_at,
                   t.snoozed_until,
                   t.snoozed_at,
                   COALESCE(pr.pull_requests, '[]'::jsonb) AS pull_requests
               FROM listed t
               LEFT JOIN sandboxes s ON s.sandbox_id = t.sandbox_id
               -- Whose Task this is, for the rows that are somebody else's. The
               -- viewer test is in the `ON` clause rather than a `CASE`, so a
               -- viewer's own Task simply misses the join and reports no name.
               -- The id is compared on its text form for the reason
               -- `TRANSCRIPT_ORIGIN_JOINS` states.
               LEFT JOIN users owner
                 ON owner.id::text = COALESCE(t.owner_user_id, t.created_by)
                AND COALESCE(t.owner_user_id, t.created_by) IS DISTINCT FROM ${follower}
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
                     AND m.user_type <> '{harness}'
                   ORDER BY m.created_at DESC
                   LIMIT 1
               ) person ON TRUE
               LEFT JOIN LATERAL (
                   SELECT m.created_at AS latest_queued_message_at
                   FROM task_inbox m
                   WHERE m.task_id = t.task_id
                     AND m.status <> 'cancelled'
                   ORDER BY m.created_at DESC
                   LIMIT 1
               ) queued ON TRUE
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
               -- What this no longer clears on is text: a question followed by
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
               -- The pull requests the thread row links to, read from the
               -- Task's GitHub PR bindings rather than from its branch: the
               -- agent commits on whatever branch it cut, and `tasks.branch`
               -- holds the base the Task was created on, so a branch-matched
               -- read reports nothing for most Tasks that have a PR. A Task can
               -- carry several — the unique index keys on `subject_key`, and
               -- nothing retires the binding a new one lands beside.
               --
               -- `subject_key` is `owner/repo#N`; the regex guard is what makes
               -- the `::bigint` casts total, so a malformed key drops its row
               -- instead of failing the whole listing.
               --
               -- `subject_key` is the only column a row needs to be reportable,
               -- so a binding links before its status blob is fetched. Reading
               -- a key out of that blob as a *condition* would hide every
               -- freshly opened pull request: the blob is written NULL and
               -- filled in by a refresh this query cannot trigger.
               LEFT JOIN LATERAL (
                   SELECT jsonb_agg(
                              jsonb_build_object(
                                  'repository', bound.repository,
                                  'number', bound.number,
                                  'url', bound.url,
                                  'title', bound.title,
                                  'base_ref', bound.base_ref,
                                  'head_ref', bound.head_ref,
                                  'state_kind', bound.state_kind,
                                  'status_read_at', bound.status_read_at,
                                  'linked_at', bound.linked_at,
                                  'origin', bound.origin,
                                  'review_decision', bound.review_decision,
                                  'mergeable_state', bound.mergeable_state
                              )
                              ORDER BY bound.live DESC, bound.number DESC
                          ) AS pull_requests
                   FROM (
                       SELECT
                           split_part(b.subject_key, '#', 1) AS repository,
                           split_part(b.subject_key, '#', 2)::bigint AS number,
                           COALESCE(
                               NULLIF(b.external_status_raw ->> 'url', ''),
                               'https://github.com/' || split_part(b.subject_key, '#', 1)
                                   || '/pull/' || split_part(b.subject_key, '#', 2)
                           ) AS url,
                           NULLIF(b.external_status_raw ->> 'title', '')  AS title,
                           NULLIF(b.external_status_raw ->> 'baseRef', '') AS base_ref,
                           NULLIF(b.external_status_raw ->> 'headRef', '') AS head_ref,
                           b.external_state_kind AS state_kind,
                           COALESCE(b.external_status_updated_at, b.last_event_at)
                               AS status_read_at,
                           b.created_at AS linked_at,
                           b.origin AS origin,
                           b.external_review_decision AS review_decision,
                           b.external_mergeable_state AS mergeable_state,
                           (COALESCE(b.external_state_kind, 'open')
                                NOT IN ('merged', 'closed')) AS live
                       FROM task_bindings b
                       WHERE b.task_id = t.task_id
                         AND b.active
                         AND b.adapter_kind = 'github'
                         AND b.subject_kind = 'github_pr'
                         AND b.subject_key ~ '^[^#]+#[0-9]+$'
                   ) bound
               ) pr ON TRUE
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

        let mut query = sqlx::query_as::<_, Conversation>(AssertSqlSafe(sql))
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .bind(&viewer.user_id)
            .bind(QUESTION_TOOL)
            .bind(PLAN_TOOL);
        // Bound in the order the clauses above allocated their positions.
        if let Some(task_ids) = task_ids {
            query = query.bind(task_ids.to_vec());
        }
        if let Some(browse) = browse {
            if let Some(owner_user_id) = browse.owner_user_id.clone() {
                query = query.bind(owner_user_id);
            }
            if let Some(tag) = browse.tag.clone() {
                query = query.bind(tag);
            }
        }
        if let Some(before) = before {
            query = query.bind(before.updated_at).bind(before.task_id.clone());
        }
        let result = match task_ids {
            Some(_) => query.fetch_all(&self.pool).await,
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
                    metrics::counter!(
                        LISTING_REJECTIONS,
                        "reason" => "statement_timeout",
                        "kind" => kind
                    )
                    .increment(1);
                    "statement_timeout"
                }
                Err(_) => "error",
            };
            metrics::histogram!(LISTING_DURATION, "outcome" => outcome, "kind" => kind)
                .record(started.elapsed().as_secs_f64());
        }
        result
    }
}

#[async_trait]
impl T3WorkspaceDao for PgConversationDao {
    async fn list_readable_including(
        &self,
        viewer: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<UiRpcListedWorkspace>, sqlx::Error> {
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "", 1);
        let ids_param = access.next_param();
        // `deleted` narrows the readable half only. A git sync tombstones a
        // Workspace while open Tasks still name it, and excluding it there
        // takes those Tasks out of the sidebar and refuses their links.
        let sql = format!(
            "SELECT *, {} AS readable FROM workspaces \
             WHERE ((deleted IS NULL OR deleted = false) AND {}) \
             OR id = ANY(${ids_param}) \
             ORDER BY name ASC",
            access.sql(),
            access.sql()
        );
        sqlx::query_as::<_, UiRpcListedWorkspace>(AssertSqlSafe(sql))
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .bind(also_include)
            .fetch_all(&self.pool)
            .await
    }

    async fn primary_repository(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        // Joined rather than read in two steps so the scoped read and the pick
        // are one statement: a Workspace the viewer cannot be shown returns no
        // row, which is the same answer as one that declares no repository.
        let access =
            moatless_core::db::access_control::access_filter(ScopedEntity::Resource, "w.", 2);
        let sql = format!(
            "SELECT r.repository_id FROM workspace_repos r              JOIN workspaces w ON w.id = r.workspace_id              WHERE r.workspace_id = $1              AND (w.deleted IS NULL OR w.deleted = false)              AND {}              ORDER BY r.is_primary DESC, r.position ASC, r.id ASC              LIMIT 1",
            access.sql()
        );
        sqlx::query_scalar::<_, String>(AssertSqlSafe(sql))
            .bind(workspace_id)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_optional(&self.pool)
            .await
    }

    async fn scripts_for_task(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Vec<ScriptConfig>>, sqlx::Error> {
        // Joined and scoped on the Task, so a Task the viewer cannot be shown
        // returns no row — the same "not found" the caller gets for a missing
        // one. `COALESCE` so a workspace predating the scripts column reads as an
        // empty list rather than a NULL that fails to decode.
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 2);
        let sql = format!(
            "SELECT COALESCE(w.scripts, '[]'::jsonb) \
             FROM tasks t JOIN workspaces w ON w.id = t.workspace_id \
             WHERE t.task_id = $1 \
             AND (w.deleted IS NULL OR w.deleted = false) \
             AND {}",
            access.sql()
        );
        let raw = sqlx::query_scalar::<_, serde_json::Value>(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        // A stored value that will not decode is reported as no scripts rather
        // than failing the run — the same tolerance the projection applies.
        Ok(Some(serde_json::from_value(raw).unwrap_or_default()))
    }
}

#[async_trait]
impl T3TaskDao for PgConversationDao {
    async fn find_by_id(
        &self,
        task_id: &str,
        viewer: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error> {
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "", 2);
        self.task_view(task_id, viewer, &access).await
    }

    async fn controlled_by_id(
        &self,
        task_id: &str,
        viewer: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error> {
        let control = moatless_core::db::access_control::control_filter(ScopedEntity::Task, "", 2);
        self.task_view(task_id, viewer, &control).await
    }

    async fn written_by_id(
        &self,
        task_id: &str,
        viewer: &ScopeContext,
    ) -> Result<Option<TaskView>, sqlx::Error> {
        let write = moatless_core::db::access_control::write_filter("", 2);
        self.task_view(task_id, viewer, &write).await
    }
}

impl PgConversationDao {
    /// One Task's [`TaskView`] under whichever grant the caller's filter states
    /// — the three `T3TaskDao` reads differ only in that filter.
    ///
    /// `grant` must be built with `start = 2`: `task_id` takes `$1` and the
    /// filter's own two parameters follow it. Built at any other start, the
    /// query reads the wrong bind positions and returns rows for the wrong
    /// viewer.
    async fn task_view(
        &self,
        task_id: &str,
        viewer: &ScopeContext,
        grant: &moatless_core::db::access_control::AccessFilter,
    ) -> Result<Option<TaskView>, sqlx::Error> {
        let sql = format!(
            "SELECT status, ttl_minutes FROM tasks WHERE task_id = $1 AND {}",
            grant.sql()
        );
        sqlx::query_as::<_, TaskView>(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_optional(&self.pool)
            .await
    }
}

#[async_trait]
impl ConversationDao for PgConversationDao {
    async fn list_followed(&self, viewer: &ScopeContext) -> Result<Vec<Conversation>, sqlx::Error> {
        self.list(
            viewer,
            Reach::Followed,
            &[TaskStatus::Open],
            None,
            None,
            None,
        )
        .await
    }

    async fn list_followed_closed(
        &self,
        viewer: &ScopeContext,
        before: Option<ArchiveCursor>,
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.list(
            viewer,
            Reach::Followed,
            &[TaskStatus::Closed],
            None,
            None,
            Some(Page {
                cap: ARCHIVED_LIMIT + 1,
                before: before.as_ref(),
            }),
        )
        .await
    }

    async fn list_browsable(
        &self,
        viewer: &ScopeContext,
        filter: &ConversationBrowse,
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.list(
            viewer,
            Reach::Readable,
            filter.statuses(),
            None,
            Some(filter),
            Some(Page {
                cap: BROWSE_LIMIT + 1,
                before: None,
            }),
        )
        .await
    }

    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.list(
            viewer,
            Reach::Followed,
            LISTED_STATUSES,
            Some(task_ids),
            None,
            None,
        )
        .await
    }

    async fn readable_by_id(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Conversation>, sqlx::Error> {
        Ok(self
            .list(
                viewer,
                Reach::Readable,
                LISTED_STATUSES,
                Some(&[task_id.to_string()]),
                None,
                None,
            )
            .await?
            .into_iter()
            .next())
    }

    async fn readable_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.list(
            viewer,
            Reach::Readable,
            LISTED_STATUSES,
            Some(task_ids),
            None,
            None,
        )
        .await
    }

    async fn controls_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let control = moatless_core::db::access_control::control_filter(ScopedEntity::Task, "", 2);
        let sql = format!(
            "SELECT 1 FROM tasks WHERE task_id = $1 AND {}",
            control.sql()
        );
        let found = sqlx::query_scalar::<_, i32>(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(found.is_some())
    }

    async fn search_conversations(
        &self,
        viewer: &ScopeContext,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ConversationMatch>, sqlx::Error> {
        // The same three filters the listing applies, written the same way:
        // the follow join, the shared access fragment, and the open status. A
        // fourth copy of the read rule is the drift the narrowed listing query
        // exists to avoid, and here a drift is a disclosure rather than a
        // missing row.
        let open = TaskStatus::Open.as_str();
        let harness = moatless_types::message::HARNESS_USER_TYPE;
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "t.", 1);
        let follower = access.next_param();
        let pattern = follower + 1;
        let row_limit = pattern + 1;
        let sql = format!(
            r#"WITH listed AS MATERIALIZED (
                   SELECT t.task_id, t.workspace_id, t.name, t.updated_at
                   FROM task_follows tf
                   JOIN tasks t ON t.task_id = tf.task_id
                   WHERE tf.user_id = ${follower}
                     AND t.status = '{open}'
                     AND {}
               ),
               matched AS (
                   -- A Message match. `message_type` distinguishes the person
                   -- from the agent; `parent_uuid IS NULL` drops subagent rows,
                   -- which are never rendered inline and would otherwise be the
                   -- bulk of what a long conversation matches.
                   SELECT
                       t.task_id,
                       t.workspace_id,
                       m.message_type = 'user' AS from_user,
                       m.message_text        AS matched_text,
                       m.created_at          AS matched_at,
                       t.updated_at,
                       CASE WHEN m.message_type = 'user' THEN 0 ELSE 1 END AS rank
                   FROM listed t
                   JOIN task_messages m ON m.task_id = t.task_id
                   WHERE m.parent_uuid IS NULL
                     -- A harness wake is in no conversation to jump to.
                     AND m.user_type <> '{harness}'
                     AND m.message_text ILIKE ${pattern} ESCAPE '\'
                   UNION ALL
                   -- A name match, which the reference server has no equivalent
                   -- of. `tasks.name` is Moatless's own handle for a Task and is
                   -- often the only text a person remembers; it ranks below both
                   -- Message sources because a Message match is the more
                   -- specific answer about the same Task.
                   SELECT
                       t.task_id,
                       t.workspace_id,
                       true   AS from_user,
                       t.name AS matched_text,
                       NULL::timestamptz AS matched_at,
                       t.updated_at,
                       2 AS rank
                   FROM listed t
                   WHERE t.name ILIKE ${pattern} ESCAPE '\'
               ),
               ranked AS (
                   SELECT
                       matched.*,
                       ROW_NUMBER() OVER (
                           PARTITION BY task_id
                           ORDER BY rank ASC, matched_at DESC NULLS LAST
                       ) AS per_task
                   FROM matched
               )
               SELECT task_id, workspace_id, from_user, matched_text, matched_at
               FROM ranked
               WHERE per_task = 1
               ORDER BY rank ASC, updated_at DESC NULLS LAST, task_id ASC
               LIMIT ${row_limit}"#,
            access.sql(),
        );

        sqlx::query_as::<_, ConversationMatch>(AssertSqlSafe(sql))
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .bind(&viewer.user_id)
            .bind(format!("%{}%", escape_like(query)))
            .bind(limit)
            .fetch_all(&self.pool)
            .await
    }

    async fn interactive_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<InteractiveCall>, sqlx::Error> {
        Ok(self
            .activities
            .tool_call(task_id, tool_use_id)
            .await
            .map_err(store_read)?
            .map(|activity| InteractiveCall {
                settled: activity.is_settled(),
                tool_name: activity.summary.clone(),
            }))
    }

    async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error> {
        crate::transcript_dao::transcript(&self.pool, &*self.activities, task_id).await
    }

    async fn source_task_names(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<HashMap<String, String>, sqlx::Error> {
        // Scoped, so a Task outside the viewer's reach yields no row and the
        // origin built from it carries neither a name nor a link. `$1` is the
        // id set; the access filter binds `is_admin` then `user_id` from `$2`.
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "", 2);
        let sql = format!(
            "SELECT task_id, name FROM tasks WHERE task_id = ANY($1) AND {}",
            access.sql()
        );
        let rows: Vec<(String, String)> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(task_ids)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().collect())
    }

    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
        moatless_core::change_feed::conversation::current_head(&self.pool, task_id).await
    }

    async fn conversation_changes(
        &self,
        task_id: &str,
        from: i64,
        limit: i64,
    ) -> Result<Vec<ConversationChange>, sqlx::Error> {
        moatless_core::change_feed::conversation::changes_from(&self.pool, task_id, from, limit)
            .await
    }

    async fn changed_messages(
        &self,
        task_id: &str,
        uuids: &[String],
        activity_ids: &[String],
    ) -> Result<Transcript, sqlx::Error> {
        crate::transcript_dao::changed_messages(
            &self.pool,
            &*self.activities,
            task_id,
            uuids,
            activity_ids,
        )
        .await
    }

    async fn listing_head(&self) -> Result<UiRpcListingHead, sqlx::Error> {
        crate::listing::current_head(&self.pool).await
    }

    async fn listing_changes_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<UiRpcListingChange>, sqlx::Error> {
        crate::listing::changes_after(&self.pool, after, limit).await
    }
}
/// Creator identity for a Task whose readability the caller has already
/// checked.
pub async fn task_creator(
    pool: &sqlx::PgPool,
    task_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    Ok(
        sqlx::query_scalar::<_, Option<String>>("SELECT created_by FROM tasks WHERE task_id = $1")
            .bind(task_id)
            .fetch_optional(pool)
            .await?
            .flatten(),
    )
}

/// Whether a message has claimed an attachment that is no longer a draft.
pub async fn attachment_is_on_message(
    pool: &sqlx::PgPool,
    attachment_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM task_message_contexts WHERE reference_id = $1 AND type = 'file')")
        .bind(attachment_id).fetch_one(pool).await
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
