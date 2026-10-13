//! `AppUserNotification` — Linear telling the agent, as a user, that something
//! happened on an issue it is subscribed to.
//!
//! It carries no state transition, so nothing here forwards to a Task; its only
//! job is re-deriving whether the issue reached a terminal state when
//! `Issue.update` never arrives (a webhook scoped to notifications only).
//!
//! Its own delivery kind, so folding it into `Issue.update` can't let one's
//! permanent failure spend the other's retry budget.

use tracing::debug;

use moatless_core::config::Config;

use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::loops::LoopReader;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::task::TaskFinisher;
use moatless_ports::task::TaskReader;
use moatless_types::adapter::AdapterConnection;

use super::agent_api::LinearAgentApi;
use super::close::close_active_bindings_for_issue;
use super::config::LinearRuleConfig;
use super::subscription_rules::resolve_rules_for_team;
use super::webhook_payloads::AppUserNotificationData;

/// Close active bindings when an `AppUserNotification.issueStatusChanged`
/// arrives for a terminal state — the analog of `handle_issue_update`'s
/// close path, for `AppUserNotification`-only workspaces.
///
/// Acts on the current state alone (no diff); idempotent against double
/// delivery from the Issue path.
#[expect(
    clippy::too_many_arguments,
    reason = "this forward-only path threads the union its two callees need — the \
              loops port `resolve_rules_for_team` reads and the five close ports \
              `close_active_bindings_for_issue` reads — inverted out of the \
              single AppState it used to carry"
)]
pub(super) async fn handle_issue_status_notification(
    task_bindings: &Bindings,
    tasks: &dyn TaskReader,
    config: &Config,
    task_closer: &dyn TaskFinisher,
    loops: &dyn LoopReader,
    api: &LinearAgentApi,
    connection: &AdapterConnection,
    notif: &AppUserNotificationData,
) -> Result<(), AdapterError> {
    let Some(issue) = notif.resolved_issue() else {
        debug!(
            notif_id = notif.id.as_deref().unwrap_or(""),
            "AppUserNotification.issueStatusChanged missing issue payload — ignoring"
        );
        return Ok(());
    };

    let Some(team_id) = issue.resolved_team_id().map(str::to_string) else {
        debug!(
            issue = %issue.identifier,
            "AppUserNotification missing team_id — ignoring"
        );
        return Ok(());
    };

    let rules = resolve_rules_for_team(loops, connection, &team_id).await?;
    let Some(rule) = rules.first() else {
        debug!(
            team_id = %team_id,
            "No active Linear rule for team — ignoring AppUserNotification"
        );
        return Ok(());
    };

    let rule_config = LinearRuleConfig::from_rule(rule);
    let state_kind = issue.state_kind();
    if rule_config.close_task_on_close
        && rule_config.terminal_states.iter().any(|s| s == state_kind)
    {
        close_active_bindings_for_issue(
            task_bindings,
            tasks,
            config,
            task_closer,
            api,
            rule,
            issue,
        )
        .await?;
    }

    Ok(())
}
