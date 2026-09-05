//! Telegram sender identity.

use tracing::{info, warn};

use moatless_adapter_model::error::AdapterError;
use moatless_ports::identity::ProviderIdentityLink;

/// The system user a Telegram sender is linked to, if any.
///
/// A free function rather than a method because normalization resolves the
/// sender without holding the Adapter — the lookup is a database read, not a
/// Bot API call, which is what keeps `webhook::normalize` clientless.
pub(super) async fn resolve_sender(
    telegram_user_id: &str,
    users: &dyn ProviderIdentityLink,
) -> Result<Option<String>, AdapterError> {
    let row = users
        .user_who_connected(super::delivery::ADAPTER, telegram_user_id)
        .await?;

    if let Some(user_id) = row {
        info!(telegram_user_id = %telegram_user_id, user_id = %user_id, "Resolved Telegram user to system user");
        Ok(Some(user_id))
    } else {
        warn!(telegram_user_id = %telegram_user_id, "No system user found for Telegram user");
        Ok(None)
    }
}
