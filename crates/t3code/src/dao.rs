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

use async_trait::async_trait;

use crate::UiRpcDbPool;
use crate::transcript::{JoinedOrigin, Transcript};
use moatless_core::change_feed::conversation::{ConversationChange, ConversationHead};
use moatless_core::change_feed::listing::{ListingChange, ListingHead};
use moatless_ports::activity::ActivitySource;
use moatless_ports::task::TaskReader;
use moatless_tasks::conversation::transcript::{TranscriptEntry, TranscriptReads};
use moatless_types::access_control::ScopeContext;

pub use moatless_tasks::search::TaskMatch as ConversationMatch;
use moatless_tasks::search::TaskSearch;
pub use moatless_tasks::task_listing::{
    ARCHIVED_LIMIT, BROWSE_LIMIT, ListedPullRequest as ConversationPullRequest,
    ListedTask as Conversation, ListingCursor as ArchiveCursor, ListingError,
    ListingFilter as ConversationBrowse,
};
use moatless_tasks::task_listing::{LISTED_STATUSES, TaskListing};

/// The stored tool call an answer names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveCall {
    /// The tool that asked, as the call recorded it.
    pub tool_name: String,
    /// Whether it has already come back. An answer to a settled call changes
    /// nothing, and saying so is more honest than reporting a send failure.
    pub settled: bool,
    /// The text of each question an `AskUserQuestion` asked, in order.
    pub questions: Vec<String>,
}

/// Read the conversations one viewer's sidebar is built from, and the position
/// a client watching it resumes from.
///
/// A trait because the module's transport tests stub it; the only production
/// implementation is [`PgConversationDao`].
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ConversationDao: Send + Sync {
    /// The **open** Tasks the viewer follows **and** may still read, newest
    /// first: at most [`moatless_tasks::task_listing::FOLLOWED_LIMIT`].
    ///
    /// `error` counts as not-open too, matching the Moatless sidebar's own
    /// filter; a Task returns to `open` the instant its agent produces
    /// anything, so nothing running is hidden.
    async fn list_followed(&self, viewer: &ScopeContext)
    -> Result<Vec<Conversation>, ListingError>;

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
    ) -> Result<Vec<Conversation>, ListingError>;

    /// Every Task the viewer may **read** that `filter` names, newest first,
    /// whether or not they follow it.
    ///
    /// At most [`BROWSE_LIMIT`] rows plus one; caller drops the extra. Read
    /// rule only, same as [`Self::readable_by_id`]; status moves into
    /// [`ConversationBrowse::include_closed`].
    async fn list_browsable(
        &self,
        viewer: &ScopeContext,
        filter: &ConversationBrowse,
    ) -> Result<Vec<Conversation>, ListingError>;

    /// Both halves at once, narrowed to Tasks a change has named.
    ///
    /// Unfollowed or unreadable comes back absent, which the feed turns into a
    /// removal. A closed-but-followed-and-readable Task comes back present
    /// with `archivedAt` instead, since a resumed cursor can only upsert,
    /// never retract. `error` is excluded from both.
    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error>;

    /// One conversation this viewer may **read**, whether or not they follow
    /// it.
    ///
    /// What a link resolves through: a readable Task opens even when
    /// unfollowed; writing to it is what acquires the Follow.
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

    /// Whether [`Self::readable_by_id`] would return the named Task, read
    /// from `tasks` alone: the same read and status rules, none of the row.
    async fn can_read_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Whether this viewer may *operate* the named Task — the `control_filter`
    /// rule: the write rule, widened by `visibility = 'public'`.
    ///
    /// Asked directly rather than after a Follow, since anyone can acquire a
    /// Follow on a public Task by posting one message.
    async fn controls_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error>;

    /// Conversations whose name or Message text contains `query`, at most one
    /// match each, best first.
    ///
    /// Every open Task the viewer may read, followed or not. A Message match
    /// beats a name match, person beats agent, per the
    /// reference server's ranking (`ProjectionSnapshotQuery.ts:725`).
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
    /// An answer's shape depends on which tool asked, so the command reads
    /// the kind off the stored call. Unscoped for the reason
    /// [`ConversationDao::transcript`] is.
    async fn interactive_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<InteractiveCall>, sqlx::Error>;

    /// The highest position this conversation's change log has reached.
    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error>;

    /// The head of each of `task_ids` in one read, in no particular order; a
    /// conversation with no change yet is at position 0.
    async fn conversation_heads(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<(String, ConversationHead)>, sqlx::Error>;

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

    /// The oldest `limit` settled turns numbered above `above` (from the first
    /// for `None`), with their Messages. Unscoped for the reason
    /// [`ConversationDao::transcript`] is.
    async fn settled_turns(
        &self,
        task_id: &str,
        above: Option<i32>,
        limit: i64,
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

/// [`ConversationDao`] over Postgres; the listing reads go to [`TaskListing`].
#[derive(Clone)]
pub struct PgConversationDao {
    pool: UiRpcDbPool,
    /// The Task band's activity rows. This crate projects them; it does not own
    /// the table, so it reads them through the published seam.
    activities: Arc<dyn ActivitySource>,
    transcripts: Arc<TranscriptReads>,
    listing: TaskListing,
    search: TaskSearch,
    /// The read, control and write rules over `tasks`, which moatless-tasks
    /// owns.
    tasks: Arc<dyn TaskReader>,
}

/// A published-seam read that failed, in the `sqlx::Error` the T3 DAO trait
/// returns. The whole chain is kept as the protocol text so the response
/// boundary logs what actually failed.
pub(crate) fn store_read(error: moatless_types::store::StoreError) -> sqlx::Error {
    sqlx::Error::Protocol(moatless_core::error::causal_chain(&error))
}

impl PgConversationDao {
    pub fn new(
        pool: UiRpcDbPool,
        activities: Arc<dyn ActivitySource>,
        transcripts: Arc<TranscriptReads>,
        tasks: Arc<dyn TaskReader>,
    ) -> Self {
        Self {
            listing: TaskListing::new(pool.clone()),
            search: TaskSearch::new(pool.clone()),
            pool,
            activities,
            transcripts,
            tasks,
        }
    }
}

#[async_trait]
impl ConversationDao for PgConversationDao {
    async fn list_followed(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<Conversation>, ListingError> {
        self.listing.followed(viewer).await
    }

    async fn list_followed_closed(
        &self,
        viewer: &ScopeContext,
        before: Option<ArchiveCursor>,
    ) -> Result<Vec<Conversation>, ListingError> {
        self.listing.followed_closed(viewer, before).await
    }

    async fn list_browsable(
        &self,
        viewer: &ScopeContext,
        filter: &ConversationBrowse,
    ) -> Result<Vec<Conversation>, ListingError> {
        self.listing.browsable(viewer, filter).await
    }

    async fn list_followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.listing.followed_by_ids(viewer, task_ids).await
    }

    async fn readable_by_id(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Conversation>, sqlx::Error> {
        self.listing.readable_by_id(viewer, task_id).await
    }

    async fn readable_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.listing.readable_by_ids(viewer, task_ids).await
    }

    async fn can_read_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let status = self
            .tasks
            .readable_status(task_id, viewer)
            .await
            .map_err(store_read)?;
        Ok(status.is_some_and(|status| LISTED_STATUSES.contains(&status)))
    }

    async fn controls_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let task = self
            .tasks
            .controlled(task_id, viewer)
            .await
            .map_err(store_read)?;
        Ok(task.is_some())
    }

    async fn search_conversations(
        &self,
        viewer: &ScopeContext,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ConversationMatch>, sqlx::Error> {
        self.search.search(viewer, query, limit).await
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
                questions: activity
                    .tool_input()
                    .get("questions")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|q| q.get("question").and_then(serde_json::Value::as_str))
                    .map(str::to_string)
                    .collect(),
            }))
    }

    async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error> {
        let window = self.transcripts.read(task_id).await?;
        crate::transcript::resolve(&self.pool, &*self.activities, task_id, window).await
    }

    async fn source_task_names(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<HashMap<String, String>, sqlx::Error> {
        self.listing.readable_names(viewer, task_ids).await
    }

    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
        moatless_core::change_feed::conversation::current_head(&self.pool, task_id).await
    }

    async fn conversation_heads(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<(String, ConversationHead)>, sqlx::Error> {
        moatless_core::change_feed::conversation::current_heads(&self.pool, task_ids).await
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
        let window = self
            .transcripts
            .changed(task_id, uuids, activity_ids)
            .await?;
        crate::transcript::resolve(&self.pool, &*self.activities, task_id, window).await
    }

    async fn settled_turns(
        &self,
        task_id: &str,
        above: Option<i32>,
        limit: i64,
    ) -> Result<Transcript, sqlx::Error> {
        let window = self
            .transcripts
            .settled_turns(task_id, above, limit)
            .await?;
        crate::transcript::resolve(&self.pool, &*self.activities, task_id, window).await
    }

    async fn listing_head(&self) -> Result<ListingHead, sqlx::Error> {
        moatless_core::change_feed::listing::current_head(&self.pool).await
    }

    async fn listing_changes_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<ListingChange>, sqlx::Error> {
        moatless_core::change_feed::listing::changes_after(&self.pool, after, limit).await
    }
}

/// The Binding, Loop and user each of one Task's `entries` names, by Message
/// uuid. An entry naming none of them has no row.
///
/// Subject key alone isn't unique, so `ORDER BY created_at DESC LIMIT 1`
/// picks deterministically. `users.id` casts to text, not `sent_by` to
/// `uuid`, which would raise on a non-uuid value.
pub(crate) async fn transcript_origins(
    pool: &UiRpcDbPool,
    task_id: &str,
    entries: &[TranscriptEntry],
) -> Result<HashMap<String, JoinedOrigin>, sqlx::Error> {
    let named: Vec<&TranscriptEntry> = entries
        .iter()
        .filter(|entry| entry.external_adapter.is_some() || entry.sent_by.is_some())
        .collect();
    if named.is_empty() {
        return Ok(HashMap::new());
    }
    let column = |field: fn(&TranscriptEntry) -> Option<&String>| -> Vec<Option<String>> {
        named.iter().map(|entry| field(entry).cloned()).collect()
    };
    let uuids: Vec<String> = named.iter().map(|entry| entry.uuid.clone()).collect();
    let rows: Vec<JoinedOrigin> = sqlx::query_as(
        r#"SELECT m.uuid,
                  COALESCE(b.destination_config ->> 'issue_url',
                           b.destination_config ->> 'url') AS binding_url,
                  COALESCE(b.destination_config ->> 'identifier',
                           b.destination_config ->> 'issue_identifier')
                      AS binding_label,
                  lp.name AS loop_name,
                  COALESCE(u.name, u.login) AS sent_by_name
             FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[], $6::text[])
                  AS m(uuid, external_adapter, external_subject_key,
                       external_source_name, sent_by)
             LEFT JOIN LATERAL (
                 SELECT tb.destination_config
                 FROM task_bindings tb
                 WHERE m.external_subject_key IS NOT NULL
                   AND tb.task_id = $1
                   AND tb.adapter_kind = m.external_adapter
                   AND tb.subject_key = m.external_subject_key
                 ORDER BY tb.created_at DESC
                 LIMIT 1
             ) b ON true
             LEFT JOIN loops lp ON m.external_adapter = 'scheduled'
                               AND lp.id = m.external_source_name
                               AND NOT lp.deleted
             LEFT JOIN users u ON u.id::text = m.sent_by"#,
    )
    .bind(task_id)
    .bind(&uuids)
    .bind(column(|entry| entry.external_adapter.as_ref()))
    .bind(column(|entry| entry.external_subject_key.as_ref()))
    .bind(column(|entry| entry.external_source_name.as_ref()))
    .bind(column(|entry| entry.sent_by.as_ref()))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.uuid.clone(), row))
        .collect())
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
