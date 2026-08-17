//! Closing the Task an issue produced.
//!
//! Reached from two workflows — an `Issue.update` and an `AppUserNotification`
//! can each be the first to observe the issue entering a terminal state — which
//! is why this is its own module rather than a dependency between the two.
//!
//! Closing is the one irreversible thing this Adapter does to a Task, so
//! eligibility is four separate questions, each of which can refuse on its own:
//! the Binding must be the Task's only one (a Task also bound to a GitHub PR is
//! not this issue's to close), it must have been created by an adapter event
//! rather than by hand, it must name the trigger that is asking, and the Task
//! must not already be closed. Everything after the decision is best-effort and
//! logged: the Task is closed first, and a failure to post the closing
//! activity or reclaim the sandbox does not un-close it.

use std::fmt::Write as _;

use tracing::{debug, info, warn};

use moatless_backend_core::models::TaskStatus;
use moatless_backend_core::scope::ScopeContext;
use moatless_core::config::Config;
use moatless_core::db::DbPool;
use moatless_core::events::EventPublisher;
use moatless_loops::adapters::error::AdapterError;
use moatless_loops::adapters::models::{
    ADAPTER_EVENT_BINDING_ORIGIN, SubscriptionMatch, TaskBinding,
};
use moatless_sandbox_control::lifecycle::{ArchiveRetention, Lifecycle};
use moatless_tasks::task::binding_dao::TaskBindingDao;
use moatless_tasks::task::dao::{TaskDao, TaskPatch};

use super::agent_api::{ActivityKind, LinearAgentApi};
use super::webhook_payloads::IssuePayload;

/// Close every eligible Task bound to `issue`.
///
/// Scoped by `rule`, not by Adapter connection: a Binding names the trigger
/// that created it, and only that trigger's terminal states may close it. The
/// connection is already implied — the trigger belongs to one.
#[expect(
    clippy::too_many_arguments,
    reason = "the six downward ports this close path reads (bindings, task, \
              config, sandbox, events, plus the db the binding query needs) \
              inverted out of the single AppState it used to carry; there is no \
              narrower grouping that is not itself an AppState-shaped bag"
)]
pub(super) async fn close_active_bindings_for_issue(
    db: &DbPool,
    task_bindings: &dyn TaskBindingDao,
    tasks: &dyn TaskDao,
    config: &Config,
    sandbox: &Lifecycle,
    events: Option<&dyn EventPublisher>,
    api: &LinearAgentApi,
    rule: &SubscriptionMatch,
    issue: &IssuePayload,
) -> Result<(), AdapterError> {
    let rows =
        super::subject::bindings_in_scope(db, super::subject::BindingScope::OnIssue(&issue.id))
            .await?;

    for binding in rows {
        if !binding_is_close_eligible(task_bindings, tasks, rule, issue, &binding).await {
            continue;
        }
        close_linear_bound_task(
            task_bindings,
            tasks,
            config,
            sandbox,
            events,
            api,
            issue,
            &binding,
        )
        .await;
    }

    Ok(())
}

async fn binding_is_close_eligible(
    task_bindings: &dyn TaskBindingDao,
    tasks: &dyn TaskDao,
    rule: &SubscriptionMatch,
    issue: &IssuePayload,
    binding: &TaskBinding,
) -> bool {
    if !sole_binding_for_task(task_bindings, issue, binding).await {
        return false;
    }

    if binding.origin != ADAPTER_EVENT_BINDING_ORIGIN
        || binding.loop_id.as_deref() != Some(rule.id.as_str())
    {
        debug!(
            task_id = %binding.task_id,
            issue = %issue.identifier,
            "Binding not eligible for auto-close (different origin or rule)"
        );
        return false;
    }

    task_can_be_closed(tasks, issue, binding).await
}

async fn sole_binding_for_task(
    binding_repo: &dyn TaskBindingDao,
    issue: &IssuePayload,
    binding: &TaskBinding,
) -> bool {
    let siblings_count = match binding_repo.find_by_task_id(&binding.task_id).await {
        Ok(rows) => rows.len(),
        Err(e) => {
            warn!(
                task_id = %binding.task_id,
                issue = %issue.identifier,
                error = %e,
                "Failed to look up sibling bindings — skipping auto-close"
            );
            return false;
        }
    };

    if siblings_count > 1 {
        info!(
            task_id = %binding.task_id,
            issue = %issue.identifier,
            other_binding_count = siblings_count - 1,
            "Linear issue closed but task has other connected bindings — leaving task open"
        );
        return false;
    }

    true
}

async fn task_can_be_closed(
    tasks: &dyn TaskDao,
    issue: &IssuePayload,
    binding: &TaskBinding,
) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "Linear issue-close handler runs as background system, no caller identity"
    )]
    let sys = ScopeContext::system();
    match tasks.find_by_id(&binding.task_id, &sys).await {
        Ok(Some(task)) if task.status == TaskStatus::Closed => {
            debug!(task_id = %binding.task_id, issue = %issue.identifier, "Task already closed, skipping");
            false
        }
        Ok(None) => {
            warn!(task_id = %binding.task_id, issue = %issue.identifier, "Task not found for close");
            false
        }
        Err(e) => {
            warn!(task_id = %binding.task_id, error = %e, "Failed to fetch task for close");
            false
        }
        _ => true,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the five downward ports the close effects touch (bindings, task, \
              config, sandbox, events) inverted out of the single AppState it \
              used to carry; there is no narrower grouping that is not itself an \
              AppState-shaped bag"
)]
async fn close_linear_bound_task(
    task_bindings: &dyn TaskBindingDao,
    tasks: &dyn TaskDao,
    config: &Config,
    sandbox: &Lifecycle,
    events: Option<&dyn EventPublisher>,
    api: &LinearAgentApi,
    issue: &IssuePayload,
    binding: &TaskBinding,
) {
    emit_linear_close_response(tasks, config, api, issue, binding).await;

    if !mark_task_closed(tasks, binding).await {
        return;
    }

    remove_sandbox_after_linear_close(config, sandbox, binding).await;
    deactivate_linear_task_bindings(task_bindings, binding).await;
    publish_linear_task_closed(events, issue, binding).await;

    info!(
        task_id = %binding.task_id,
        issue = %issue.identifier,
        "Task closed due to Linear issue terminal state"
    );
}

async fn emit_linear_close_response(
    tasks: &dyn TaskDao,
    config: &Config,
    api: &LinearAgentApi,
    issue: &IssuePayload,
    binding: &TaskBinding,
) {
    let session_id = binding
        .destination_config
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return;
    }

    let mut text = format!(
        "Linear issue {} transitioned to {}; closing this task.",
        issue.identifier,
        issue.state_name()
    );
    if let Some(ref url) = issue.url {
        let _ = write!(text, "\n\n{url}");
    }
    // Append the `Moatless task:` footer so the inbound echo filter drops the
    // resulting webhook instead of forwarding it back into the task.
    if let Some(footer) = moatless_loops::bindings::footer::task_comment_footer(
        tasks,
        &config.auth.frontend_url,
        &binding.task_id,
    )
    .await
    {
        text.push_str(&footer);
    }

    if let Err(e) = api
        .agent_activity_create(session_id, ActivityKind::Response, &text, false)
        .await
    {
        warn!(session_id, error = %e, "Failed to emit close response activity");
    }
}

async fn mark_task_closed(tasks: &dyn TaskDao, binding: &TaskBinding) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "Linear issue-close handler runs as background system, no caller identity"
    )]
    let sys = ScopeContext::system();
    match tasks
        .update_task(
            &binding.task_id,
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..Default::default()
            },
            &sys,
        )
        .await
    {
        Ok(_) => true,
        Err(e) => {
            warn!(task_id = %binding.task_id, error = %e, "Failed to close task");
            false
        }
    }
}

async fn remove_sandbox_after_linear_close(
    config: &Config,
    sandbox: &Lifecycle,
    binding: &TaskBinding,
) {
    let retention = ArchiveRetention::on_close(config.sandbox.retain_storage_on_close);
    if let Err(e) = sandbox.remove(&binding.task_id, retention).await {
        warn!(task_id = %binding.task_id, error = %e, "Failed to remove sandbox on Linear close");
    }
}

async fn deactivate_linear_task_bindings(binding_repo: &dyn TaskBindingDao, binding: &TaskBinding) {
    if let Err(e) = binding_repo.deactivate_by_task(&binding.task_id).await {
        warn!(task_id = %binding.task_id, error = %e, "Failed to deactivate bindings on Linear close");
    }
}

async fn publish_linear_task_closed(
    events: Option<&dyn EventPublisher>,
    issue: &IssuePayload,
    binding: &TaskBinding,
) {
    if let Some(events) = events {
        events
            .publish_task_event(
                &binding.task_id,
                moatless_events::TaskSubType::Closed,
                Some("claude-code"),
                serde_json::json!({
                    "reason": "linear_state_terminal",
                    "issue_identifier": issue.identifier,
                    "state_name": issue.state_name(),
                }),
            )
            .await;
    }
}
