use crate::subject::ADAPTER;
use moatless_adapter_model::error::AdapterError;
use moatless_adapter_model::loops::LoopReader;
use moatless_types::adapter::AdapterConnection;
use moatless_types::loops::SubscriptionMatch;

/// Resolve every active subscription bound to a Linear team (multi-listener:
/// all matches, not just the first).
///
/// `delivery.rs`'s `session_provision`/`notification` executors re-resolve
/// through here rather than snapshotting the webhook's view, so disabling a
/// loop between admission and execution still stops the work.
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
