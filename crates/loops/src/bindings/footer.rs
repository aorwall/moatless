//! The `task_comment_footer` wrapper: resolves a task's name through the task
//! DAO, then renders the shared footer. The pure render/parse layer lives in
//! `moatless_adapter_model::footer`.

use moatless_adapter_model::footer::render_footer;
use moatless_types::access_control::ScopeContext;

use moatless_tasks::task::dao::TaskDao;

/// Build the footer suffix appended to an outbound comment body, or `None` when
/// no frontend URL is configured. Loads the task name (best effort) to use as
/// the link text.
pub async fn task_comment_footer(
    tasks: &dyn TaskDao,
    frontend_url: &str,
    task_id: &str,
) -> Option<String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "footer lookup is a background formatting step with no caller identity"
    )]
    let sys = ScopeContext::system();
    let name = match tasks.find_by_id(task_id, &sys).await {
        Ok(Some(task)) => task.name,
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to load task name for comment footer");
            None
        }
    };
    render_footer(frontend_url, task_id, name.as_deref())
}
