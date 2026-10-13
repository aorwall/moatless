//! The Task listing: what a sidebar of followed Tasks, its archive, a browse
//! and a re-read by id each return, as one [`ListedTask`] row per Task.

use std::sync::Arc;

use moatless_core::db::DbPool;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::TaskStatus;
use tokio::sync::Semaphore;

use super::listed::{ListedTask, ListingCursor};
use super::listed_dao::{
    self, FULL_LISTING_CONCURRENCY, LISTING_DURATION, LISTING_REJECTIONS, Page, Reach,
};

/// Reads the Task listing for one viewer.
///
/// A read that is not narrowed by task id scans the deployment, so one runs
/// per replica at a time; another is refused as `sqlx::Error::PoolTimedOut`,
/// leaving pool capacity for task control, ingress and reconciliation.
#[derive(Clone)]
pub struct TaskListing {
    pool: DbPool,
    full_listing: Arc<Semaphore>,
}

impl TaskListing {
    pub fn new(pool: DbPool) -> Self {
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
            full_listing: Arc::new(Semaphore::new(FULL_LISTING_CONCURRENCY)),
        }
    }

    /// Every open Task the viewer follows and may read, newest first.
    pub async fn followed(&self, viewer: &ScopeContext) -> Result<Vec<ListedTask>, sqlx::Error> {
        listed_dao::list(
            &self.pool,
            &self.full_listing,
            viewer,
            Reach::Followed,
            &[TaskStatus::Open],
            None,
            None,
            None,
        )
        .await
    }

    /// The newest closed Tasks the viewer follows and may read, older than
    /// `before`: at most [`ARCHIVED_LIMIT`] plus one.
    pub async fn followed_closed(
        &self,
        viewer: &ScopeContext,
        before: Option<ListingCursor>,
    ) -> Result<Vec<ListedTask>, sqlx::Error> {
        listed_dao::list(
            &self.pool,
            &self.full_listing,
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

    /// Every Task the viewer may read that `filter` names, followed or not: at
    /// most [`BROWSE_LIMIT`] plus one.
    pub async fn browsable(
        &self,
        viewer: &ScopeContext,
        filter: &ListingFilter,
    ) -> Result<Vec<ListedTask>, sqlx::Error> {
        listed_dao::list(
            &self.pool,
            &self.full_listing,
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

    /// The named Tasks the viewer follows and may read, at either listed
    /// status.
    pub async fn followed_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<ListedTask>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        listed_dao::list(
            &self.pool,
            &self.full_listing,
            viewer,
            Reach::Followed,
            LISTED_STATUSES,
            Some(task_ids),
            None,
            None,
        )
        .await
    }

    /// One Task the viewer may read, followed or not.
    pub async fn readable_by_id(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<ListedTask>, sqlx::Error> {
        Ok(listed_dao::list(
            &self.pool,
            &self.full_listing,
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

    /// [`Self::readable_by_id`] over a set of Tasks, in one query and in no
    /// specified order.
    pub async fn readable_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<ListedTask>, sqlx::Error> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        listed_dao::list(
            &self.pool,
            &self.full_listing,
            viewer,
            Reach::Readable,
            LISTED_STATUSES,
            Some(task_ids),
            None,
            None,
        )
        .await
    }
}

/// What narrows a browse of conversations the viewer does **not** follow.
///
/// At least one of `owner_user_id` and `tag` is always set — enforced by the
/// only constructor, [`ListingFilter::new`] — since [`TaskListing::browsable`]
/// reads past the follow join, and unfiltered would be every Task an
/// administrator may read: the whole deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingFilter {
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

impl ListingFilter {
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

    pub(crate) fn statuses(&self) -> &'static [TaskStatus] {
        if self.include_closed {
            LISTED_STATUSES
        } else {
            &[TaskStatus::Open]
        }
    }
}

/// How many rows a browse returns: the newest, since unlike the follow-bounded
/// listings a browse is bounded only by the filter and has no natural ceiling.
///
/// The query asks for one extra row past this so `snapshots::browse_snapshot`
/// can tell a cut list from one that ends exactly on the cap, rather than
/// calling a complete 200-row list truncated.
pub const BROWSE_LIMIT: i64 = 200;

/// How many rows the archived listing returns: the newest.
///
/// Unlike the open half, which shrinks as work closes, the archive only grows, so every close
/// would otherwise add a row to every later read. Asked for plus one, as
/// [`BROWSE_LIMIT`] is.
pub const ARCHIVED_LIMIT: i64 = 200;

/// The statuses a Task can be listed under: the sidebar and the archive.
///
/// A Task at any other status is in neither, and comes back absent
/// from a re-read the same way one the viewer no longer follows does.
pub const LISTED_STATUSES: &[TaskStatus] = &[TaskStatus::Open, TaskStatus::Closed];

/// `statuses` as the operand of an `IN`, in the enum's own spelling.
pub fn status_list(statuses: &[TaskStatus]) -> String {
    statuses
        .iter()
        .map(|status| format!("'{}'", status.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}
