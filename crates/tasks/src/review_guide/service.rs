//! What a guide-writing Task's agent asks for and submits.

use chrono::Utc;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::Task;

use super::error::ReviewGuideError;
use super::models::{ReviewGuideRun, StoredReviewGuide};
use super::normalize::normalize;
use super::operations::ReviewGuides;
use super::schemas::{ReviewGuideRunResponse, SubmitReviewGuideRequest, SubmitReviewGuideResponse};
use crate::task::operations::TaskCommands;

/// Most stops a guide may have. The prompt asks for at most eight; this only
/// bounds what is stored.
const MAX_STOPS: usize = 20;

/// The pull request and range `task_id` was started to write a guide for.
pub async fn run_for_task(
    tasks: &TaskCommands,
    guides: &ReviewGuides,
    scope: &ScopeContext,
    task_id: &str,
) -> Result<ReviewGuideRunResponse, ReviewGuideError> {
    writable_task(tasks, scope, task_id).await?;
    let run = running_run(guides, task_id).await?;
    Ok(ReviewGuideRunResponse {
        repository: run.repository,
        number: u64::try_from(run.number).unwrap_or_default(),
        base_sha: run.base_sha,
        head_sha: run.head_sha,
    })
}

/// Hold `req` to its change and store it as the guide of the pull request
/// `task_id`'s run is writing.
///
/// A guide whose every excerpt fell outside the change is refused rather than
/// stored, because that means the line numbers were counted on the wrong side
/// or against the wrong range, which the agent can fix and resubmit.
pub async fn submit(
    tasks: &TaskCommands,
    guides: &ReviewGuides,
    scope: &ScopeContext,
    task_id: &str,
    req: SubmitReviewGuideRequest,
) -> Result<SubmitReviewGuideResponse, ReviewGuideError> {
    let task = writable_task(tasks, scope, task_id).await?;
    if req.stops.is_empty() {
        return Err(ReviewGuideError::BadRequest(
            "A guide needs at least one stop.".to_string(),
        ));
    }
    if req.stops.len() > MAX_STOPS {
        return Err(ReviewGuideError::BadRequest(format!(
            "A guide has at most {MAX_STOPS} stops; this one has {}.",
            req.stops.len()
        )));
    }
    if req.change.files.is_empty() {
        return Err(ReviewGuideError::BadRequest(
            "The change lists no files. Run `moat review-guide submit` from the checkout, \
             with the pull request's base and head commits fetched."
                .to_string(),
        ));
    }
    let run = running_run(guides, task_id).await?;

    let normalized = normalize(req.stops, &req.change);
    if normalized.submitted_excerpts > 0 && normalized.excerpts() == 0 {
        return Err(ReviewGuideError::BadRequest(format!(
            "None of the {} excerpts is inside the change. Excerpt lines must fall inside a \
             hunk of `git diff {}...{}`, counted on the new side unless the excerpt says \
             side \"old\". Fix the line numbers and submit again.",
            normalized.submitted_excerpts, run.base_sha, run.head_sha
        )));
    }

    let response = SubmitReviewGuideResponse {
        stops: normalized.stops.len(),
        excerpts: normalized.excerpts(),
        dropped_excerpts: normalized.dropped_excerpts,
        dropped_notes: normalized.dropped_notes,
        unplaced_files: normalized.unplaced_files,
    };
    let stored = StoredReviewGuide {
        head_sha: run.head_sha.clone(),
        guide: serde_json::json!({ "stops": serde_json::to_value(&normalized.stops)? }),
        task_id: task.task_id.clone(),
        agent_profile: Some(task.agent_profile.to_string()),
        model: task.model.clone(),
        generated_at: Utc::now(),
    };
    if !guides.complete_run(&run.run_id, &stored).await? {
        return Err(ReviewGuideError::NoRun(
            "A newer guide run replaced this one before it finished, so this guide was not \
             stored."
                .to_string(),
        ));
    }
    Ok(response)
}

async fn writable_task(
    tasks: &TaskCommands,
    scope: &ScopeContext,
    task_id: &str,
) -> Result<Task, ReviewGuideError> {
    let task = tasks
        .find_by_id(task_id, scope)
        .await?
        .ok_or_else(|| ReviewGuideError::TaskNotFound(task_id.to_string()))?;
    if !scope.can_write(&task.scoped_resource()) {
        return Err(ReviewGuideError::Forbidden(task_id.to_string()));
    }
    Ok(task)
}

async fn running_run(
    guides: &ReviewGuides,
    task_id: &str,
) -> Result<ReviewGuideRun, ReviewGuideError> {
    guides.running_run_for_task(task_id).await?.ok_or_else(|| {
        ReviewGuideError::NoRun(
            "This task is not writing a review guide: none was started for it, a newer run \
                 replaced it, or it was marked failed."
                .to_string(),
        )
    })
}
