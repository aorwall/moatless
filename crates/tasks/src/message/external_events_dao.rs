//! The external events a Task received, read off the Messages that carry them.
//!
//! `task_messages.external_event_id` is the inbound side of the echo check
//! `crate::task::outbound_events_dao` documents: a Message the Task received
//! under an id, against an event the Task itself caused. Every read here falls
//! through to that table, because a webhook redelivery has to match either.

use moatless_core::db::DbPool;
use moatless_ports::task::EventSubject;

/// Stamp a Message with the external event it arrived as; returns whether it
/// took. A Message that already names an event keeps it — the first claim wins;
/// a later write would rename history.
pub(crate) async fn set_source_metadata(
    pool: &DbPool,
    message_uuid: &str,
    external_id: &str,
    provider: &str,
    source_name: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"UPDATE task_messages
           SET external_event_id = $2,
               external_adapter    = $3,
               external_source_name = $4
           WHERE uuid = $1 AND external_event_id IS NULL"#,
    )
    .bind(message_uuid)
    .bind(external_id)
    .bind(provider)
    .bind(source_name)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Whether `task_id` already holds `external_id`.
pub(crate) async fn has(
    pool: &DbPool,
    task_id: &str,
    external_id: &str,
) -> Result<bool, sqlx::Error> {
    has_any(
        pool,
        task_id,
        std::slice::from_ref(&external_id.to_string()),
    )
    .await
}

/// Whether `task_id` already holds any of `external_ids`.
pub(crate) async fn has_any(
    pool: &DbPool,
    task_id: &str,
    external_ids: &[String],
) -> Result<bool, sqlx::Error> {
    if external_ids.is_empty() {
        return Ok(false);
    }
    let row: (bool,) = sqlx::query_as(
        r#"SELECT EXISTS(
            SELECT 1 FROM task_messages
            WHERE task_id = $1 AND external_event_id = ANY($2)
        )"#,
    )
    .bind(task_id)
    .bind(external_ids)
    .fetch_one(pool)
    .await?;

    if row.0 {
        return Ok(true);
    }
    // An event the Task caused through a command rather than through a
    // Message it wrote.
    crate::task::outbound_events_dao::has_any(pool, task_id, external_ids).await
}

/// Every external event id `task_id` holds, inbound and outbound.
pub(crate) async fn ids_for_task(pool: &DbPool, task_id: &str) -> Result<Vec<String>, sqlx::Error> {
    let mut ids: Vec<String> = sqlx::query_scalar(
        r#"SELECT external_event_id FROM task_messages
           WHERE task_id = $1 AND external_event_id IS NOT NULL"#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;

    ids.extend(crate::task::outbound_events_dao::ids_for_task(pool, task_id).await?);
    Ok(ids)
}

/// The Task holding any of `external_ids`, most recent first.
pub(crate) async fn task_for_any(
    pool: &DbPool,
    external_ids: &[String],
) -> Result<Option<String>, sqlx::Error> {
    if external_ids.is_empty() {
        return Ok(None);
    }

    let by_message = sqlx::query_scalar::<_, String>(
        r#"SELECT task_id
           FROM task_messages
           WHERE external_event_id = ANY($1)
           ORDER BY id DESC
           LIMIT 1"#,
    )
    .bind(external_ids)
    .fetch_optional(pool)
    .await?;

    if let Some(task_id) = by_message {
        return Ok(Some(task_id));
    }
    crate::task::outbound_events_dao::task_for_any(pool, external_ids).await
}

/// The Task holding any of `external_ids` on one adapter's subject.
pub(crate) async fn task_for_any_on_subject(
    pool: &DbPool,
    external_ids: &[String],
    subject: EventSubject<'_>,
) -> Result<Option<String>, sqlx::Error> {
    if external_ids.is_empty() {
        return Ok(None);
    }

    let by_message = sqlx::query_scalar::<_, String>(
        r#"SELECT task_id
           FROM task_messages
           WHERE external_event_id = ANY($1)
             AND external_adapter = $2
             AND external_subject_key = $3
           ORDER BY id DESC
           LIMIT 1"#,
    )
    .bind(external_ids)
    .bind(subject.adapter)
    .bind(subject.subject_key)
    .fetch_optional(pool)
    .await?;

    if let Some(task_id) = by_message {
        return Ok(Some(task_id));
    }
    crate::task::outbound_events_dao::task_for_any_from_source(
        pool,
        external_ids,
        subject.adapter,
        subject.source_name,
    )
    .await
}
