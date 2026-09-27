//! Every statement that writes `conversation_messages`, `conversation_activities`
//! and `conversation_usage`, each inside the Projector's transaction.

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use moatless_core::change_feed::conversation::{record_activity_change, record_message_change};

use super::ids;
use crate::activities::mapping::{ActivityRow, AdmittedUsage, TextPart, ToolOutcome, detail};

/// The one row a conversation row was projected from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Source<'a> {
    Inbox(Uuid),
    Line(Uuid),
    TaskEvent(&'a str),
}

impl<'a> Source<'a> {
    fn inbox(self) -> Option<Uuid> {
        match self {
            Self::Inbox(id) => Some(id),
            _ => None,
        }
    }

    fn line(self) -> Option<Uuid> {
        match self {
            Self::Line(id) => Some(id),
            _ => None,
        }
    }

    fn task_event(self) -> Option<&'a str> {
        match self {
            Self::TaskEvent(id) => Some(id),
            _ => None,
        }
    }
}

/// A Message as the Projector writes it; see 4.8 for which author takes which
/// source.
#[derive(Debug, Clone)]
pub(crate) struct MessageRow<'a> {
    pub uuid: Uuid,
    pub task_id: &'a str,
    pub author: &'static str,
    pub kind: &'static str,
    pub text: Option<&'a str>,
    pub parent_tool_use_id: Option<&'a str>,
    pub turn_number: Option<i32>,
    pub harness: Option<&'a str>,
    pub model: Option<&'a str>,
    pub usage: Option<Value>,
    pub source: Source<'a>,
}

/// Writes `row`, or on its second write keeps its source and position and fills
/// what it lacked. Returns the position its activities take: a new change, or
/// the next slot of its subagent transcript.
pub(crate) async fn upsert_message(
    tx: &mut PgConnection,
    row: &MessageRow<'_>,
) -> Result<i64, sqlx::Error> {
    let position = position_for(tx, row.task_id, row.parent_tool_use_id, row.uuid).await?;
    sqlx::query(
        r#"INSERT INTO conversation_messages
               (uuid, task_id, position, author, kind, text, parent_tool_use_id, turn_number,
                harness, model, usage, inbox_id, trajectory_event_id, task_event_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
           ON CONFLICT (uuid) DO UPDATE SET
               turn_number = COALESCE(EXCLUDED.turn_number, conversation_messages.turn_number),
               model       = COALESCE(conversation_messages.model, EXCLUDED.model),
               text        = COALESCE(EXCLUDED.text, conversation_messages.text),
               usage       = COALESCE(EXCLUDED.usage, conversation_messages.usage)"#,
    )
    .bind(row.uuid)
    .bind(row.task_id)
    .bind(position)
    .bind(row.author)
    .bind(row.kind)
    .bind(row.text)
    .bind(row.parent_tool_use_id)
    .bind(row.turn_number)
    .bind(row.harness)
    .bind(row.model)
    .bind(&row.usage)
    .bind(row.source.inbox())
    .bind(row.source.line())
    .bind(row.source.task_event())
    .execute(tx)
    .await?;
    Ok(position)
}

/// A new change naming `anchor` for the Task's own stream, or the next slot of
/// the subagent transcript `parent` names, which records no change.
pub(crate) async fn position_for(
    tx: &mut PgConnection,
    task_id: &str,
    parent: Option<&str>,
    anchor: Uuid,
) -> Result<i64, sqlx::Error> {
    match parent {
        None => record_message_change(tx, task_id, &anchor.to_string()).await,
        Some(parent) => {
            sqlx::query_scalar(
                r#"SELECT GREATEST(
                       (SELECT MAX(position) FROM conversation_messages
                         WHERE task_id = $1 AND parent_tool_use_id = $2),
                       (SELECT MAX(position) FROM conversation_activities
                         WHERE task_id = $1 AND parent_tool_use_id = $2),
                       0) + 1"#,
            )
            .bind(task_id)
            .bind(parent)
            .fetch_one(tx)
            .await
        }
    }
}

/// Splices text at each block's own index; a redelivered block lands in the
/// same slot. PostgreSQL subscripts from 1, and `array_to_string` skips the
/// NULL slots of blocks that are not text.
pub(crate) async fn splice_text(
    tx: &mut PgConnection,
    uuid: Uuid,
    parts: &[TextPart],
) -> Result<(), sqlx::Error> {
    for part in parts {
        sqlx::query("UPDATE conversation_messages SET text_parts[$2] = $3 WHERE uuid = $1")
            .bind(uuid)
            .bind(i32::try_from(part.block_index).unwrap_or(i32::MAX) + 1)
            .bind(&part.text)
            .execute(&mut *tx)
            .await?;
    }
    if !parts.is_empty() {
        sqlx::query(
            r#"UPDATE conversation_messages
                  SET text = array_to_string(text_parts, E'\n\n')
                WHERE uuid = $1"#,
        )
        .bind(uuid)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Writes one activity, its id hashed from `row.activity_id` (4.9). A
/// redelivery keeps its place and outcome; the call's own fields are re-applied.
pub(crate) async fn upsert_activity(
    tx: &mut PgConnection,
    position: i64,
    row: &ActivityRow,
    source: Source<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO conversation_activities (
               activity_id, task_id, position, ordinal, message_uuid, parent_tool_use_id,
               kind, status, tool_use_id, summary, payload, is_error,
               trajectory_event_id, inbox_id, task_event_id
           )
           SELECT $1, $2, $3, $4, $5::uuid, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
           WHERE NOT EXISTS (
               SELECT 1 FROM conversation_activities
                WHERE task_id = $2 AND tool_use_id = $9 AND activity_id <> $1
           )
           ON CONFLICT (activity_id) DO UPDATE SET
               summary = EXCLUDED.summary,
               payload = EXCLUDED.payload || jsonb_strip_nulls(jsonb_build_object(
                             'status',      conversation_activities.payload -> 'status',
                             'result',      conversation_activities.payload -> 'result',
                             'typedResult', conversation_activities.payload -> 'typedResult'
                         ))"#,
    )
    .bind(ids::activity(&row.activity_id))
    .bind(&row.task_id)
    .bind(position)
    .bind(row.ordinal)
    .bind(&row.message_uuid)
    .bind(&row.parent_uuid)
    .bind(row.kind.as_str())
    .bind(row.status.as_str())
    .bind(&row.tool_use_id)
    .bind(&row.summary)
    .bind(&row.payload)
    .bind(row.is_error)
    .bind(source.line())
    .bind(source.inbox())
    .bind(source.task_event())
    .execute(tx)
    .await?;
    Ok(())
}

/// Settles the call `outcome` answers, by `(task_id, tool_use_id)`. A frozen
/// call settles only if the turn open at the freeze wrote it
/// (`conversation_frozen.open_since`). No match is not an error.
pub(crate) async fn settle(
    tx: &mut PgConnection,
    outcome: &ToolOutcome,
    result_event_id: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    let settled = sqlx::query(
        r#"UPDATE conversation_activities a SET
               status          = $3,
               is_error        = $4,
               detail          = $8,
               result_event_id = $9,
               payload         = payload
                                   || jsonb_build_object('status', $5::text)
                                   || jsonb_build_object('result', $6::jsonb)
                                   || jsonb_strip_nulls(jsonb_build_object('typedResult', $7::jsonb))
           WHERE a.task_id = $1 AND a.tool_use_id = $2
             AND (NOT a.frozen OR a.created_at >= (
                     SELECT f.open_since FROM conversation_frozen f WHERE f.task_id = a.task_id))"#,
    )
    .bind(&outcome.task_id)
    .bind(&outcome.tool_use_id)
    .bind(outcome.status.as_str())
    .bind(outcome.is_error)
    .bind(outcome.status.wire())
    .bind(&outcome.result)
    .bind(&outcome.typed_result)
    .bind(detail(&outcome.result))
    .bind(result_event_id)
    .execute(tx)
    .await?
    .rows_affected();
    Ok(settled > 0)
}

/// Admits the usage `event_id` reported, once per model response: a Claude
/// response repeats its usage on each of its lines, so a later line of the same
/// `item` in the session admits nothing.
pub(crate) async fn admit_usage(
    tx: &mut PgConnection,
    task_id: &str,
    event_id: Uuid,
    item: Option<&str>,
    usage: AdmittedUsage,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO conversation_usage
               (trajectory_event_id, task_id, input_tokens, output_tokens,
                cache_creation_input_tokens, cache_read_input_tokens)
           SELECT $1, $2, $4, $5, $6, $7
            WHERE $3::text IS NULL OR NOT EXISTS (
                SELECT 1 FROM conversation_usage u
                  JOIN trajectory_events e ON e.id = u.trajectory_event_id
                  JOIN trajectory_events this ON this.id = $1
                 WHERE u.task_id = $2 AND e.session_id = this.session_id
                   AND e.harness_item_id = $3)
           ON CONFLICT (task_id, trajectory_event_id) DO NOTHING"#,
    )
    .bind(event_id)
    .bind(task_id)
    .bind(item)
    .bind(usage.input_tokens)
    .bind(usage.output_tokens)
    .bind(usage.cache_creation_input_tokens)
    .bind(usage.cache_read_input_tokens)
    .execute(tx)
    .await?;
    Ok(())
}

/// The usage a Task's own lines reported in `turn_number`, summed, as the turn
/// result Message stores it.
pub(crate) async fn turn_usage(
    tx: &mut PgConnection,
    task_id: &str,
    turn_number: i32,
) -> Result<Option<Value>, sqlx::Error> {
    let sums: Option<(i64, i64, i64, i64)> = sqlx::query_as(
        r#"SELECT SUM(u.input_tokens)::bigint, SUM(u.output_tokens)::bigint,
                  SUM(u.cache_creation_input_tokens)::bigint,
                  SUM(u.cache_read_input_tokens)::bigint
             FROM conversation_usage u
             JOIN trajectory_events e ON e.id = u.trajectory_event_id
             JOIN trajectory_sessions s ON s.id = e.session_id
            WHERE u.task_id = $1 AND e.turn_number = $2 AND s.parent_event_id IS NULL
           HAVING COUNT(*) > 0"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .fetch_optional(tx)
    .await?;
    Ok(sums.map(|(input, output, cache_write, cache_read)| {
        serde_json::json!({
            "input_tokens": input,
            "output_tokens": output,
            "cache_creation_input_tokens": cache_write,
            "cache_read_input_tokens": cache_read,
        })
    }))
}

/// Writes an activity on no Message, in a change of its own. `false` when its
/// id is already written.
pub(crate) async fn standalone_activity(
    tx: &mut PgConnection,
    row: &ActivityRow,
    task_event_id: &str,
) -> Result<bool, sqlx::Error> {
    let id = ids::activity(&row.activity_id);
    let position = record_activity_change(tx, &row.task_id, &id.to_string()).await?;
    let written = sqlx::query(
        r#"INSERT INTO conversation_activities (
               activity_id, task_id, position, ordinal, kind, status, summary, payload,
               is_error, task_event_id
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           ON CONFLICT (activity_id) DO NOTHING"#,
    )
    .bind(id)
    .bind(&row.task_id)
    .bind(position)
    .bind(row.ordinal)
    .bind(row.kind.as_str())
    .bind(row.status.as_str())
    .bind(&row.summary)
    .bind(&row.payload)
    .bind(row.is_error)
    .bind(task_event_id)
    .execute(tx)
    .await?
    .rows_affected();
    Ok(written > 0)
}

/// Whether `row` restates the Task's latest Sandbox activity, and so is not
/// written; checked before a position is spent.
pub(crate) async fn restates_sandbox(
    tx: &mut PgConnection,
    row: &ActivityRow,
) -> Result<bool, sqlx::Error> {
    let previous: Option<(String, bool)> = sqlx::query_as(
        r#"SELECT summary, is_error FROM conversation_activities
            WHERE task_id = $1 AND kind IN ('sandbox_status', 'sandbox_warning', 'sandbox_error')
            ORDER BY position DESC
            LIMIT 1"#,
    )
    .bind(&row.task_id)
    .fetch_optional(tx)
    .await?;
    Ok(previous
        .is_some_and(|(summary, is_error)| summary == row.summary && is_error == row.is_error))
}

/// Sets the turn on the Messages of delivered inbox items and records a change
/// for each; their positions do not move. Returns their uuids.
pub(crate) async fn stamp_delivered(
    tx: &mut PgConnection,
    task_id: &str,
    inbox_ids: &[Uuid],
    turn_number: i32,
) -> Result<(), sqlx::Error> {
    let stamped: Vec<Uuid> = sqlx::query_scalar(
        r#"UPDATE conversation_messages SET turn_number = $3
            WHERE task_id = $1 AND inbox_id = ANY($2) AND NOT frozen
        RETURNING uuid"#,
    )
    .bind(task_id)
    .bind(inbox_ids)
    .bind(turn_number)
    .fetch_all(&mut *tx)
    .await?;
    for uuid in stamped {
        record_message_change(tx, task_id, &uuid.to_string()).await?;
    }
    Ok(())
}

/// Deletes a projected Message and the activities on it. A live reader that
/// already holds it keeps it until it reloads; see `docs/deferred-changes.md`.
pub(crate) async fn delete_message(
    tx: &mut PgConnection,
    task_id: &str,
    uuid: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"DELETE FROM conversation_activities
            WHERE task_id = $1 AND message_uuid = $2 AND NOT frozen"#,
    )
    .bind(task_id)
    .bind(uuid)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM conversation_messages
            WHERE task_id = $1 AND uuid = $2 AND NOT frozen"#,
    )
    .bind(task_id)
    .bind(uuid)
    .execute(tx)
    .await?;
    Ok(())
}

/// The Message a tool call is on, and its tool name.
pub(crate) async fn call_in(
    tx: &mut PgConnection,
    task_id: &str,
    tool_use_id: &str,
) -> Result<Option<(Option<String>, String)>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT message_uuid::text, summary FROM conversation_activities
            WHERE task_id = $1 AND tool_use_id = $2"#,
    )
    .bind(task_id)
    .bind(tool_use_id)
    .fetch_optional(tx)
    .await
}

/// The first line of `item` in `session_id`, which names the Message its run
/// folds into (4.9).
pub(crate) async fn first_line_of_item(
    tx: &mut PgConnection,
    session_id: Uuid,
    item: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT id FROM trajectory_events
            WHERE session_id = $1 AND harness_item_id = $2
            ORDER BY seq
            LIMIT 1"#,
    )
    .bind(session_id)
    .bind(item)
    .fetch_optional(tx)
    .await
}

/// Deletes a Task's rows: every projected one, leaving frozen ones, which no
/// source reproduces; or, for a rewind, every one of a turn after
/// `after_turn`, frozen or not, keeping queued Messages (no turn yet) and
/// Sandbox activities. Returns the Messages it deleted.
pub(crate) async fn delete_projected(
    tx: &mut PgConnection,
    task_id: &str,
    after_turn: Option<i32>,
) -> Result<u64, sqlx::Error> {
    // Before the Messages: a frozen activity or usage row is cut by the
    // frozen Message it hangs on.
    sqlx::query(
        r#"DELETE FROM conversation_usage u
            WHERE u.task_id = $1
              AND CASE WHEN $2::int IS NULL THEN u.trajectory_event_id IS NOT NULL
                  ELSE EXISTS (SELECT 1 FROM trajectory_events e
                                WHERE e.id = u.trajectory_event_id AND e.turn_number > $2)
                    OR EXISTS (SELECT 1 FROM conversation_messages m
                                WHERE m.uuid = u.message_uuid AND m.turn_number > $2)
                  END"#,
    )
    .bind(task_id)
    .bind(after_turn)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM conversation_activities a
            WHERE a.task_id = $1
              AND CASE WHEN $2::int IS NULL THEN NOT a.frozen
                  ELSE EXISTS (SELECT 1 FROM trajectory_events e
                                WHERE e.id = a.trajectory_event_id AND e.turn_number > $2)
                    OR (a.kind = 'context_window_updated' AND EXISTS (
                            SELECT 1 FROM task_events t
                             WHERE t.event_id = a.task_event_id AND t.turn_number > $2))
                    OR EXISTS (SELECT 1 FROM task_inbox i
                                WHERE i.id = a.inbox_id AND i.turn_number > $2)
                    OR (a.frozen AND EXISTS (
                            SELECT 1 FROM conversation_messages m
                             WHERE m.uuid = a.message_uuid AND m.turn_number > $2))
                  END"#,
    )
    .bind(task_id)
    .bind(after_turn)
    .execute(&mut *tx)
    .await?;
    let deleted = sqlx::query(
        r#"DELETE FROM conversation_messages m
            WHERE m.task_id = $1
              AND CASE WHEN $2::int IS NULL THEN NOT m.frozen
                  ELSE m.turn_number > $2 END"#,
    )
    .bind(task_id)
    .bind(after_turn)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(deleted)
}

/// A frozen row's copy into a fork: the row it copies, its id and position
/// there, and for an activity the copied Message it is on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrozenCopy {
    pub from: Uuid,
    pub to: Uuid,
    pub position: i64,
    pub message: Option<Uuid>,
}

/// Copies `source_task_id`'s frozen `messages` and `activities` into
/// `fork_task_id`, still frozen, and the usage rows naming a Message in
/// `usage` (`from`, `to`). A copy already there is kept.
pub(crate) async fn copy_frozen(
    tx: &mut PgConnection,
    fork_task_id: &str,
    source_task_id: &str,
    messages: &[FrozenCopy],
    activities: &[FrozenCopy],
    usage: &[(Uuid, Uuid)],
) -> Result<(), sqlx::Error> {
    let column = |rows: &[FrozenCopy], pick: fn(&FrozenCopy) -> Uuid| -> Vec<Uuid> {
        rows.iter().map(pick).collect()
    };
    let positions = |rows: &[FrozenCopy]| -> Vec<i64> { rows.iter().map(|c| c.position).collect() };
    sqlx::query(
        r#"INSERT INTO conversation_messages
               (uuid, task_id, position, author, kind, text, text_parts, parent_tool_use_id,
                turn_number, harness, model, usage, inbox_id, frozen, legacy, created_at)
           SELECT c.to_id, $1, c.position, m.author, m.kind, m.text, m.text_parts,
                  m.parent_tool_use_id, m.turn_number, m.harness, m.model, m.usage, m.inbox_id,
                  true, m.legacy, m.created_at
             FROM unnest($2::uuid[], $3::uuid[], $4::bigint[]) AS c(from_id, to_id, position)
             JOIN conversation_messages m ON m.uuid = c.from_id AND m.task_id = $5
           ON CONFLICT DO NOTHING"#,
    )
    .bind(fork_task_id)
    .bind(column(messages, |c| c.from))
    .bind(column(messages, |c| c.to))
    .bind(positions(messages))
    .bind(source_task_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO conversation_activities
               (activity_id, task_id, position, ordinal, message_uuid, parent_tool_use_id, kind,
                status, tool_use_id, summary, payload, detail, is_error, frozen, created_at)
           SELECT c.to_id, $1, c.position, a.ordinal, c.message_uuid, a.parent_tool_use_id,
                  a.kind, a.status, a.tool_use_id, a.summary, a.payload, a.detail, a.is_error,
                  true, a.created_at
             FROM unnest($2::uuid[], $3::uuid[], $4::bigint[], $5::uuid[])
                  AS c(from_id, to_id, position, message_uuid)
             JOIN conversation_activities a ON a.activity_id = c.from_id AND a.task_id = $6
           ON CONFLICT DO NOTHING"#,
    )
    .bind(fork_task_id)
    .bind(column(activities, |c| c.from))
    .bind(column(activities, |c| c.to))
    .bind(positions(activities))
    .bind(activities.iter().map(|c| c.message).collect::<Vec<_>>())
    .bind(source_task_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO conversation_usage
               (message_uuid, task_id, parent_tool_use_id, input_tokens, output_tokens,
                cache_creation_input_tokens, cache_read_input_tokens)
           SELECT c.to_id, $1, u.parent_tool_use_id, u.input_tokens, u.output_tokens,
                  u.cache_creation_input_tokens, u.cache_read_input_tokens
             FROM unnest($2::uuid[], $3::uuid[]) AS c(from_id, to_id)
             JOIN conversation_usage u ON u.message_uuid = c.from_id AND u.task_id = $4
           ON CONFLICT DO NOTHING"#,
    )
    .bind(fork_task_id)
    .bind(usage.iter().map(|(from, _)| *from).collect::<Vec<_>>())
    .bind(usage.iter().map(|(_, to)| *to).collect::<Vec<_>>())
    .bind(source_task_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Whether `task_id` holds a frozen row.
pub(crate) async fn holds_frozen(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM conversation_messages WHERE task_id = $1 AND frozen)
               OR EXISTS (SELECT 1 FROM conversation_activities WHERE task_id = $1 AND frozen)"#,
    )
    .bind(task_id)
    .fetch_one(tx)
    .await
}

/// Deletes a deleted Task's conversation, and its inbox items no other Task's
/// conversation reads: a fork's inherited Message outlives its ancestor.
pub(crate) async fn delete_task(tx: &mut PgConnection, task_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM conversation_usage WHERE task_id = $1")
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM conversation_activities WHERE task_id = $1")
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM conversation_messages WHERE task_id = $1")
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"DELETE FROM task_inbox i
            WHERE i.task_id = $1
              AND NOT EXISTS (SELECT 1 FROM conversation_messages m WHERE m.inbox_id = i.id)
              AND NOT EXISTS (SELECT 1 FROM conversation_activities a WHERE a.inbox_id = i.id)"#,
    )
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Whether a row of `task_id`'s conversation came from `event_id`.
pub(crate) async fn event_projected(
    tx: &mut PgConnection,
    task_id: &str,
    event_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM conversation_messages
                           WHERE task_id = $1 AND task_event_id = $2)
               OR EXISTS (SELECT 1 FROM conversation_activities
                           WHERE task_id = $1 AND task_event_id = $2)"#,
    )
    .bind(task_id)
    .bind(event_id)
    .fetch_one(tx)
    .await
}

/// A frozen Message as a test seeds one, without a source to project it from.
#[cfg(any(test, feature = "test-support"))]
pub(crate) struct FrozenSeed<'a> {
    pub uuid: Uuid,
    pub task_id: &'a str,
    pub author: &'static str,
    pub kind: &'static str,
    pub text: Option<&'a str>,
    pub parent_tool_use_id: Option<&'a str>,
    pub turn_number: Option<i32>,
    pub model: Option<&'a str>,
    pub usage: Option<Value>,
    pub inbox_id: Option<Uuid>,
    pub legacy: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Writes `seed` at the next position of its stream; returns the position.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn seed_frozen_message(
    tx: &mut PgConnection,
    seed: &FrozenSeed<'_>,
) -> Result<i64, sqlx::Error> {
    let position = position_for(tx, seed.task_id, seed.parent_tool_use_id, seed.uuid).await?;
    sqlx::query(
        r#"INSERT INTO conversation_messages
               (uuid, task_id, position, author, kind, text, parent_tool_use_id, turn_number,
                model, usage, inbox_id, frozen, legacy, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, true, $12, $13)"#,
    )
    .bind(seed.uuid)
    .bind(seed.task_id)
    .bind(position)
    .bind(seed.author)
    .bind(seed.kind)
    .bind(seed.text)
    .bind(seed.parent_tool_use_id)
    .bind(seed.turn_number)
    .bind(seed.model)
    .bind(&seed.usage)
    .bind(seed.inbox_id)
    .bind(&seed.legacy)
    .bind(seed.created_at)
    .execute(tx)
    .await?;
    Ok(position)
}

/// The position of the Message `uuid` names.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn message_position(
    tx: &mut PgConnection,
    uuid: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT position FROM conversation_messages WHERE uuid = $1")
        .bind(uuid)
        .fetch_optional(tx)
        .await
}

/// Writes a frozen activity at `position`, on the Message `message_uuid`.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn seed_frozen_activity(
    tx: &mut PgConnection,
    position: i64,
    row: &ActivityRow,
    message_uuid: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO conversation_activities (
               activity_id, task_id, position, ordinal, message_uuid, parent_tool_use_id,
               kind, status, tool_use_id, summary, payload, is_error, frozen
           )
           SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, true
           WHERE NOT EXISTS (
               SELECT 1 FROM conversation_activities WHERE task_id = $2 AND tool_use_id = $9
           )
           ON CONFLICT (activity_id) DO NOTHING"#,
    )
    .bind(ids::activity(&row.activity_id))
    .bind(&row.task_id)
    .bind(position)
    .bind(row.ordinal)
    .bind(message_uuid)
    .bind(&row.parent_uuid)
    .bind(row.kind.as_str())
    .bind(row.status.as_str())
    .bind(&row.tool_use_id)
    .bind(&row.summary)
    .bind(&row.payload)
    .bind(row.is_error)
    .execute(tx)
    .await?;
    Ok(())
}

/// Marks `task_id`'s history frozen, as the migration did; `open_since`
/// starts the turn then still open.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn seed_frozen_task(
    tx: &mut PgConnection,
    task_id: &str,
    open_since: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO conversation_frozen (task_id, open_turn, open_since)
           SELECT $1, t.turn_number, $2
             FROM (SELECT 1) one
             LEFT JOIN task_turns t ON t.task_id = $1 AND $2::timestamptz IS NOT NULL
                  AND t.turn_number = (SELECT max(turn_number) FROM task_turns WHERE task_id = $1)
           ON CONFLICT (task_id) DO UPDATE SET open_turn = EXCLUDED.open_turn,
                                               open_since = EXCLUDED.open_since"#,
    )
    .bind(task_id)
    .bind(open_since)
    .execute(tx)
    .await?;
    Ok(())
}

/// Settles a seeded call, frozen or not.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn settle_seeded(
    tx: &mut PgConnection,
    outcome: &ToolOutcome,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE conversation_activities SET
               status   = $3,
               is_error = $4,
               detail   = $7,
               payload  = payload
                            || jsonb_build_object('status', $5::text)
                            || jsonb_build_object('result', $6::jsonb)
                            || jsonb_strip_nulls(jsonb_build_object('typedResult', $8::jsonb))
            WHERE task_id = $1 AND tool_use_id = $2"#,
    )
    .bind(&outcome.task_id)
    .bind(&outcome.tool_use_id)
    .bind(outcome.status.as_str())
    .bind(outcome.is_error)
    .bind(outcome.status.wire())
    .bind(&outcome.result)
    .bind(detail(&outcome.result))
    .bind(&outcome.typed_result)
    .execute(tx)
    .await?;
    Ok(())
}

/// Writes a frozen Message's usage row.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn seed_frozen_usage(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: Uuid,
    parent_tool_use_id: Option<&str>,
    usage: AdmittedUsage,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO conversation_usage
               (message_uuid, task_id, parent_tool_use_id, input_tokens, output_tokens,
                cache_creation_input_tokens, cache_read_input_tokens)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(message_uuid)
    .bind(task_id)
    .bind(parent_tool_use_id)
    .bind(usage.input_tokens)
    .bind(usage.output_tokens)
    .bind(usage.cache_creation_input_tokens)
    .bind(usage.cache_read_input_tokens)
    .execute(tx)
    .await?;
    Ok(())
}
