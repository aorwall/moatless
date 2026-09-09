use super::error::FeedbackError;
use super::models::Feedback;
use super::operations::FeedbackNotes;
use super::schemas::{CreateFeedbackRequest, UpdateFeedbackStatusRequest};
use crate::task::operations::TaskCommands;
use chrono::Utc;
use moatless_types::access_control::ScopeContext;
use moatless_types::feedback::{FeedbackSource, FeedbackStatus, FeedbackSurface};

const MAX_STATUS_NOTE: usize = 2_000;

/// Record task feedback with caller access and activity selection.
pub async fn create_feedback(
    tasks: &TaskCommands,
    feedback: &FeedbackNotes,
    scope: &ScopeContext,
    source: FeedbackSource,
    req: CreateFeedbackRequest,
) -> Result<Feedback, FeedbackError> {
    // Validate scope: tool_use_id requires message_uuid.
    if req.tool_use_id.is_some() && req.message_uuid.is_none() {
        return Err(FeedbackError::BadRequest(
            "tool_use_id requires message_uuid to be set".to_string(),
        ));
    }

    // Validate scope is consistent with surface.
    match (
        req.surface,
        req.message_uuid.is_some(),
        req.tool_use_id.is_some(),
    ) {
        (FeedbackSurface::Task, false, false) => {}
        (FeedbackSurface::Message, true, false) => {}
        (FeedbackSurface::ToolCall, true, true) => {}
        (FeedbackSurface::PreviewFailure, _, _) => {}
        (surface, _, _) => {
            return Err(FeedbackError::BadRequest(format!(
                "surface={surface} does not match the provided scope (message_uuid, tool_use_id)"
            )));
        }
    }

    if req.comment.trim().is_empty() {
        return Err(FeedbackError::BadRequest(
            "comment must not be empty".to_string(),
        ));
    }
    if req.comment.len() > 8_000 {
        return Err(FeedbackError::BadRequest(
            "comment too long (max 8000)".to_string(),
        ));
    }

    // Verify task exists and the caller has access.
    let _task = tasks
        .find_by_id(&req.task_id, scope)
        .await?
        .ok_or_else(|| FeedbackError::TaskNotFound(req.task_id.clone()))?;

    // A caller who names no message asked for task-level feedback, but the
    // common case — the agent running `moat feedback` with no flags — means
    // "about what I am doing right now", not "about the task as a whole".
    // Best effort: point it at the task's latest activity instead. A caller
    // that did name a message (or asked for `preview_failure`) keeps exactly
    // what it sent.
    let (message_uuid, tool_use_id, surface) =
        if req.surface == FeedbackSurface::Task && req.message_uuid.is_none() {
            match feedback.latest_activity(&req.task_id).await? {
                Some(activity) if activity.tool_use_id.is_some() => (
                    Some(activity.message_uuid),
                    activity.tool_use_id,
                    FeedbackSurface::ToolCall,
                ),
                Some(activity) => (Some(activity.message_uuid), None, FeedbackSurface::Message),
                None => (None, None, FeedbackSurface::Task),
            }
        } else {
            (
                req.message_uuid.clone(),
                req.tool_use_id.clone(),
                req.surface,
            )
        };

    let row = Feedback {
        feedback_id: uuid::Uuid::new_v4().to_string(),
        task_id: req.task_id.clone(),
        message_uuid,
        tool_use_id,
        source: source.as_str().to_string(),
        surface: surface.as_str().to_string(),
        category: req.category.map(|c| c.as_str().to_string()),
        comment: req.comment.clone(),
        metadata: req.metadata.clone(),
        created_by: scope.user_id.clone(),
        created_at: Utc::now(),
        status: FeedbackStatus::Open.as_str().to_string(),
        status_note: None,
        status_updated_by: None,
        status_updated_at: None,
    };

    feedback.insert(&row).await?;
    tracing::info!(
        task_id = %row.task_id,
        feedback_id = %row.feedback_id,
        surface = %row.surface,
        source = %row.source,
        category = ?row.category,
        message_uuid = ?row.message_uuid,
        tool_use_id = ?row.tool_use_id,
        "Feedback recorded"
    );

    Ok(row)
}

/// Set feedback status when its task is readable.
pub async fn set_feedback_status(
    tasks: &TaskCommands,
    feedback: &FeedbackNotes,
    scope: &ScopeContext,
    feedback_id: String,
    req: UpdateFeedbackStatusRequest,
) -> Result<Feedback, FeedbackError> {
    let note = req
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty())
        .map(str::to_string);
    if note
        .as_deref()
        .is_some_and(|note| note.len() > MAX_STATUS_NOTE)
    {
        return Err(FeedbackError::BadRequest(format!(
            "note too long (max {MAX_STATUS_NOTE})"
        )));
    }

    let row = feedback
        .find_by_id(&feedback_id)
        .await?
        .ok_or_else(|| FeedbackError::FeedbackNotFound(feedback_id.clone()))?;

    // Access control reads the note's task: `feedback` carries no scope columns
    // of its own, so a row is writable exactly when its task is readable.
    tasks
        .find_by_id(&row.task_id, scope)
        .await?
        .ok_or_else(|| FeedbackError::FeedbackNotFound(feedback_id.clone()))?;

    let updated = feedback
        .update_status(
            &feedback_id,
            req.status.as_str(),
            note,
            &scope.user_id.clone(),
        )
        .await?
        .ok_or_else(|| FeedbackError::FeedbackNotFound(feedback_id.clone()))?;

    tracing::info!(
        task_id = %updated.task_id,
        feedback_id = %updated.feedback_id,
        status = %updated.status,
        status_updated_by = ?updated.status_updated_by,
        "Feedback status set"
    );

    Ok(updated)
}
