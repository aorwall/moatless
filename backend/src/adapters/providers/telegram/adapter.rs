//! Telegram adapter type and composition root.

use std::sync::Arc;

use crate::adapters::error::AdapterError;
use crate::adapters::{Adapter, AdapterInboundPresentation, AdapterKind, AdapterOutboundContent};
use moatless_core::config::Config;
use moatless_core::db::DbPool;

use super::api::TelegramApi;

/// Telegram adapter.
pub struct TelegramAdapter {
    api: TelegramApi,
    db: Option<DbPool>,
    config: Option<Arc<Config>>,
}

impl TelegramAdapter {
    #[cfg(test)]
    pub fn new(bot_token: &str) -> Self {
        Self {
            api: TelegramApi::new(bot_token),
            db: None,
            config: None,
        }
    }

    pub fn from_db(config: Arc<Config>, db: DbPool) -> Self {
        let bot_token = config.telegram.bot_token.as_deref().unwrap_or("");
        Self {
            api: TelegramApi::new(bot_token),
            db: Some(db),
            config: Some(config),
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
        Self {
            api,
            db: None,
            config: None,
        }
    }

    /// Access the underlying Telegram API client.
    #[cfg(test)]
    pub(super) const fn api(&self) -> &TelegramApi {
        &self.api
    }

    pub(super) async fn app_api(&self, app_key: &str) -> Result<TelegramApi, AdapterError> {
        let Some(db) = &self.db else {
            return Ok(self.api.clone());
        };
        let config = self
            .config
            .as_deref()
            .ok_or_else(|| AdapterError::Internal("Telegram Adapter missing config".into()))?;
        let Some(app) = crate::adapter_apps::telegram_app_config(db, config, app_key)
            .await
            .map_err(|e| AdapterError::AdapterApi(e.to_string()))?
        else {
            return Err(AdapterError::AdapterNotConfigured(format!(
                "telegram app '{app_key}'"
            )));
        };
        Ok(TelegramApi::new(&app.bot_token))
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
