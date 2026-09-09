//! The external events a Task caused.
//!
//! A provider webhooks a Task's own change straight back, so without this table
//! the Task reads its own comment as a new inbound message and answers it.
//!
//! Every row is outbound, and is written under the id the *inbound* event will
//! carry, so matching an arriving webhook is equality on one column. Where the
//! writer cannot know that id the Adapter maps between the two in one place: a
//! GitHub push records `github:commit:<sha>` and the following
//! `pull_request.synchronize` aliases to it.
//!
//! A row is a fact about the Task, so nothing here names the Message or tool
//! call behind it. The inbound side is `task_messages.external_event_id`; a
//! reader that cannot tell the two apart cannot tell a redelivered webhook from
//! its own echo.

use moatless_core::db::DbPool;

/// Record `external_event_id` as `task_id`'s own work.
///
/// Idempotent, and an id another Task already holds is left alone: an id names
/// one event, and the Task that made it is the one that recorded it first.
pub(crate) async fn record(
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
pub(crate) async fn has_any(
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

/// Every external event id `task_id` caused.
pub(crate) async fn ids_for_task(pool: &DbPool, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(r#"SELECT external_event_id FROM task_outbound_events WHERE task_id = $1"#)
        .bind(task_id)
        .fetch_all(pool)
        .await
}

/// The Task that produced any of `external_event_ids`, if one did.
pub(crate) async fn task_for_any(
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

/// The Task that produced any of `external_event_ids` **at `source_name`**,
/// if one did.
///
/// The table is keyed on the id alone, because that is all a writer holds, and
/// it holds every source's rows together. `source_name` is the column that
/// tells them apart, so a caller that knows where it is asking says so and
/// reads back only its own. That an id names its own subject — GitHub's do,
/// `owner/repo#607` rather than `607` — is a property of each provider's codec,
/// and this answer does not rest on it: a row written under an earlier shape,
/// or by a provider that numbers per subject, is placed by its source here
/// either way.
///
/// A row that records no source is not a match: it cannot be shown to have
/// happened here, and naming the wrong Task as an event's author is what the
/// scoping exists to prevent.
pub(crate) async fn task_for_any_from_source(
    pool: &DbPool,
    external_event_ids: &[String],
    provider: &str,
    source_name: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT task_id FROM task_outbound_events
           WHERE external_event_id = ANY($1)
             AND provider = $2
             AND source_name = $3
           LIMIT 1"#,
    )
    .bind(external_event_ids)
    .bind(provider)
    .bind(source_name)
    .fetch_optional(pool)
    .await
}
