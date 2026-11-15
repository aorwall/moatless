//! One Task's transcript as a reader opens it: the top-level Messages, what
//! happened on the way, and how each Turn ended.
//!
//! Unscoped: every caller resolves the Task under its own access rule first.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;
use moatless_types::activity::{Activity, ActivityKind};
use moatless_types::message::{MessageRole, MessageType};
use serde_json::Value;

use super::transcript_dao;
use crate::activities::dao as activities;
use crate::activities::outputs::OutputCache;

/// One Message of a Task's transcript, with the columns a transcript is
/// rendered from and nothing else.
///
/// Deliberately not `TaskMessage`, whose forty write-model columns would pull
/// every tool result's raw payload and every run's metrics out of the database
/// to project six fields.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TranscriptEntry {
    /// The insertion order this transcript is assembled in.
    pub id: i32,
    pub uuid: String,
    #[sqlx(try_from = "String")]
    pub message_type: MessageType,
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
    /// Where a person moved this queued Message among the others still
    /// waiting. `None` keeps its arrival order, after every moved one.
    pub queue_order: Option<i32>,
    /// When the writer said it happened (RFC 3339 text, `NOT NULL`).
    pub timestamp: String,
    /// When the database recorded it. `None` is impossible under the current
    /// schema's default and is treated as a missing value rather than assumed.
    pub created_at: Option<DateTime<Utc>>,
    /// The adapter a Message reached this Task through — `slack`, `github_pr`,
    /// `linear`. `None` on a Message that came from inside Moatless.
    pub external_adapter: Option<String>,
    /// The name the subscription knows the source by — a channel, a repository,
    /// or for a `scheduled` Message the Loop's id.
    pub external_source_name: Option<String>,
    /// The binding identity this Message belongs to — the key to its Binding,
    /// and for GitHub PR the `owner/repo#N` the pull-request URL is built from.
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
    /// The Moatless user id the send resolved to. Text, and not always a uuid.
    pub sent_by: Option<String>,
    /// The uploads attached to this Message, from its `file` context rows as a
    /// JSON array of `{referenceId, data}` — the upload's id and the filename,
    /// mime type and size resolved when it was sent.
    ///
    /// Read from the inbox item's contexts rather than its `file_ids`, which
    /// are ids alone; `None` on a Message with no uploads.
    pub file_contexts: Option<Value>,
    /// `completed` or `errored` on the notice a child Task's run ending sent
    /// this, its parent; `None` on every other Message.
    pub child_outcome: Option<String>,
    /// The harness that wrote an agent Message; `None` on every other row.
    pub harness: Option<String>,
    /// The model an agent Message says produced it; `None` on every other row.
    pub model: Option<String>,
    /// The reasoning effort a user Message asked for, in the agent's own
    /// vocabulary. `None` where the sender expressed no preference.
    pub effort: Option<String>,
}

/// The rows one transcript read returns.
#[derive(Debug, Default)]
pub struct TranscriptWindow {
    /// Oldest first — the order a transcript is read in.
    pub entries: Vec<TranscriptEntry>,
    /// What happened on the way, oldest first. Not a subset of `entries`: a
    /// turn whose agent only called tools said nothing and has no Message row
    /// at all, and its calls reach a reader only through here.
    pub activities: Vec<Activity>,
    /// The rows past [`TRANSCRIPT_LIMIT`] that were left out, oldest first.
    /// Zero for every transcript that fits, which is nearly all of them.
    pub omitted: i64,
    /// Every `task_turns.turn_number` this Task has, ascending.
    ///
    /// Read from `task_turns`, not off `entries`: a tool-only turn has no
    /// Message, and a turn older than [`TRANSCRIPT_LIMIT`] has none left here.
    /// Empty from [`TranscriptReads::changed`].
    pub turns: Vec<i32>,
    /// How each of [`Self::turns`] ended, ascending. Read by
    /// [`TranscriptReads::read`] alone, and empty from every other read.
    pub turn_outcomes: Vec<TurnOutcome>,
    /// The turns a rewind cut out of this Task, ascending. Read by
    /// [`TranscriptReads::read`] alone, and empty from every other read.
    pub rewound_turns: Vec<RewoundTurn>,
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

/// A `task_rewound_turns` row: a turn a rewind cut.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RewoundTurn {
    pub turn_number: i32,
    pub started_at: DateTime<Utc>,
    pub rewound_at: DateTime<Utc>,
}

/// How many of a Task's Messages one transcript read returns.
///
/// A transcript is served in one unpaginated snapshot, so this cap is the only
/// guard against a multi-megabyte frame; the **newest** rows are kept, and what
/// was dropped is reported via [`TranscriptWindow::omitted`].
pub const TRANSCRIPT_LIMIT: i64 = 1_000;

/// The transcript reads over `conversation_messages` and the tables a
/// transcript is rendered with.
pub struct TranscriptReads {
    pool: DbPool,
    outputs: OutputCache,
}

impl TranscriptReads {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self {
            pool,
            outputs: OutputCache::new(),
        }
    }

    /// One Task's transcript, oldest first, capped at [`TRANSCRIPT_LIMIT`].
    pub async fn read(&self, task_id: &str) -> Result<TranscriptWindow, sqlx::Error> {
        let entries = transcript_dao::newest(&self.pool, task_id, TRANSCRIPT_LIMIT).await?;

        // Only when the cap was actually reached. Nearly every conversation is
        // shorter than it, and paying for a count on all of them to discover a
        // zero would be the wrong trade.
        let omitted = if entries.len() as i64 == TRANSCRIPT_LIMIT {
            (transcript_dao::top_level_count(&self.pool, task_id).await? - TRANSCRIPT_LIMIT).max(0)
        } else {
            0
        };

        // From the oldest Message kept, not by that Message's uuid: a turn that
        // only called tools has no Message row to be named by, and its calls
        // are exactly what the cap must not drop. `None` — an empty
        // conversation, or one whose newest rows predate `created_at` — reads
        // the activities whole, which for an empty conversation is none.
        let activities = activities::for_task_since_without_output(
            &self.pool,
            task_id,
            entries.first().and_then(|entry| entry.created_at),
        )
        .await?;
        let activities = self
            .outputs
            .restore(&self.pool, task_id, activities)
            .await?;

        // Uncapped, unlike the Messages above: one `i32` per turn, and a
        // checkpoint the rewind will refuse is a control that refuses when
        // pressed.
        let turn_outcomes = transcript_dao::turn_outcomes(&self.pool, task_id).await?;
        let turns = turn_outcomes.iter().map(|turn| turn.turn_number).collect();
        let rewound_turns = transcript_dao::rewound_turns(&self.pool, task_id).await?;

        Ok(TranscriptWindow {
            entries,
            activities,
            omitted,
            turns,
            turn_outcomes,
            rewound_turns,
        })
    }

    /// The Messages and activities a set of changes names, oldest first, in
    /// the shape [`Self::read`] returns. Elides nothing.
    ///
    /// `uuids` name Messages and the activities on them; `activity_ids` name
    /// activities on no Message. The interactive calls the named answers
    /// settle come back too, each with the Message it sits on.
    pub async fn changed(
        &self,
        task_id: &str,
        uuids: &[String],
        activity_ids: &[String],
    ) -> Result<TranscriptWindow, sqlx::Error> {
        if uuids.is_empty() && activity_ids.is_empty() {
            return Ok(TranscriptWindow::default());
        }
        // By the named uuids, not by the Messages just read: a change can name
        // a uuid that has no Message row at all — a turn that only called
        // tools — and those activities are the whole of what it changed. The
        // activities named directly by their own id — the ones on no Message —
        // join the same set by a primary-key lookup.
        let mut found = activities::for_messages(&self.pool, task_id, uuids).await?;
        found.extend(activities::for_activity_ids(&self.pool, task_id, activity_ids).await?);
        let recovered = self.answered_calls(task_id, &found).await?;

        // The recovered calls' own Messages, so a call reaches the window with
        // what it is anchored on. Without them the call falls back to the
        // conversation's newest Turn and its own insert time, and the card a
        // change emits carries a turn and time a reload would disagree with.
        let mut wanted: Vec<String> = uuids.to_vec();
        wanted.extend(
            recovered
                .iter()
                .filter_map(|call| call.message_uuid.clone())
                .filter(|uuid| !uuids.contains(uuid)),
        );
        found.extend(recovered);

        // The DAO's own order, restored: `answered_calls` appends a call older
        // than everything already here, and this read promises oldest first.
        found.sort_by(|left, right| {
            (left.position, left.ordinal, &left.activity_id).cmp(&(
                right.position,
                right.ordinal,
                &right.activity_id,
            ))
        });

        let entries = transcript_dao::by_uuids(&self.pool, task_id, &wanted).await?;
        Ok(TranscriptWindow {
            entries,
            activities: found,
            ..TranscriptWindow::default()
        })
    }

    /// The oldest `limit` settled turns numbered above `above` (from the first
    /// for `None`), with their Messages, in the shape [`Self::read`] returns.
    ///
    /// Settled is `TurnStatus::is_open`'s complement.
    pub async fn settled_turns(
        &self,
        task_id: &str,
        above: Option<i32>,
        limit: i64,
    ) -> Result<TranscriptWindow, sqlx::Error> {
        let turns = transcript_dao::settled_turn_numbers(&self.pool, task_id, above, limit).await?;
        let entries = if turns.is_empty() {
            Vec::new()
        } else {
            transcript_dao::in_turns(&self.pool, task_id, &turns).await?
        };
        Ok(TranscriptWindow {
            entries,
            turns,
            ..TranscriptWindow::default()
        })
    }

    /// The oldest `limit` top-level Messages after the one at `after` (from
    /// the first for `None`), without activities or turns. A reader pages a
    /// whole conversation with it by passing the last entry's `id` back.
    pub async fn messages_after(
        &self,
        task_id: &str,
        after: Option<i32>,
        limit: i64,
    ) -> Result<Vec<TranscriptEntry>, sqlx::Error> {
        transcript_dao::after_position(&self.pool, task_id, after, limit).await
    }

    /// The interactive calls the resolved rows in `found` answer, where the
    /// window does not already hold them.
    ///
    /// An answer settles its call in place on an earlier Message, which records
    /// no change, so a narrowed window holds the answer alone. Found by request
    /// id.
    async fn answered_calls(
        &self,
        task_id: &str,
        found: &[Activity],
    ) -> Result<Vec<Activity>, sqlx::Error> {
        let held: HashSet<&str> = found
            .iter()
            .filter_map(|activity| activity.tool_use_id.as_deref())
            .collect();
        let mut wanted: Vec<&str> = found
            .iter()
            .filter(|activity| activity.kind == ActivityKind::UserInputResolved)
            .filter_map(|activity| activity.payload.get("requestId").and_then(Value::as_str))
            .filter(|request_id| !held.contains(request_id))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();

        let mut calls = Vec::new();
        for request_id in wanted {
            if let Some(call) = activities::find_tool_call(&self.pool, task_id, request_id).await? {
                calls.push(call);
            }
        }
        Ok(calls)
    }
}
