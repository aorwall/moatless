//! Telegram sender identity.

use tracing::{info, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::AdapterUserIdentity;
use crate::db::DbPool;

use super::adapter::TelegramAdapter;

/// The system user a Telegram sender is linked to, if any.
///
/// A free function rather than a method because normalization resolves the
/// sender without holding the Adapter — the lookup is a database read, not a
/// Bot API call, which is what keeps `webhook::normalize` clientless.
pub(super) async fn resolve_sender(
    telegram_user_id: &str,
    db: &DbPool,
) -> Result<Option<String>, AdapterError> {
    // Look up by provider_user_id in provider_tokens table
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM provider_tokens WHERE provider = 'telegram' AND provider_user_id = $1 LIMIT 1",
    )
    .bind(telegram_user_id)
    .fetch_optional(db)
    .await
    .map_err(AdapterError::Db)?;

    if let Some((user_id,)) = row {
        info!(telegram_user_id = %telegram_user_id, user_id = %user_id, "Resolved Telegram user to system user");
        Ok(Some(user_id))
    } else {
        warn!(telegram_user_id = %telegram_user_id, "No system user found for Telegram user");
        Ok(None)
    }
}

#[async_trait::async_trait]
impl AdapterUserIdentity for TelegramAdapter {
    async fn resolve_user(
        &self,
        identity: &str,
        db: &DbPool,
    ) -> Result<Option<String>, AdapterError> {
        resolve_sender(identity, db).await
    }
}
