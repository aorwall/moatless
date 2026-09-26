//! Finding a turn that never reached a terminal event, and telling a genuine
//! orphan from one whose reply landed elsewhere.
//!
//! A turn is recovered by re-sending it, so a false positive re-runs work the
//! agent already did. Both reads here exist to narrow that: one finds the
//! candidate, the other decides whether a re-drive got past resume.

use moatless_core::db::DbPool;

use super::models::{InterruptedTurn, MessageType};
use moatless_core::db_enum::decode_required;

/// Whether the Task's last message closes `turn_number` as answered.
pub(crate) async fn last_message_is_completed_reply(
    pool: &DbPool,
    task_id: &str,
    turn_number: Option<i32>,
) -> Result<bool, sqlx::Error> {
    // Scope to the reconciled turn first: a stale open turn must not be
    // closed as completed on the strength of a later turn's final reply
    // when it already has substantive assistant/system/result state of its
    // own (for example, a mid-tool interruption).
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        r#"SELECT message_type, assistant_stop_reason FROM task_messages
           WHERE task_id = $1
             AND ($2::int IS NULL OR turn_number = $2)
           ORDER BY timestamp DESC, id DESC
           LIMIT 1"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .fetch_optional(pool)
    .await?;

    // A final assistant reply ends with stop_reason `end_turn`; a turn cut
    // off mid-tool ends with `tool_use` (or no assistant row at all), which
    // stays a genuine orphan.
    let row = row
        .map(|(kind, reason)| {
            decode_required::<MessageType>("message_type", kind).map(|kind| (kind, reason))
        })
        .transpose()?;
    if matches!(
        row,
        Some((MessageType::Assistant, Some(reason))) if reason == "end_turn"
    ) {
        return Ok(true);
    }

    let Some(turn_number) = turn_number else {
        return Ok(false);
    };

    // Coalesced pending-message run: the orphaned lower turn contains only
    // the inbound user message (plus possible empty assistant placeholders),
    // while the actual final assistant reply was attributed to a later
    // sibling turn by MAX(turn_number)-at-event-arrival resolution.
    let (has_user, has_substantive_non_user, latest_later_completed): (bool, bool, bool) =
        sqlx::query_as(
            r#"SELECT
                 EXISTS (
                   SELECT 1 FROM task_inbox
                   WHERE task_id = $1
                     AND turn_number = $2
                     AND status = 'delivered'
                 ) AS has_user,
                 EXISTS (
                   SELECT 1 FROM task_messages
                   WHERE task_id = $1
                     AND turn_number = $2
                     AND message_type <> 'user'
                     -- An empty assistant placeholder: a row that stopped
                     -- for no stated reason and said nothing. Text is the
                     -- row itself, so an empty one is a row with no text
                     -- on it.
                     AND NOT (
                       message_type = 'assistant'
                       AND assistant_stop_reason IS NULL
                       AND COALESCE(message_text, '') = ''
                     )
                 ) AS has_substantive_non_user,
                 COALESCE((
                   SELECT message_type = 'assistant'
                      AND assistant_stop_reason = 'end_turn'
                      AND COALESCE(turn_number > $2, false)
                   FROM task_messages
                   WHERE task_id = $1
                   ORDER BY timestamp DESC, id DESC
                   LIMIT 1
                 ), false) AS latest_later_completed"#,
        )
        .bind(task_id)
        .bind(turn_number)
        .fetch_one(pool)
        .await?;

    Ok(has_user && !has_substantive_non_user && latest_later_completed)
}

/// One delivered item of an interrupted turn: `session_id`, `turn_number`,
/// `text`, `agent_mode`, `effort`.
type InterruptedTurnRow = (Option<String>, i32, String, Option<String>, Option<String>);

/// The Task's newest turn that it ran itself and that no terminal event, on it
/// or a later turn, has closed.
pub(crate) async fn find_interrupted_turn(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<InterruptedTurn>, sqlx::Error> {
    // A turn can take several items — everything the sandbox coalesced into
    // one agent run — so this selects the newest turn that never reached a
    // terminal event and then returns *all* of its items, oldest first.
    let rows: Vec<InterruptedTurnRow> = sqlx::query_as(
        r#"SELECT i.session_id, i.turn_number, i.text,
                  i.delivery_options ->> 'agentMode', i.delivery_options ->> 'effort'
           FROM task_inbox i
           WHERE i.task_id = $1
             AND i.status = 'delivered'
             AND i.turn_number = (
               SELECT c.turn_number
               FROM task_inbox c
               WHERE c.task_id = $1
                 AND c.status = 'delivered'
                 AND c.session_id IS NOT NULL
                 AND c.session_id <> ''
                 -- Only a turn still open: a fork's copied turn is closed, and
                 -- replaying it would re-run the source's work.
                 AND EXISTS (
                   SELECT 1 FROM task_turns t
                   WHERE t.task_id = c.task_id AND t.turn_number = c.turn_number
                     AND t.status IN ('active', 'interrupted')
                 )
                 -- `>=`: a later turn that finished superseded this one.
                 AND NOT EXISTS (
                   SELECT 1 FROM task_events e
                   WHERE e.task_id = c.task_id
                     AND e.turn_number >= c.turn_number
                     AND (
                       (e.type = 'agent' AND e.sub_type IN ('completed', 'error', 'stopped'))
                       OR (e.type = 'taskTurn' AND e.sub_type IN ('completed', 'error', 'stopped'))
                       OR (e.type = 'sandbox' AND e.sub_type = 'stopped')
                     )
                 )
               ORDER BY c.turn_number DESC
               LIMIT 1
             )
           ORDER BY i.created_at, i.id"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;

    let Some(turn_number) = rows.first().map(|row| row.1) else {
        return Ok(None);
    };
    // Every item of the turn carries its session (they were delivered
    // together); take the first that has one.
    let Some(session_id) = rows
        .iter()
        .find_map(|(sid, _, _, _, _)| sid.clone().filter(|s| !s.is_empty()))
    else {
        return Ok(None);
    };
    let agent_mode = rows
        .iter()
        .rev()
        .find_map(|(_, _, _, mode, _)| mode.clone());
    let effort = rows
        .iter()
        .rev()
        .find_map(|(_, _, _, _, effort)| effort.clone());

    // Reproduces the text the sandbox dispatcher built from the same items,
    // joined with newlines in their order, so a replay re-sends that prompt.
    let prompt_text = rows
        .iter()
        .map(|(_, _, text, _, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let turn_row: Option<(i32, Option<String>)> = sqlx::query_as(
        "SELECT recovery_attempts, run_id FROM task_turns WHERE task_id = $1 AND turn_number = $2",
    )
    .bind(task_id)
    .bind(turn_number)
    .fetch_optional(pool)
    .await?;
    let (recovery_attempts, run_id) = turn_row.unwrap_or((0, None));

    Ok(Some(InterruptedTurn {
        task_id: task_id.to_string(),
        session_id,
        turn_number,
        prompt_text,
        agent_mode,
        effort,
        recovery_attempts,
        run_id,
    }))
}

/// Whether a re-drive of `turn_number` produced assistant output after the last
/// recovery attempt started.
pub(crate) async fn turn_progressed_since_last_recovery(
    pool: &DbPool,
    task_id: &str,
    turn_number: i32,
) -> Result<bool, sqlx::Error> {
    // An `agent.message` row is only written once the resumed run produced
    // assistant output (its content lives in `task_messages`; the event is a
    // skeleton). Its presence after the last `recovery_started` means the
    // re-drive got past resume and did real work. `created_at` (timestamptz)
    // is the reliable ordering key; `timestamp` is free-form text.
    let (progressed,): (bool,) = sqlx::query_as(
        r#"SELECT EXISTS (
               SELECT 1 FROM task_events e
               WHERE e.task_id = $1
                 AND e.turn_number = $2
                 AND e.type = 'agent'
                 AND e.sub_type = 'message'
                 AND e.created_at > COALESCE(
                   (SELECT MAX(r.created_at) FROM task_events r
                      WHERE r.task_id = $1
                        AND r.turn_number = $2
                        AND r.type = 'agent'
                        AND r.sub_type = 'recovery_started'),
                   '-infinity'::timestamptz)
           )"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .fetch_one(pool)
    .await?;
    Ok(progressed)
}
