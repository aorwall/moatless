//! Telegram adapter type and composition root.

use std::sync::Arc;

use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::delivery::AdapterDeliveryStore;
use crate::adapters::error::{AdapterError, SharedAdapterError};
use crate::adapters::{
    Adapter, AdapterInboundPresentation, AdapterKind, AdapterOutboundContent, Adapters,
};
use crate::loops::dao::LoopDao;
use moatless_core::config::Config;
use moatless_core::db::DbPool;

use super::api::TelegramApi;

/// Telegram adapter.
///
/// The ingress ports (`adapter_connections`, `loops`, `adapter_delivery`) are
/// carried here so the webhook route and the poller read them through this
/// adapter rather than an `AppState`. They are present on the composition-root
/// instance (see [`Self::with_ingress_ports`]) and absent on the `#[cfg(test)]`
/// constructors, which never reach [`super::ingress::receive`].
pub struct TelegramAdapter {
    api: TelegramApi,
    db: Option<DbPool>,
    config: Option<Arc<Config>>,
    adapter_connections: Option<Arc<dyn AdapterConnectionDao>>,
    loops: Option<Arc<LoopDao>>,
    adapter_delivery: Option<Arc<AdapterDeliveryStore>>,
}

/// The internal error for an ingress port read on an adapter that never
/// received one — a composition-root bug, not a request fault.
fn missing_port(name: &str) -> AdapterError {
    SharedAdapterError::Internal(format!("Telegram adapter missing {name} port")).into()
}

impl TelegramAdapter {
    /// Register this adapter's capabilities into `adapters`.
    ///
    /// Construction lives at the composition root; the provider owns the
    /// mapping from itself to the capability surfaces it implements.
    pub(crate) fn register_into(self: Arc<Self>, adapters: &mut Adapters) {
        let kind = AdapterKind::Telegram;
        adapters.register_trait_object(self.clone());
        adapters.register_binding_reply(self.clone());
        adapters.register_message_reply(self.clone());
        adapters.register_reactions(self.clone());
        adapters.register_inbound_presentation(kind, self);
    }

    #[cfg(test)]
    pub fn new(bot_token: &str) -> Self {
        Self {
            api: TelegramApi::new(bot_token),
            db: None,
            config: None,
            adapter_connections: None,
            loops: None,
            adapter_delivery: None,
        }
    }

    pub fn from_db(config: Arc<Config>, db: DbPool) -> Self {
        let bot_token = config.telegram.bot_token.as_deref().unwrap_or("");
        Self {
            api: TelegramApi::new(bot_token),
            db: Some(db),
            config: Some(config),
            adapter_connections: None,
            loops: None,
            adapter_delivery: None,
        }
    }

    /// Attach the ingress ports the webhook route and the poller reach through
    /// this adapter, so neither has to take an `AppState`.
    ///
    /// Built at the composition root ([`crate::compose`]) from the pool it
    /// already holds — the same local-construction move as
    /// `LinearAdapter::with_tasks`. All three are pool-wrapper DAOs, which the
    /// backend guidelines allow constructing locally precisely because they
    /// stay stateless, so a per-provider instance is interchangeable with the
    /// one `AppState` holds (`AdapterDeliveryStore` carries the one non-obvious
    /// case — policy state — and pins the invariant at its own definition).
    pub fn with_ingress_ports(
        mut self,
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<LoopDao>,
        adapter_delivery: Arc<AdapterDeliveryStore>,
    ) -> Self {
        self.adapter_connections = Some(adapter_connections);
        self.loops = Some(loops);
        self.adapter_delivery = Some(adapter_delivery);
        self
    }

    /// The connection store, for chat→subscription resolution. `Err` only if
    /// this adapter was built without ingress ports (a test constructor).
    pub(super) fn require_adapter_connections(
        &self,
    ) -> Result<&dyn AdapterConnectionDao, AdapterError> {
        self.adapter_connections
            .as_deref()
            .ok_or_else(|| missing_port("adapter_connections"))
    }

    /// The Loop store, for chat→subscription resolution.
    pub(super) fn require_loops(&self) -> Result<&LoopDao, AdapterError> {
        self.loops.as_deref().ok_or_else(|| missing_port("loops"))
    }

    /// The durable delivery store admitted updates are handed to.
    pub(super) fn require_adapter_delivery(&self) -> Result<&AdapterDeliveryStore, AdapterError> {
        self.adapter_delivery
            .as_deref()
            .ok_or_else(|| missing_port("adapter_delivery"))
    }

    /// The pool, for normalization reads and effect writes. Distinct from the
    /// `app_api` read of `db`, which treats absence as "use the default client"
    /// rather than an error.
    pub(super) fn require_db(&self) -> Result<&DbPool, AdapterError> {
        self.db.as_ref().ok_or_else(|| missing_port("db"))
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
            adapter_connections: None,
            loops: None,
            adapter_delivery: None,
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
        let config = self.config.as_deref().ok_or_else(|| {
            SharedAdapterError::Internal("Telegram Adapter missing config".into())
        })?;
        let Some(app) = crate::adapter_apps::telegram_app_config(db, config, app_key)
            .await
            .map_err(|e| SharedAdapterError::AdapterApi(e.to_string()))?
        else {
            return Err(SharedAdapterError::AdapterNotConfigured(format!(
                "telegram app '{app_key}'"
            ))
            .into());
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
