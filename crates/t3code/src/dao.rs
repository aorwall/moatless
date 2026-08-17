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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use tokio::sync::Semaphore;
use tracing::warn;

use crate::UiRpcDbPool;
use crate::activity_dao;
use crate::activity_dao::SubagentUsage;
use crate::data::{T3TaskDao, T3WorkspaceDao, TaskView};
use crate::listing::{UiRpcListingChange, UiRpcListingHead};
use crate::workspace::UiRpcListedWorkspace;
use moatless_api_schemas::workspace::ScriptConfig;
use moatless_tasks::task_activities::dao::Activity;
use moatless_tasks::task_activities::mapping::{PLAN_TOOL, QUESTION_TOOL};
use moatless_tasks::task_activities::{ConversationChange, ConversationHead};
use moatless_backend_core::models::AgentType;
use moatless_backend_core::models::{MessageRole, MessageType};
use moatless_backend_core::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};
use moatless_backend_core::scope::{ScopeContext, ScopedEntity};
use moatless_tasks::task_activities::dao as activities;

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
    /// When this Task was closed. `None` while it is open, which is every row
    /// the live listing returns.
    pub closed_at: Option<DateTime<Utc>>,

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

    pub settled_override: Option<String>,
    pub settled_at: Option<DateTime<Utc>>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub snoozed_at: Option<DateTime<Utc>>,
}

/// One Message of a Task's transcript, with the columns a transcript is
/// rendered from and nothing else.
///
/// Deliberately not the backend's `TaskMessage`: that row carries
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
    /// text: the files, contexts and attribution the agent is sent are rebuilt
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
    /// The adapter a Message reached this Task through — `slack`, `github_pr`,
    /// `linear`. `None` on a Message that came from inside Moatless.
    pub external_adapter: Option<String>,
    /// The name the subscription knows the source by — a channel, a repository.
    pub external_source_name: Option<String>,
    /// The binding identity this Message belongs to — the join key to its
    /// Binding, and for GitHub PR the `owner/repo#N` the pull-request URL is
    /// built from.
    pub external_subject_key: Option<String>,
    /// The adapter's own id for this event — `github:ic:{id}`, `slack:ts:{ts}`.
    /// Distinguishes a PR comment url from a review-comment url.
    pub external_event_id: Option<String>,
    /// The handle the external system knows the writer by. `None` on a Message
    /// written before the column existed.
    pub external_author: Option<String>,
    /// The Task a Message was authored by, when it came from `moat send --task`.
    /// Present means the origin is a Task rather than an adapter.
    pub source_task_id: Option<String>,
    /// The web address of the Binding this Message's subject key resolves to,
    /// joined in. `None` when no Binding matches or it records no url.
    pub binding_url: Option<String>,
    /// What the Binding calls its subject — a Linear issue identifier such as
    /// `SP-2454`. Preferred over `external_source_name`, which for Linear is the
    /// subscription's team id rather than anything a reader would recognise.
    pub binding_label: Option<String>,
    /// The display name of the Moatless user `sent_by` resolved to, joined in.
    /// The `user` fallback where no external author was recorded.
    pub sent_by_name: Option<String>,
    /// The uploads attached to this Message, joined in from its `file` context
    /// rows as a JSON array of `{referenceId, data}` — the upload's id and the
    /// filename, mime type and size resolved when it was sent. Read from the
    /// contexts rather than from `task_messages.attachments`, which holds ids
    /// alone. `None` on a Message with no uploads.
    pub file_contexts: Option<serde_json::Value>,
}

/// A Task's transcript, and whether it is all of it.
pub struct Transcript {
    /// Oldest first — the order a transcript is read in.
    pub messages: Vec<TranscriptMessage>,
    /// What happened on the way, oldest first. Not a subset of `messages`: a
    /// turn whose agent only called tools said nothing and has no Message row
    /// at all, and its calls reach a reader only through here.
    pub activities: Vec<Activity>,
    /// The rows past `TRANSCRIPT_LIMIT` that were left out, oldest first.
    /// Zero for every transcript that fits, which is nearly all of them.
    pub omitted: i64,
    /// Of the requests these rows ask, the ones whose call has come back.
    ///
    /// A panel row is written *beside* the call rather than being it, so no
    /// result ever lands on it and its own status never moves: the call's status
    /// is the whole answer. The call is not always among these rows. It is
    /// stored once — `(task_id, tool_use_id)` is unique — while the panel row
    /// beside it carries no `tool_use_id` and so is written again every time a
    /// resuming session re-presents the call, on whichever Message it lands on.
    /// From then on the two sit on different Messages, and a read narrowed to
    /// one of them holds the panel without the call. So this is looked up by
    /// request id rather than read out of `activities`.
    pub settled_requests: HashSet<String>,
    /// What each subagent these rows spawned has spent, by its call's
    /// `tool_use_id`. Looked up rather than read out of the rows for the reason
    /// the transcript excludes them: a subagent's own Messages carry the
    /// spawning id as `parent_uuid` and are never among `activities`.
    pub subagent_usage: HashMap<String, SubagentUsage>,
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
    async fn list_followed(&self, viewer: &ScopeContext) -> Result<Vec<Conversation>, sqlx::Error>;

    /// The other half: every **closed** Task the viewer follows and may still
    /// read, newest first.
    ///
    /// Closing is what T3 calls archiving, so this is the archived listing. The
    /// follow rule and the read rule are the same two; only the status rule is
    /// inverted, which is what makes the two listings a partition of what a
    /// person follows rather than two independent queries.
    async fn list_followed_closed(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// Both halves at once, narrowed to Tasks a change has named.
    ///
    /// The follow rule and the read rule still apply, and that is the point of
    /// re-reading through this rather than projecting the change record itself:
    /// a change is a deployment-wide fact, and what a viewer may be shown of it
    /// is not. A Task the viewer does not follow or may no longer read comes
    /// back absent — and absent is what the feed above turns into the removal
    /// that takes the row off the screen.
    ///
    /// The status rule is the one that widens. A Task that was closed is still
    /// followed and still readable, so it comes back present, carrying the
    /// `archivedAt` that tells the client which half it now belongs to. Absence
    /// is reserved for a Task that left the viewer's world entirely, because the
    /// feed may only take back an id it can prove it sent: a client that
    /// resumed from a cursor is holding a listing this server never built, so
    /// there is nothing to prove it with and a removal is dropped. An upsert
    /// carries no such condition, which is what makes archiving reach a resumed
    /// sidebar.
    ///
    /// `error` stays out with the statuses neither half claims: it is not open
    /// work and it was not put away, and a row upserted under it would return to
    /// the sidebar rather than leave it.
    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// Whether this viewer may *mutate* the named Task — the `control_filter`
    /// rule: owner or admin, and never a follower or a public reader. The
    /// listing above is a read and widens to what a person follows; a write
    /// path resolves the Task through it and then gates here, because
    /// visibility never grants control.
    async fn controls_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Conversations whose name or Message text contains `query`, at most one
    /// match each, best first.
    ///
    /// **Filtered by follow, access and open status — the same three rules
    /// [`ConversationDao::list_followed`] applies, and reusing the same
    /// fragments rather than restating them.** A search that returned a Task
    /// the sidebar would not is a disclosure, and the fact that someone can
    /// guess at a word in a Task is not authorization to be told it exists.
    ///
    /// One row per Task, because the contract's result is a list of
    /// conversations and not a list of Messages. A Message match beats a name
    /// match on the same Task, and a person's Message beats the agent's — the
    /// reference server ranks the same way
    /// (`ProjectionSnapshotQuery.ts:725`).
    async fn search_conversations(
        &self,
        viewer: &ScopeContext,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ConversationMatch>, sqlx::Error>;

    /// One Task's transcript, oldest first, capped at `TRANSCRIPT_LIMIT`.
    ///
    /// Unscoped, and deliberately: every caller reaches this having already
    /// resolved the same Task through [`ConversationDao::list_followed_by_ids`],
    /// which is where the follow rule and the read rule live. Repeating them
    /// here would be a second copy of the access filter to keep in step with
    /// the first, which is the drift the narrowed listing query exists to
    /// avoid.
    async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error>;

    /// The names of the Tasks a transcript's Messages came from, by task id.
    ///
    /// Scoped, unlike [`ConversationDao::transcript`], and for the reason that
    /// one is not: these are *other* Tasks, which the caller has resolved
    /// nothing about. A Task this viewer cannot read yields no entry, and the
    /// origin built from it then names no Task and links nowhere — the same
    /// answer as one that has been deleted.
    async fn source_task_names(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<HashMap<String, String>, sqlx::Error>;

    /// The interactive call an answer names, by the id it was made under.
    ///
    /// What an answer is depends on which tool asked — an `AskUserQuestion`
    /// takes answers per question, an `ExitPlanMode` takes a yes or a no — so
    /// the command reads the name off the stored call rather than assuming one.
    /// `None` where no call by that id was ever stored.
    ///
    /// Unscoped for the reason [`ConversationDao::transcript`] is.
    async fn interactive_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<InteractiveCall>, sqlx::Error>;

    /// The highest position this conversation's change log has reached.
    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error>;

    /// This conversation's changes at or after `from`, oldest first, capped at
    /// `limit`. At `from` rather than after it — see
    /// [`moatless_tasks::task_activities::changes_from`].
    async fn conversation_changes(
        &self,
        task_id: &str,
        from: i64,
        limit: i64,
    ) -> Result<Vec<ConversationChange>, sqlx::Error>;

    /// The rows a batch of changes names — the Messages, the activities under
    /// those uuids, and the activities named directly by their own id — oldest
    /// first.
    ///
    /// A change names either a Message uuid or an activity id, never both: an
    /// activity that happened on a Message rides that Message's uuid, and one
    /// that happened on none — a Sandbox lifecycle change, a context-window
    /// reading — is named by its own id. Both sets of activities land in the same
    /// [`Transcript::activities`].
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
/// contract, and the archived listing is the same read at the other status. One
/// of either may run per backend replica; live change re-reads are narrowed by
/// task id and do not take this permit. This reserves the rest of the main pool
/// for task control, ingress, and reconciliation under shell overload.
#[derive(Clone)]
pub struct PgConversationDao {
    pool: UiRpcDbPool,
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

/// The columns a transcript read projects, over the capped set aliased `m` and
/// the two tables joined onto it. Kept in one place because the snapshot read
/// and the live-delta read must select exactly the same shape into
/// [`TranscriptMessage`] — a column on one and not the other is a field that is
/// present on reload and absent on the frame that delivered it.
const TRANSCRIPT_COLUMNS: &str = r#"m.id, m.uuid, m.message_type, m.message_role,
                      m.message_text, m.user_type, m.turn_number, m."timestamp",
                      m.created_at, m.external_adapter, m.external_source_name,
                      m.external_subject_key, m.external_event_id,
                      m.external_author, m.source_task_id,
                      COALESCE(b.destination_config ->> 'issue_url',
                               b.destination_config ->> 'url') AS binding_url,
                      COALESCE(b.destination_config ->> 'identifier',
                               b.destination_config ->> 'issue_identifier')
                          AS binding_label,
                      COALESCE(u.name, u.login) AS sent_by_name,
                      files.file_contexts"#;

/// The two joins that carry a Message's origin out of the database: what its
/// Binding says the subject is called and where it lives, and the display name
/// of the Moatless user `sent_by` resolved to.
///
/// Both destination-config reads take the two keys each value has been written
/// under, the same pair and the same precedence as
/// `adapters::providers::linear::status::binding_label_and_url`: a Binding made
/// from an inbound issue or agent session writes `issue_url`, and one made by
/// posting out to Linear writes `url`. An adapter that stores neither — Slack
/// and Telegram keep channel coordinates, not links — yields no url, and the
/// origin carries a label without one.
///
/// The message carries a subject *key* but no subject *kind*, and the unique
/// binding index is on `(task_id, adapter_kind, subject_kind, subject_key)`, so
/// the key is not unique on its own. The lateral `ORDER BY created_at DESC LIMIT
/// 1` makes the result deterministic — the most recent binding on that subject —
/// rather than whichever row the planner returns first.
///
/// `sent_by` is a polymorphic text column: a user uuid's text form, but also
/// sentinels like `system` and the Linear adapters' raw actor ids. `users.id` is
/// a native `uuid`, so the comparison is made on its text form — casting
/// `sent_by` the other way would raise on every value that is not a uuid. Those
/// yield no name rather than a wrong one.
const TRANSCRIPT_ORIGIN_JOINS: &str = r#"LEFT JOIN LATERAL (
                          SELECT tb.destination_config
                          FROM task_bindings tb
                          WHERE m.external_subject_key IS NOT NULL
                            AND tb.task_id = m.task_id
                            AND tb.adapter_kind = m.external_adapter
                            AND tb.subject_key = m.external_subject_key
                          ORDER BY tb.created_at DESC
                          LIMIT 1
                      ) b ON true
                      LEFT JOIN users u ON u.id::text = m.sent_by
                      LEFT JOIN LATERAL (
                          SELECT jsonb_agg(
                                     jsonb_build_object(
                                         'referenceId', c.reference_id,
                                         'data', c.data
                                     )
                                     ORDER BY c.id
                                 ) AS file_contexts
                          FROM task_message_contexts c
                          WHERE c.task_message_id = m.id
                            AND c.type = 'file'
                      ) files ON true"#;

/// The statuses a conversation can be listed under: the sidebar and the archive
/// together. A Task at any other status is in neither, and comes back absent
/// from a re-read the same way one the viewer no longer follows does.
const LISTED_STATUSES: &[TaskStatus] = &[TaskStatus::Open, TaskStatus::Closed];

/// A search term as a `LIKE` operand rather than a `LIKE` pattern.
///
/// `%` and `_` are ordinary characters to the person typing them — someone
/// searching for `100%` means a percent sign, not "anything at all" — so both
/// are escaped, along with the escape character itself. The escape character is
/// `\` and every query using this declares it as such, because Postgres's
/// default `ESCAPE` depends on `standard_conforming_strings`.
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
    pub fn new(pool: UiRpcDbPool) -> Self {
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

    /// The one listing query, at a set of statuses, optionally narrowed to a set
    /// of Tasks.
    ///
    /// Narrowing is a `WHERE` clause on the same query rather than a second one
    /// because the filters underneath it — the read rule and the follow rule —
    /// are the part that must not drift. A live re-read that admitted a row the
    /// snapshot would not have is the same disclosure bug in slow motion. The
    /// archived listing is the same query at `closed` for the same reason: it
    /// shows a person the conversations they follow, and only those.
    ///
    /// `statuses` is a set because the two listings are one partition and the
    /// re-read behind the feed spans both of them.
    async fn list(
        &self,
        viewer: &ScopeContext,
        statuses: &[TaskStatus],
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
        let statuses = statuses
            .iter()
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let access = viewer.access_filter(ScopedEntity::Task, "t.", 1);
        let follower = access.next_param();
        let question_tool = follower + 1;
        let plan_tool = follower + 2;
        let narrowed = task_ids
            .map(|_| format!("\n                 AND t.task_id = ANY(${})", plan_tool + 1))
            .unwrap_or_default();
        let sql = format!(
            r#"WITH listed AS MATERIALIZED (
                   SELECT t.*, tf.settled_override, tf.settled_at, tf.snoozed_until, tf.snoozed_at
                   FROM task_follows tf
                   JOIN tasks t ON t.task_id = tf.task_id
                   WHERE tf.user_id = ${follower}
                     AND t.status IN ({statuses})
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
                   t.closed_at,
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
                   COALESCE(pending.plan, false)     AS pending_plan,
                   t.settled_override,
                   t.settled_at,
                   t.snoozed_until,
                   t.snoozed_at
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
impl T3WorkspaceDao for PgConversationDao {
    async fn list_readable_including(
        &self,
        viewer: &ScopeContext,
        also_include: &[String],
    ) -> Result<Vec<UiRpcListedWorkspace>, sqlx::Error> {
        let access = viewer.access_filter(ScopedEntity::Resource, "", 1);
        let ids_param = access.next_param();
        let sql = format!(
            "SELECT *, {} AS readable FROM workspaces \
             WHERE (deleted IS NULL OR deleted = false) \
             AND ({} OR id = ANY(${ids_param})) \
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
        let access = viewer.access_filter(ScopedEntity::Resource, "w.", 2);
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
        let access = viewer.access_filter(ScopedEntity::Task, "t.", 2);
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
        let access = viewer.access_filter(ScopedEntity::Task, "", 2);
        let sql = format!(
            "SELECT status FROM tasks WHERE task_id = $1 AND {}",
            access.sql()
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
        self.list(viewer, &[TaskStatus::Open], None).await
    }

    async fn list_followed_closed(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.list(viewer, &[TaskStatus::Closed], None).await
    }

    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.list(viewer, LISTED_STATUSES, Some(task_ids)).await
    }

    async fn controls_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let control = viewer.control_filter("", 2);
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
        let access = viewer.access_filter(ScopedEntity::Task, "t.", 1);
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
        Ok(activities::find_tool_call(&self.pool, task_id, tool_use_id)
            .await?
            .map(|activity| InteractiveCall {
                settled: activity.is_settled(),
                tool_name: activity.summary.clone(),
            }))
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
        // The cap is applied in the inner subquery, before the origin joins, so
        // the index-driven bounded scan still decides which rows are read and
        // the joins run over at most `TRANSCRIPT_LIMIT` of them rather than the
        // whole conversation. The outer `ORDER BY` re-establishes the order a
        // subquery does not promise to keep; `reverse` then reads it forward.
        let sql = format!(
            r#"SELECT {TRANSCRIPT_COLUMNS}
               FROM (
                   SELECT *
                   FROM task_messages
                   WHERE task_id = $1
                     AND parent_uuid IS NULL
                   ORDER BY "timestamp" DESC, id DESC
                   LIMIT $2
               ) m
               {TRANSCRIPT_ORIGIN_JOINS}
               ORDER BY m."timestamp" DESC, m.id DESC"#,
        );
        let mut messages = sqlx::query_as::<_, TranscriptMessage>(AssertSqlSafe(sql))
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

        let settled_requests =
            activity_dao::settled_requests(&self.pool, task_id, &activities).await?;
        let subagent_usage = activity_dao::subagent_usage(&self.pool, task_id, &activities).await?;

        Ok(Transcript {
            messages,
            activities,
            omitted,
            settled_requests,
            subagent_usage,
        })
    }

    async fn source_task_names(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<HashMap<String, String>, sqlx::Error> {
        // Scoped, so a Task outside the viewer's reach yields no row and the
        // origin built from it carries neither a name nor a link. `$1` is the
        // id set; the access filter binds `is_admin` then `user_id` from `$2`.
        let access = viewer.access_filter(ScopedEntity::Task, "", 2);
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
        activities::current_head(&self.pool, task_id).await
    }

    async fn conversation_changes(
        &self,
        task_id: &str,
        from: i64,
        limit: i64,
    ) -> Result<Vec<ConversationChange>, sqlx::Error> {
        activities::changes_from(&self.pool, task_id, from, limit).await
    }

    async fn changed_messages(
        &self,
        task_id: &str,
        uuids: &[String],
        activity_ids: &[String],
    ) -> Result<Transcript, sqlx::Error> {
        if uuids.is_empty() && activity_ids.is_empty() {
            return Ok(Transcript {
                messages: Vec::new(),
                activities: Vec::new(),
                omitted: 0,
                settled_requests: HashSet::new(),
                subagent_usage: HashMap::new(),
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
        let sql = format!(
            r#"SELECT {TRANSCRIPT_COLUMNS}
               FROM (
                   SELECT *
                   FROM task_messages
                   WHERE task_id = $1
                     AND uuid = ANY($2)
                     AND parent_uuid IS NULL
               ) m
               {TRANSCRIPT_ORIGIN_JOINS}
               ORDER BY m.id"#,
        );
        let messages = sqlx::query_as::<_, TranscriptMessage>(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(uuids)
            .fetch_all(&self.pool)
            .await?;

        // By the named uuids, not by the Messages just read: a change can name
        // a uuid that has no Message row at all — a turn that only called
        // tools — and those activities are the whole of what it changed. The
        // activities named directly by their own id — the ones on no Message —
        // join the same set by a primary-key lookup.
        let mut activities = activities::for_messages(&self.pool, task_id, uuids).await?;
        activities.extend(activities::for_activity_ids(&self.pool, task_id, activity_ids).await?);
        let settled_requests =
            activity_dao::settled_requests(&self.pool, task_id, &activities).await?;
        let subagent_usage = activity_dao::subagent_usage(&self.pool, task_id, &activities).await?;

        Ok(Transcript {
            messages,
            activities,
            omitted: 0,
            settled_requests,
            subagent_usage,
        })
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
