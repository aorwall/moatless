//! Copying one Task's conversation into a fork.
//!
//! A fork holds its own rows rather than pointing at the parent's: every
//! conversation reader, the T3 projection and the per-Task change feed would
//! otherwise each have to learn the indirection, and deleting the parent would
//! take the fork's history with it.
//!
//! The copy re-keys everything that is unique outside its Task — a Message's
//! `uuid`, an activity's `activity_id` — and drops everything that claims
//! something in the world: `external_event_id` is what tells a webhook echo
//! apart from a new Message, and two Tasks claiming one id would make the
//! parent's echo look like the fork's own work.

use std::collections::HashMap;

use moatless_core::db::DbPool;

use super::dao::{bind_message, message_columns, message_placeholders};
use super::models::TaskMessage;

/// The parent's agent session at a fork cut, and the transcript line the cut
/// lands on.
#[derive(Debug, Clone)]
pub struct ForkAnchors {
    /// The parent's `session_id` at the cut — the Claude session or Codex thread
    /// the pod branches from. Read from `session_id` rather than
    /// `agent_session_id`: only `user` and `error` rows are ever stamped with the
    /// latter, so an assistant row carries the session in `session_id` alone.
    pub session_id: String,
    /// The last assistant `task_messages.uuid` at or before the cut. For a
    /// Claude turn that is the Anthropic message id, which the CLI writes as
    /// `message.id` on every transcript line belonging to the message — the one
    /// identifier the stored row and the transcript share. `None` only when the
    /// session is known from a row that carries no usable uuid.
    pub message_uuid: Option<String>,
}

/// The columns the copy needs to re-key one activity. The rest travel inside
/// the `INSERT … SELECT` that writes it, so they are never decoded here — which
/// is what keeps a row of a `kind` this build cannot read copyable.
#[derive(sqlx::FromRow)]
struct CopiedActivity {
    activity_id: String,
    position: i64,
    message_uuid: Option<String>,
    parent_uuid: Option<String>,
}

/// How much of the parent the copy carried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForkCopyCounts {
    pub messages: usize,
    pub activities: usize,
    pub contexts: usize,
}

/// The predicate that decides which of `task_id`'s Messages a cut at `at_turn`
/// keeps.
///
/// `$2` is the turn, `NULL` for a cut at head. A row with no `turn_number` is
/// kept only at head: it cannot be placed relative to the cut, and the turn it
/// belongs to may be a later one.
///
/// A macro rather than a `const`: sqlx 0.9 takes only `&'static str`, and a
/// `format!` here would be dynamic SQL the workspace lint refuses.
macro_rules! at_or_before_cut {
    () => {
        "($2::int IS NULL OR (turn_number IS NOT NULL AND turn_number <= $2))"
    };
}

/// The parent's session and transcript anchor at the cut.
///
/// `None` when the parent has no agent-written Message at or before the cut
/// carrying a session — a Task whose agent never ran, or one forked at a turn
/// before it did. Such a fork carries the conversation and no agent context.
pub async fn anchors(
    pool: &DbPool,
    task_id: &str,
    at_turn: Option<i32>,
) -> Result<Option<ForkAnchors>, sqlx::Error> {
    // `session_id`, not `agent_session_id`: the latter is written only by
    // `set_session_id_for_task` and `acknowledge_messages`, both of which filter
    // to `message_type IN ('user', 'error')`, so no assistant row has ever
    // carried it and this predicate matched nothing.
    //
    // `assistant` alone, not `('assistant', 'system')`: the anchor has to name a
    // line in the transcript, and only an assistant row's uuid does — it holds
    // the Anthropic message id the CLI repeats as `message.id`. A `system` row
    // sorting newest would otherwise win the `ORDER BY` and hand the pod an
    // anchor no transcript line carries.
    let row: Option<(String, String)> = sqlx::query_as(concat!(
        "SELECT session_id, uuid
           FROM task_messages
          WHERE task_id = $1
            AND session_id IS NOT NULL
            AND message_type = 'assistant'
            AND parent_uuid IS NULL
            AND ",
        at_or_before_cut!(),
        " ORDER BY id DESC LIMIT 1"
    ))
    .bind(task_id)
    .bind(at_turn)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(session_id, uuid)| ForkAnchors {
        session_id,
        message_uuid: Some(uuid),
    }))
}

/// Copy `source_task_id`'s conversation up to and including `at_turn` onto
/// `dest_task_id`, in one transaction.
///
/// Runs before the fork's own Task row exists: neither `task_messages` nor
/// `task_activities` has a foreign key to `tasks` — both say so in their
/// migrations, for the same reason this needs it — so the copy can land first
/// and the create that follows delivers the fork's first message after it.
pub async fn copy_conversation(
    pool: &DbPool,
    source_task_id: &str,
    dest_task_id: &str,
    at_turn: Option<i32>,
) -> Result<ForkCopyCounts, sqlx::Error> {
    // Read outside the transaction: the parent is quiescent (a Task with a turn
    // in flight is refused before this runs), and the write below is what needs
    // to be atomic.
    let sources: Vec<TaskMessage> = sqlx::query_as(concat!(
        "SELECT * FROM task_messages
          WHERE task_id = $1
            AND ",
        at_or_before_cut!(),
        // A user Message the parent's agent never consumed still reads as
        // pending work. Copying one would hand the fork a prompt the parent is
        // about to answer itself.
        " AND NOT (message_type = 'user' AND agent_session_id IS NULL)
          ORDER BY id"
    ))
    .bind(source_task_id)
    .bind(at_turn)
    .fetch_all(pool)
    .await?;

    let uuids: HashMap<String, String> = sources
        .iter()
        .map(|m| (m.uuid.clone(), uuid::Uuid::new_v4().to_string()))
        .collect();
    let kept: Vec<&str> = sources.iter().map(|m| m.uuid.as_str()).collect();

    // Activities are cut by `position`, not by turn: `task_activities` has no
    // turn column, and a `sandbox_status` activity has no `message_uuid` to
    // resolve one from. The cut is the last position any kept Message reached
    // in the Task's own stream, which is what "up to the last kept Message"
    // means in the table this reads.
    let cut_position: Option<i64> = sqlx::query_scalar(
        r#"SELECT MAX(position) FROM task_activities
           WHERE task_id = $1 AND parent_uuid IS NULL AND message_uuid = ANY($2)"#,
    )
    .bind(source_task_id)
    .bind(&kept)
    .fetch_one(pool)
    .await?;

    // Everything that happened on a kept Message, plus the Sandbox activities —
    // which happened on no Message — that fall at or before the cut.
    let activities: Vec<CopiedActivity> = sqlx::query_as(
        r#"SELECT activity_id, position, message_uuid, parent_uuid
           FROM task_activities
           WHERE task_id = $1
             AND (message_uuid = ANY($2)
                  OR (message_uuid IS NULL AND parent_uuid IS NULL
                      AND $3::bigint IS NOT NULL AND position <= $3))
           ORDER BY position, ordinal, activity_id"#,
    )
    .bind(source_task_id)
    .bind(&kept)
    .bind(cut_position)
    .fetch_all(pool)
    .await?;

    let mut tx = pool.begin().await?;

    let mut messages = 0usize;
    // The serial id each copied Message landed on, paired with the source id it
    // came from. `task_message_contexts` is keyed by that id rather than by
    // `uuid`, so the copy below cannot be written until the inserts have run.
    let mut source_ids: Vec<i32> = Vec::with_capacity(sources.len());
    let mut new_ids: Vec<i32> = Vec::with_capacity(sources.len());
    for source in &sources {
        let mut row = source.clone();
        row.task_id = dest_task_id.to_string();
        row.uuid = uuids
            .get(&source.uuid)
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // `parent_uuid` holds the spawning `Agent` tool-use id, which is
        // Task-scoped and copied as it stands. It is still looked up in the
        // map, because a row written before that meaning settled may hold a
        // Message uuid instead, and such a row must not point out of the fork.
        row.parent_uuid = source
            .parent_uuid
            .as_ref()
            .map(|parent| uuids.get(parent).cloned().unwrap_or_else(|| parent.clone()));
        row.external_event_id = None;
        // The Message came from the parent, and says so. This is provenance,
        // not a claim on anything outside the fork.
        row.source_task_id = Some(source_task_id.to_string());

        let (new_id,): (i32,) = bind_message!(
            sqlx::query_as(concat!(
                "INSERT INTO task_messages (",
                message_columns!(),
                ") VALUES (",
                message_placeholders!(),
                ") RETURNING id"
            )),
            row
        )
        .fetch_one(&mut *tx)
        .await?;
        source_ids.push(source.id);
        new_ids.push(new_id);
        messages += 1;
    }

    // A user Message's attached contexts — the preview, element and file rows
    // the composer wrote beside it — live in their own table, and
    // `message_text` holds only what the user typed. A copy that left them
    // behind would drop what the Message was pointing at from the fork's
    // conversation. One statement over the id pairs, so the copy costs a round
    // trip rather than one per Message.
    let contexts = sqlx::query(
        r#"INSERT INTO task_message_contexts
               (task_message_id, task_id, "type", reference_id, data, created_at)
           SELECT m.new_id, $3, c."type", c.reference_id, c.data, c.created_at
           FROM task_message_contexts c
           JOIN UNNEST($1::int[], $2::int[]) AS m(source_id, new_id)
             ON c.task_message_id = m.source_id"#,
    )
    .bind(&source_ids)
    .bind(&new_ids)
    .bind(dest_task_id)
    .execute(&mut *tx)
    .await?
    .rows_affected() as usize;

    // Token usage lives on its own table so a subagent's aggregate survives the
    // Messages it was computed from. Copied under the new uuids for the same
    // reason the Messages are: the fork's `Agent` calls must still report what
    // they cost.
    for source in &sources {
        let Some(new_uuid) = uuids.get(&source.uuid) else {
            continue;
        };
        sqlx::query(
            r#"INSERT INTO task_message_usage (message_uuid, task_id, parent_uuid, input_tokens, output_tokens)
               SELECT $1, $2, $3, input_tokens, output_tokens
               FROM task_message_usage WHERE message_uuid = $4
               ON CONFLICT (message_uuid) DO NOTHING"#,
        )
        .bind(new_uuid)
        .bind(dest_task_id)
        .bind(
            source
                .parent_uuid
                .as_ref()
                .map(|p| uuids.get(p).cloned().unwrap_or_else(|| p.clone())),
        )
        .bind(&source.uuid)
        .execute(&mut *tx)
        .await?;
    }

    let mut copied_activities = 0usize;
    for activity in &activities {
        // A subagent's activities are ordered by their own per-`parent_uuid`
        // sequence, never mixed into the Task's stream, so they carry their
        // position across. The Task's own take fresh ones: the fork's stream
        // starts at 1 however far along the parent's the copied rows sat.
        let position = match activity.parent_uuid {
            Some(_) => activity.position,
            None => {
                moatless_backend_core::change_feed::conversation::allocate_position(
                    &mut tx,
                    dest_task_id,
                )
                .await?
            }
        };
        let remap = |uuid: &Option<String>| {
            uuid.as_ref()
                .map(|u| uuids.get(u).cloned().unwrap_or_else(|| u.clone()))
        };

        sqlx::query(
            r#"INSERT INTO task_activities (
                   activity_id, task_id, position, ordinal, message_uuid, parent_uuid,
                   kind, status, tool_use_id, summary, payload, detail, is_error, created_at
               )
               SELECT $1, $2, $3, ordinal, $4, $5,
                      kind, status, tool_use_id, summary, payload, detail, is_error, created_at
               FROM task_activities
               WHERE activity_id = $6"#,
        )
        .bind(rekey_activity_id(&activity.activity_id, &uuids))
        .bind(dest_task_id)
        .bind(position)
        .bind(remap(&activity.message_uuid))
        .bind(remap(&activity.parent_uuid))
        .bind(&activity.activity_id)
        .execute(&mut *tx)
        .await?;
        copied_activities += 1;
    }

    tx.commit().await?;

    Ok(ForkCopyCounts {
        messages,
        activities: copied_activities,
        contexts,
    })
}

/// Delete a fork's copied conversation.
///
/// `copy_conversation` commits `task_messages`, `task_message_usage` and
/// `task_activities` under the fork's id before the fork's Task row exists, so a
/// create that fails after the copy would leave those rows with no Task to
/// belong to — no foreign key takes them down, by the same design that lets the
/// copy land first. This is the undo the create path runs on that failure, and
/// only then: once the fork's own turns start writing under this id, its history
/// is indistinguishable from the copy and must not be swept.
///
/// One transaction, so a fork's rows never half-vanish. `task_message_contexts`
/// is not deleted here: it has a foreign key to `task_messages` with `ON DELETE
/// CASCADE`, so the Message delete below takes each Message's contexts with it.
pub async fn discard_conversation(pool: &DbPool, dest_task_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM task_message_usage WHERE task_id = $1")
        .bind(dest_task_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM task_messages WHERE task_id = $1")
        .bind(dest_task_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM task_activities WHERE task_id = $1")
        .bind(dest_task_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Re-derive an activity's primary key for the fork.
///
/// `activity_id` is `{message_uuid}:{block_index}` for a block and the event id
/// for a Sandbox activity, and the column is a primary key over the whole
/// table — so a copied row that kept its id would collide with the parent's.
/// A block's id follows its Message's new uuid; anything else takes a fresh one.
fn rekey_activity_id(activity_id: &str, uuids: &HashMap<String, String>) -> String {
    if let Some((prefix, suffix)) = activity_id.split_once(':') {
        if let Some(new_uuid) = uuids.get(prefix) {
            return format!("{new_uuid}:{suffix}");
        }
    }
    format!("fk_{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_id_follows_its_message_and_anything_else_takes_a_fresh_one() {
        let uuids = HashMap::from([("old-uuid".to_string(), "new-uuid".to_string())]);
        assert_eq!(rekey_activity_id("old-uuid:3", &uuids), "new-uuid:3");
        // A Sandbox activity's id is an event id, which names no Message.
        let sandbox = rekey_activity_id("evt_9f21", &uuids);
        assert!(sandbox.starts_with("fk_"), "{sandbox}");
        // A block whose Message was not copied must not keep the parent's key.
        let orphan = rekey_activity_id("uncopied-uuid:0", &uuids);
        assert!(orphan.starts_with("fk_"), "{orphan}");
    }
}
