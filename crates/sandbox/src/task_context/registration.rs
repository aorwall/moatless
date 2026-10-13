use super::models::{CheckoutMode, RegisterTaskContextError, TaskContext};
use crate::state::SandboxState;
use std::sync::Arc;

/// Register a task as a member of this sandbox process.
///
/// Idempotent per task: replaying `JoinTask` after a mid-join failure returns
/// the existing context. `agent_profile` overrides the harness on this Member's
/// `Config` clone; `None` leaves the pod's `AGENT_PROFILE`.
pub(crate) async fn register(
    state: &SandboxState,
    task_id: String,
    checkout_mode: Option<&str>,
    agent_profile: Option<&str>,
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
    // Refuse an unsupported harness here, not at `spawn_agent`: that answers
    // one by poisoning the Member's command channel, and `/livez` reads every
    // Member's channel, so one bad join would replace the whole shared pod
    // and take the other Members' turns with it.
    if let Some(raw) = agent_profile {
        crate::agent::handle::resolve_agent_harness(raw)
            .map_err(RegisterTaskContextError::UnsupportedAgentProfile)?;
    }
    let _registration = state.task_context_registration.lock().await;
    {
        let contexts = state.task_contexts.read().await;
        if let Some(context) = contexts.get(&task_id) {
            return Ok(Arc::clone(context));
        }
    }

    let git_credentials = state.git_credentials.read().await.clone();
    let (workspace_dir, checkout_dir) = super::checkout::materialize_checkout(
        &state.config,
        &git_credentials,
        &task_id,
        checkout_mode,
    )
    .await
    .map_err(RegisterTaskContextError::CheckoutMaterialization)?;
    // Before the insert below: `run_command` resolves this Member's registry
    // from `task_contexts`, and the first `run` mirrors absolutely over both
    // copies this reads.
    let lost_commands =
        crate::agent::restart::take_lost_commands(state, &task_id, &workspace_dir).await;

    let mut task_config = (*state.config).clone();
    task_config.task_id = task_id.clone();
    task_config.workspace_dir = workspace_dir;
    task_config.repo_dir = checkout_dir.clone();
    if let Some(raw) = agent_profile {
        task_config.agent_profile = Some(raw.to_string());
    }
    let task_config = Arc::new(task_config);

    let context = crate::compose::compose_task_context(task_config, checkout_mode, state).await;
    {
        let mut contexts = state.task_contexts.write().await;
        contexts.insert(task_id.clone(), Arc::clone(&context));
    }

    recover_interrupted_run(state, &context).await;
    report_lost_commands(state, task_id, lost_commands);
    Ok(context)
}

/// Replay a Member's interrupted Run after a pod replacement, once it joins
/// post-Ready.
///
/// Awaited under the caller's registration lock, which `open_for_work` shares
/// across its own recovery pass, so each Member is recovered exactly once.
async fn recover_interrupted_run(state: &SandboxState, context: &TaskContext) {
    if !state.is_ready().await {
        return;
    }
    let origin = *state.workspace_origin.read().await;
    crate::agent::restart::recover_interrupted_run_unless_pending(state, context, origin).await;
}

/// Wake a joining Member about commands a pod replacement destroyed.
///
/// Detached: `JoinTask` is the reconciler's own call, so a slow backend must
/// not hold the RPC open behind it.
fn report_lost_commands(
    state: &SandboxState,
    task_id: String,
    lost: Vec<crate::commands::JournaledCommand>,
) {
    let Some(text) = crate::agent::restart::lost_commands_wake(&lost) else {
        return;
    };
    let state = state.clone();
    crate::agent::dispatcher::spawn_supervised("member-lost-commands", async move {
        // No idempotency key: the take cleared both copies, so this Member is
        // owed exactly one wake and its text is what distinguishes it.
        crate::agent::dispatcher::queue_and_dispatch(&state, &task_id, text, None).await;
    });
}
