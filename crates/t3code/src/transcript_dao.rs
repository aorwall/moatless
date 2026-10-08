//! Opening one conversation: what was said, what happened on the way, and the
//! rows a live change names.
//!
//! Both reads select the same columns into the same row type, because a column
//! on one and not the other is a field that is present on reload and absent on
//! the frame that delivered it.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use moatless_ports::activity::ActivitySource;
use moatless_types::activity::{Activity, ActivityKind};
use moatless_types::message::{MessageRole, MessageType};
use serde_json::Value;
use sqlx::AssertSqlSafe;

use crate::UiRpcDbPool;
use crate::activity_dao::{self, SubagentUsage};
use crate::dao::store_read;

/// One Message of a Task's transcript, with the columns a transcript is
/// rendered from and nothing else.
///
/// Deliberately not the backend's `TaskMessage`, whose forty write-model
/// columns would pull every tool result's raw payload and every run's metrics
/// out of the database to project six fields.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TranscriptMessage {
    /// The insertion order this transcript is assembled in.
    // Selected by `TRANSCRIPT_COLUMNS` and ordered on in SQL; the wire carries
    // `uuid`.
    #[allow(dead_code)]
    pub id: i32,
    pub uuid: String,
    #[sqlx(try_from = "String")]
    pub message_type: MessageType,
    // Projected from `message_type` and `user_type`, not from this column.
    #[allow(dead_code)]
    #[sqlx(try_from = "String")]
    pub message_role: MessageRole,
    /// What was said, and only that. A user Message holds the person's own
    /// text: the files, contexts and attribution the agent is sent are rebuilt
    /// at delivery time and were never part of this column.
    pub message_text: Option<String>,
    /// Which kind of `user` row this is. The agent's tool responses are
    /// persisted as `user` too, and are not something a person said.
    pub user_type: String,
    /// A `user` Message the sandbox wrote to wake its own agent, which nobody
    /// said. Its inbox item's `origin` is `sandbox`.
    pub sandbox_wake: bool,
    /// The Turn this Message was acknowledged into. `None` while it is queued.
    pub turn_number: Option<i32>,
    /// Sent to wait for the active turn to end (`DeliveryOptions::queue`)
    /// rather than steer into it.
    pub held: bool,
    /// When the writer said it happened (RFC 3339 text, `NOT NULL`).
    pub timestamp: String,
    /// When the database recorded it. `None` is impossible under the current
    /// schema's default and is treated as a missing value rather than assumed.
    pub created_at: Option<DateTime<Utc>>,
    /// The adapter a Message reached this Task through — `slack`, `github_pr`,
    /// `linear`. `None` on a Message that came from inside Moatless.
    pub external_adapter: Option<String>,
    /// The name the subscription knows the source by — a channel, a repository.
    pub external_source_name: Option<String>,
    /// The binding identity this Message belongs to — the join key to its
    /// Binding, and for GitHub PR the `owner/repo#N` the pull-request URL is
    /// built from.
    pub external_subject_key: Option<String>,
    /// The adapter's own id for this event — `github:ic:{id}`, `slack:ts:{ts}`.
    /// Distinguishes a PR comment url from a review-comment url.
    pub external_event_id: Option<String>,
    /// The handle the external system knows the writer by. `None` on a Message
    /// written before the column existed.
    pub external_author: Option<String>,
    /// The Task a Message was authored by, when it came from `moat send
    /// --task`. Present means the origin is a Task rather than an adapter.
    pub source_task_id: Option<String>,
    /// The web address of the Binding this Message's subject key resolves to,
    /// joined in. `None` when no Binding matches or it records no url.
    pub binding_url: Option<String>,
    /// What the Binding calls its subject — a Linear issue identifier such as
    /// `SP-2454`. Preferred over `external_source_name`, which for Linear is
    /// the subscription's team id rather than anything a reader would
    /// recognise.
    pub binding_label: Option<String>,
    /// The display name of the Moatless user `sent_by` resolved to, joined in.
    /// The `user` fallback where no external author was recorded.
    pub sent_by_name: Option<String>,
    /// The uploads attached to this Message, joined in from its `file` context
    /// rows as a JSON array of `{referenceId, data}` — the upload's id and the
    /// filename, mime type and size resolved when it was sent.
    ///
    /// Read from the inbox item's contexts rather than its `file_ids`, which
    /// are ids alone; `None` on a Message with no uploads.
    pub file_contexts: Option<serde_json::Value>,
    /// `completed` or `errored` on the notice a child Task's run ending sent
    /// this, its parent; `None` on every other Message.
    pub child_outcome: Option<String>,
}

/// A Task's transcript, and whether it is all of it.
#[derive(Default)]
pub struct Transcript {
    /// Oldest first — the order a transcript is read in.
    pub messages: Vec<TranscriptMessage>,
    /// What happened on the way, oldest first. Not a subset of `messages`: a
    /// turn whose agent only called tools said nothing and has no Message row
    /// at all, and its calls reach a reader only through here.
    pub activities: Vec<Activity>,
    /// The rows past `TRANSCRIPT_LIMIT` that were left out, oldest first. Zero
    /// for every transcript that fits, which is nearly all of them.
    pub omitted: i64,
    /// Every `task_turns.turn_number` this Task has, ascending.
    ///
    /// Read from `task_turns` rather than off `messages`, since that is the set
    /// a rewind cuts against — a tool-only turn has no Message, and a turn
    /// older than `TRANSCRIPT_LIMIT` has none left here. Empty from
    /// [`crate::dao::ConversationDao::changed_messages`], whose caller projects
    /// no checkpoints.
    pub turns: Vec<i32>,
    /// How each of [`Self::turns`] ended, ascending. Read by [`transcript`]
    /// alone, and empty from every other read.
    pub turn_outcomes: Vec<TurnOutcome>,
    /// The turns a rewind cut out of this Task, ascending. Read by
    /// [`transcript`] alone, and empty from every other read.
    pub rewound_turns: Vec<RewoundTurn>,
    /// Of the requests these rows ask, the ones whose call has come back.
    ///
    /// A panel row sits *beside* the call rather than being it — the call's
    /// status is the whole answer — and a resumed session can rewrite the panel
    /// onto a different Message than the call's, so this is looked up by
    /// request id rather than read out of `activities`.
    pub settled_requests: HashSet<String>,
    /// What each subagent these rows spawned has spent, by its call's
    /// `tool_use_id`.
    ///
    /// Looked up rather than read out of the rows: a subagent's own Messages
    /// carry the spawning id as `parent_uuid` and are never among `activities`.
    pub subagent_usage: HashMap<String, SubagentUsage>,
}

/// One `task_turns` row's ending: its status, and for an `error` turn the
/// `agent.error` it ended with.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct TurnOutcome {
    pub turn_number: i32,
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub reason: Option<String>,
    /// RFC 3339 text, as the event carried it.
    pub resets_at: Option<String>,
}

/// A `task_rewound_turns` row: a turn a rewind cut, which T3 shows as a
/// rolled-back run.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RewoundTurn {
    pub turn_number: i32,
    pub started_at: DateTime<Utc>,
    pub rewound_at: DateTime<Utc>,
}

/// How many of a Task's Messages one transcript read returns.
///
/// T3's contract carries the whole transcript in one unpaginated snapshot, so
/// this cap is the only guard against a multi-megabyte frame; the **newest**
/// rows are kept, and what was dropped is reported via [`Transcript::omitted`]
/// rather than silently absent.
const TRANSCRIPT_LIMIT: i64 = 1_000;

/// A Task's top-level Messages in the columns a transcript reads, aliased
/// `m`: a user Message's text, origin and uploads are its inbox item's, a
/// frozen row's the columns it was frozen with. `$1` is the Task.
macro_rules! transcript_rows {
    () => {
        r#"SELECT m.position::int AS id, m.uuid::text AS uuid, m.task_id, m.position,
                  CASE WHEN m.frozen THEN m.legacy ->> 'message_type'
                       WHEN m.author = 'user' THEN 'user'
                       WHEN m.author = 'agent' THEN 'assistant'
                       WHEN m.kind = 'error' THEN 'error'
                       ELSE 'result' END AS message_type,
                  CASE WHEN m.frozen THEN m.legacy ->> 'message_role'
                       WHEN m.author = 'user' THEN 'user'
                       ELSE 'assistant' END AS message_role,
                  COALESCE(m.text, i.text) AS message_text,
                  CASE WHEN i.delivery_options ? 'toolResponse' THEN 'tool_response'
                       WHEN m.frozen THEN COALESCE(m.legacy ->> 'user_type', 'system')
                       WHEN m.author = 'user' THEN 'human'
                       ELSE 'system' END AS user_type,
                  COALESCE(i.origin = 'sandbox', false) AS sandbox_wake,
                  m.turn_number,
                  COALESCE((i.delivery_options ->> 'queue')::boolean, false) AS held,
                  COALESCE(m.legacy ->> 'timestamp',
                           to_char(m.created_at AT TIME ZONE 'UTC',
                                   'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')) AS "timestamp",
                  m.created_at,
                  COALESCE(i.adapter, m.legacy ->> 'external_adapter') AS external_adapter,
                  COALESCE(i.external_source_name, m.legacy ->> 'external_source_name')
                      AS external_source_name,
                  COALESCE(i.external_subject_key, m.legacy ->> 'external_subject_key')
                      AS external_subject_key,
                  -- An inherited Message claims no event of its ancestor's.
                  CASE WHEN i.task_id <> m.task_id THEN NULL
                       ELSE COALESCE(i.external_message_id, m.legacy ->> 'external_event_id')
                  END AS external_event_id,
                  COALESCE(i.external_author, m.legacy ->> 'external_author') AS external_author,
                  CASE WHEN i.task_id <> m.task_id THEN i.task_id
                       ELSE COALESCE(i.source_task_id, m.legacy ->> 'source_task_id')
                  END AS source_task_id,
                  COALESCE(i.sent_by, m.legacy ->> 'sent_by') AS sent_by,
                  i.delivery_options -> 'contexts' AS contexts,
                  i.delivery_options ->> 'childOutcome' AS child_outcome
             FROM conversation_messages m
             LEFT JOIN task_inbox i ON i.id = m.inbox_id
            WHERE m.task_id = $1 AND m.parent_tool_use_id IS NULL"#
    };
}

/// The columns a transcript read projects, over the capped set aliased `m` and
/// the two tables joined onto it.
///
/// Kept in one place because the snapshot read and the live-delta read must
/// select exactly the same shape into [`TranscriptMessage`] — a column on one
/// and not the other is present on reload and absent on the frame that
/// delivered it.
const TRANSCRIPT_COLUMNS: &str = r#"m.id, m.uuid, m.message_type, m.message_role,
                      m.message_text, m.user_type, m.sandbox_wake, m.turn_number,
                      m.held, m."timestamp",
                      m.created_at, m.external_adapter, m.external_source_name,
                      m.external_subject_key, m.external_event_id,
                      m.external_author, m.source_task_id,
                      COALESCE(b.destination_config ->> 'issue_url',
                               b.destination_config ->> 'url') AS binding_url,
                      COALESCE(b.destination_config ->> 'identifier',
                               b.destination_config ->> 'issue_identifier')
                          AS binding_label,
                      COALESCE(u.name, u.login) AS sent_by_name,
                      files.file_contexts, m.child_outcome"#;

/// The two joins carrying a Message's origin out of the database: what its
/// Binding calls the subject and where it lives, and the display name `sent_by`
/// resolves to.
///
/// Destination-config precedence matches
/// `adapters::providers::linear::status::binding_label_and_url`; the subject
/// key alone is not unique, so `ORDER BY created_at DESC LIMIT 1` picks the
/// most recent binding deterministically; and the `users.id` comparison casts
/// to text rather than casting the polymorphic `sent_by` to `uuid`, which would
/// raise on a non-uuid value.
const TRANSCRIPT_ORIGIN_JOINS: &str = r#"LEFT JOIN LATERAL (
                          SELECT tb.destination_config
                          FROM task_bindings tb
                          WHERE m.external_subject_key IS NOT NULL
                            AND tb.task_id = m.task_id
                            AND tb.adapter_kind = m.external_adapter
                            AND tb.subject_key = m.external_subject_key
                          ORDER BY tb.created_at DESC
                          LIMIT 1
                      ) b ON true
                      LEFT JOIN users u ON u.id::text = m.sent_by
                      LEFT JOIN LATERAL (
                          SELECT jsonb_agg(
                                     jsonb_build_object(
                                         'referenceId', c ->> 'referenceId',
                                         'data', c -> 'data'
                                     )
                                     ORDER BY n
                                 ) AS file_contexts
                          FROM jsonb_array_elements(COALESCE(m.contexts, '[]'::jsonb))
                               WITH ORDINALITY AS e(c, n)
                          WHERE c ->> 'type' = 'file'
                      ) files ON true"#;

/// One Task's transcript, oldest first, capped at [`TRANSCRIPT_LIMIT`].
pub(crate) async fn transcript(
    pool: &UiRpcDbPool,
    activity_source: &dyn ActivitySource,
    task_id: &str,
) -> Result<Transcript, sqlx::Error> {
    // Newest first so the cap keeps the end of the conversation, then
    // reversed below into the order it is read in. `position` is the
    // conversation's own order, and its unique index read backwards makes
    // the cap a bounded index scan rather than a sort of the whole
    // conversation.
    //
    // Subagent rows are excluded here rather than after the cap: they are
    // the bulk of a long transcript, they are never rendered inline
    // (`message/transform.rs` assembles them on demand behind their
    // spawning tool call), and counting them against the cap would evict
    // the conversation a person is reading.
    // The cap is applied in the inner subquery, before the origin joins, so
    // the index-driven bounded scan still decides which rows are read and
    // the joins run over at most `TRANSCRIPT_LIMIT` of them rather than the
    // whole conversation. The outer `ORDER BY` re-establishes the order a
    // subquery does not promise to keep; `reverse` then reads it forward.
    let sql = format!(
        r#"SELECT {TRANSCRIPT_COLUMNS}
           FROM ({} ORDER BY m.position DESC LIMIT $2) m
           {TRANSCRIPT_ORIGIN_JOINS}
           ORDER BY m.position DESC"#,
        transcript_rows!(),
    );
    let mut messages = sqlx::query_as::<_, TranscriptMessage>(AssertSqlSafe(sql))
        .bind(task_id)
        .bind(TRANSCRIPT_LIMIT)
        .fetch_all(pool)
        .await?;
    messages.reverse();

    // Only when the cap was actually reached. Nearly every conversation is
    // shorter than it, and paying for a count on all of them to discover a
    // zero would be the wrong trade.
    let omitted = if messages.len() as i64 == TRANSCRIPT_LIMIT {
        let total: i64 = sqlx::query_scalar(
            r#"SELECT count(*)
               FROM conversation_messages
               WHERE task_id = $1
                 AND parent_tool_use_id IS NULL"#,
        )
        .bind(task_id)
        .fetch_one(pool)
        .await?;
        (total - TRANSCRIPT_LIMIT).max(0)
    } else {
        0
    };

    // From the oldest Message kept, not by that Message's uuid: a turn that
    // only called tools has no Message row to be named by, and its calls
    // are exactly what the cap must not drop. `None` — an empty
    // conversation, or one whose newest rows predate `created_at` — reads
    // the activities whole, which for an empty conversation is none.
    let activities = activity_source
        .since(
            task_id,
            messages.first().and_then(|message| message.created_at),
        )
        .await
        .map_err(store_read)?;

    let settled_requests = activity_dao::settled_requests(pool, task_id, &activities).await?;
    let subagent_usage = activity_dao::subagent_usage(pool, task_id, &activities).await?;

    // Uncapped, unlike the Messages above: one `i32` per turn, and a
    // checkpoint the rewind will refuse is a control that refuses when
    // pressed.
    let turn_outcomes: Vec<TurnOutcome> = sqlx::query_as(
        r#"SELECT tt.turn_number, tt.status, tt.started_at, tt.ended_at,
                  failure.error, failure.reason, failure.resets_at
             FROM task_turns tt
             LEFT JOIN LATERAL (
                 SELECT (e.payload::jsonb)->>'error' AS error,
                        (e.payload::jsonb)->>'reason' AS reason,
                        (e.payload::jsonb)->>'resetsAt' AS resets_at
                 FROM task_events e
                 WHERE tt.status = 'error'
                   AND e.task_id = tt.task_id
                   AND e.turn_number = tt.turn_number
                   AND e.type IN ('agent', 'taskTurn')
                   AND e.sub_type = 'error'
                 ORDER BY e.id DESC
                 LIMIT 1
             ) failure ON TRUE
            WHERE tt.task_id = $1
            ORDER BY tt.turn_number"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    let turns = turn_outcomes.iter().map(|turn| turn.turn_number).collect();
    let rewound_turns: Vec<RewoundTurn> = sqlx::query_as(
        r#"SELECT turn_number, started_at, rewound_at FROM task_rewound_turns
           WHERE task_id = $1 ORDER BY turn_number"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;

    Ok(Transcript {
        messages,
        activities,
        omitted,
        turns,
        turn_outcomes,
        rewound_turns,
        settled_requests,
        subagent_usage,
    })
}

/// The Messages and activities a set of changes names, in the same shape
/// [`transcript`] returns.
pub(crate) async fn changed_messages(
    pool: &UiRpcDbPool,
    activity_source: &dyn ActivitySource,
    task_id: &str,
    uuids: &[String],
    activity_ids: &[String],
) -> Result<Transcript, sqlx::Error> {
    if uuids.is_empty() && activity_ids.is_empty() {
        return Ok(Transcript {
            messages: Vec::new(),
            activities: Vec::new(),
            omitted: 0,
            turns: Vec::new(),
            turn_outcomes: Vec::new(),
            rewound_turns: Vec::new(),
            settled_requests: HashSet::new(),
            subagent_usage: HashMap::new(),
        });
    }
    // By the named uuids, not by the Messages just read: a change can name
    // a uuid that has no Message row at all — a turn that only called
    // tools — and those activities are the whole of what it changed. The
    // activities named directly by their own id — the ones on no Message —
    // join the same set by a primary-key lookup.
    let mut activities = activity_source
        .for_messages(task_id, uuids)
        .await
        .map_err(store_read)?;
    activities.extend(
        activity_source
            .for_ids(task_id, activity_ids)
            .await
            .map_err(store_read)?,
    );
    let recovered = answered_calls(activity_source, task_id, &activities).await?;

    // The recovered calls' own Messages, so a call reaches the window with
    // what `activity_anchor` anchors it on. Without them the call falls back
    // to the conversation's newest Turn and its own insert time, and the card
    // a change emits carries a `turnId` and `createdAt` a reload would
    // disagree with.
    let mut wanted: Vec<String> = uuids.to_vec();
    wanted.extend(
        recovered
            .iter()
            .filter_map(|call| call.message_uuid.clone())
            .filter(|uuid| !uuids.contains(uuid)),
    );
    activities.extend(recovered);

    // The DAO's own order, restored: `answered_calls` appends a call older
    // than everything already here, and this read promises oldest first.
    activities.sort_by(|left, right| {
        (left.position, left.ordinal, &left.activity_id).cmp(&(
            right.position,
            right.ordinal,
            &right.activity_id,
        ))
    });

    // Named Messages plus those. A result no longer lands on a Message of
    // its own: it settles the activity the call already wrote, and the
    // change that records it names the Message that *made* the call
    // (`agent_handlers::anchor_message_uuid`).
    //
    // Top-level only, for the reason the transcript has it: subagent rows
    // are never rendered inline.
    let sql = format!(
        r#"SELECT {TRANSCRIPT_COLUMNS}
           FROM ({} AND m.uuid::text = ANY($2)) m
           {TRANSCRIPT_ORIGIN_JOINS}
           ORDER BY m.position"#,
        transcript_rows!(),
    );
    let messages = sqlx::query_as::<_, TranscriptMessage>(AssertSqlSafe(sql))
        .bind(task_id)
        .bind(&wanted)
        .fetch_all(pool)
        .await?;

    let settled_requests = activity_dao::settled_requests(pool, task_id, &activities).await?;
    let subagent_usage = activity_dao::subagent_usage(pool, task_id, &activities).await?;

    Ok(Transcript {
        messages,
        activities,
        omitted: 0,
        turns: Vec::new(),
        turn_outcomes: Vec::new(),
        rewound_turns: Vec::new(),
        settled_requests,
        subagent_usage,
    })
}

/// The oldest `limit` settled turns numbered above `above`, with their
/// Messages, in the shape [`transcript`] returns.
///
/// Settled is `TurnStatus::is_open`'s complement, so these are the turns
/// [`crate::projection`]'s `checkpoints` offers.
pub(crate) async fn settled_turns(
    pool: &UiRpcDbPool,
    task_id: &str,
    above: Option<i32>,
    limit: i64,
) -> Result<Transcript, sqlx::Error> {
    let turns: Vec<i32> = sqlx::query_scalar(
        r#"SELECT turn_number FROM task_turns
           WHERE task_id = $1 AND status NOT IN ('active', 'interrupted')
             AND turn_number > COALESCE($2, -1)
           ORDER BY turn_number
           LIMIT $3"#,
    )
    .bind(task_id)
    .bind(above)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let messages = if turns.is_empty() {
        Vec::new()
    } else {
        let sql = format!(
            r#"SELECT {TRANSCRIPT_COLUMNS}
               FROM ({} AND m.turn_number = ANY($2)) m
               {TRANSCRIPT_ORIGIN_JOINS}
               ORDER BY m.position"#,
            transcript_rows!(),
        );
        sqlx::query_as::<_, TranscriptMessage>(AssertSqlSafe(sql))
            .bind(task_id)
            .bind(&turns)
            .fetch_all(pool)
            .await?
    };
    Ok(Transcript {
        messages,
        activities: Vec::new(),
        omitted: 0,
        turns,
        turn_outcomes: Vec::new(),
        rewound_turns: Vec::new(),
        settled_requests: HashSet::new(),
        subagent_usage: HashMap::new(),
    })
}

/// The interactive calls the resolved rows in `activities` answer, where the
/// window does not already hold them.
///
/// An answer settles its call in place on an earlier Message, which records no
/// change, so a narrowed window holds the answer alone. Found by request id, as
/// `settled_requests` is.
async fn answered_calls(
    activity_source: &dyn ActivitySource,
    task_id: &str,
    activities: &[Activity],
) -> Result<Vec<Activity>, sqlx::Error> {
    let held: HashSet<&str> = activities
        .iter()
        .filter_map(|activity| activity.tool_use_id.as_deref())
        .collect();
    let mut wanted: Vec<&str> = activities
        .iter()
        .filter(|activity| activity.kind == ActivityKind::UserInputResolved)
        .filter_map(|activity| activity.payload.get("requestId").and_then(Value::as_str))
        .filter(|request_id| !held.contains(request_id))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();

    let mut calls = Vec::new();
    for request_id in wanted {
        if let Some(call) = activity_source
            .tool_call(task_id, request_id)
            .await
            .map_err(store_read)?
        {
            calls.push(call);
        }
    }
    Ok(calls)
}
