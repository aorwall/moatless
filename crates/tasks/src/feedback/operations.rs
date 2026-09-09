//! The `feedback` table, as a host and the feedback routes reach it.

use std::sync::Arc;

use super::dao::FeedbackDao;
use super::models::{Feedback, FeedbackFilter, FeedbackWithTask, LatestActivity};

/// The `feedback` table, as the backend and this crate's own handlers reach it.
///
/// A host holds this to route; the validation and the activity selection stay
/// in [`crate::feedback::service`].
pub struct FeedbackNotes {
    store: Arc<dyn FeedbackDao>,
}

impl FeedbackNotes {
    pub(crate) fn new(store: Arc<dyn FeedbackDao>) -> Self {
        Self { store }
    }

    /// See [`FeedbackDao::insert`].
    pub(crate) async fn insert(&self, row: &Feedback) -> Result<(), sqlx::Error> {
        self.store.insert(row).await
    }

    /// See [`FeedbackDao::find_by_id`].
    pub(crate) async fn find_by_id(
        &self,
        feedback_id: &str,
    ) -> Result<Option<Feedback>, sqlx::Error> {
        self.store.find_by_id(feedback_id).await
    }

    /// See [`FeedbackDao::list_for_task`].
    pub(crate) async fn list_for_task(&self, task_id: &str) -> Result<Vec<Feedback>, sqlx::Error> {
        self.store.list_for_task(task_id).await
    }

    /// See [`FeedbackDao::update_status`].
    pub(crate) async fn update_status(
        &self,
        feedback_id: &str,
        status: &str,
        note: Option<String>,
        updated_by: &str,
    ) -> Result<Option<Feedback>, sqlx::Error> {
        self.store
            .update_status(feedback_id, status, note, updated_by)
            .await
    }

    /// See [`FeedbackDao::list`].
    pub(crate) async fn list(
        &self,
        filter: &FeedbackFilter,
    ) -> Result<(Vec<FeedbackWithTask>, i64), sqlx::Error> {
        self.store.list(filter).await
    }

    /// See [`FeedbackDao::latest_activity`].
    pub(crate) async fn latest_activity(
        &self,
        task_id: &str,
    ) -> Result<Option<LatestActivity>, sqlx::Error> {
        self.store.latest_activity(task_id).await
    }
}
