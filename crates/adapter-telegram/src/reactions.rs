//! Reaction capability for Telegram messages.

use moatless_adapter_model::binding::BindingContext;
use moatless_adapter_model::capabilities::AdapterReactions;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

use super::adapter::TelegramAdapter;
use super::subject::{TelegramDestination, parse_message_target};

/// Resolve the `(chat_id, message_id)` pair for a Telegram reaction call.
///
/// `target_external_id` carries the message id (`tg:m:<id>`); `chat_id` comes
/// from the binding's `destination_config`.
fn reaction_target(
    binding: &BindingContext,
    target_external_id: &str,
) -> Result<(String, i64), AdapterError> {
    let message_id = parse_message_target(target_external_id, "reaction")?;
    let destination = TelegramDestination::from_destination_config(&binding.destination_config)?;
    Ok((destination.chat_id().to_string(), message_id))
}

#[async_trait::async_trait]
impl AdapterReactions for TelegramAdapter {
    async fn add_reaction(
        &self,
        binding: &BindingContext,
        // Telegram acts as its own bot, so the caller's identity mints no token.
        _acting_user_id: &str,
        target_external_id: &str,
        emoji: &str,
    ) -> Result<Option<String>, AdapterError> {
        let (chat_id, message_id) = reaction_target(binding, target_external_id)?;
        let trimmed = emoji.trim();
        if trimmed.is_empty() {
            return Err(SharedAdapterError::BadRequest("Empty emoji".into()).into());
        }
        let api = self
            .app_api(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;
        api.set_message_reaction(&chat_id, message_id, Some(trimmed))
            .await?;
        Ok(None)
    }

    async fn remove_reaction(
        &self,
        binding: &BindingContext,
        _acting_user_id: &str,
        target_external_id: &str,
        _emoji: &str,
    ) -> Result<(), AdapterError> {
        // Telegram's API is an idempotent set: clearing means setting an empty list.
        // The `_emoji` argument is accepted for parity with other providers but
        // ignored — Telegram replaces the bot's full reaction list either way.
        let (chat_id, message_id) = reaction_target(binding, target_external_id)?;
        let api = self
            .app_api(moatless_adapter_model::apps::DEFAULT_APP_KEY)
            .await?;
        api.set_message_reaction(&chat_id, message_id, None).await
    }
}
