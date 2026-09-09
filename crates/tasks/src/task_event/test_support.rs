// This module compiles only for test builds (`#[cfg(test)]`) or downstream test
// tiers (`feature = "test-support"`), never a shipping build, so the fixture's
// `.expect()` calls are test scaffolding — allow them the way the crate's own
// `#[cfg(test)]` code is allowed (the crate-level allow is `test`-gated and does
// not cover the feature-gated build).
#![allow(clippy::expect_used)]

//! A canned event log a host's handler tests read [`TaskEvents`] from.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::dao::TaskEventDao;
use super::models::{InsertTaskEvent, TaskEvent};
use super::operations::TaskEvents;

/// One [`TaskEvents::list_recent_events`] call, as the handler made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentEventsQuery {
    pub task_id: String,
    pub event_types: Option<Vec<String>>,
    pub limit: i64,
    pub offset: i64,
}

/// A fixed event log, answered the way the DAO answers it: the newest `limit`
/// rows, oldest-first. Every query is recorded, so a caller that reads from the
/// start of the log instead is visible in [`RecentEventLog::queries`].
pub struct RecentEventLog {
    log: Vec<TaskEvent>,
    queries: Mutex<Vec<RecentEventsQuery>>,
}

impl RecentEventLog {
    /// `log` is oldest-first, as a task's event history is read.
    pub fn new(log: Vec<TaskEvent>) -> Arc<Self> {
        Arc::new(Self {
            log,
            queries: Mutex::new(Vec::new()),
        })
    }

    /// The operations handle a handler takes, over this log.
    pub fn operations(self: &Arc<Self>) -> Arc<TaskEvents> {
        Arc::new(TaskEvents::new(self.clone()))
    }

    /// Every query made so far, in order.
    pub fn queries(&self) -> Vec<RecentEventsQuery> {
        self.queries
            .lock()
            .expect("recorded queries poisoned")
            .clone()
    }
}

#[async_trait]
impl TaskEventDao for RecentEventLog {
    async fn insert_event(&self, _event: &InsertTaskEvent) -> Result<(), sqlx::Error> {
        unimplemented!("the log is fixed")
    }

    async fn list_recent_events(
        &self,
        task_id: &str,
        event_types: &Option<Vec<String>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TaskEvent>, sqlx::Error> {
        self.queries
            .lock()
            .expect("recorded queries poisoned")
            .push(RecentEventsQuery {
                task_id: task_id.to_string(),
                event_types: event_types.clone(),
                limit,
                offset,
            });
        let newest_first = self.log.iter().rev().skip(offset.max(0) as usize);
        let mut page: Vec<TaskEvent> = newest_first.take(limit.max(0) as usize).cloned().collect();
        page.reverse();
        Ok(page)
    }

    async fn agent_run_already_terminal(
        &self,
        _task_id: &str,
        _event_id: &str,
    ) -> Result<bool, sqlx::Error> {
        unimplemented!("the log is fixed")
    }

    async fn get_latest_agent_lifecycle(
        &self,
        _task_ids: &[String],
    ) -> Result<HashMap<String, (String, DateTime<Utc>)>, sqlx::Error> {
        unimplemented!("the log is fixed")
    }

    async fn get_latest_agent_activity(
        &self,
        _task_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
        unimplemented!("the log is fixed")
    }
}
