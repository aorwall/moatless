//! The inbox's reads and writes, behind the one handle a send path holds.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use moatless_core::db::DbPool;

use super::dao;
use super::models::{InboxItem, NewInboxItem, RunStamp};

/// How many pending items one fetch reads; the rest wait for the next.
pub(crate) const PENDING_LIMIT: u32 = 100;

/// The inbox's reads and writes. A send path submits in the transaction that
/// also runs the Projector, so an item and its Message commit together.
#[derive(Clone)]
pub(crate) struct InboxService(DbPool);

impl InboxService {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self(pool)
    }

    pub(crate) async fn begin(&self) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
        self.0.begin().await
    }

    /// `None` when an item with this id exists: a redelivery, which must not wake again.
    pub(crate) async fn submit(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: NewInboxItem,
    ) -> Result<Option<InboxItem>, sqlx::Error> {
        dao::submit(tx, &item).await
    }

    /// See [`dao::pending`].
    pub(crate) async fn pending(
        &self,
        task_id: &str,
        limit: u32,
        hold_queued: bool,
    ) -> Result<Vec<InboxItem>, sqlx::Error> {
        dao::pending(&self.0, task_id, limit, hold_queued).await
    }

    pub(crate) async fn delivered_in_turn(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<Vec<InboxItem>, sqlx::Error> {
        dao::delivered_in_turn(&self.0, task_id, turn_number).await
    }

    /// See [`dao::steer`].
    pub(crate) async fn steer(&self, task_id: &str, id: Uuid) -> Result<bool, sqlx::Error> {
        dao::steer(&self.0, task_id, id).await
    }

    /// See [`dao::dispose_pending`].
    pub(crate) async fn dispose_pending(&self, task_ids: &[String]) -> Result<u64, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let disposed = dao::dispose_pending(&mut tx, task_ids).await?;
        tx.commit().await?;
        Ok(disposed)
    }

    pub(crate) async fn find(&self, id: Uuid) -> Result<Option<InboxItem>, sqlx::Error> {
        dao::find(&self.0, id).await
    }

    /// Marks the `pending` ones among `ids`; already-delivered ids are skipped.
    /// Decides the turn's trigger when this is its first delivery.
    pub(crate) async fn mark_delivered(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_id: &str,
        ids: &[Uuid],
        run: &RunStamp,
    ) -> Result<u64, sqlx::Error> {
        dao::mark_delivered(tx, task_id, ids, run).await
    }
}
