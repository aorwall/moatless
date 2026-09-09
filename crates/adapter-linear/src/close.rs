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
//! must not already be closed. The close itself belongs to
//! [`TaskCloser`](moatless_tasks::task::closer::TaskCloser) — this module
//! decides *whether*, and carries the Linear-shaped `Closed` event.

use std::fmt::Write as _;

use tracing::{debug, info, warn};

use moatless_adapter_model::binding::ADAPTER_EVENT_BINDING_ORIGIN;
use moatless_adapter_model::error::AdapterError;
use moatless_core::config::Config;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::task::TaskReader;
use moatless_ports::task::{CloseReason, TaskFinisher};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::TaskBinding;
use moatless_types::loops::SubscriptionMatch;
use moatless_types::task::TaskStatus;

use super::agent_api::{ActivityKind, LinearAgentApi};
use super::webhook_payloads::IssuePayload;

/// Close every eligible Task bound to `issue`.
///
/// Scoped by `rule`, not by Adapter connection: a Binding names the trigger
/// that created it, and only that trigger's terminal states may close it. The
/// connection is already implied — the trigger belongs to one.
pub(super) async fn close_active_bindings_for_issue(
    task_bindings: &Bindings,
    tasks: &dyn TaskReader,
    config: &Config,
    task_closer: &dyn TaskFinisher,
    api: &LinearAgentApi,
    rule: &SubscriptionMatch,
    issue: &IssuePayload,
) -> Result<(), AdapterError> {
    let rows = super::subject::bindings_in_scope(
        task_bindings,
        super::subject::BindingScope::OnIssue(&issue.id),
    )
    .await?;

    for binding in rows {
        if !binding_is_close_eligible(task_bindings, tasks, rule, issue, &binding).await {
            continue;
        }
        close_linear_bound_task(tasks, config, task_closer, api, issue, &binding).await;
    }

    Ok(())
}

async fn binding_is_close_eligible(
    task_bindings: &Bindings,
    tasks: &dyn TaskReader,
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
    binding_repo: &Bindings,
    issue: &IssuePayload,
    binding: &TaskBinding,
) -> bool {
    let siblings_count = match binding_repo.for_task(&binding.task_id).await {
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

/// Whether the Task is still open.
///
/// [`TaskCloser::close_single`] repeats this check, but too late to be a
/// substitute: `emit_linear_close_response` posts to Linear *before* the close
/// runs, so an already-closed Task would get a second comment on every repeated
/// terminal delivery. Both checks are load-bearing.
async fn task_can_be_closed(
    tasks: &dyn TaskReader,
    issue: &IssuePayload,
    binding: &TaskBinding,
) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "Linear issue-close handler runs as background system, no caller identity"
    )]
    let sys = ScopeContext::system();
    match tasks.describe(&binding.task_id, &sys).await {
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

async fn close_linear_bound_task(
    tasks: &dyn TaskReader,
    config: &Config,
    task_closer: &dyn TaskFinisher,
    api: &LinearAgentApi,
    issue: &IssuePayload,
    binding: &TaskBinding,
) {
    // Post the closing activity while the Task is still open — see
    // `task_can_be_closed`.
    emit_linear_close_response(tasks, config, api, issue, binding).await;

    // Terminate through the task band's own finisher: it persists `Closed`,
    // removes the sandbox, deactivates the bindings, and publishes the event.
    // Only the close reason is this Adapter's — the `"claude-code"` actor and
    // the `{reason, issue_identifier, state_name}` payload the UI renders.
    let close_reason = CloseReason {
        actor: "claude-code".to_string(),
        payload: serde_json::json!({
            "reason": "linear_state_terminal",
            "issue_identifier": issue.identifier,
            "state_name": issue.state_name(),
        }),
    };
    if let Err(e) = task_closer.close(&binding.task_id, close_reason).await {
        warn!(task_id = %binding.task_id, error = %e, "Failed to close task on Linear close");
        return;
    }

    info!(
        task_id = %binding.task_id,
        issue = %issue.identifier,
        "Task closed due to Linear issue terminal state"
    );
}

async fn emit_linear_close_response(
    tasks: &dyn TaskReader,
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
