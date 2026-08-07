use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use moatless_backend_core::activity::dao::Activity;
use moatless_backend_core::activity::mapping::ActivityKind;
use moatless_backend_core::activity::models::{ConversationChange, ConversationHead};
use moatless_backend_core::activity::view::ActivityView;
use serde_json::Value;

use crate::UiRpcDbPool;

macro_rules! known_kinds {
    () => {
        "kind IN ('tool_call', 'thinking', 'sandbox_status', 'sandbox_warning', \
          'sandbox_error', 'user_input_requested', 'user_input_resolved', \
          'plan_updated', 'context_window_updated')"
    };
}

macro_rules! read_activities {
    ($predicate:literal) => {
        concat!(
            "SELECT activity_id, task_id, position, ordinal, message_uuid, parent_uuid, ",
            "kind, status, tool_use_id, summary, payload, detail, is_error, ",
            "external_event_id, external_adapter, external_source_name, created_at ",
            "FROM task_activities WHERE ",
            known_kinds!(),
            " AND ",
            $predicate
        )
    };
}

pub async fn for_task_since(
    pool: &UiRpcDbPool,
    task_id: &str,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<Activity>, sqlx::Error> {
    sqlx::query_as::<_, Activity>(read_activities!(
        "task_id = $1 AND parent_uuid IS NULL \
         AND ($2::timestamptz IS NULL OR created_at >= $2) \
         ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(since)
    .fetch_all(pool)
    .await
}

pub async fn for_messages(
    pool: &UiRpcDbPool,
    task_id: &str,
    message_uuids: &[String],
) -> Result<Vec<Activity>, sqlx::Error> {
    if message_uuids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, Activity>(read_activities!(
        "task_id = $1 AND message_uuid = ANY($2) ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(message_uuids)
    .fetch_all(pool)
    .await
}

/// The activities named directly by their own id — the ones that happened on no
/// Message and so are named by the change rather than by a Message uuid.
pub async fn for_activity_ids(
    pool: &UiRpcDbPool,
    task_id: &str,
    activity_ids: &[String],
) -> Result<Vec<Activity>, sqlx::Error> {
    if activity_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, Activity>(read_activities!(
        "task_id = $1 AND activity_id = ANY($2) ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(activity_ids)
    .fetch_all(pool)
    .await
}

/// Of the requests `activities` ask, the ones whose call has come back.
///
/// One indexed lookup on `(task_id, tool_use_id)` per read, and none at all for
/// a read that asks nothing — which is nearly every one. See
/// [`crate::dao::Transcript::settled_requests`] for why the call cannot be found
/// among the rows themselves.
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
/// That id is the `taskId` the projection stamps on the row, and the key the
/// subagent's own Messages carry as `parent_uuid`. Usage is summed over
/// `task_message_usage` rather than `task_messages` because a subagent spends
/// most of its tokens on turns that only called tools — turns that said nothing
/// and so have no Message row to sum. One indexed grouped read per transcript,
/// and none at all for a transcript that spawned nothing.
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
    pub parent_uuid: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

pub async fn find_tool_call(
    pool: &UiRpcDbPool,
    task_id: &str,
    tool_use_id: &str,
) -> Result<Option<Activity>, sqlx::Error> {
    sqlx::query_as::<_, Activity>(read_activities!("task_id = $1 AND tool_use_id = $2"))
        .bind(task_id)
        .bind(tool_use_id)
        .fetch_optional(pool)
        .await
}

pub async fn current_head(
    pool: &UiRpcDbPool,
    task_id: &str,
) -> Result<ConversationHead, sqlx::Error> {
    let position: Option<(i64,)> =
        sqlx::query_as(r#"SELECT position FROM task_conversation_position WHERE task_id = $1"#)
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(ConversationHead {
        position: position.map_or(0, |(position,)| position),
    })
}

pub async fn changes_from(
    pool: &UiRpcDbPool,
    task_id: &str,
    from: i64,
    limit: i64,
) -> Result<Vec<ConversationChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, message_uuid, activity_id
           FROM task_conversation_changes
           WHERE task_id = $1 AND position >= $2
           ORDER BY position
           LIMIT $3"#,
    )
    .bind(task_id)
    .bind(from)
    .bind(limit)
    .fetch_all(pool)
    .await
}
