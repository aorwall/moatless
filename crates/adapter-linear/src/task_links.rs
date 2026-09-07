use crate::agent_api::LinearAgentApi;
use crate::text::format_attachment_title;
use moatless_core::config::Config;
use moatless_ports::task::TaskReader;
use moatless_types::access_control::ScopeContext;
use tracing::warn;

pub(super) async fn task_attachment_title(
    tasks: &dyn TaskReader,
    task_id: &str,
    prefix: &str,
) -> String {
    #[allow(
        clippy::disallowed_methods,
        reason = "attachment title fetch is a background formatting step, no caller identity"
    )]
    let sys = ScopeContext::system();
    match tasks.describe(task_id, &sys).await {
        Ok(Some(task)) => format_attachment_title(prefix, task.name.as_deref()),
        Ok(None) => prefix.to_string(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                task_id,
                "Failed to load task name for Linear attachment title"
            );
            prefix.to_string()
        }
    }
}

/// Attach the moatless task URL to a Linear issue as a "source" pill, titled
/// for the case where the Moatless side is the parent. Idempotent on
/// `(issue_id, url)`. Failures are logged and swallowed — the caller has already
/// created the issue / posted the comment this links back from.
pub(super) async fn attach_parent_task_link(
    config: &Config,
    tasks: &dyn TaskReader,
    api: &LinearAgentApi,
    issue_uuid: &str,
    task_id: &str,
) {
    let url = super::text::task_url(config, task_id);
    let title = task_attachment_title(tasks, task_id, super::text::ATTACHMENT_TITLE_PARENT).await;
    if let Err(e) = api.attachment_link_url(issue_uuid, &url, &title).await {
        warn!(
            error = %e,
            task_id = %task_id,
            issue = %issue_uuid,
            "Failed to attach Moatless task URL to Linear issue"
        );
    }
}
