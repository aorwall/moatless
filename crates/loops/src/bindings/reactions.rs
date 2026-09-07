//! Reactions on authorized external message targets.

use super::reply::ExternalReplyService;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_api_schemas::message::MessageReactionResponse;
use moatless_types::access_control::ScopeContext;

impl ExternalReplyService {
    /// Add a reaction after authorizing the external message target.
    pub async fn add_message_reaction(
        &self,
        ctx: &ScopeContext,
        message_id: &str,
        emoji: &str,
    ) -> Result<MessageReactionResponse, AdapterError> {
        let (binding, ext_id, provider_key) = self.resolve_message_target(ctx, message_id).await?;
        let providers = self.adapters.as_ref().ok_or_else(|| {
            SharedAdapterError::AdapterNotConfigured("No adapters configured".into())
        })?;
        let adapter = providers.reactions_by_name(&provider_key)?;
        let binding_ctx = moatless_adapter_model::binding::binding_context(&binding);
        let external_id = adapter.add_reaction(&binding_ctx, &ext_id, emoji).await?;
        Ok(MessageReactionResponse {
            ok: true,
            external_id,
        })
    }

    /// Remove a reaction after authorizing the external message target.
    pub async fn remove_message_reaction(
        &self,
        ctx: &ScopeContext,
        message_id: &str,
        emoji: &str,
    ) -> Result<MessageReactionResponse, AdapterError> {
        let (binding, ext_id, provider_key) = self.resolve_message_target(ctx, message_id).await?;
        let providers = self.adapters.as_ref().ok_or_else(|| {
            SharedAdapterError::AdapterNotConfigured("No adapters configured".into())
        })?;
        let adapter = providers.reactions_by_name(&provider_key)?;
        let binding_ctx = moatless_adapter_model::binding::binding_context(&binding);
        adapter
            .remove_reaction(&binding_ctx, &ext_id, emoji)
            .await?;
        Ok(MessageReactionResponse {
            ok: true,
            external_id: None,
        })
    }
}
