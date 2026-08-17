//! The external events a Task caused.
//!
//! When a Task changes a provider — posts a comment, submits a review, pushes a
//! commit — the provider webhooks that change straight back. Something has to
//! say "this is the Task's own work", or the Task reads its own comment as a
//! new inbound message and answers it.
//!
//! This table says it. A row is a fact about the Task, so nothing here names
//! the Message or the tool call that made the change: the `moat` command that
//! records one runs in a sandbox shell and holds `TASK_ID` and the user token,
//! and nothing that names the call it is running inside.
//!
//! It is the second of the two ways a Task connects to external things.
//! `task_bindings` holds the *subject* — an issue, a pull request, a channel —
//! and routes delivery to it. This holds the individual events caused under
//! those subjects. Both last as long as the Task.
//!
//! # Direction
//!
//! Every row here is outbound. The inbound side is
//! `task_messages.external_event_id`, the external event a Message arrived as.
//! The two answer different questions — "I already received this" against "I
//! caused this" — and a reader that cannot tell them apart cannot tell a
//! redelivered webhook from its own echo.
//!
//! A row is written under the id the *inbound* event will carry, so matching an
//! arriving webhook is equality on one column. Where the writer cannot know
//! that id, the Adapter maps between the two in one place: for GitHub a push
//! records `github:commit:<sha>`, and the `pull_request.synchronize` that
//! follows aliases to it, because the pusher cannot know the pull request
//! number the notification will be keyed on.
//!
//! Read by three callers on [`MessageDao`](crate::message::dao::MessageDao):
//! the dedup gate, provenance routing, and the thread-context exclusion list.

use moatless_core::db::DbPool;

/// Record `external_event_id` as `task_id`'s own work.
///
/// Idempotent, and an id another Task already holds is left alone: an id names
/// one event, and the Task that made it is the one that recorded it first.
pub async fn record(
    pool: &DbPool,
    task_id: &str,
    external_event_id: &str,
    provider: &str,
    source_name: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO task_outbound_events
               (external_event_id, task_id, provider, source_name)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (external_event_id) DO NOTHING"#,
    )
    .bind(external_event_id)
    .bind(task_id)
    .bind(provider)
    .bind(source_name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Whether `task_id` produced any of `external_event_ids`.
pub async fn has_any(
    pool: &DbPool,
    task_id: &str,
    external_event_ids: &[String],
) -> Result<bool, sqlx::Error> {
    let row: (bool,) = sqlx::query_as(
        r#"SELECT EXISTS(
               SELECT 1 FROM task_outbound_events
               WHERE task_id = $1 AND external_event_id = ANY($2)
           )"#,
    )
    .bind(task_id)
    .bind(external_event_ids)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// The Task that produced any of `external_event_ids`, if one did.
pub async fn task_for_any(
    pool: &DbPool,
    external_event_ids: &[String],
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT task_id FROM task_outbound_events
           WHERE external_event_id = ANY($1)
           LIMIT 1"#,
    )
    .bind(external_event_ids)
    .fetch_optional(pool)
    .await
}
