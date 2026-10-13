//! The read port over activities recorded by a Task.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_types::activity::Activity;
use moatless_types::store::StoreError;

/// What one subagent has spent, summed over every model response it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentUsage {
    /// The spawning call's `tool_use_id`.
    pub tool_use_id: String,
    /// Model responses that reported usage.
    pub responses: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// The activities a Task recorded, for a read model that projects them.
///
/// arch-allow(port-admission): keeps the T3 projection independent of the Task
/// implementation; remove when another read seam replaces it.
/// arch-allow(single-implementation-trait): the Task-owned read port keeps the
/// T3 projection independent; remove when that projection uses another source.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait ActivitySource: Send + Sync {
    /// Top-level activities for a Task, oldest first, from `since` onward.
    async fn since(
        &self,
        task_id: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Activity>, StoreError>;

    /// The activities these Messages produced. An empty slice reads nothing.
    async fn for_messages(
        &self,
        task_id: &str,
        message_uuids: &[String],
    ) -> Result<Vec<Activity>, StoreError>;

    /// The activities these ids name. An empty slice reads nothing.
    async fn for_ids(
        &self,
        task_id: &str,
        activity_ids: &[String],
    ) -> Result<Vec<Activity>, StoreError>;

    /// The one tool call this `tool_use_id` names, whatever its state.
    async fn tool_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<Activity>, StoreError>;

    /// Of these tool calls, the ones no longer in progress. An empty slice
    /// reads nothing.
    async fn settled_calls(
        &self,
        task_id: &str,
        tool_use_ids: &[String],
    ) -> Result<HashSet<String>, StoreError>;

    /// What the subagents these calls spawned have spent, one entry per call
    /// that reported any. An empty slice reads nothing.
    ///
    /// Summed over the usage lines rather than the Messages, since a tool-only
    /// response spends tokens and leaves no Message.
    async fn subagent_usage(
        &self,
        task_id: &str,
        calls: &[String],
    ) -> Result<Vec<SubagentUsage>, StoreError>;
}
