//! Process installation seam for the built-in external Adapters.
//!
//! The backend chooses the concrete Adapters. Each private installation below
//! keeps construction, capability registration, durable-delivery wiring, and
//! concrete handler access together. The loops-band registry receives only
//! capability trait objects and Adapter crates remain independent of
//! [`AppState`].

use std::sync::Arc;

use crate::adapters::inbound_delivery::InboundMessageHandler;
use crate::adapters::inbound_files::InboundFiles;
use crate::adapters::providers::github_pr::adapter::GithubPrAdapter;
use crate::adapters::providers::github_pr::check_run_trigger::{
    TriggerWorkspace, TriggerWorkspaceRequest, TriggerWorkspaces,
};
use crate::adapters::providers::github_pr::router::ReviewGuideTasks;
use crate::adapters::providers::linear::active_turns::ActiveLinearTurns;
use crate::adapters::providers::linear::adapter::LinearAdapter;
use crate::adapters::providers::slack::adapter::SlackAdapter;
use crate::adapters::providers::telegram::adapter::TelegramAdapter;
use crate::adapters::providers::webhook::adapter::WebhookAdapter;
use crate::compose::Ports;
use crate::state::AppState;
use moatless_adapter_model::delivery::AdapterDeliveryHandlers;
use moatless_adapter_model::registry::Adapters;
use moatless_core::config::Config;
use moatless_loops::adapters::events::TaskEventSubscribers;
use moatless_types::store::StoreError;

/// The external Adapters installed in one backend process.
///
/// Production installs all five unconditionally, since credentials and
/// transport are resolved per workspace; a process without database ports
/// carries none.
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
        WebhookInstallation::install(config, &mut adapters);
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
    /// Assembled only after `AppState`, since GitHub PR and Linear need routing
    /// services that consume the capability registry — this avoids an Adapter
    /// retaining the aggregate that contains it.
    pub fn delivery_handlers(&self, state: &AppState) -> AdapterDeliveryHandlers {
        let mut handlers = AdapterDeliveryHandlers::new();
        self.slack.register_delivery(&mut handlers, state);
        WebhookInstallation::register_delivery(&mut handlers, state);
        TelegramInstallation::register_delivery(&mut handlers, state);
        self.github_pr.register_delivery(&mut handlers, state);
        self.linear.register_delivery(&mut handlers, state);
        handlers
    }

    /// Build the set of Adapters that react to a task's own event stream.
    ///
    /// Assembled after `AppState` for the same reason as
    /// [`delivery_handlers`](Self::delivery_handlers): the reactions read
    /// process ports the aggregate owns.
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
}

/// Register the one-handler inbound-message delivery shared by the Adapters
/// whose `register_delivery` does nothing but bind their own
/// `delivery::INBOUND_MESSAGE` and `delivery::ADAPTER` to an
/// [`InboundMessageHandler`].
fn register_inbound_message_delivery(
    handlers: &mut AdapterDeliveryHandlers,
    state: &AppState,
    kind: &'static str,
    adapter: &'static str,
) {
    handlers.register(
        kind,
        Arc::new(InboundMessageHandler::new(adapter, state.routing.clone())),
    );
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

    /// Slack's messages share files, so its handler stores them before routing.
    fn register_delivery(&self, handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        use crate::adapters::providers::slack::delivery;
        let files = InboundFiles::new(self.adapter.clone(), state.files.clone());
        handlers.register(
            delivery::INBOUND_MESSAGE,
            Arc::new(
                InboundMessageHandler::new(delivery::ADAPTER, state.routing.clone())
                    .with_files(Arc::new(files)),
            ),
        );
    }
}

struct WebhookInstallation;

impl WebhookInstallation {
    fn install(config: &Config, adapters: &mut Adapters) {
        Arc::new(WebhookAdapter::new(config.http.webhook)).register_into(adapters);
        tracing::info!("Webhook Adapter installed");
    }

    fn register_delivery(handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        use crate::adapters::providers::webhook::delivery;
        register_inbound_message_delivery(
            handlers,
            state,
            delivery::INBOUND_MESSAGE,
            delivery::ADAPTER,
        );
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
                    ports.connection_routing.clone(),
                    ports.loop_reader.clone(),
                    ports.adapter_delivery.clone(),
                ),
        );
        adapter.clone().register_into(adapters);
        tracing::info!("Telegram Adapter installed");
        Self { adapter }
    }

    fn register_delivery(handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        use crate::adapters::providers::telegram::delivery;
        register_inbound_message_delivery(
            handlers,
            state,
            delivery::INBOUND_MESSAGE,
            delivery::ADAPTER,
        );
    }
}

struct GithubPrInstallation {
    adapter: Arc<GithubPrAdapter>,
    connector: Arc<moatless_loops::loops::firing::listen::Connector>,
}

impl GithubPrInstallation {
    fn install(config: &Config, ports: &Ports, adapters: &mut Adapters) -> Self {
        let adapter = Arc::new(
            GithubPrAdapter::new(
                config.github_pr.webhook_secret.clone(),
                config.auth.frontend_url.clone(),
                config.github_api_base.clone(),
                ports.git_host_tokens.clone(),
                config.http.github,
            )
            .with_ingress_ports(
                ports.connection_routing.clone(),
                ports.loop_reader.clone(),
                ports.task_bindings.clone(),
                ports.user_directory.clone(),
                ports.adapter_delivery.clone(),
            ),
        );
        adapter.clone().register_into(adapters);
        tracing::info!("GitHub PR Adapter installed");
        let connector = Arc::new(
            ports
                .loop_graph
                .connector(Some(adapter.clone()), ports.connection_routing.clone()),
        );
        Self { adapter, connector }
    }

    fn register_delivery(&self, handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        let router = Arc::new(
            crate::adapters::providers::github_pr::router::GithubPrInboundRouter::new(
                state.connection_routing.clone(),
                state.loop_reader.clone(),
                state.task_bindings.clone(),
                state.user_directory.clone(),
                state.routing.clone(),
                state.transcript.clone(),
                state.relation_recorder.clone(),
                state.task_reader.clone(),
                state.task_creation.clone(),
                state.message_send.clone(),
                state.task_closer.clone(),
                Arc::new(GuideTasks(state.review_guides.clone())),
                Arc::new(ProvisionedWorkspaces {
                    workspaces: state.workspaces.clone(),
                    repositories: state.repositories.clone(),
                    git_sync: state.git_sync.clone(),
                    pool: state.db.clone(),
                }),
                Some(self.adapter.clone()),
            ),
        );
        crate::adapters::providers::github_pr::delivery::register(handlers, router);
    }
}

struct LinearInstallation {
    adapter: Arc<LinearAdapter>,
    oauth: Arc<moatless_adapter_linear::installation::Installation>,
    /// Shared by the inbound router (which opens a turn) and the event
    /// subscriber (which reads and closes it).
    active_turns: Arc<ActiveLinearTurns>,
}

impl LinearInstallation {
    fn install(config: Arc<Config>, ports: &Ports, adapters: &mut Adapters) -> Self {
        let oauth = Arc::new(moatless_adapter_linear::installation::Installation::new(
            config.clone(),
            ports.connection_routing.clone(),
            ports.harness_credentials.clone(),
        ));
        let adapter = Arc::new(
            LinearAdapter::from_db(
                config,
                ports.harness_credentials.clone(),
                ports.adapter_apps.clone(),
            )
            .with_tasks(ports.task_reader.clone())
            .with_ingress_ports(
                ports.connection_routing.clone(),
                ports.adapter_connections.clone(),
                ports.loop_reader.clone(),
                ports.adapter_delivery.clone(),
            ),
        );
        adapter.clone().register_into(adapters);
        tracing::info!("Linear Adapter installed");
        Self {
            adapter,
            oauth,
            active_turns: Arc::new(ActiveLinearTurns::new()),
        }
    }

    fn register_delivery(&self, handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
        let router = Arc::new(
            crate::adapters::providers::linear::delivery::LinearInboundRouter::new(
                state.connection_routing.clone(),
                state.harness_credentials.clone(),
                state.config.clone(),
                state.adapter_apps.clone(),
                state.loop_reader.clone(),
                state.task_bindings.clone(),
                state.task_reader.clone(),
                state.task_creation.clone(),
                state.relation_recorder.clone(),
                state.message_send.clone(),
                state.sandbox_waker.clone(),
                state.task_closer.clone(),
                self.active_turns.clone(),
                state.routing.clone(),
                state.user_directory.clone(),
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
                    state.connection_routing.clone(),
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

impl AdapterInstallations {
    pub(crate) fn github_connector(&self) -> Arc<moatless_loops::loops::firing::listen::Connector> {
        self.github_pr.connector.clone()
    }
    pub(crate) fn linear_installation(
        &self,
    ) -> Arc<moatless_adapter_linear::installation::Installation> {
        self.linear.oauth.clone()
    }
}

/// [`ReviewGuideTasks`] over the review guide tables: the Task that wrote the
/// stored guide and the one writing the current run.
struct GuideTasks(Arc<moatless_tasks::review_guide::operations::ReviewGuides>);

#[async_trait::async_trait]
impl ReviewGuideTasks for GuideTasks {
    async fn guide_tasks(
        &self,
        host: &str,
        repository: &str,
        number: u64,
    ) -> Result<Vec<String>, StoreError> {
        let key =
            moatless_tasks::review_guide::models::PullRequestKey::new(host, repository, number);
        let read = |e| {
            StoreError::with_source(format!("read the review guide of {repository}#{number}"), e)
        };
        let written = self
            .0
            .guide(&key)
            .await
            .map_err(read)?
            .map(|guide| guide.task_id);
        let writing = self
            .0
            .run(&key)
            .await
            .map_err(read)?
            .and_then(|run| run.task_id);
        let writing = writing.filter(|task_id| written.as_ref() != Some(task_id));
        Ok(written.into_iter().chain(writing).collect())
    }
}

/// [`TriggerWorkspaces`] over the Workspace band's provisioning.
struct ProvisionedWorkspaces {
    workspaces: Arc<moatless_workspaces::workspace::Workspaces>,
    repositories: Arc<moatless_workspaces::repository::operations::Repositories>,
    git_sync: Arc<moatless_workspaces::workspace::git_sync::GitSyncService>,
    pool: moatless_core::db::DbPool,
}

#[async_trait::async_trait]
impl TriggerWorkspaces for ProvisionedWorkspaces {
    async fn workspace_for(
        &self,
        request: &TriggerWorkspaceRequest,
    ) -> Result<TriggerWorkspace, StoreError> {
        use moatless_workspaces::workspace::provision::{
            ProvisionError, ProvisionRequest, ProvisionStores, Provisioned, provision_workspace,
        };
        // Only an administrator creates or syncs a Loop, so its creator may
        // register the global repository and Workspace this makes.
        let ctx = moatless_types::access_control::ScopeContext::admin(&request.acting_user_id);
        let outcome = provision_workspace(
            &ProvisionStores {
                workspaces: &self.workspaces,
                repositories: &self.repositories,
                git_sync: &self.git_sync,
                pool: &self.pool,
            },
            &ctx,
            &ProvisionRequest {
                owner: request.owner.clone(),
                repo: request.repo.clone(),
                default_branch: request.default_branch.clone(),
                description: request.description.clone(),
                template: request.template.clone(),
                preferred_workspace_id: Some(request.loop_workspace_id.clone()),
            },
        )
        .await;
        match outcome {
            Ok(Provisioned::Workspace(id)) => Ok(TriggerWorkspace::Found(id)),
            Ok(Provisioned::RepositoryDeleted) => Ok(TriggerWorkspace::Skipped(
                "the repository was deleted".to_string(),
            )),
            Ok(Provisioned::NoTemplate) => Ok(TriggerWorkspace::Skipped(
                "no workspace holds the repository and the loop sets no \
                 provision.workspace_template"
                    .to_string(),
            )),
            Err(error @ ProvisionError::NoWorkspace(_)) => {
                Ok(TriggerWorkspace::Skipped(error.to_string()))
            }
            Err(error) => Err(StoreError::with_source(
                format!(
                    "provision a workspace for {}/{}",
                    request.owner, request.repo
                ),
                error,
            )),
        }
    }
}
