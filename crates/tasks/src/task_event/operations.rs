//! The `task_events` table, as the backend's consumer, reaper and SSE handlers
//! reach it.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::dao::TaskEventDao;
use super::models::{InsertTaskEvent, TaskEvent};

/// The `task_events` table, as a host reaches it.
pub struct TaskEvents {
    store: Arc<dyn TaskEventDao>,
}

impl TaskEvents {
    pub(crate) fn new(store: Arc<dyn TaskEventDao>) -> Self {
        Self { store }
    }

    /// See [`TaskEventDao::insert_event`].
    pub async fn insert_event(&self, event: &InsertTaskEvent) -> Result<(), sqlx::Error> {
        self.store.insert_event(event).await
    }

    /// See [`TaskEventDao::list_recent_events`].
    pub async fn list_recent_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error> {
        self.store
            .list_recent_events(task_id, event_types, limit, offset)
            .await
    }

    /// See [`TaskEventDao::agent_run_already_terminal`].
    pub async fn agent_run_already_terminal(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .agent_run_already_terminal(task_id, event_id)
            .await
    }

    /// See [`TaskEventDao::get_latest_agent_lifecycle`].
    pub async fn get_latest_agent_lifecycle(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, (String, DateTime<Utc>)>, sqlx::Error> {
        self.store.get_latest_agent_lifecycle(task_ids).await
    }

    /// See [`TaskEventDao::get_latest_agent_activity`].
    pub async fn get_latest_agent_activity(
        &self,
        task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        self.store.get_latest_agent_activity(task_ids).await
    }
}
