use super::models::{CheckoutMode, RegisterTaskContextError, TaskContext};
use crate::state::SandboxState;
use std::sync::Arc;

/// Register a task as a member of this sandbox process.
///
/// Idempotent per task: replaying `JoinTask` after a mid-join failure
/// returns the existing context and does not spawn another agent loop.
pub(crate) async fn register(
    state: &SandboxState,
    task_id: String,
    checkout_mode: Option<&str>,
) -> Result<Arc<TaskContext>, RegisterTaskContextError> {
    if task_id.is_empty() {
        return Err(RegisterTaskContextError::EmptyTaskId);
    }

    {
        let contexts = state.task_contexts.read().await;
        if let Some(context) = contexts.get(&task_id) {
            return Ok(Arc::clone(context));
        }
    }

    let checkout_mode = CheckoutMode::parse(checkout_mode)?;
    let _registration = state.task_context_registration.lock().await;
    {
        let contexts = state.task_contexts.read().await;
        if let Some(context) = contexts.get(&task_id) {
            return Ok(Arc::clone(context));
        }
    }

    let (workspace_dir, checkout_dir) =
        super::checkout::materialize_checkout(&state.config, &task_id, checkout_mode)
            .await
            .map_err(RegisterTaskContextError::CheckoutMaterialization)?;
    let mut task_config = (*state.config).clone();
    task_config.task_id = task_id.clone();
    task_config.workspace_dir = workspace_dir;
    task_config.repo_dir = checkout_dir.clone();
    let task_config = Arc::new(task_config);

    let context = crate::compose::compose_task_context(task_config, checkout_mode, state).await;
    let mut contexts = state.task_contexts.write().await;
    contexts.insert(task_id, Arc::clone(&context));
    Ok(context)
}
