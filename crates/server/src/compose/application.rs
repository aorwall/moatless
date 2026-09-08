//! Assemble application services over the process ports.

use crate::adapter_installations::AdapterInstallations;
use crate::adapters::self_echo::SelfEchoFilter;
use crate::agent_harness_access::{AgentHarnessAccess, ProviderCodexDeviceLogin};
use crate::app_administration::{AppAdministration, ProviderGitHubAppProver};
use crate::compose::Ports;
use crate::sandbox::join::ConnectorSandboxJoiner;
use crate::sandbox::lifecycle::{Lifecycle, NotifyPort};
use crate::sandbox::preflight::{PreflightCredentials, RealGithubValidator, RealPreflight};
use crate::state::AppState;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_identity::git_host_access::{GitHostAccess, ProviderGitHostTokenValidator};
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_loops::loops::sync::LoopSyncService;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_ports::sandbox::{PreflightPort, SandboxJoiner};
use moatless_tasks::credential_sync::CredentialSync;
use moatless_tasks::message::send::MessageSendService;
use moatless_tasks::task::closer::TaskCloser;
use moatless_tasks::task::creation::TaskCreationService;
use moatless_tasks::task::fork::TaskForkService;
use moatless_tasks::task::presenter::TaskPresenter;
use moatless_workspaces::credentials::CredentialService;
use moatless_workspaces::plugin::sync::PluginSyncService;
use moatless_workspaces::skill::fetcher::GitHubSkillFetcher;
use moatless_workspaces::skill::fetcher::SkillFetcher;
use moatless_workspaces::skill::service::SkillService;
use moatless_workspaces::workspace::git_sync::GitSyncService;
use sandbox_proto::client::SandboxConnector;
use std::sync::Arc;

impl AppState {
    /// Build `AppState` over the ports the composition root assembled.
    ///
    /// The services wired here are the ones that need a runtime handle the
    /// composition root does not have — the sandbox connector and the lifecycle
    /// handle — so they are assembled from `ports` rather than in it.
    #[expect(
        clippy::too_many_arguments,
        reason = "eight independent infrastructure handles — ports, config, \
                  lifecycle, connector, sandbox_notify, events, s3, and \
                  adapter_installations — each wired in from the composition \
                  root; bundling them behind a one-use struct only shuffles the \
                  same fields."
    )]
    pub fn new(
        ports: &Ports,
        config: Arc<Config>,
        lifecycle: Arc<Lifecycle>,
        connector: Arc<dyn SandboxConnector>,
        sandbox_notify: Arc<dyn NotifyPort>,
        events: Option<Arc<dyn EventPublisher>>,
        s3: Option<crate::file_storage::s3::S3Storage>,
        adapter_installations: Option<Arc<AdapterInstallations>>,
    ) -> Self {
        let Ports {
            pool,
            tasks,
            task_reader,
            task_relations,
            task_forks,
            task_bindings,
            binding_directory,
            task_events,
            task_turns,
            turn_counter,
            messages,
            transcript,
            feedback,
            repositories,
            repository_reader,
            workspaces,
            workspace_resolver,
            workspace_credentials,
            tags,
            tag_resolver,
            files,
            teams,
            plugins,
            loops,
            adapter_connections,
            event_receipts,
            ingress_outbox,
            adapter_delivery,
            users,
            user_directory,
            provider_tokens,
            harness_credentials,
            adapter_apps,
            provider_token_auth,
            secret_records,
            secrets,
            github_apps,
            github_app_service,
            git_host_tokens,
            token_verifier,
            ..
        } = ports.clone();

        // The concrete issuer is built once in `compose`; every consumer below
        // takes the port.
        let git_host_tokens: Arc<dyn GitHostTokenIssuer> = git_host_tokens;

        let adapters = adapter_installations
            .as_ref()
            .map(|installations| installations.adapters());

        // Shared HTTP client for Codex token refresh — kept warm across calls.
        let codex_http = Arc::new(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        "Failed to build reqwest client for Codex refresh, falling back to default"
                    );
                    reqwest::Client::new()
                }),
        );
        let credentials_svc = Arc::new(
            CredentialService::new(provider_tokens.clone())
                .with_codex_refresh(codex_http.clone())
                .with_secrets(secrets.clone()),
        );
        let repository_branches = Arc::new(
            moatless_workspaces::repository::branches::RepositoryBranchService::new(
                repositories.clone(),
                workspaces.clone(),
                git_host_tokens.clone(),
                provider_tokens.clone(),
                codex_http,
            ),
        );
        // Tasks-band orchestrator (9c): the in-flight guard + sandbox fan-out
        // that reach up out of the workspaces-band credential service.
        let credential_sync = Arc::new(CredentialSync::new(
            credentials_svc.clone(),
            tasks.clone(),
            Arc::new(crate::sandbox::notify::CredentialBroadcastService::new(
                connector.clone(),
            )),
        ));

        let git_sync = Arc::new(GitSyncService::new(
            pool.clone(),
            workspaces.clone(),
            repositories.clone(),
            git_host_tokens.clone(),
        ));

        let loop_sync = Arc::new(LoopSyncService::new(
            adapter_connections.clone(),
            loops.clone(),
            tag_resolver.clone(),
            workspace_resolver.clone(),
            git_host_tokens.clone(),
        ));

        // Hoisted so the self-echo filter and the struct literal share one cache.
        let recently_sent = Arc::new(RecentlySent::new());
        let message_send = Arc::new(MessageSendService::new(
            messages.clone(),
            tasks.clone(),
            task_turns.clone(),
            files.clone(),
            Arc::new(SelfEchoFilter::new(recently_sent.clone(), adapters.clone())),
        ));

        let task_presenter = Arc::new(TaskPresenter::new(
            tags.clone(),
            repository_reader.clone(),
            workspace_resolver.clone(),
            user_directory.clone(),
            binding_directory.clone(),
            task_relations.clone(),
            tasks.clone(),
        ));

        let task_closer = Arc::new(TaskCloser::new(
            tasks.clone(),
            task_relations.clone(),
            config.clone(),
            lifecycle.clone(),
            binding_directory.clone(),
            events.clone(),
        ));

        let skill_http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("moatless-vibe-backend")
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "Failed to build reqwest client for skill fetcher, falling back to default"
                );
                reqwest::Client::new()
            });
        let skill_fetcher: Arc<dyn SkillFetcher> = Arc::new(GitHubSkillFetcher::new(skill_http));
        let plugin_sync = Arc::new(PluginSyncService::new(
            plugins.clone(),
            skill_fetcher.clone(),
            git_host_tokens.clone(),
        ));
        let skills = Arc::new(SkillService::new(
            repositories.clone(),
            workspaces.clone(),
            plugins.clone(),
            git_host_tokens.clone(),
            skill_fetcher,
            plugin_sync.clone(),
            std::time::Duration::from_secs(config.skill.cache_ttl_secs),
        ));

        // Wire GitHub-token preflight once here so every task-creation path
        // (HTTP, loops, adapters) runs the same check. Held on `AppState` as the
        // `PreflightPort` so the task-start handler calls the same port
        // `task_creation` does rather than reassembling the check per request.
        let preflight: Arc<dyn PreflightPort> = Arc::new(RealPreflight::new(
            PreflightCredentials {
                git_host_tokens: git_host_tokens.clone(),
                users: user_directory.clone(),
                workspaces: workspace_resolver.clone(),
                credential_sync: credential_sync.clone(),
                workspace_credentials: workspace_credentials.clone(),
            },
            Arc::new(RealGithubValidator),
        ));

        let sandbox_joiner: Arc<dyn SandboxJoiner> =
            Arc::new(ConnectorSandboxJoiner::new(connector.clone()));
        let task_fork = Arc::new(TaskForkService::new(
            tasks.clone(),
            task_turns.clone(),
            messages.clone(),
            tags.clone(),
        ));
        let task_creation = Arc::new(TaskCreationService::new(
            tasks.clone(),
            tags.clone(),
            task_relations.clone(),
            task_forks.clone(),
            repository_reader.clone(),
            workspace_resolver.clone(),
            events.clone(),
            lifecycle.clone(),
            sandbox_joiner,
            preflight.clone(),
        ));

        // Config-section ports, built once so a secret-bearing section is never
        // re-cloned per request. `config` still holds the whole tree for
        // unmigrated call sites.
        let auth_config = Arc::new(config.auth.clone());
        let codex_oauth_config = Arc::new(config.codex_oauth.clone());
        let secrets_config = Arc::new(config.secrets.clone());
        let s3_config = Arc::new(config.s3.clone());

        let git_host_access = Arc::new(GitHostAccess::new(
            provider_tokens.clone(),
            secrets.clone(),
            Arc::new(ProviderGitHostTokenValidator::default()),
            config.auth.github_app_oauth().is_some(),
        ));
        let agent_harness_access = Arc::new(AgentHarnessAccess::new(
            provider_tokens.clone(),
            credential_sync.clone(),
            codex_oauth_config.clone(),
            Arc::new(ProviderCodexDeviceLogin::default()),
            users.clone(),
        ));
        let app_administration = Arc::new(AppAdministration::new(
            github_apps.clone(),
            users.clone(),
            Arc::new(ProviderGitHubAppProver),
        ));

        // The wake port, implemented in sandbox-control. Built here because it
        // needs the lifecycle handle and the notify port — both of which the
        // composition root already assembled.
        let sandbox_waker: Arc<dyn moatless_ports::sandbox::SandboxWaker> =
            Arc::new(crate::sandbox::notify::SandboxWakeService::new(
                lifecycle.clone(),
                sandbox_notify.clone(),
            ));

        // The inbound-routing subgraph owns the ten ports it reads, so the
        // routing logic in `bindings/runtime.rs` names none of them through
        // `AppState`. Built here from the ports the composition root already
        // assembled; `event_receipts` moves in whole because nothing else on
        // `AppState` reads it.
        let routing = Arc::new(moatless_loops::bindings::runtime::RoutingService::new(
            adapter_connections.clone(),
            loops.clone(),
            task_bindings.clone(),
            transcript.clone(),
            task_reader.clone(),
            task_creation.clone(),
            message_send.clone(),
            event_receipts,
            sandbox_waker.clone(),
            adapters.clone(),
        ));

        // Outbound reply delivery owns the ports its send path reads. The
        // `OutboundSender` is built here and handed to `ExternalReplyService`
        // (the SelfEchoFilter/RoutingService pattern), so neither struct's
        // constructor takes more than the ports it holds and neither names
        // `AppState`.
        let outbound_sender = Arc::new(moatless_loops::bindings::outbound::OutboundSender::new(
            task_bindings.clone(),
            adapter_connections.clone(),
            loops.clone(),
            task_reader.clone(),
            recently_sent.clone(),
            transcript.clone(),
            turn_counter.clone(),
            s3.clone(),
            auth_config.clone(),
            s3_config.clone(),
            files.clone(),
            workspace_resolver.clone(),
            user_directory.clone(),
        ));
        let external_reply = Arc::new(moatless_loops::bindings::reply::ExternalReplyService::new(
            adapters.clone(),
            task_bindings.clone(),
            adapter_connections.clone(),
            transcript.clone(),
            task_reader.clone(),
            loops.clone(),
            recently_sent.clone(),
            turn_counter.clone(),
            outbound_sender,
        ));

        let adapter_commands = Arc::new(crate::adapters::operations::AdapterCommands {
            adapter_connections: adapter_connections.clone(),
            adapters: adapters.clone(),
            task_reader: task_reader.clone(),
            s3: s3.clone(),
            s3_config: s3_config.clone(),
            files: files.clone(),
            external_reply: external_reply.clone(),
            task_bindings: task_bindings.clone(),
            users: users.clone(),
        });

        Self {
            adapter_commands,
            tasks,
            task_reader,
            task_creation,
            task_fork,
            preflight,
            relation_recorder: Arc::new(moatless_tasks::ports::TaskRelations::new(
                task_relations.clone(),
            )),
            task_relations,
            task_bindings,
            binding_directory,
            adapter_connections,
            routing,
            external_reply,
            loops,
            messages,
            transcript,
            feedback,
            repositories,
            repository_branches,
            repository_reader,
            git_host_tokens,
            tags,
            tag_resolver,
            files,
            users,
            user_directory,
            provider_tokens,
            harness_credentials,
            adapter_apps,
            provider_token_auth,
            secret_records,
            secrets,
            teams,
            task_events,
            ingress_outbox,
            task_turns,
            turn_counter,
            plugins,
            plugin_sync,
            workspaces,
            workspace_resolver,
            workspace_credentials,
            git_sync,
            loop_sync,
            message_send,
            task_presenter,
            task_closer,
            skills,
            adapter_delivery,
            db: pool,
            config,
            auth_config,
            codex_oauth_config,
            secrets_config,
            s3_config,
            token_verifier,
            sandbox: lifecycle,
            sandbox_waker,
            events,
            s3,
            adapters,
            adapter_installations,
            github_app_service,
            github_apps,
            credential_sync,
            git_host_access,
            agent_harness_access,
            app_administration,
            recently_sent,
            tool_call_names: Arc::new(
                moka::future::Cache::builder()
                    .max_capacity(50_000)
                    .time_to_live(std::time::Duration::from_mins(30))
                    .build(),
            ),
        }
    }
}

pub(crate) struct ApplicationAssembly {
    pub app_state: AppState,
    pub adapter_installations: Arc<AdapterInstallations>,
    pub sandbox_notify: Arc<dyn crate::sandbox::lifecycle::NotifyPort>,
    pub sandbox_member_registry: Arc<dyn crate::sandbox::lifecycle::MemberRegistryPort>,
}

pub(crate) fn build(
    config: &Config,
    ports: &Ports,
    lifecycle: Option<Arc<Lifecycle>>,
    connector: Arc<dyn SandboxConnector>,
    events: Option<Arc<dyn EventPublisher>>,
    s3: Option<moatless_s3::S3Storage>,
) -> Result<ApplicationAssembly, Box<dyn std::error::Error>> {
    let adapter_installations = Arc::new(AdapterInstallations::install(config, ports));
    // The shared lifecycle was built above; it is present whenever the DB is.
    let lifecycle =
        lifecycle.ok_or("lifecycle handle required to build AppState (missing database pool)")?;
    let sandbox_notify: Arc<dyn crate::sandbox::lifecycle::NotifyPort> = Arc::new(
        crate::sandbox::lifecycle::GrpcNotify::new(connector.clone()),
    );
    let sandbox_member_registry: Arc<dyn crate::sandbox::lifecycle::MemberRegistryPort> = Arc::new(
        crate::sandbox::lifecycle::GrpcMemberRegistry::new(connector.clone()),
    );
    let app_state = AppState::new(
        ports,
        Arc::new(config.clone()),
        lifecycle,
        connector.clone(),
        sandbox_notify.clone(),
        events,
        s3,
        Some(adapter_installations.clone()),
    );
    Ok(ApplicationAssembly {
        app_state,
        adapter_installations,
        sandbox_notify,
        sandbox_member_registry,
    })
}
