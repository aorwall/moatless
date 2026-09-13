//! Telegram adapter type and composition root.

use std::sync::Arc;

use moatless_adapter_model::apps::AdapterAppReader;
use moatless_adapter_model::capabilities::{
    Adapter, AdapterInboundPresentation, AdapterOutboundContent,
};
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_adapter_model::delivery::DeliveryAdmission;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::loops::LoopReader;
use moatless_adapter_model::registry::Adapters;
use moatless_core::config::Config;
use moatless_core::db::DbPool;
use moatless_types::adapter::AdapterKind;

use super::api::TelegramApi;

/// The ingress ports (`adapter_connections`, `loops`, `adapter_delivery`).
///
/// Present only on the composition-root instance
/// ([`Self::with_ingress_ports`]) — absent on `#[cfg(test)]` constructors,
/// which never reach [`super::ingress::receive`].
pub struct TelegramAdapter {
    api: TelegramApi,
    db: Option<DbPool>,
    /// The stored Telegram apps, when this Adapter was built over a database.
    /// `None` leaves every send on the client above.
    apps: Option<Arc<dyn AdapterAppReader>>,
    adapter_connections: Option<Arc<dyn ConnectionLookup>>,
    loops: Option<Arc<dyn LoopReader>>,
    adapter_delivery: Option<Arc<dyn DeliveryAdmission>>,
}

/// The internal error for an ingress port `TelegramAdapter` never received: a
/// composition-root bug, not a request fault.
fn missing_port(name: &str) -> AdapterError {
    SharedAdapterError::Internal(format!("Telegram adapter missing {name} port")).into()
}

impl TelegramAdapter {
    /// Register `TelegramAdapter`'s capabilities into `adapters`. See
    /// [`moatless_adapter_model::registry::Adapters`].
    pub fn register_into(self: Arc<Self>, adapters: &mut Adapters) {
        let kind = AdapterKind::Telegram;
        adapters.register_trait_object(self.clone());
        adapters.register_binding_reply(self.clone());
        adapters.register_message_reply(self.clone());
        adapters.register_reactions(self.clone());
        adapters.register_inbound_presentation(kind, self);
    }

    pub fn new(bot_token: &str) -> Self {
        Self {
            api: TelegramApi::new(bot_token),
            db: None,
            apps: None,
            adapter_connections: None,
            loops: None,
            adapter_delivery: None,
        }
    }

    pub fn from_db(config: &Config, db: DbPool, apps: Arc<dyn AdapterAppReader>) -> Self {
        let bot_token = config.telegram.bot_token.as_deref().unwrap_or("");
        Self {
            api: TelegramApi::new(bot_token),
            db: Some(db),
            apps: Some(apps),
            adapter_connections: None,
            loops: None,
            adapter_delivery: None,
        }
    }

    /// Attach the ingress ports the webhook route and the poller reach through
    /// this adapter, so neither has to take an `AppState`.
    ///
    /// Built locally as stateless pool-wrapper DAOs, interchangeable with the
    /// ones `AppState` holds — except the `AdapterDeliveryStore` behind
    /// `DeliveryAdmission`, whose policy state is pinned at its own definition.
    pub fn with_ingress_ports(
        mut self,
        adapter_connections: Arc<dyn ConnectionLookup>,
        loops: Arc<dyn LoopReader>,
        adapter_delivery: Arc<dyn DeliveryAdmission>,
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
    ) -> Result<&dyn ConnectionLookup, AdapterError> {
        self.adapter_connections
            .as_deref()
            .ok_or_else(|| missing_port("adapter_connections"))
    }

    /// The Subscription reads, for chat→subscription resolution.
    pub(super) fn require_loops(&self) -> Result<&dyn LoopReader, AdapterError> {
        self.loops.as_deref().ok_or_else(|| missing_port("loops"))
    }

    /// The durable delivery seam admitted updates are handed to.
    pub(super) fn require_adapter_delivery(&self) -> Result<&dyn DeliveryAdmission, AdapterError> {
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
    /// [`Self::new`] builds its client from a bot token, leaving no path to a
    /// stub; pair this with [`TelegramApi::with_base_url`] and
    /// [`moatless_adapter_model::test_support::stub_provider`] so no test
    /// reaches api.telegram.org.
    #[cfg(test)]
    pub(super) const fn with_api(api: TelegramApi) -> Self {
        Self {
            api,
            db: None,
            apps: None,
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

    /// The bot client for one stored app. Telegram's whole app shape is its
    /// token, so the stored app is read straight into a client.
    pub(super) async fn app_api(&self, app_key: &str) -> Result<TelegramApi, AdapterError> {
        let Some(apps) = &self.apps else {
            return Ok(self.api.clone());
        };
        let Some(app) = apps.app("telegram", app_key).await? else {
            return Err(SharedAdapterError::AdapterNotConfigured(format!(
                "telegram app '{app_key}'"
            ))
            .into());
        };
        let Some(bot_token) = app.secret("bot_token") else {
            return Err(SharedAdapterError::AdapterApi(format!(
                "Telegram Adapter app '{}' is missing bot_token",
                app.app_key
            ))
            .into());
        };
        Ok(TelegramApi::new(&bot_token))
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
