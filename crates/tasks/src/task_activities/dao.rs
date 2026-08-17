//! Every statement this module runs: the activity rows, and the counter and
//! change log that order and announce them.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, Row, postgres::PgRow};

use super::mapping::{self, ActivityKind, ActivityRow, ActivityStatus, ToolOutcome};
use moatless_backend_core::change_feed::conversation::record_activity_change;
use moatless_core::db::DbPool;
/// One stored activity.
#[derive(Debug, Clone)]
pub struct Activity {
    pub activity_id: String,
    pub task_id: String,
    pub position: i64,
    /// Where this row sits inside its change. See
    /// [`ActivityRow::ordinal`](ActivityRow::ordinal).
    pub ordinal: i32,
    pub message_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub kind: ActivityKind,
    pub status: ActivityStatus,
    pub tool_use_id: Option<String>,
    pub summary: String,
    pub payload: Value,
    /// What a settled call shows before it is expanded. `None` until the call
    /// comes back, and on every activity that is not a tool call.
    pub detail: Option<String>,
    pub is_error: bool,
    pub external_event_id: Option<String>,
    pub external_adapter: Option<String>,
    pub external_source_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Activity {
    /// The tool's input as the call carried it, or `Null` for an activity that
    /// is not a tool call.
    pub fn tool_input(&self) -> &Value {
        self.payload.get("input").unwrap_or(&Value::Null)
    }

    /// The result whole, or `None` while the call is still in progress.
    pub fn result(&self) -> Option<&Value> {
        self.payload.get("result")
    }

    /// The transport's structured view of the outcome, where it reported one.
    pub fn typed_result(&self) -> Option<&Value> {
        self.payload.get("typedResult")
    }

    /// Whether the call this activity records has come back.
    pub fn is_settled(&self) -> bool {
        self.status != ActivityStatus::InProgress
    }
}

impl sqlx::FromRow<'_, PgRow> for Activity {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        let kind: String = row.try_get("kind")?;
        let status: String = row.try_get("status")?;
        Ok(Self {
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
            external_event_id: row.try_get("external_event_id")?,
            external_adapter: row.try_get("external_adapter")?,
            external_source_name: row.try_get("external_source_name")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

/// The kinds this build can decode, as SQL.
///
/// The narrowing is what keeps a rollback survivable: a row written by a newer
/// backend carries a kind [`ActivityKind::from_str`] refuses, and a refused
/// decode fails the whole read — one unknown row would blank a conversation.
/// Skipped in SQL, it costs that row and nothing else.
///
/// Kept honest by `every_known_kind_is_readable`, which is a test rather than a
/// derive because sqlx 0.9 takes only `&'static str` and the list has to be one.
macro_rules! known_kinds {
    () => {
        "kind IN ('tool_call', 'thinking', 'sandbox_status', 'sandbox_warning', \
          'sandbox_error', 'user_input_requested', 'user_input_resolved', \
          'plan_updated', 'context_window_updated')"
    };
}

/// The columns every read of this table selects, in one place so four queries
/// cannot drift into four different row shapes.
///
/// A macro rather than a `const` joined with `format!`: sqlx 0.9 takes only
/// `&'static str`, so the concatenation has to happen at compile time.
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

/// Write one activity, or leave the existing one as it is.
///
/// Redelivery lands on `activity_id`, which is derived from the source. The
/// update deliberately does *not* touch `status`, `is_error` or the result: a
/// call whose result already arrived must not be reset to `in_progress` by a
/// second copy of the block that started it — the two arrive on different
/// Messages and nothing orders them.
///
/// The `NOT EXISTS` guard covers the other way the same call can arrive twice:
/// re-presented under a *different* Message, so it derives a different
/// `activity_id` that the arbiter above cannot see. `(task_id, tool_use_id)` is
/// unique, so that insert raises rather than merging, and the raise aborts the
/// whole Message write. One call is one row and the stored one is the row
/// [`settle`] will find, so the later arrival is dropped. A NULL `tool_use_id`
/// matches nothing and so is never guarded — several are not a conflict.
pub async fn upsert(
    tx: &mut PgConnection,
    position: i64,
    activity: &ActivityRow,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO task_activities (
               activity_id, task_id, position, ordinal, message_uuid, parent_uuid,
               kind, status, tool_use_id, summary, payload, is_error
           )
           SELECT $1::text, $2::text, $3::bigint, $4::int, $5::text, $6::text,
                  $7::text, $8::text, $9::text, $10::text, $11::jsonb, $12::boolean
           WHERE NOT EXISTS (
               SELECT 1 FROM task_activities
               WHERE task_id = $2 AND tool_use_id = $9 AND activity_id <> $1
           )
           ON CONFLICT (activity_id) DO UPDATE SET
               -- `position` and `ordinal` are deliberately absent: a redelivered
               -- block must not move where it already sits in the stream, and
               -- between them they are that place. So is `detail`, which belongs
               -- to a result this statement does not carry.
               summary  = EXCLUDED.summary,
               -- The call is the authority for its own name, shape and input,
               -- so those are re-applied. The result and the status it moved to
               -- are the *outcome's*, and are laid back over the top.
               payload  = EXCLUDED.payload || jsonb_strip_nulls(jsonb_build_object(
                              'status',      task_activities.payload -> 'status',
                              'result',      task_activities.payload -> 'result',
                              'typedResult', task_activities.payload -> 'typedResult'
                          ))"#,
    )
    .bind(&activity.activity_id)
    .bind(&activity.task_id)
    .bind(position)
    .bind(activity.ordinal)
    .bind(&activity.message_uuid)
    .bind(&activity.parent_uuid)
    .bind(activity.kind.as_str())
    .bind(activity.status.as_str())
    .bind(&activity.tool_use_id)
    .bind(&activity.summary)
    .bind(&activity.payload)
    .bind(activity.is_error)
    .execute(tx)
    .await?;
    Ok(())
}

/// Settle the call a result answers.
///
/// Finds the row by `(task_id, tool_use_id)` because the result arrives on a
/// later Message than the call and cannot derive its key. If no row matches —
/// the call was never received, or the Task predates the migration — nothing is
/// created and nothing errors: a result whose call predates the change is the
/// common case.
///
/// This is where `detail` is cut, because this is where the result arrives. Any
/// other placement leaves a settled call whose preview is missing.
pub async fn settle(tx: &mut PgConnection, outcome: &ToolOutcome) -> Result<bool, sqlx::Error> {
    let settled = sqlx::query(
        r#"UPDATE task_activities SET
               status   = $3,
               is_error = $4,
               detail   = $8,
               payload  = payload
                            || jsonb_build_object('status', $5::text)
                            || jsonb_build_object('result', $6::jsonb)
                            -- Stripped rather than bound as SQL NULL: a
                            -- transport reporting no typed result must leave
                            -- the key absent, not overwrite it with `null`.
                            || jsonb_strip_nulls(jsonb_build_object('typedResult', $7::jsonb))
           WHERE task_id = $1 AND tool_use_id = $2"#,
    )
    .bind(&outcome.task_id)
    .bind(&outcome.tool_use_id)
    .bind(outcome.status.as_str())
    .bind(outcome.is_error)
    .bind(outcome.status.wire())
    .bind(&outcome.result)
    .bind(&outcome.typed_result)
    .bind(mapping::detail(&outcome.result))
    .execute(tx)
    .await?
    .rows_affected();
    Ok(settled > 0)
}

/// Write a Sandbox status change or error, unless it restates the one before it.
///
/// The collapse rule is not new to this codebase — the lifecycle *intent* path
/// already has the equivalent `IS DISTINCT FROM` guard around
/// [`record_session_change`]. The event path simply never had it, and 92,165 of
/// 139,595 Sandbox rows today restate the status before them.
///
/// The collapse check runs *before* the position is allocated, so a restated
/// status spends no position and records no change: allocating first would burn
/// one of each to project to nothing. Returns whether a row was written. The
/// event itself is recorded either way by its own handler, so the Events tab is
/// unaffected.
pub async fn record_sandbox(
    tx: &mut PgConnection,
    activity: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    let previous: Option<(String, bool)> = sqlx::query_as(
        r#"SELECT summary, is_error FROM task_activities
           WHERE task_id = $1 AND kind IN ('sandbox_status', 'sandbox_warning', 'sandbox_error')
           ORDER BY position DESC
           LIMIT 1"#,
    )
    .bind(&activity.task_id)
    .fetch_optional(&mut *tx)
    .await?;

    if previous.is_some_and(|(summary, is_error)| {
        summary == activity.summary && is_error == activity.is_error
    }) {
        return Ok(false);
    }

    let position = record_activity_change(tx, &activity.task_id, &activity.activity_id).await?;
    record_standalone(tx, position, activity).await
}

/// Write an activity that happened on no Message.
///
/// A Sandbox lifecycle change and a context-window reading are both this: alone
/// in their change, named by the event that carried them, ordinal 0.
///
/// No merge on conflict — the id *is* the event, so a conflict is the same event
/// redelivered and the stored row is already what this would write.
///
/// Returns whether a row was written.
pub async fn record_standalone(
    tx: &mut PgConnection,
    position: i64,
    activity: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    let written = sqlx::query(
        r#"INSERT INTO task_activities (
               activity_id, task_id, position, ordinal, message_uuid, parent_uuid,
               kind, status, tool_use_id, summary, payload, is_error
           ) VALUES ($1, $2, $3, $4, NULL, NULL, $5, $6, NULL, $7, $8, $9)
           ON CONFLICT (activity_id) DO NOTHING"#,
    )
    .bind(&activity.activity_id)
    .bind(&activity.task_id)
    .bind(position)
    .bind(activity.ordinal)
    .bind(activity.kind.as_str())
    .bind(activity.status.as_str())
    .bind(&activity.summary)
    .bind(&activity.payload)
    .bind(activity.is_error)
    .execute(tx)
    .await?
    .rows_affected();
    Ok(written > 0)
}

/// Admit a Message's token usage exactly once.
///
/// Where the usage of a turn that said nothing survives the row it was reported
/// on. `DO NOTHING` rather than an update, for the reason `task_messages` keeps
/// the first non-null value: a CLI that repeats usage on every per-block source
/// event must not count it twice.
pub async fn admit_usage(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: &str,
    parent_uuid: Option<&str>,
    (input_tokens, output_tokens): (i32, i32),
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO task_message_usage
               (message_uuid, task_id, parent_uuid, input_tokens, output_tokens)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (message_uuid) DO NOTHING"#,
    )
    .bind(message_uuid)
    .bind(task_id)
    .bind(parent_uuid)
    .bind(input_tokens)
    .bind(output_tokens)
    .execute(tx)
    .await?;
    Ok(())
}

/// Every activity of a Task's own conversation, in stream order.
///
/// Subagent activities are excluded for the reason their Messages are: they are
/// assembled on demand behind the `Agent` call that spawned them, and they
/// outnumber the rows that render.
pub async fn for_task(pool: &DbPool, task_id: &str) -> Result<Vec<Activity>, sqlx::Error> {
    sqlx::query_as::<_, Activity>(read_activities!(
        "task_id = $1 AND parent_uuid IS NULL ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// A Task's own activities from a point in time onwards.
///
/// What a paginated conversation read needs, and it is a time window rather than
/// a set of named Messages for one reason: a turn that only called tools said
/// nothing and has no Message to be named by, and its calls are exactly what the
/// page has to show. `None` means from the beginning.
pub async fn for_task_since(
    pool: &DbPool,
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

/// One subagent's activities, behind the `Agent` call that spawned it.
pub async fn for_subagent(
    pool: &DbPool,
    task_id: &str,
    parent_uuid: &str,
) -> Result<Vec<Activity>, sqlx::Error> {
    sqlx::query_as::<_, Activity>(read_activities!(
        "task_id = $1 AND parent_uuid = $2 ORDER BY position, ordinal, activity_id"
    ))
    .bind(task_id)
    .bind(parent_uuid)
    .fetch_all(pool)
    .await
}

/// The activities that happened on a named set of Messages.
pub async fn for_messages(
    pool: &DbPool,
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
    pool: &DbPool,
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

/// One tool call by the id it was made under.
///
/// One row, found by an index: the call and its outcome are one row, so there is
/// no second lookup for whichever later Message carried the result.
pub async fn find_tool_call(
    pool: &DbPool,
    task_id: &str,
    tool_use_id: &str,
) -> Result<Option<Activity>, sqlx::Error> {
    sqlx::query_as::<_, Activity>(read_activities!("task_id = $1 AND tool_use_id = $2"))
        .bind(task_id)
        .bind(tool_use_id)
        .fetch_optional(pool)
        .await
}

/// Claim what a call created on the other side, once.
///
/// `external_event_id IS NULL` is first-writer-wins: a command re-parsed from a
/// redelivered result must not restate a claim, and two different ids for one
/// call would mean the parse is wrong.
///
/// Returns whether this call took the claim.
pub async fn claim_external(
    pool: &DbPool,
    task_id: &str,
    tool_use_id: &str,
    external_id: &str,
    adapter: &str,
    source_name: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let claimed = sqlx::query(
        r#"UPDATE task_activities
              SET external_event_id    = $3,
                  external_adapter     = $4,
                  external_source_name = $5
            WHERE task_id = $1 AND tool_use_id = $2 AND external_event_id IS NULL"#,
    )
    .bind(task_id)
    .bind(tool_use_id)
    .bind(external_id)
    .bind(adapter)
    .bind(source_name)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(claimed > 0)
}

/// Whether any of this Task's calls already claims one of these ids.
pub async fn claims_any_external(
    pool: &DbPool,
    task_id: &str,
    external_ids: &[String],
) -> Result<bool, sqlx::Error> {
    let found: Option<i32> = sqlx::query_scalar(
        r#"SELECT 1 FROM task_activities
            WHERE task_id = $1 AND external_event_id = ANY($2)
            LIMIT 1"#,
    )
    .bind(task_id)
    .bind(external_ids)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Every external id this Task's calls claim.
pub async fn external_claims(pool: &DbPool, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT external_event_id FROM task_activities
            WHERE task_id = $1 AND external_event_id IS NOT NULL"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// Which Task, if any, claims one of these ids.
pub async fn task_claiming_external(
    pool: &DbPool,
    external_ids: &[String],
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT task_id FROM task_activities
            WHERE external_event_id = ANY($1)
            LIMIT 1"#,
    )
    .bind(external_ids)
    .fetch_optional(pool)
    .await
}

/// How many tool calls a Task made and how many came back failed.
///
/// `(total, failed)`. A Task with no activities returns `(0, 0)`, which the
/// caller must not report as a fact — see `task/statistics.rs`.
pub async fn tool_call_counts(pool: &DbPool, task_id: &str) -> Result<(i64, i64), sqlx::Error> {
    let row: (i64, i64) = sqlx::query_as(
        r#"SELECT count(*),
                  count(*) FILTER (WHERE is_error)
           FROM task_activities
           WHERE task_id = $1 AND kind = 'tool_call'"#,
    )
    .bind(task_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::mapping::ActivityKind;

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
