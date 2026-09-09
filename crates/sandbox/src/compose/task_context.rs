use crate::agent::agent_state::AgentState;
use crate::agent::handle::spawn_agent;
use crate::agent::scheduled_job::{LateInjector, ScheduledJobRegistry};
use crate::commands::CommandRegistry;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::state::{CheckoutMode, SandboxState, TaskContext};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

pub(crate) async fn compose_task_context(
    task_config: Arc<Config>,
    checkout_mode: CheckoutMode,
    shared: &SandboxState,
) -> Arc<TaskContext> {
    let agent_state = Arc::new(AgentState::new());
    let late_injector = Arc::new(LateInjector::new());
    let scheduled_jobs = Arc::new(ScheduledJobRegistry::new(Arc::clone(&late_injector) as _));
    let commands = Arc::new(
        CommandRegistry::new(
            Duration::from_secs(task_config.command_default_timeout_secs),
            crate::commands::default_log_dir(&task_config.task_id),
            task_config.task_id.clone(),
        )
        .with_journal(crate::commands::default_journal_path(
            &task_config.workspace_dir,
        ))
        .with_backend_mirror(Arc::clone(&shared.backend_client))
        .with_liveness(shared.events.clone()),
    );
    commands.report_initial_liveness().await;
    let credential_failures = CredentialFailureDetector::default();
    let system_prompt_cache: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

    let agent_handle = spawn_agent(
        Arc::clone(&task_config),
        Arc::clone(&agent_state),
        shared.events.clone(),
        Arc::clone(&shared.credentials),
        Arc::clone(&shared.backend_client),
        Arc::clone(&scheduled_jobs),
        Arc::clone(&commands),
        credential_failures.clone(),
        Arc::clone(&shared.credential_dispatcher),
        Arc::clone(&system_prompt_cache),
        shared.heartbeat.clone(),
        shared.workspace_sync.clone(),
    );
    late_injector.set(Box::new(agent_handle.clone()));

    Arc::new(TaskContext::new(
        task_config.task_id.clone(),
        task_config.workspace_dir.clone(),
        task_config.repo_dir.clone(),
        checkout_mode,
        agent_state,
        agent_handle,
        scheduled_jobs,
        commands,
        credential_failures,
        system_prompt_cache,
    ))
}
