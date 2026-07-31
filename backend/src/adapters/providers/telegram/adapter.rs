//! Telegram adapter type and composition root.

use crate::adapters::{Adapter, AdapterInboundPresentation, AdapterKind, AdapterOutboundContent};

use super::api::TelegramApi;

/// Telegram adapter.
pub struct TelegramAdapter {
    api: TelegramApi,
}

impl TelegramAdapter {
    pub fn new(bot_token: &str) -> Self {
        Self {
            api: TelegramApi::new(bot_token),
        }
    }

    /// Build the Adapter over an already-constructed client.
    ///
    /// [`Self::new`] builds its client from a bot token, so a test that drives
    /// any path reaching the Bot API has nowhere to put a stub. Pair this with
    /// [`TelegramApi::with_base_url`] and
    /// [`crate::adapters::test_support::stub_provider`]; no test reaches
    /// api.telegram.org.
    #[cfg(test)]
    pub(super) const fn with_api(api: TelegramApi) -> Self {
        Self { api }
    }

    /// Access the underlying Telegram API client.
    pub(super) const fn api(&self) -> &TelegramApi {
        &self.api
    }
}

impl Adapter for TelegramAdapter {
    fn adapter_kind(&self) -> AdapterKind {
        AdapterKind::Telegram
    }
}

impl AdapterOutboundContent for TelegramAdapter {}

#[async_trait::async_trait]
impl AdapterInboundPresentation for TelegramAdapter {
    fn external_id_aliases(&self, external_id: &str) -> Vec<String> {
        super::external_id::aliases(external_id)
    }
}
