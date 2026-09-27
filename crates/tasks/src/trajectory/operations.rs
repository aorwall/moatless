//! The trajectory tables, as the backend's `AppendTrajectory` and
//! `ReadTrajectory` handlers reach them.

use moatless_types::agent::Harness;
use uuid::Uuid;

use super::dao::PgTrajectoryDao;
use super::lineage::{LineageRow, cut_at_turn, latest_of, transcode_items};
use super::models::{
    AppendError, AppendOutcome, AppendRequest, ChainError, Cut, CutAnchors, Head, OpenedSession,
    TrajectorySession,
};

/// The `trajectory_sessions` and `trajectory_events` tables, as a host reaches
/// them.
pub struct Trajectories {
    store: PgTrajectoryDao,
}

impl Trajectories {
    pub(crate) fn new(store: PgTrajectoryDao) -> Self {
        Self { store }
    }

    /// See [`PgTrajectoryDao::append`].
    pub async fn append(&self, req: AppendRequest<'_>) -> Result<AppendOutcome, AppendError> {
        self.store.append(req).await
    }

    /// See [`PgTrajectoryDao::find_session`].
    pub async fn find_session(
        &self,
        harness_session_id: &str,
    ) -> Result<Option<TrajectorySession>, sqlx::Error> {
        self.store.find_session(harness_session_id).await
    }

    /// See [`PgTrajectoryDao::chain`].
    pub async fn chain(&self, session: &TrajectorySession) -> Result<Vec<String>, ChainError> {
        self.store.chain(session).await
    }

    /// See [`PgTrajectoryDao::head`].
    pub async fn head(&self, task_id: &str) -> Result<Option<Head>, sqlx::Error> {
        self.store.head(task_id).await
    }

    /// See [`PgTrajectoryDao::cut_anchors`].
    pub async fn cut_anchors(&self, event_id: Uuid) -> Result<Option<CutAnchors>, sqlx::Error> {
        self.store.cut_anchors(event_id).await
    }

    /// See [`PgTrajectoryDao::event_harness`].
    pub async fn event_harness(&self, event_id: Uuid) -> Result<Option<String>, sqlx::Error> {
        self.store.event_harness(event_id).await
    }

    /// The event a fork or rewind of `task_id` after turn `at_turn` continues
    /// from, found on the lineage of the Task's head; `at_turn = None` is the
    /// head. `None` when the Task has no trajectory or no line at or before
    /// that turn.
    pub async fn resolve_cut(
        &self,
        task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<Option<Cut>, sqlx::Error> {
        let Some(head) = self.store.head(task_id).await? else {
            return Ok(None);
        };
        let Some(at_turn) = at_turn else {
            return Ok(Some(Cut {
                event_id: head.event_id,
                harness: head.harness,
            }));
        };
        let rows = self.store.lineage(head.event_id).await?;
        Ok(cut_at_turn(&rows, at_turn).and_then(|event_id| {
            let row = rows.iter().find(|r| r.id == event_id)?;
            Some(Cut {
                event_id,
                harness: row.harness.clone(),
            })
        }))
    }

    /// The session `task_id`'s next turn runs in `harness` (S4): its head's
    /// own session when the head is already `harness`, else a switch
    /// session. `None` when the Task has recorded nothing to continue.
    pub async fn open_switch_session(
        &self,
        task_id: &str,
        harness: Harness,
    ) -> Result<Option<OpenedSession>, sqlx::Error> {
        let Some(head) = self.store.head(task_id).await? else {
            return Ok(None);
        };
        if head.forked {
            return self
                .open_fork_session(task_id, harness, head.event_id)
                .await;
        }
        if head.harness == harness.as_str() {
            return Ok(self
                .store
                .find_session(&head.harness_session_id)
                .await?
                .map(|session| OpenedSession {
                    session,
                    transcode: Vec::new(),
                }));
        }
        let rows = self.store.lineage(head.event_id).await?;
        self.open_transcoded(task_id, harness, head.event_id, &rows)
            .await
    }

    /// The session a fork (or rewind) `fork_task_id` runs in `harness` from
    /// `cut`: a verbatim fork session when the cut is in a `harness` session,
    /// else a switch session transcoded from the cut (S7).
    pub async fn open_fork_session(
        &self,
        fork_task_id: &str,
        harness: Harness,
        cut: Uuid,
    ) -> Result<Option<OpenedSession>, sqlx::Error> {
        let rows = self.store.lineage(cut).await?;
        let Some(at) = rows.iter().find(|r| r.id == cut) else {
            return Ok(None);
        };
        if at.harness == harness.as_str() {
            let session = self
                .store
                .insert_fork_session(fork_task_id, harness, cut)
                .await?;
            return Ok(Some(OpenedSession {
                session,
                transcode: Vec::new(),
            }));
        }
        self.open_transcoded(fork_task_id, harness, cut, &rows)
            .await
    }

    async fn open_transcoded(
        &self,
        task_id: &str,
        harness: Harness,
        tip: Uuid,
        rows: &[LineageRow],
    ) -> Result<Option<OpenedSession>, sqlx::Error> {
        let since = latest_of(rows, harness.as_str());
        let transcode = transcode_items(rows, since, harness.as_str());
        if since.is_none() && transcode.is_empty() {
            return Ok(None);
        }
        let session = self
            .store
            .insert_switch_session(task_id, harness, since, tip)
            .await?;
        Ok(Some(OpenedSession { session, transcode }))
    }

    /// See [`PgTrajectoryDao::insert_fork_session`].
    pub async fn insert_fork_session(
        &self,
        fork_task_id: &str,
        harness: Harness,
        forked_from_event_id: Uuid,
    ) -> Result<TrajectorySession, sqlx::Error> {
        self.store
            .insert_fork_session(fork_task_id, harness, forked_from_event_id)
            .await
    }
}
