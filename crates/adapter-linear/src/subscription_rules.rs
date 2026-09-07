use crate::subject::ADAPTER;
use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::loops::LoopReader;
use moatless_types::adapter::AdapterConnection;
use moatless_types::loops::SubscriptionMatch;

/// Resolve every active subscription bound to a given Linear team. Returns
/// every subscription attached to the team — multi-listener support is preserved.
///
/// Also the worker's: `delivery.rs`'s `session_provision` and `notification`
/// executors re-resolve through here rather than snapshotting the subscriptions the
/// webhook saw, so an operator disabling a loop between admission and
/// execution stops the work.
pub(super) async fn resolve_rules_for_team(
    loops: &dyn LoopReader,
    connection: &AdapterConnection,
    team_id: &str,
) -> Result<Vec<SubscriptionMatch>, AdapterError> {
    let rules = loops.subscriptions_for_source(ADAPTER, team_id).await?;
    Ok(rules
        .into_iter()
        .filter(|r| {
            if r.connection_id == connection.id && r.active {
                true
            } else {
                tracing::debug!(
                    subscription_id = %r.id,
                    "Linear trigger ignored (different connection or inactive)"
                );
                false
            }
        })
        .collect())
}
