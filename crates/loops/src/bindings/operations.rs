//! The Binding lifecycle, as a caller states it.
//!
//! [`Bindings`] is the one handle on the `task_bindings` table. Every write is
//! an intent — bind a subject to a task, record what arrived, record what the
//! provider now says — and each read is named for the question it answers.
//! [`crate::compose::loop_graph`] builds it.

use std::sync::Arc;

use chrono::Utc;
use moatless_adapter_model::error::AdapterError;
use moatless_types::adapter::{AdapterConnection, AdapterKind, TaskBinding};
use moatless_types::loops::SubscriptionMatch;

use crate::bindings::dao::{LoopRecentTaskRow, PgTaskBindingDao, TaskBindingInfo};

/// The Binding lifecycle over the private `task_bindings` store.
pub struct Bindings {
    store: Arc<PgTaskBindingDao>,
}

impl Bindings {
    pub(crate) fn new(store: Arc<PgTaskBindingDao>) -> Self {
        Self { store }
    }

    /// One Binding by id.
    pub async fn get(&self, id: &str) -> Result<Option<TaskBinding>, sqlx::Error> {
        self.store.find_by_id(id).await
    }

    /// Every Binding on a subject, oldest first.
    pub async fn for_subject(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        self.store
            .find_by_subject(adapter_kind, subject_kind, subject_key)
            .await
    }

    /// Every Binding a Task carries, oldest first.
    pub async fn for_task(&self, task_id: &str) -> Result<Vec<TaskBinding>, sqlx::Error> {
        self.store.find_by_task_id(task_id).await
    }

    /// The Binding that joins one Task to one subject.
    pub async fn for_task_and_subject(
        &self,
        task_id: &str,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        self.store
            .find_by_task_id_and_subject(task_id, adapter_kind, subject_kind, subject_key)
            .await
    }

    /// The oldest Binding on a subject whose Task an Adapter event created.
    pub async fn adapter_event_binding(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error> {
        self.store
            .find_provider_event_created_for_subject(adapter_kind, subject_kind, subject_key)
            .await
    }

    /// Active Bindings whose `destination_config` names `field_value` under
    /// `field_name`. Routes an inbound event that carries a destination-scoped
    /// identifier (e.g. Linear `issue_id`) rather than the Binding's
    /// `subject_key`. `field_name` is whitelisted, so an unknown one errors.
    pub async fn active_by_destination_field(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        field_name: &str,
        field_value: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error> {
        self.store
            .find_active_by_destination_field(adapter_kind, subject_kind, field_name, field_value)
            .await
    }

    /// Every Binding of a set of Tasks, oldest first.
    pub async fn for_tasks(&self, task_ids: &[String]) -> Result<Vec<TaskBinding>, sqlx::Error> {
        self.store.find_by_task_ids(task_ids).await
    }

    /// The most recent Tasks a Loop spawned, each with its first Binding.
    pub(crate) async fn recent_for_loop(
        &self,
        loop_id: &str,
        limit: i64,
    ) -> Result<Vec<LoopRecentTaskRow>, sqlx::Error> {
        self.store.find_recent_for_loop(loop_id, limit).await
    }

    /// The Loop label each Binding of these Tasks carries.
    pub(crate) async fn labels_for_tasks(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<TaskBindingInfo>, sqlx::Error> {
        self.store.get_bindings_for_tasks(task_ids).await
    }

    /// Bind a subject to a Task, or fold this identity into the Binding that
    /// already joins the two. `loop_id`, `last_external_event_id` and
    /// `last_event_at` only ever move forward, so a re-bind never drops what an
    /// earlier one recorded.
    #[expect(
        clippy::too_many_arguments,
        reason = "nine binding-identity data parameters; there is no port bundle \
                  to hoist, so the width is the binding's own identity"
    )]
    pub async fn bind(
        &self,
        connection: &AdapterConnection,
        subscription: Option<&SubscriptionMatch>,
        task_id: &str,
        provider: &str,
        subject_kind: &str,
        subject_key: &str,
        destination_config: serde_json::Value,
        origin: &str,
        last_external_event_id: Option<String>,
    ) -> Result<TaskBinding, AdapterError> {
        let now = Utc::now();
        let filter_uid = destination_config
            .get("filter_user_id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let binding = TaskBinding {
            id: format!("ts_{}", uuid::Uuid::new_v4()),
            task_id: task_id.to_string(),
            adapter_kind: AdapterKind::parse_or_webhook(provider),
            subject_kind: subject_kind.to_string(),
            subject_key: subject_key.to_string(),
            connection_id: connection.id.clone(),
            loop_id: subscription.map(|r| r.id.clone()),
            destination_config,
            metadata: serde_json::json!({ "filter_user_id": filter_uid }),
            last_external_event_id,
            origin: origin.to_string(),
            created_at: now,
            last_event_at: now,
            external_status: None,
            external_state_kind: None,
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: None,
            active: true,
            deactivated_at: None,
        };
        Ok(self.store.upsert(&binding).await?)
    }

    /// Write a Binding whose identity the caller already assembled.
    pub(crate) async fn upsert(&self, row: &TaskBinding) -> Result<TaskBinding, sqlx::Error> {
        self.store.upsert(row).await
    }

    /// Record that an inbound event reached this Binding's Task. An event that
    /// names itself also advances the replay cursor, so context replay can skip
    /// what the agent itself posted; one that does not only bumps
    /// `last_event_at`.
    pub async fn record_inbound_event(
        &self,
        binding: &TaskBinding,
        external_event_id: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        match external_event_id {
            Some(external_event_id) => self.advance_cursor(&binding.id, external_event_id).await,
            None => self.store.touch_last_event(&binding.id).await,
        }
    }

    /// Stamp the latest external id seen on this Binding, and bump
    /// `last_event_at` with it.
    pub(crate) async fn advance_cursor(
        &self,
        id: &str,
        external_event_id: &str,
    ) -> Result<(), sqlx::Error> {
        self.store.touch_cursor(id, external_event_id).await
    }

    /// Record what the provider now says about the connected subject. Leaves
    /// `destination_config` untouched, which [`Self::bind`] would clobber.
    pub async fn record_external_status(
        &self,
        id: &str,
        status: Option<&str>,
        state_kind: Option<&str>,
        raw: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        self.store
            .update_external_status(id, status, state_kind, raw)
            .await
    }

    /// Record the pull request's latest review decision.
    pub async fn record_review_decision(
        &self,
        id: &str,
        review_decision: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        self.store.update_review_decision(id, review_decision).await
    }

    /// Record the pull request's mergeable state.
    pub async fn record_mergeable_state(
        &self,
        id: &str,
        mergeable_state: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        self.store.update_mergeable_state(id, mergeable_state).await
    }

    /// Record that this Binding's thread has carried a task-provenance footer,
    /// so later replies in the same thread do not repeat it.
    pub async fn mark_task_link_sent(&self, id: &str) -> Result<(), sqlx::Error> {
        self.store.mark_task_link_sent(id).await
    }

    /// Stop every still-active Binding of a closed Task from routing, and
    /// return how many changed. Idempotent: a second call reports 0.
    pub async fn deactivate_for_task(&self, task_id: &str) -> Result<u64, sqlx::Error> {
        self.store.deactivate_by_task(task_id).await
    }
}
