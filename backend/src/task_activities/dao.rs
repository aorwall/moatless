//! Every statement this module runs: the activity rows, and the counter and
//! change log that order and announce them.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, Row, postgres::PgRow};

use super::mapping::{self, ActivityKind, ActivityRow, ActivityStatus, ToolOutcome};
use super::models::{ConversationChange, ConversationHead, RETAINED_CHANGES};
use crate::db::DbPool;

/// One stored activity.
#[derive(Debug, Clone)]
pub struct Activity {
    pub activity_id: String,
    pub task_id: String,
    pub position: i64,
    /// Where this row sits inside its change. See
    /// [`ActivityRow::ordinal`](mapping::ActivityRow::ordinal).
    pub ordinal: i32,
    pub message_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub kind: ActivityKind,
    pub status: ActivityStatus,
    pub tool_use_id: Option<String>,
    pub summary: String,
    pub payload: Value,
    /// What a settled call shows before it is expanded, cut to
    /// [`mapping::DETAIL_LIMIT`]. `None` until the call comes back, and on every
    /// activity that is not a tool call.
    pub detail: Option<String>,
    pub is_error: bool,
    /// What this call created on the other side, claimed so the webhook echo of
    /// it is recognised as the agent's own. `None` on every call that created
    /// nothing, which is nearly all of them.
    pub external_event_id: Option<String>,
    pub external_adapter: Option<String>,
    pub external_source_name: Option<String>,
    /// When the database recorded it. A Sandbox activity belongs to no Message
    /// and has no other timestamp to be stamped with.
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
    /// This is what `enrich_codex_typed_result` validates; the block content
    /// `result()` returns is the text the tool printed.
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
            kind: kind.parse().map_err(|e: String| sqlx::Error::ColumnDecode {
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
        "kind IN ('tool_call', 'thinking', 'sandbox_status', 'sandbox_error', \
          'user_input_requested', 'user_input_resolved', 'plan_updated', \
          'context_window_updated')"
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
pub async fn upsert(
    tx: &mut PgConnection,
    position: i64,
    activity: &ActivityRow,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO task_activities (
               activity_id, task_id, position, ordinal, message_uuid, parent_uuid,
               kind, status, tool_use_id, summary, payload, is_error
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
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
/// Returns whether a row was written. The event itself is recorded either way by
/// its own handler, so the Events tab is unaffected.
pub async fn record_sandbox(
    tx: &mut PgConnection,
    position: i64,
    activity: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    let previous: Option<(String, bool)> = sqlx::query_as(
        r#"SELECT summary, is_error FROM task_activities
           WHERE task_id = $1 AND kind IN ('sandbox_status', 'sandbox_error')
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

// ── The conversation's position counter and change log ──────────────────────
//
// Activities and Messages are ordered by one counter per Task, and a reader
// learns the conversation moved from the log that counter writes.

/// Record that `message_uuid` was written or rewritten, returning the position
/// allocated for it. `tx` is the connection the mutation's transaction is
/// running on — `&mut *tx`, never a pool, which would commit immediately and
/// make the record visible before the write it describes.
///
/// # Call this before `task_listing::record_change`
///
/// Both allocators hold a counter row until commit, so every transaction that
/// takes both must take them in one order or the pair deadlocks. This one goes
/// first and the shell's stays last, exactly as its own "call this last" says:
/// this counter is one row per Task and the shell's is one row for the whole
/// deployment, so this order also holds the contended one for the shorter time.
pub async fn record_message_change(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, Some(message_uuid)).await
}

/// Record that the conversation itself moved — its Turn, or the Sandbox its
/// session status is read from. Names no Message, because none changed: the
/// reader answers this by re-reading the conversation's own row.
///
/// Same transaction rule and same ordering against the shell's allocator as
/// [`record_message_change`].
pub async fn record_session_change(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None).await
}

/// Take the next position without recording a change.
///
/// Activities are ordered by this counter and notified through the Message they
/// belong to, so each one takes a position but leaves no record: a record per
/// activity would multiply the feed's row count by the number of blocks in a
/// turn to say something the Message's own record already said.
///
/// One position per activity rather than one shared by all of a Message's is
/// what keeps `(task_id, position)` a total order. Sharing one would leave
/// activities within a Message tied, and the only tiebreaker left is the id —
/// where `msg:10` sorts before `msg:2`.
///
/// Same transaction rule as [`record_message_change`], and no ordering hazard
/// against it: the first allocation in a transaction takes the counter row and
/// holds it until commit, so every later one on the same connection is free.
pub async fn allocate_position(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"INSERT INTO task_conversation_position (task_id, position) VALUES ($1, 1)
           ON CONFLICT (task_id) DO UPDATE
             SET position = task_conversation_position.position + 1
           RETURNING position"#,
    )
    .bind(task_id)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The one allocating statement, shared so the two subjects cannot drift into
/// two different orderings of the same counter row. The `ON CONFLICT DO UPDATE`
/// is the allocation, and it holds that row for the rest of the transaction.
async fn allocate(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO task_conversation_position (task_id, position) VALUES ($1, 1)
               ON CONFLICT (task_id) DO UPDATE
                 SET position = task_conversation_position.position + 1
               RETURNING position
           )
           INSERT INTO task_conversation_changes (task_id, position, message_uuid)
           SELECT $1, position, $2 FROM allocated
           RETURNING position"#,
    )
    .bind(task_id)
    .bind(message_uuid)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The highest position this conversation has reached — 0 before its first
/// change, and 0 for a Task that has never had one.
///
/// The counter is the authority, never `MAX(task_conversation_changes.position)`:
/// [`prune_changes`] keeps the records for a bounded window, so on a
/// conversation older than that window a `MAX` drops back to a position long
/// since passed and hands a client a stale place to resume from, silently.
pub async fn current_head(pool: &DbPool, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
    let position: Option<(i64,)> =
        sqlx::query_as(r#"SELECT position FROM task_conversation_position WHERE task_id = $1"#)
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(ConversationHead {
        position: position.map_or(0, |(position,)| position),
    })
}

/// Every change to this conversation recorded at or after `from`, oldest first,
/// capped at `limit`.
///
/// At `from` rather than after it: a client disconnected partway through one
/// change's events holds a cursor inside that position, and the events it
/// missed are the rest of that same change. Filtering the *events* by sequence
/// is what makes this exact — see
/// [`position_of`](super::models::position_of).
///
/// Safe as a polling cursor only because commits happen in position order, so
/// nothing can appear below a position this has already returned.
pub async fn changes_from(
    pool: &DbPool,
    task_id: &str,
    from: i64,
    limit: i64,
) -> Result<Vec<ConversationChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, message_uuid
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

/// Delete the changes that have fallen out of each conversation's retention
/// window, returning how many went. The cut comes from each Task's own counter,
/// per [`current_head`]. Safe from every replica — one bounded `DELETE` over
/// durable state, so two racing delete the same rows and the second finds
/// nothing. No lease, no leader.
pub async fn prune_changes(pool: &DbPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM task_conversation_changes c
           USING task_conversation_position p
           WHERE c.task_id = p.task_id
             AND c.position <= p.position - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

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
