//! Every read of `conversation_messages` and `conversation_usage`: the
//! records [`super::assemble`] turns into `TaskMessage`s, and usage.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;
use moatless_ports::activity::SubagentUsage;
use moatless_types::agent::AgentMode;
use serde_json::Value;
use uuid::Uuid;

use crate::inbox::InboxItem;
use crate::inbox::models::InboxRow;

/// A Message row with what it is read with: its line's session and item, and
/// its task event's sub-type and payload.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MessageRecord {
    pub uuid: Uuid,
    pub task_id: String,
    pub position: i64,
    pub author: String,
    pub kind: String,
    pub text: Option<String>,
    pub parent_tool_use_id: Option<String>,
    pub turn_number: Option<i32>,
    pub model: Option<String>,
    pub usage: Option<Value>,
    pub inbox_id: Option<Uuid>,
    pub frozen: bool,
    pub legacy: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub harness_session_id: Option<String>,
    pub harness_item_id: Option<String>,
    pub event_sub_type: Option<String>,
    pub event_payload: Option<Value>,
}

/// The `SELECT` every Message read runs, narrowed by `$predicate` over `m`.
macro_rules! read_messages {
    ($predicate:literal) => {
        concat!(
            "SELECT m.uuid, m.task_id, m.position, m.author, m.kind, m.text, ",
            "m.parent_tool_use_id, m.turn_number, m.model, m.usage, m.inbox_id, m.frozen, ",
            "m.legacy, m.created_at, s.harness_session_id, e.harness_item_id, ",
            "t.sub_type AS event_sub_type, t.payload::jsonb AS event_payload ",
            "FROM conversation_messages m ",
            "LEFT JOIN trajectory_events e ON e.id = m.trajectory_event_id ",
            "LEFT JOIN trajectory_sessions s ON s.id = e.session_id ",
            "LEFT JOIN task_events t ON t.event_id = m.task_event_id ",
            "WHERE ",
            $predicate
        )
    };
}

/// Every Message of a Task, subagent transcripts included, in stream order.
pub(crate) async fn all(pool: &DbPool, task_id: &str) -> Result<Vec<MessageRecord>, sqlx::Error> {
    sqlx::query_as(read_messages!(
        "m.task_id = $1 ORDER BY m.parent_tool_use_id NULLS FIRST, m.position"
    ))
    .bind(task_id)
    .fetch_all(pool)
    .await
}

/// The Message `uuid` names.
pub(crate) async fn by_uuid(
    pool: &DbPool,
    uuid: Uuid,
) -> Result<Option<MessageRecord>, sqlx::Error> {
    sqlx::query_as(read_messages!("m.uuid = $1"))
        .bind(uuid)
        .fetch_optional(pool)
        .await
}

/// The newest top-level Messages before `before` (all when `None`), newest
/// first.
pub(crate) async fn page(
    pool: &DbPool,
    task_id: &str,
    before: Option<i64>,
    limit: i32,
) -> Result<Vec<MessageRecord>, sqlx::Error> {
    sqlx::query_as(read_messages!(
        "m.task_id = $1 AND m.parent_tool_use_id IS NULL \
         AND ($2::bigint IS NULL OR m.position < $2) \
         ORDER BY m.position DESC LIMIT $3"
    ))
    .bind(task_id)
    .bind(before)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
}

/// The newest top-level Message by `author` and `kinds`; every author and kind
/// when `None`.
pub(crate) async fn latest(
    pool: &DbPool,
    task_id: &str,
    author: Option<&str>,
    kinds: Option<&[&str]>,
) -> Result<Option<MessageRecord>, sqlx::Error> {
    sqlx::query_as(read_messages!(
        "m.task_id = $1 AND m.parent_tool_use_id IS NULL \
         AND ($2::text IS NULL OR m.author = $2) \
         AND ($3::text[] IS NULL OR m.kind = ANY($3)) \
         ORDER BY m.position DESC LIMIT 1"
    ))
    .bind(task_id)
    .bind(author)
    .bind(kinds)
    .fetch_optional(pool)
    .await
}

/// One subagent transcript's Messages, behind the call `parent` names.
pub(crate) async fn subagent(
    pool: &DbPool,
    task_id: &str,
    parent: &str,
) -> Result<Vec<MessageRecord>, sqlx::Error> {
    sqlx::query_as(read_messages!(
        "m.task_id = $1 AND m.parent_tool_use_id = $2 ORDER BY m.position"
    ))
    .bind(task_id)
    .bind(parent)
    .fetch_all(pool)
    .await
}

/// The inbox items `ids` names, by id.
pub(crate) async fn inbox_items(
    pool: &DbPool,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, InboxItem>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<InboxRow> = sqlx::query_as("SELECT * FROM task_inbox WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|row| InboxItem::try_from(row).map(|item| (item.id, item)))
        .collect()
}

/// The agent mode of the newest user Message at or before `position`, which an
/// agent Message after it ran under.
pub(crate) async fn agent_mode_at(
    pool: &DbPool,
    task_id: &str,
    position: i64,
) -> Result<Option<AgentMode>, sqlx::Error> {
    let mode: Option<Option<String>> = sqlx::query_scalar(
        r#"SELECT COALESCE(i.delivery_options ->> 'agentMode', m.legacy ->> 'agent_mode')
             FROM conversation_messages m
             LEFT JOIN task_inbox i ON i.id = m.inbox_id
            WHERE m.task_id = $1 AND m.author = 'user' AND m.parent_tool_use_id IS NULL
              AND m.position <= $2
            ORDER BY m.position DESC LIMIT 1"#,
    )
    .bind(task_id)
    .bind(position)
    .fetch_optional(pool)
    .await?;
    Ok(mode.flatten().map(AgentMode::from))
}

/// See [`crate::message::dao::MessageDao::latest_agent_session_id`].
pub(crate) async fn latest_agent_session_id(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT COALESCE(s.harness_session_id, i.session_id, m.legacy ->> 'agent_session_id')
             FROM conversation_messages m
             LEFT JOIN trajectory_events e ON e.id = m.trajectory_event_id
             LEFT JOIN trajectory_sessions s ON s.id = e.session_id
             LEFT JOIN task_inbox i ON i.id = m.inbox_id
            WHERE m.task_id = $1 AND m.parent_tool_use_id IS NULL
              AND COALESCE(s.harness_session_id, i.session_id, m.legacy ->> 'agent_session_id')
                  IS NOT NULL
            ORDER BY m.position DESC LIMIT 1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
}

/// Each Task's newest Message time.
pub(crate) async fn latest_times(
    pool: &DbPool,
    task_ids: &[String],
) -> Result<HashMap<String, DateTime<Utc>>, sqlx::Error> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT task_id, MAX(created_at) FROM conversation_messages
            WHERE task_id = ANY($1) GROUP BY task_id"#,
    )
    .bind(task_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Per spawning call among `calls`: how many model responses its subagent
/// reported usage for, and their input and output tokens.
pub(crate) async fn subagent_usage(
    pool: &DbPool,
    task_id: &str,
    calls: &[String],
) -> Result<Vec<SubagentUsage>, sqlx::Error> {
    if calls.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        r#"SELECT parent, COUNT(*)::bigint,
                  COALESCE(SUM(input_tokens), 0)::bigint,
                  COALESCE(SUM(output_tokens), 0)::bigint
             FROM (
                 SELECT u.parent_tool_use_id AS parent, u.input_tokens, u.output_tokens
                   FROM conversation_usage u
                  WHERE u.task_id = $1 AND u.trajectory_event_id IS NULL
                 UNION ALL
                 SELECT (SELECT b ->> 'id'
                           FROM trajectory_events p,
                                jsonb_array_elements(COALESCE(p.content -> 'blocks', '[]'::jsonb)) b
                          WHERE p.id = s.parent_event_id AND b ->> 'type' = 'tool_use'
                          LIMIT 1),
                        u.input_tokens, u.output_tokens
                   FROM conversation_usage u
                   JOIN trajectory_events e ON e.id = u.trajectory_event_id
                   JOIN trajectory_sessions s ON s.id = e.session_id
                  WHERE u.task_id = $1 AND s.parent_event_id IS NOT NULL
             ) usage
            WHERE parent = ANY($2)
            GROUP BY parent"#,
    )
    .bind(task_id)
    .bind(calls)
    .fetch_all(pool)
    .await
    .map(|rows: Vec<(String, i64, i64, i64)>| {
        rows.into_iter()
            .map(
                |(tool_use_id, responses, input_tokens, output_tokens)| SubagentUsage {
                    tool_use_id,
                    responses,
                    input_tokens,
                    output_tokens,
                },
            )
            .collect()
    })
}

/// One model response of a Task's own agent, as `conversation_usage` admitted
/// it once. `message_uuid` is a Message of that response, or else the latest
/// agent Message before its usage line in the same session and turn, since a
/// Codex `token_count` line opens none; `None` when that turn opened none.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResponseUsage {
    pub message_uuid: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub turn_number: Option<i32>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
}

type UsageRow = (
    Option<Uuid>,
    DateTime<Utc>,
    Option<i32>,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
);

/// The usage of every response the Task's own agent made, oldest first;
/// subagent sessions' responses are left out.
pub(crate) async fn response_usage(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<ResponseUsage>, sqlx::Error> {
    // A Codex usage line names no model — the session's latest one up to it
    // does. `COALESCE` runs the subquery only when prior args are NULL; the
    // turn split uses NULL because `IS NOT DISTINCT FROM` isn't an index
    // condition — either way, `trajectory_events_session_turn_seq_idx` gets
    // scanned.
    let rows: Vec<UsageRow> = sqlx::query_as(
        r#"SELECT COALESCE(
                      u.message_uuid,
                      (SELECT rm.uuid FROM trajectory_events re
                         JOIN conversation_messages rm ON rm.trajectory_event_id = re.id
                        WHERE re.session_id = e.session_id
                          AND re.harness_item_id = e.harness_item_id
                          AND re.turn_number IS NOT DISTINCT FROM e.turn_number
                          AND rm.task_id = u.task_id AND rm.author = 'agent'
                        ORDER BY re.seq DESC LIMIT 1),
                      CASE WHEN e.turn_number IS NULL THEN
                          (SELECT rm.uuid FROM trajectory_events re
                             JOIN conversation_messages rm ON rm.trajectory_event_id = re.id
                            WHERE re.session_id = e.session_id AND re.turn_number IS NULL
                              AND re.seq <= e.seq
                              AND rm.task_id = u.task_id AND rm.author = 'agent'
                            ORDER BY re.seq DESC LIMIT 1)
                      ELSE
                          (SELECT rm.uuid FROM trajectory_events re
                             JOIN conversation_messages rm ON rm.trajectory_event_id = re.id
                            WHERE re.session_id = e.session_id
                              AND re.turn_number = e.turn_number AND re.seq <= e.seq
                              AND rm.task_id = u.task_id AND rm.author = 'agent'
                            ORDER BY re.seq DESC LIMIT 1)
                      END),
                  COALESCE(e.created_at, m.created_at, now()),
                  COALESCE(e.turn_number, m.turn_number),
                  COALESCE(
                      (SELECT p.content ->> 'model' FROM trajectory_events p
                        WHERE p.session_id = e.session_id AND p.seq <= e.seq
                          AND p.content ? 'model'
                        ORDER BY p.seq DESC LIMIT 1),
                      m.model),
                  u.input_tokens::bigint, u.output_tokens::bigint,
                  u.cache_creation_input_tokens::bigint, u.cache_read_input_tokens::bigint
             FROM conversation_usage u
             LEFT JOIN trajectory_events e ON e.id = u.trajectory_event_id
             LEFT JOIN trajectory_sessions s ON s.id = e.session_id
             LEFT JOIN conversation_messages m ON m.uuid = u.message_uuid
            WHERE u.task_id = $1 AND u.parent_tool_use_id IS NULL
              AND (u.trajectory_event_id IS NULL OR s.parent_event_id IS NULL)
            ORDER BY 3, e.seq, m.position"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(message_uuid, created_at, turn_number, model, i, o, cc, cr)| ResponseUsage {
                message_uuid,
                created_at,
                turn_number,
                model,
                input_tokens: i,
                output_tokens: o,
                cache_creation_input_tokens: cc,
                cache_read_input_tokens: cr,
            },
        )
        .collect())
}

/// Whether a Message of `task_id`'s conversation attached `file_id`.
pub(crate) async fn file_attached(
    pool: &DbPool,
    task_id: &str,
    file_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM conversation_messages m JOIN task_inbox i ON i.id = m.inbox_id
                WHERE m.task_id = $1 AND $2 = ANY(i.file_ids))
               OR EXISTS (
               SELECT 1 FROM task_inbox i WHERE i.task_id = $1 AND $2 = ANY(i.file_ids))"#,
    )
    .bind(task_id)
    .bind(file_id)
    .fetch_one(pool)
    .await
}

/// Whether any Task's inbox item carries `file_id`, by id or as a `file`
/// context.
pub(crate) async fn file_on_any_message(pool: &DbPool, file_id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS (
               SELECT 1 FROM task_inbox i
                WHERE $1 = ANY(i.file_ids)
                   OR EXISTS (SELECT 1 FROM jsonb_array_elements(
                                  COALESCE(i.delivery_options -> 'contexts', '[]'::jsonb)) c
                               WHERE c ->> 'referenceId' = $1 AND c ->> 'type' = 'file'))"#,
    )
    .bind(file_id)
    .fetch_one(pool)
    .await
}
