//! Secret request persistence: the two task events and their activities.
//! Callers check Task access first; neither event carries a scope column.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::conversation::projector::Projector;
use crate::task_event::models::InsertTaskEvent;
use moatless_core::db::DbPool;
use moatless_types::activity::{ActivityKind, SecretRequestAnswered, SecretRequested};
use moatless_types::event::EventType;
use moatless_types::task::TaskSubType;

/// Store `asked` as its task event and project its activity, in one
/// transaction so the card appears with the event.
pub(crate) async fn create(
    db: &DbPool,
    task_id: &str,
    asked: &SecretRequested,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    let event = event(
        &mut tx,
        task_id,
        asked.request_id.clone(),
        TaskSubType::SecretRequested,
        payload(asked)?,
    )
    .await?;
    crate::task_event::dao::insert_in(&mut tx, &event).await?;
    Projector.on_task_event(&mut tx, &event).await?;
    tx.commit().await
}

/// The request whose `SecretRequested` activity is `activity_id` on `task_id`,
/// and whether it has been answered.
pub(crate) async fn find(
    db: &DbPool,
    task_id: &str,
    activity_id: Uuid,
) -> Result<Option<(SecretRequested, bool)>, sqlx::Error> {
    let payload: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT payload FROM conversation_activities
            WHERE activity_id = $1 AND task_id = $2 AND kind = $3"#,
    )
    .bind(activity_id)
    .bind(task_id)
    .bind(ActivityKind::SecretRequested.as_str())
    .fetch_optional(db)
    .await?;
    let Some(asked) = payload.and_then(|p| serde_json::from_value::<SecretRequested>(p).ok())
    else {
        return Ok(None);
    };
    let answered: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM task_events WHERE event_id = $1)")
            .bind(answered_event_id(&asked.request_id))
            .fetch_one(db)
            .await?;
    Ok(Some((asked, answered)))
}

/// Store `answered` and project its activity. `false` when the request already
/// had an answer: the event id is derived from the request id, so a second
/// answer conflicts rather than adding a row.
pub(crate) async fn record_answer(
    db: &DbPool,
    task_id: &str,
    answered: &SecretRequestAnswered,
) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;
    let event = event(
        &mut tx,
        task_id,
        answered_event_id(&answered.request_id),
        TaskSubType::SecretRequestAnswered,
        payload(answered)?,
    )
    .await?;
    if !crate::task_event::dao::insert_in(&mut tx, &event).await? {
        return Ok(false);
    }
    Projector.on_task_event(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(true)
}

fn payload(value: &impl serde::Serialize) -> Result<serde_json::Value, sqlx::Error> {
    serde_json::to_value(value).map_err(|e| sqlx::Error::Encode(Box::new(e)))
}

fn answered_event_id(request_id: &str) -> String {
    let name = format!("secret_request_answered:{request_id}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()).to_string()
}

/// A task event in the Task's latest turn.
async fn event(
    tx: &mut PgConnection,
    task_id: &str,
    event_id: String,
    sub_type: TaskSubType,
    payload: serde_json::Value,
) -> Result<InsertTaskEvent, sqlx::Error> {
    let turn_number: Option<i32> =
        sqlx::query_scalar("SELECT MAX(turn_number) FROM task_turns WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(&mut *tx)
            .await?;
    Ok(InsertTaskEvent {
        event_id,
        task_id: task_id.to_string(),
        event_type: EventType::Task.as_str().to_string(),
        sub_type: sub_type.as_str().to_string(),
        is_error: false,
        agent_profile: None,
        turn_number,
        payload,
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    })
}
