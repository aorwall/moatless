//! The trajectory tables, as the backend's `AppendTrajectory` and
//! `ReadTrajectory` handlers reach them.

use moatless_types::agent::Harness;
use uuid::Uuid;

use super::dao::PgTrajectoryDao;
use super::models::{AppendError, AppendOutcome, ChainError, ForkCut, NewEvent, TrajectorySession};

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
    pub async fn append(
        &self,
        task_id: &str,
        harness: Harness,
        harness_session_id: &str,
        first_seq: i64,
        lines: &[NewEvent],
        turn_number: Option<i32>,
    ) -> Result<AppendOutcome, AppendError> {
        self.store
            .append(
                task_id,
                harness,
                harness_session_id,
                first_seq,
                lines,
                turn_number,
            )
            .await
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

    /// See [`PgTrajectoryDao::resolve_cut`].
    pub async fn resolve_cut(
        &self,
        source_harness_session_id: &str,
        cut: &ForkCut,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        self.store.resolve_cut(source_harness_session_id, cut).await
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
