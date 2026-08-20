//! The `task_bindings` port, declared in the tasks band.
//!
//! `task/{closer,reaper,presenter,handlers}` read bindings to close, reap, and
//! present a task; the concrete `PgTaskBindingDao` that talks to Postgres lives
//! in `bindings/dao.rs` (loops band) and implements this trait. Declaring the
//! contract here — beside its lowest-band consumer — is what makes the
//! `tasks → loops` reference point downward instead of up.

use chrono::{DateTime, Utc};

use moatless_adapter_model::models::TaskBinding;

#[async_trait::async_trait]
pub trait TaskBindingDao: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<TaskBinding>, sqlx::Error>;

    async fn find_by_subject(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn find_all_by_subject_excluding_task(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
        exclude_task_id: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn find_by_task_id(&self, task_id: &str) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn find_by_task_id_and_subject(
        &self,
        task_id: &str,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error>;

    async fn find_provider_event_created_for_subject(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Option<TaskBinding>, sqlx::Error>;

    /// Look up active bindings whose `destination_config->>field_name` equals
    /// `field_value`. Used to route inbound events that carry a destination-
    /// scoped identifier (e.g. Linear `issue_id`) rather than the binding's
    /// `subject_key`. `field_name` is whitelisted to prevent SQL injection
    /// through the JSONB path expression.
    async fn find_active_by_destination_field(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        field_name: &str,
        field_value: &str,
    ) -> Result<Vec<TaskBinding>, sqlx::Error>;

    async fn upsert(&self, row: &TaskBinding) -> Result<TaskBinding, sqlx::Error>;

    async fn delete(&self, id: &str) -> Result<u64, sqlx::Error>;

    /// Soft-deactivate every still-active binding of a task and return how many
    /// rows changed. Idempotent: a second call reports 0 because the
    /// `active = TRUE` predicate no longer matches.
    async fn deactivate_by_task(&self, task_id: &str) -> Result<u64, sqlx::Error>;

    async fn touch_last_event(&self, id: &str) -> Result<(), sqlx::Error>;

    /// Record that this binding's thread has carried a task-provenance footer,
    /// so later replies in the same thread do not repeat it. Merges into
    /// `metadata` rather than taking a column: the footer is presentation
    /// state, not part of the binding's identity.
    async fn mark_task_link_sent(&self, id: &str) -> Result<(), sqlx::Error>;

    async fn touch_cursor(&self, id: &str, last_external_event_id: &str)
        -> Result<(), sqlx::Error>;

    /// Persist the external status of a binding's connected subject (GitHub PR
    /// / Linear issue). Writes the dedicated `external_status*` columns and
    /// stamps `external_status_updated_at = now()`. Leaves
    /// `destination_config` untouched (which `upsert` would clobber).
    async fn update_external_status(
        &self,
        id: &str,
        status: Option<&str>,
        state_kind: Option<&str>,
        raw: &serde_json::Value,
    ) -> Result<(), sqlx::Error>;

    /// Persist the PR's latest review decision (approved / changes_requested /
    /// …) onto a binding. Leaves the `external_status*` columns untouched.
    async fn update_review_decision(
        &self,
        id: &str,
        review_decision: Option<&str>,
    ) -> Result<(), sqlx::Error>;

    /// Persist the PR's mergeable state (clean / dirty / …) onto a binding.
    /// Leaves the `external_status*` columns untouched.
    async fn update_mergeable_state(
        &self,
        id: &str,
        mergeable_state: Option<&str>,
    ) -> Result<(), sqlx::Error>;

    /// Batch-fetch full binding rows (including `destination_config` and the
    /// `external_status*` columns) for a set of tasks. Drives the
    /// task-connections admin view.
    async fn find_by_task_ids(&self, task_ids: &[String]) -> Result<Vec<TaskBinding>, sqlx::Error>;

    /// Batch-fetch the task-list display projection — one row per binding, with
    /// the owning loop's `source_matcher`/`source_name` joined in from
    /// `subscriptions` for labelling. The DAO seam the task-list handler reads
    /// instead of the raw pool. See [`TaskBindingInfo`].
    async fn get_bindings_for_tasks(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<TaskBindingInfo>, sqlx::Error>;

    /// The most recent tasks a loop spawned, each with its first binding (if
    /// any). Drives the loop detail view. See the `PgTaskBindingDao` impl of
    /// this method (in the backend's `bindings::dao`) for the join semantics.
    async fn find_recent_for_loop(
        &self,
        loop_id: &str,
        limit: i64,
    ) -> Result<Vec<LoopRecentTaskRow>, sqlx::Error>;
}

/// Flat row returned by `find_recent_for_loop`. Binding columns are
/// optional — schedule loops spawn tasks without a binding.
#[derive(sqlx::FromRow, Debug)]
pub struct LoopRecentTaskRow {
    pub task_id: String,
    pub short_id: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub task_created_at: String,
    // binding fields (None for schedule loops — no return-channel binding)
    pub binding_id: Option<String>,
    pub adapter_kind: Option<String>,
    pub subject_kind: Option<String>,
    pub subject_key: Option<String>,
    pub connection_id: Option<String>,
    pub loop_id: Option<String>,
    pub destination_config: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub last_external_event_id: Option<String>,
    pub origin: Option<String>,
    pub binding_created_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
}

/// Flat row returned by the batch binding query.
#[derive(sqlx::FromRow, Debug)]
pub struct TaskBindingInfo {
    pub task_id: String,
    pub loop_id: String,
    pub adapter_kind: String,
    pub source_id: String,
    pub source_name: Option<String>,
}
