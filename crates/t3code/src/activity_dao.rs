//! The reads this surface needs and the conversation feed does not.
//!
//! Everything a Moatless reader already asks of `task_activities` is served by
//! [`moatless_ports::activity::ActivitySource`]. What is left here answers
//! questions only a T3 transcript asks.

use std::collections::{HashMap, HashSet};

use moatless_types::activity::Activity;
use moatless_types::activity::ActivityKind;
use moatless_types::activity::ActivityView;
use serde_json::Value;

use crate::UiRpcDbPool;

/// Of the requests `activities` ask, the ones whose call has come back.
///
/// One indexed lookup per read, none for a read that asks nothing; see
/// [`crate::transcript_dao::Transcript::settled_requests`] for why the call
/// isn't found among the rows themselves.
pub async fn settled_requests(
    pool: &UiRpcDbPool,
    task_id: &str,
    activities: &[Activity],
) -> Result<HashSet<String>, sqlx::Error> {
    let asked: Vec<String> = activities
        .iter()
        .filter(|activity| activity.kind == ActivityKind::UserInputRequested)
        .filter_map(|activity| activity.payload.get("requestId").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    if asked.is_empty() {
        return Ok(HashSet::new());
    }
    let settled: Vec<(String,)> = sqlx::query_as(
        r#"SELECT tool_use_id
           FROM task_activities
           WHERE task_id = $1
             AND tool_use_id = ANY($2)
             AND kind = 'tool_call'
             AND status <> 'in_progress'"#,
    )
    .bind(task_id)
    .bind(&asked)
    .fetch_all(pool)
    .await?;
    Ok(settled
        .into_iter()
        .map(|(tool_use_id,)| tool_use_id)
        .collect())
}

/// What each subagent named by these rows has spent, keyed by the spawning
/// `Agent` tool-use id.
///
/// Summed over `task_message_usage`, not `task_messages`, since tool-only turns
/// spend tokens but leave no Message row.
pub async fn subagent_usage(
    pool: &UiRpcDbPool,
    task_id: &str,
    activities: &[Activity],
) -> Result<HashMap<String, SubagentUsage>, sqlx::Error> {
    let spawned: Vec<String> = activities
        .iter()
        .filter(|activity| crate::activity::is_subagent_call(&ActivityView::new(activity)))
        .filter_map(|activity| activity.tool_use_id.clone())
        .collect();
    if spawned.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<SubagentUsage> = sqlx::query_as(
        r#"SELECT parent_uuid,
                  COALESCE(SUM(input_tokens), 0)  AS input_tokens,
                  COALESCE(SUM(output_tokens), 0) AS output_tokens
           FROM task_message_usage
           WHERE task_id = $1 AND parent_uuid = ANY($2)
           GROUP BY parent_uuid"#,
    )
    .bind(task_id)
    .bind(&spawned)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|usage| (usage.parent_uuid.clone(), usage))
        .collect())
}

/// One subagent's token spend.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SubagentUsage {
    /// The spawning `Agent` tool-use id.
    pub parent_uuid: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
}
