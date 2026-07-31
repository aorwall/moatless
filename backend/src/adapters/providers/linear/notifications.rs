//! `AppUserNotification` — Linear telling the agent, as a user, that something
//! happened on an issue it is subscribed to.
//!
//! It carries no state transition of its own, so there is nothing here to
//! forward to a Task: everything a notification can say about a state change,
//! the `Issue.update` webhook says first and more precisely. What it is good
//! for is the case where `Issue.update` never arrives — a workspace whose
//! webhook is scoped to notifications only — so this path re-derives the one
//! decision that must not be missed, whether the issue reached a terminal state
//! and its Tasks should close.
//!
//! Its own delivery kind because a notification and an issue update are two
//! Sender deliveries; folding them together would let one's permanent failure
//! spend the other's retry budget.

use tracing::debug;

use crate::adapters::error::AdapterError;
use crate::adapters::models::AdapterConnection;
use crate::state::AppState;

use super::agent_api::LinearAgentApi;
use super::ingress::resolve_rules_for_team;
use super::close::close_active_bindings_for_issue;
use super::config::LinearRuleConfig;
use super::webhook_payloads::AppUserNotificationData;

/// Close active bindings when an `AppUserNotification.issueStatusChanged`
/// arrives for a terminal state. Inbox-stream analog of `handle_issue_update`'s
/// close path — used when the workspace only subscribes to AppUserNotification
/// (not Issue) webhooks. Acts on the current state alone (the notification
/// carries no diff). `close_active_bindings_for_issue` is idempotent, so
/// double delivery from the Issue path is safe.
pub(super) async fn handle_issue_status_notification(
    state: &AppState,
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

    let rules = resolve_rules_for_team(state, connection, &team_id).await?;
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
        close_active_bindings_for_issue(state, api, rule, issue).await?;
    }

    Ok(())
}
