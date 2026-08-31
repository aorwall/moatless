//! Process installation seam for the built-in external Adapters.
//!
//! The backend chooses the concrete Adapters. Each private installation below
//! keeps construction, capability registration, durable-delivery wiring, and
//! concrete handler access together. The loops-band registry receives only
//! capability trait objects and Adapter crates remain independent of
//! [`AppState`].

use std::sync::Arc;

use crate::adapters::delivery::AdapterDeliveryHandlers;
use crate::adapters::providers::github_pr::adapter::GithubPrAdapter;
use crate::adapters::providers::linear::active_turns::ActiveLinearTurns;
use crate::adapters::providers::linear::adapter::LinearAdapter;
use crate::adapters::providers::slack::adapter::SlackAdapter;
use crate::adapters::providers::telegram::adapter::TelegramAdapter;
use crate::adapters::providers::webhook::adapter::WebhookAdapter;
use moatless_adapter_model::capabilities::AdapterDestinationBuilder;
use moatless_adapter_model::registry::Adapters;
use crate::compose::Ports;
use crate::state::AppState;
use moatless_core::config::Config;
use moatless_loops::adapters::events::TaskEventSubscribers;

/// The external Adapters installed in one backend process.
///
/// Production installation is all-or-nothing: all five Adapters are installed
/// unconditionally because their credentials and transport configuration are
/// resolved per workspace. A process without database ports carries no
/// `AdapterInstallations` at all.
pub struct AdapterInstallations {
    adapters: Arc<Adapters>,
    slack: SlackInstallation,
    telegram: TelegramInstallation,
    github_pr: GithubPrInstallation,
    linear: LinearInstallation,
}

impl AdapterInstallations {
    /// Install every built-in Adapter over the canonical process ports.
    pub fn install(config: &Config, ports: &Ports) -> Self {
        let mut adapters = Adapters::new();
        let runtime_config = Arc::new(config.clone());

        let slack = SlackInstallation::install(runtime_config.clone(), ports, &mut adapters);
        WebhookInstallation::install(&mut adapters);
        let telegram = TelegramInstallation::install(runtime_config.clone(), ports, &mut adapters);
        let github_pr = GithubPrInstallation::install(config, ports, &mut adapters);
        let linear = LinearInstallation::install(runtime_config, ports, &mut adapters);

        Self {
            adapters: Arc::new(adapters),
            slack,
            telegram,
            github_pr,
            linear,
        }
    }

    /// The capability-only registry consumed below the server band.
    pub fn adapters(&self) -> Arc<Adapters> {
        self.adapters.clone()
    }

    /// Build the complete durable-delivery handler set for this installation.
    ///
    /// Delivery routers are assembled only after `AppState`, because GitHub PR
    /// and Linear need routing services that themselves consume the capability
    /// registry. Keeping those dependencies here avoids an Adapter retaining
    /// the aggregate that contains it.
    pub fn delivery_handlers(&self, state: &AppState) -> AdapterDeliveryHandlers {
        let mut handlers = AdapterDeliveryHandlers::new();
        SlackInstallation::register_delivery(&mut handlers, state);
        WebhookInstallation::register_delivery(&mut handlers, state);
        TelegramInstallation::register_delivery(&mut handlers, state);
        self.github_pr.register_delivery(&mut handlers, state);
        self.linear.register_delivery(&mut handlers, state);
        handlers
    }

    /// Build the set of Adapters that react to a task's own event stream.
    ///
    /// Assembled after `AppState` for the same reason as
    /// [`delivery_handlers`](Self::delivery_handlers) — the reactions read
    /// process ports the aggregate owns. An Adapter that reacts to no event
    /// registers nothing and the event handler is unchanged by it.
    pub fn event_subscribers(&self, state: &AppState) -> TaskEventSubscribers {
        let mut subscribers = TaskEventSubscribers::new();
        self.linear.subscribe_events(&mut subscribers, state);
        subscribers
    }

    /// Concrete Slack Adapter for Slack-specific HTTP handlers.
    pub fn slack(&self) -> Arc<SlackAdapter> {
        self.slack.adapter.clone()
    }

    /// Concrete Telegram Adapter for Telegram-specific handlers and polling.
    pub fn telegram(&self) -> Arc<TelegramAdapter> {
        self.telegram.adapter.clone()
    }

    /// Concrete GitHub PR Adapter for GitHub-specific handlers.
    pub fn github_pr(&self) -> Arc<GithubPrAdapter> {
        self.github_pr.adapter.clone()
    }

    /// Concrete Linear Adapter for Linear-specific handlers.
    pub fn linear(&self) -> Arc<LinearAdapter> {
        self.linear.adapter.clone()
    }

    /// The GitHub PR destination builder needed by the binding connector.
    pub fn github_destination_builder(&self) -> Arc<dyn AdapterDestinationBuilder> {
        self.github_pr.adapter.clone()
    }
}

struct SlackInstallation {
    adapter: Arc<SlackAdapter>,
}

impl SlackInstallation {
    fn install(config: Arc<Config>, ports: &Ports, adapters: &mut Adapters) -> Self {
        let adapter = Arc::new(SlackAdapter::from_db(
            &config,
            ports.adapter_apps.clone(),
            ports.user_directory.clone(),
        ));
        adapter.clone().register_into(adapters);
        tracing::info!("Slack Adapter installed");
        Self { adapter }
    }

    fn register_delivery(handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        crate::adapters::providers::slack::delivery::register(handlers, state.routing.clone());
    }
}

struct WebhookInstallation;

impl WebhookInstallation {
    fn install(adapters: &mut Adapters) {
        Arc::new(WebhookAdapter::new()).register_into(adapters);
        tracing::info!("Webhook Adapter installed");
    }

    fn register_delivery(handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        crate::adapters::providers::webhook::delivery::register(handlers, state.routing.clone());
    }
}

struct TelegramInstallation {
    adapter: Arc<TelegramAdapter>,
}

impl TelegramInstallation {
    fn install(config: Arc<Config>, ports: &Ports, adapters: &mut Adapters) -> Self {
        let adapter = Arc::new(
            TelegramAdapter::from_db(&config, ports.pool.clone(), ports.adapter_apps.clone())
                .with_ingress_ports(
                    ports.adapter_connections.clone(),
                    ports.loops.clone(),
                    ports.adapter_delivery.clone(),
                ),
        );
        adapter.clone().register_into(adapters);
        tracing::info!("Telegram Adapter installed");
        Self { adapter }
    }

    fn register_delivery(handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        crate::adapters::providers::telegram::delivery::register(handlers, state.routing.clone());
    }
}

struct GithubPrInstallation {
    adapter: Arc<GithubPrAdapter>,
}

impl GithubPrInstallation {
    fn install(config: &Config, ports: &Ports, adapters: &mut Adapters) -> Self {
        let adapter = Arc::new(
            GithubPrAdapter::new(
                config.github_pr.webhook_secret.clone(),
                config.auth.frontend_url.clone(),
                ports.git_host_tokens.clone(),
            )
            .with_ingress_ports(
                ports.adapter_connections.clone(),
                ports.loops.clone(),
                ports.task_bindings.clone(),
                ports.user_directory.clone(),
                ports.adapter_delivery.clone(),
            ),
        );
        adapter.clone().register_into(adapters);
        tracing::info!("GitHub PR Adapter installed");
        Self { adapter }
    }

    fn register_delivery(&self, handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        let router = Arc::new(
            crate::adapters::providers::github_pr::router::GithubPrInboundRouter::new(
                state.adapter_connections.clone(),
                state.loops.clone(),
                state.task_bindings.clone(),
                state.user_directory.clone(),
                state.routing.clone(),
                state.transcript.clone(),
                state.relation_recorder.clone(),
                state.task_reader.clone(),
                state.task_creation.clone(),
                state.message_send.clone(),
                state.task_closer.clone(),
                Some(self.adapter.clone()),
            ),
        );
        crate::adapters::providers::github_pr::delivery::register(handlers, router);
    }
}

struct LinearInstallation {
    adapter: Arc<LinearAdapter>,
    /// Shared by the inbound router (which opens a turn) and the event
    /// subscriber (which reads and closes it).
    active_turns: Arc<ActiveLinearTurns>,
}

impl LinearInstallation {
    fn install(config: Arc<Config>, ports: &Ports, adapters: &mut Adapters) -> Self {
        let adapter = Arc::new(
            LinearAdapter::from_db(
                config,
                ports.harness_credentials.clone(),
                ports.adapter_apps.clone(),
            )
                .with_tasks(ports.task_reader.clone())
                .with_ingress_ports(
                    ports.adapter_connections.clone(),
                    ports.loops.clone(),
                    ports.adapter_delivery.clone(),
                ),
        );
        adapter.clone().register_into(adapters);
        tracing::info!("Linear Adapter installed");
        Self {
            adapter,
            active_turns: Arc::new(ActiveLinearTurns::new()),
        }
    }

    fn register_delivery(&self, handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        let router = Arc::new(
            crate::adapters::providers::linear::delivery::LinearInboundRouter::new(
                state.adapter_connections.clone(),
                state.harness_credentials.clone(),
                state.config.clone(),
                state.adapter_apps.clone(),
                state.loops.clone(),
                state.task_bindings.clone(),
                state.task_reader.clone(),
                state.task_creation.clone(),
                state.relation_recorder.clone(),
                state.message_send.clone(),
                state.sandbox_waker.clone(),
                state.task_closer.clone(),
                self.active_turns.clone(),
                state.routing.clone(),
            ),
        );
        crate::adapters::providers::linear::delivery::register(handlers, router);
    }

    fn subscribe_events(&self, subscribers: &mut TaskEventSubscribers, state: &AppState) {
        crate::adapters::providers::linear::event_subscriber::subscribe(
            subscribers,
            Arc::new(
                crate::adapters::providers::linear::event_subscriber::LinearEventSubscriber::new(
                    state.harness_credentials.clone(),
                    state.task_bindings.clone(),
                    state.adapter_connections.clone(),
                    state.config.clone(),
                    state.adapter_apps.clone(),
                    state.transcript.clone(),
                    state.task_reader.clone(),
                    self.active_turns.clone(),
                ),
            ),
        );
    }
}

// Adapter-specific handlers extract their concrete Adapter only where they
// need behavior beyond the cross-Adapter capability interfaces. The projection
// stays beside installation, so adding that access is part of the same edit as
// construction and delivery wiring.
impl axum::extract::FromRef<AppState> for Option<Arc<SlackAdapter>> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.slack())
    }
}

impl axum::extract::FromRef<AppState> for Option<Arc<TelegramAdapter>> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.telegram())
    }
}

impl axum::extract::FromRef<AppState> for Option<Arc<GithubPrAdapter>> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.github_pr())
    }
}

impl axum::extract::FromRef<AppState> for Option<Arc<LinearAdapter>> {
    fn from_ref(state: &AppState) -> Self {
        state.adapter_installations.as_ref().map(|i| i.linear())
    }
}
