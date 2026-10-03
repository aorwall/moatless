//! Every read of `conversation_activities`, as the `Activity` rows the read
//! model serves. The Projector writes them (`conversation::dao`).

use chrono::{DateTime, Utc};
use moatless_types::activity::ActivityStatus;
use sqlx::{Row, postgres::PgRow};

use moatless_core::db::DbPool;
pub(crate) use moatless_types::activity::Activity;
/// The kinds this build can decode, as SQL. A row with an unlisted kind is
/// skipped rather than failing the whole read. Must list every `ActivityKind`,
/// checked by `every_known_kind_is_readable`.
macro_rules! known_kinds {
    () => {
        "kind IN ('tool_call', 'thinking', 'sandbox_status', 'sandbox_warning', \
          'sandbox_error', 'user_input_requested', 'user_input_resolved', \
          'plan_updated', 'context_window_updated', 'agent_switched')"
    };
}

/// The columns every read of this table selects, in one place so queries cannot
/// drift into different row shapes.
macro_rules! read_activities {
    ($predicate:literal) => {
        concat!(
            "SELECT activity_id::text AS activity_id, task_id, position, ordinal, ",
            "message_uuid::text AS message_uuid, parent_tool_use_id AS parent_uuid, ",
            "kind, status, tool_use_id, summary, payload, detail, is_error, created_at ",
            "FROM conversation_activities WHERE ",
            known_kinds!(),
            " AND ",
            $predicate
        )
    };
}

/// Every activity of a Task's own conversation, in stream order. Subagent
/// activities are excluded — assembled on demand behind the `Agent` call that
/// spawned them, and they outnumber the rows that render.
pub(crate) async fn for_task(pool: &DbPool, task_id: &str) -> Result<Vec<Activity>, sqlx::Error> {
    sqlx::query_as::<_, StoredActivity>(read_activities!(
        "task_id = $1 AND parent_tool_use_id IS NULL ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|row| row.0).collect())
}

/// A Task's own activities from a point in time onwards (`None` = from the
/// start). A time window, not named Messages: a tool-only turn has no Message
/// to be named by, yet its calls must show.
pub(crate) async fn for_task_since(
    pool: &DbPool,
    task_id: &str,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<Activity>, sqlx::Error> {
    sqlx::query_as::<_, StoredActivity>(read_activities!(
        "task_id = $1 AND parent_tool_use_id IS NULL \
         AND ($2::timestamptz IS NULL OR created_at >= $2) \
         ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|row| row.0).collect())
}

/// One subagent's activities, behind the `Agent` call that spawned it.
pub(crate) async fn for_subagent(
    pool: &DbPool,
    task_id: &str,
    parent_uuid: &str,
) -> Result<Vec<Activity>, sqlx::Error> {
    sqlx::query_as::<_, StoredActivity>(read_activities!(
        "task_id = $1 AND parent_tool_use_id = $2 ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(parent_uuid)
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|row| row.0).collect())
}

/// The activities that happened on a named set of Messages.
pub(crate) async fn for_messages(
    pool: &DbPool,
    task_id: &str,
    message_uuids: &[String],
) -> Result<Vec<Activity>, sqlx::Error> {
    if message_uuids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, StoredActivity>(read_activities!(
        "task_id = $1 AND message_uuid::text = ANY($2) ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(message_uuids)
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|row| row.0).collect())
}

/// The activities named directly by their own id — the ones that happened on no
/// Message and so are named by the change rather than by a Message uuid.
pub(crate) async fn for_activity_ids(
    pool: &DbPool,
    task_id: &str,
    activity_ids: &[String],
) -> Result<Vec<Activity>, sqlx::Error> {
    if activity_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, StoredActivity>(read_activities!(
        "task_id = $1 AND activity_id::text = ANY($2) ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(activity_ids)
    .fetch_all(pool)
    .await
    .map(|rows| rows.into_iter().map(|row| row.0).collect())
}

/// One tool call by the id it was made under. One row, found by an index: the
/// call and its outcome are one row, so there is no second lookup for whichever
/// later Message carried the result.
pub(crate) async fn find_tool_call(
    pool: &DbPool,
    task_id: &str,
    tool_use_id: &str,
) -> Result<Option<Activity>, sqlx::Error> {
    sqlx::query_as::<_, StoredActivity>(read_activities!("task_id = $1 AND tool_use_id = $2"))
        .bind(task_id)
        .bind(tool_use_id)
        .fetch_optional(pool)
        .await
        .map(|row| row.map(|row| row.0))
}

struct StoredActivity(Activity);

impl sqlx::FromRow<'_, PgRow> for StoredActivity {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        let kind: String = row.try_get("kind")?;
        let status: String = row.try_get("status")?;
        Ok(Self(Activity {
            activity_id: row.try_get("activity_id")?,
            task_id: row.try_get("task_id")?,
            position: row.try_get("position")?,
            ordinal: row.try_get("ordinal")?,
            message_uuid: row.try_get("message_uuid")?,
            parent_uuid: row.try_get("parent_uuid")?,
            kind: kind
                .parse()
                .map_err(|e: String| sqlx::Error::ColumnDecode {
                    index: "kind".to_string(),
                    source: e.into(),
                })?,
            status: match status.as_str() {
                "in_progress" => ActivityStatus::InProgress,
                "completed" => ActivityStatus::Completed,
                "failed" => ActivityStatus::Failed,
                other => {
                    return Err(sqlx::Error::ColumnDecode {
                        index: "status".to_string(),
                        source: format!("unknown activity status: {other:?}").into(),
                    });
                }
            },
            tool_use_id: row.try_get("tool_use_id")?,
            summary: row.try_get("summary")?,
            payload: row.try_get("payload")?,
            detail: row.try_get("detail")?,
            is_error: row.try_get("is_error")?,
            created_at: row.try_get("created_at")?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use moatless_types::activity::ActivityKind;

    /// The SQL narrowing and the enum are two spellings of one list. A kind the
    /// enum knows but the SQL leaves out is a row this build writes and then
    /// never reads back.
    #[test]
    fn every_known_kind_is_readable() {
        let narrowing = known_kinds!();
        for kind in ActivityKind::ALL {
            assert!(
                narrowing.contains(&format!("'{}'", kind.as_str())),
                "{} is written but the read narrows it away",
                kind.as_str()
            );
        }
        // And nothing else: a name in the SQL that no kind spells is one the
        // decode would refuse anyway.
        assert_eq!(
            narrowing.matches('\'').count() / 2,
            ActivityKind::ALL.len(),
            "the SQL names exactly the kinds the enum does"
        );
    }
}
