//! The inbox's reads and writes, behind the one handle a send path holds.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use moatless_core::db::DbPool;

use super::dao;
use super::models::{DeliveryOptions, InboxItem, NewInboxItem, RunStamp};

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

    /// See [`dao::cancel_queued`]. Records the change on the conversation and
    /// the listing, which read the queue, in the same transaction.
    pub(crate) async fn cancel_queued(&self, task_id: &str, id: Uuid) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        if !dao::cancel_queued(&mut tx, task_id, id).await? {
            return Ok(false);
        }
        record_queue_change(&mut tx, task_id, &[id]).await?;
        moatless_core::change_feed::listing::record_change(&mut tx, task_id).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// See [`dao::edit_queued`].
    pub(crate) async fn edit_queued(
        &self,
        task_id: &str,
        id: Uuid,
        text: &str,
        file_ids: Option<&[String]>,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        if !dao::edit_queued(&mut tx, task_id, id, text, file_ids).await? {
            return Ok(false);
        }
        record_queue_change(&mut tx, task_id, &[id]).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Rewrite the newest pending item `sender_id` sent through `adapter` about
    /// `subject_key` with what `rewrite` makes of it. `false` when there is
    /// none, or `rewrite` declines it, so the caller submits a new item instead.
    pub(crate) async fn rewrite_pending_from_subject(
        &self,
        task_id: &str,
        adapter: &str,
        subject_key: &str,
        sender_id: &str,
        rewrite: impl FnOnce(&InboxItem) -> Option<(String, DeliveryOptions)>,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let Some(item) =
            dao::lock_pending_from_subject(&mut tx, task_id, adapter, subject_key, sender_id)
                .await?
        else {
            return Ok(false);
        };
        let Some((text, options)) = rewrite(&item) else {
            return Ok(false);
        };
        if !dao::rewrite_pending(&mut tx, item.id, &text, &options).await? {
            return Ok(false);
        }
        record_queue_change(&mut tx, task_id, &[item.id]).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// See [`dao::reorder_queued`].
    pub(crate) async fn reorder_queued(
        &self,
        task_id: &str,
        id: Uuid,
        before: Option<Uuid>,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let moved = dao::reorder_queued(&mut tx, task_id, id, before).await?;
        if moved.is_empty() {
            return Ok(false);
        }
        record_queue_change(&mut tx, task_id, &moved).await?;
        tx.commit().await?;
        Ok(true)
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

/// A queued item's Message shares the item's id, so naming it is what makes a
/// conversation reader re-read the item's text and place.
async fn record_queue_change(
    tx: &mut Transaction<'_, Postgres>,
    task_id: &str,
    ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    for id in ids {
        moatless_core::change_feed::conversation::record_message_change(
            tx,
            task_id,
            &id.to_string(),
        )
        .await?;
    }
    Ok(())
}
