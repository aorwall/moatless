//! The `task_turns` table, as the backend's ingress, reaper and event handlers
//! reach it.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::dao::TaskTurnDao;
use super::models::{ClosedTurn, SettledTurn, TurnTransition, UnclosedTurn};

/// The `task_turns` table, as a host reaches it.
///
/// The published seams are narrower: [`crate::ports::TurnLedger`] answers which
/// Turn a row appended now belongs to and whether one is in flight.
pub struct TaskTurns {
    store: Arc<dyn TaskTurnDao>,
}

impl TaskTurns {
    pub(crate) fn new(store: Arc<dyn TaskTurnDao>) -> Self {
        Self { store }
    }

    /// The store, for this crate's own services and mutations.
    pub(crate) fn store(&self) -> &dyn TaskTurnDao {
        self.store.as_ref()
    }

    /// See [`TaskTurnDao::project_turn`].
    pub async fn project_turn(
        &self,
        task_id: &str,
        turn_number: i32,
        agent_profile: Option<String>,
        transition: TurnTransition,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        self.store
            .project_turn(task_id, turn_number, agent_profile, transition, at)
            .await
    }

    /// See [`TaskTurnDao::list_unclosed_active_turns`].
    pub async fn list_unclosed_active_turns(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
        self.store.list_unclosed_active_turns(older_than).await
    }

    /// See [`TaskTurnDao::list_settled_turns_with_later_activity`].
    pub async fn list_settled_turns_with_later_activity(
        &self,
    ) -> Result<Vec<SettledTurn>, sqlx::Error> {
        self.store.list_settled_turns_with_later_activity().await
    }

    /// See [`TaskTurnDao::list_superseded_open_turns`].
    pub async fn list_superseded_open_turns(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<UnclosedTurn>, sqlx::Error> {
        self.store.list_superseded_open_turns(older_than).await
    }

    /// See [`TaskTurnDao::resolve_turn_for_run`].
    pub async fn resolve_turn_for_run(
        &self,
        task_id: &str,
        run_id: &str,
    ) -> Result<i32, sqlx::Error> {
        self.store.resolve_turn_for_run(task_id, run_id).await
    }

    /// See [`TaskTurnDao::latest_turn_number`].
    pub async fn latest_turn_number(&self, task_id: &str) -> Result<Option<i32>, sqlx::Error> {
        self.store.latest_turn_number(task_id).await
    }

    /// See [`TaskTurnDao::triggered_by_task_id`].
    pub async fn triggered_by_task_id(
        &self,
        task_id: &str,
        turn_number: i32,
    ) -> Result<Option<String>, sqlx::Error> {
        self.store.triggered_by_task_id(task_id, turn_number).await
    }

    /// See [`TaskTurnDao::latest_closed_turn`].
    pub async fn latest_closed_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<ClosedTurn>, sqlx::Error> {
        self.store.latest_closed_turn(task_id).await
    }

    /// See [`TaskTurnDao::list_turn_metrics`].
    pub(crate) async fn list_turn_metrics(
        &self,
        task_id: &str,
    ) -> Result<Vec<serde_json::Value>, sqlx::Error> {
        self.store.list_turn_metrics(task_id).await
    }

    /// See [`TaskTurnDao::set_turn_metrics`].
    pub async fn set_turn_metrics(
        &self,
        task_id: &str,
        turn_number: i32,
        metrics: &serde_json::Value,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .set_turn_metrics(task_id, turn_number, metrics)
            .await
    }

    /// See [`TaskTurnDao::set_agent_turn_id`].
    pub async fn set_agent_turn_id(
        &self,
        task_id: &str,
        turn_number: i32,
        agent_turn_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.store
            .set_agent_turn_id(task_id, turn_number, agent_turn_id)
            .await
    }

    /// See [`TaskTurnDao::mark_stop_unresponsive`].
    pub async fn mark_stop_unresponsive(
        &self,
        task_id: &str,
        at: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        self.store.mark_stop_unresponsive(task_id, at).await
    }
}
